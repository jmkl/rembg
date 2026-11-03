// Injected with WebViewBuilder::with_initialization_script_for_main_only.
//
//   processPrompt(base64OrNull, promptText, requestId?)
//   abortPrompt(requestId?)
//
// Results come back through wry's IPC as JSON strings:
//   { type: "ready", url }
//   { type: "prompt_sent",  request_id }
//   { type: "image_copied", request_id, base64, mime_type, width, height }
//   { type: "bridge_error", request_id, error }
(() => {
  if (window.top !== window || window.__gpasteLoaded) return;
  window.__gpasteLoaded = true;

  const STOP_SELECTOR =
    'button[data-testid="stop-button"], button[aria-label="Stop streaming"], button[aria-label="Stop generating"]';
  const SEND_SELECTORS = [
    'button[data-testid="send-button"]',
    'button[aria-label="Send"]',
    'button[aria-label="Send prompt"]',
    'button[aria-label="Send message"]',
    'form button[type="submit"]',
  ];
  const GALLERY_SELECTOR = '[data-testid="generated-image-gallery"]';

  // { id, controller, submitted } for the request in flight, else null.
  let active = null;

  const post = (msg) => window.ipc.postMessage(JSON.stringify(msg));
  const log = (...args) => console.log("[GPasTe]", ...args);
  const visible = (el) => el.getClientRects().length > 0;
  const newId = () => crypto.randomUUID?.() ?? `req-${Date.now().toString(36)}`;

  // Sleep that rejects immediately when the signal aborts.
  const sleep = (ms, signal) =>
    new Promise((resolve, reject) => {
      if (signal?.aborted) return reject(signal.reason);
      const onAbort = () => {
        clearTimeout(timer);
        reject(signal.reason);
      };
      const timer = setTimeout(() => {
        signal?.removeEventListener("abort", onAbort);
        resolve();
      }, ms);
      signal?.addEventListener("abort", onAbort, { once: true });
    });

  // Polls `probe` until it returns something truthy.
  async function waitFor(probe, { timeout, signal, what, interval = 100 }) {
    const start = Date.now();
    for (;;) {
      signal?.throwIfAborted();
      const value = probe();
      if (value) return value;
      if (Date.now() - start >= timeout) {
        throw new Error(`Timed out waiting for ${what}.`);
      }
      await sleep(interval, signal);
    }
  }

  // ----------------------------------------------------------- DOM lookups --

  const findComposer = () =>
    document.querySelector("#prompt-textarea") ||
    document.querySelector('div[contenteditable="true"][data-placeholder]') ||
    document.querySelector('div[contenteditable="true"][role="textbox"]');

  function findSendButton() {
    for (const selector of SEND_SELECTORS) {
      for (const el of document.querySelectorAll(selector)) {
        if (el instanceof HTMLButtonElement && el.isConnected && visible(el)) {
          return el;
        }
      }
    }
    return null;
  }

  function findFileInput() {
    const inputs = [...document.querySelectorAll('input[type="file"]')];
    return (
      inputs.find((i) => /image/i.test(i.accept)) ||
      inputs.find((i) => !i.accept) ||
      inputs[0]
    );
  }

  const countGalleries = () =>
    document.querySelectorAll(GALLERY_SELECTOR).length;

  // Latest finished-looking generated image newer than `baseline` galleries.
  function findGeneratedImage(baseline) {
    const galleries = document.querySelectorAll(GALLERY_SELECTOR);
    if (galleries.length <= baseline) return null;

    const latest = galleries[galleries.length - 1];
    const imgs = [...latest.querySelectorAll("img")].filter(
      (img) =>
        img.complete && img.naturalWidth >= 256 && img.naturalHeight >= 256,
    );
    return imgs[imgs.length - 1] || null;
  }

  // ---------------------------------------------------------------- inputs --

  function insertPrompt(composer, text) {
    composer.focus();

    if (composer instanceof HTMLTextAreaElement) {
      // Bypass React's value tracking by using the native setter.
      Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      ).set.call(composer, text);
      composer.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertText",
          data: text,
        }),
      );
      return;
    }

    // contenteditable (ProseMirror): select all, then use the editor's own pipeline.
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(composer);
    selection.removeAllRanges();
    selection.addRange(range);

    if (!document.execCommand("insertText", false, text)) {
      composer.textContent = text;
      composer.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertText",
          data: text,
        }),
      );
    }
    composer.dispatchEvent(new Event("change", { bubbles: true }));
  }

  // Accepts raw base64 or a full data URL; raw is assumed to be PNG.
  function toFile(input, name) {
    const m = /^data:(image\/[\w+.-]+);base64,/i.exec(input);
    const mime = m ? m[1].toLowerCase() : "image/png";
    const b64 = (m ? input.slice(m[0].length) : input).replace(/\s/g, "");
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const ext = mime.split("/")[1].replace("jpeg", "jpg");
    return new File([bytes], name || `gpaste-image.${ext}`, { type: mime });
  }

  async function attachImage(file, signal) {
    log("Attaching image:", file.name, file.type, file.size);

    let input = await waitFor(findFileInput, {
      timeout: 1500,
      signal,
      what: "file input",
    }).catch((e) => {
      if (signal.aborted) throw e;
    });

    // ChatGPT creates the input lazily: open the attachment menu to force it.
    if (!input) {
      [...document.querySelectorAll("button")]
        .find(
          (b) =>
            visible(b) &&
            /attach|upload|add photos|add files/i.test(
              [
                b.getAttribute("aria-label"),
                b.dataset.testid,
                b.innerText,
              ].join(" "),
            ),
        )
        ?.click();
      await sleep(300, signal);
      input = await waitFor(findFileInput, {
        timeout: 5000,
        signal,
        what: "file input",
      });
    }

    const transfer = new DataTransfer();
    transfer.items.add(file);
    input.files = transfer.files;
    for (const type of ["input", "change"]) {
      input.dispatchEvent(new Event(type, { bubbles: true }));
    }

    // Let ChatGPT process the attachment before we try to submit.
    await sleep(1200, signal);
  }

  // --------------------------------------------------------------- outputs --

  async function waitForImage(
    baseline,
    signal,
    timeoutMs = 5 * 60 * 1000,
    stableMs = 2000,
  ) {
    const start = Date.now();
    let lastSrc = "";
    let changedAt = start;

    while (Date.now() - start < timeoutMs) {
      signal.throwIfAborted();
      const img = findGeneratedImage(baseline);

      if (img) {
        // src can change (blurred preview -> final), so wait until it settles.
        if (img.src !== lastSrc) {
          lastSrc = img.src;
          changedAt = Date.now();
        }
        const generating = Boolean(document.querySelector(STOP_SELECTOR));
        if (!generating && Date.now() - changedAt >= stableMs) return img;
      }
      await sleep(500, signal);
    }
    throw new Error("Timed out waiting for generated image.");
  }

  async function imageToPng(img, signal) {
    const response = await fetch(img.src, { credentials: "include", signal });
    if (!response.ok) throw new Error(`Image fetch failed: ${response.status}`);

    const bitmap = await createImageBitmap(await response.blob());
    signal.throwIfAborted();

    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    canvas.getContext("2d").drawImage(bitmap, 0, 0);
    bitmap.close();

    const dataUrl = canvas.toDataURL("image/png");
    return {
      base64: dataUrl.slice(dataUrl.indexOf(",") + 1),
      mime_type: "image/png",
      width: canvas.width,
      height: canvas.height,
    };
  }

  // ------------------------------------------------------------ public API --

  async function processPrompt(base64, promptText = "", requestId = newId()) {
    const fail = (error) =>
      post({ type: "bridge_error", request_id: requestId, error });

    const text = String(promptText ?? "");
    if (active) return fail("Another send operation is in progress.");
    if (!base64 && !text.trim())
      return fail("Provide text, an image, or both.");

    const me = {
      id: requestId,
      controller: new AbortController(),
      submitted: false,
    };
    const { signal } = me.controller;
    active = me;

    try {
      if (base64) await attachImage(toFile(base64), signal);

      const composer = findComposer();
      if (!composer) throw new Error("ChatGPT composer not found.");
      if (text.trim()) insertPrompt(composer, text);

      const sendButton = await waitFor(
        () => {
          const b = findSendButton();
          return b && !b.disabled ? b : null;
        },
        { timeout: 8000, signal, what: "enabled send button" },
      );
      const baseline = countGalleries();

      signal.throwIfAborted(); // last chance to bail before submitting
      sendButton.click();
      me.submitted = true;
      post({ type: "prompt_sent", request_id: requestId });

      const img = await waitForImage(baseline, signal);
      const png = await imageToPng(img, signal);

      signal.throwIfAborted();
      post({ type: "image_copied", request_id: requestId, ...png });
    } catch (error) {
      if (signal.aborted) {
        log("Request aborted:", requestId);
        return; // the host already knows; stay silent
      }
      console.error("[GPasTe] Failed:", error);
      fail(error?.message || String(error));
    } finally {
      if (active === me) active = null;
    }
  }

  // Aborts the current request (or only `requestId` if given).
  function abortPrompt(requestId) {
    const me = active;
    if (!me || (requestId && me.id !== requestId)) return false;

    active = null; // free the slot right away so a new send isn't rejected
    me.controller.abort();
    // If generation already started, stop it so the tab isn't left busy.
    if (me.submitted) document.querySelector(STOP_SELECTOR)?.click();
    return true;
  }

  window.processPrompt = processPrompt;
  window.abortPrompt = abortPrompt;

  post({ type: "ready", url: location.href });
})();
