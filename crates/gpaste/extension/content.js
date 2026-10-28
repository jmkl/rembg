// The manifest injects this script into ChatGPT tabs, and background.js
// re-injects it when a tab predates the extension (or the extension was just
// reloaded). Re-running it at global scope would redeclare its top-level
// bindings, so everything lives inside this function.
(() => {
  if (globalThis.__gpasteContentLoaded) {
    let alive = false;
    try {
      // After an extension reload the old content script is orphaned: its
      // globals survive, but chrome.runtime calls throw and its message
      // listener no longer receives anything. Only skip when the existing
      // listener is still connected.
      alive = Boolean(chrome.runtime?.id && chrome.runtime.getManifest());
    } catch {
      alive = false;
    }
    if (alive) return;
  }
  globalThis.__gpasteContentLoaded = true;

  console.log("[GPasTe] Image + text sender loaded:", location.href);

  // { id, controller, submitted } for the request being processed, else null.
  let activeRequest = null;

  // Sleep that wakes up immediately (rejecting) when the signal aborts.
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

  function stopGeneration() {
    const stop = document.querySelector(
      'button[data-testid="stop-button"], button[aria-label="Stop streaming"], button[aria-label="Stop generating"]',
    );
    if (stop instanceof HTMLButtonElement) {
      stop.click();
      console.log("[GPasTe] Clicked ChatGPT stop button.");
    }
  }

  function findComposer() {
    return (
      document.querySelector("#prompt-textarea") ||
      document.querySelector('div[contenteditable="true"][data-placeholder]') ||
      document.querySelector('div[contenteditable="true"][role="textbox"]')
    );
  }

  function findSendButton() {
    const selectors = [
      'button[data-testid="send-button"]',
      'button[aria-label="Send"]',
      'button[aria-label="Send prompt"]',
      'button[aria-label="Send message"]',
      'form button[type="submit"]',
    ];

    for (const selector of selectors) {
      for (const button of document.querySelectorAll(selector)) {
        if (
          button instanceof HTMLButtonElement &&
          button.isConnected &&
          button.getClientRects().length > 0
        ) {
          return button;
        }
      }
    }

    return null;
  }

  function insertPrompt(composer, text) {
    composer.focus();

    if (
      composer instanceof HTMLTextAreaElement ||
      composer instanceof HTMLInputElement
    ) {
      const prototype =
        composer instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype;

      const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;

      if (setter) setter.call(composer, text);
      else composer.value = text;

      composer.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertText",
          data: text,
        }),
      );

      return;
    }

    // Use the editor's normal editing mechanism.
    composer.focus();

    const selection = window.getSelection();
    const range = document.createRange();

    range.selectNodeContents(composer);
    selection?.removeAllRanges();
    selection?.addRange(range);

    const inserted = document.execCommand("insertText", false, text);

    if (!inserted) {
      composer.replaceChildren();
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

  function dataUrlToFile(dataUrl, filename) {
    const match = dataUrl.match(
      /^data:(image\/(?:png|jpeg|webp|gif|bmp|avif));base64,([A-Za-z0-9+/=\s]+)$/i,
    );

    if (!match) {
      throw new Error(
        "Invalid image_data_url. Expected a base64 image data URL.",
      );
    }

    const mimeType = match[1].toLowerCase();
    const base64 = match[2].replace(/\s/g, "");
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);

    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }

    const extension = {
      "image/png": "png",
      "image/jpeg": "jpg",
      "image/webp": "webp",
      "image/gif": "gif",
      "image/bmp": "bmp",
      "image/avif": "avif",
    }[mimeType];

    return new File([bytes], filename || `gpaste-image.${extension}`, {
      type: mimeType,
    });
  }

  async function waitForFileInput(timeoutMs = 5000, signal) {
    const started = Date.now();

    while (Date.now() - started < timeoutMs) {
      signal?.throwIfAborted();
      const inputs = [...document.querySelectorAll('input[type="file"]')];

      const imageInput =
        inputs.find((input) => /image/i.test(input.accept || "")) ||
        inputs.find((input) => !input.accept) ||
        inputs[0];

      if (imageInput) return imageInput;

      await sleep(100, signal);
    }

    throw new Error(
      "ChatGPT file input not found. Try opening the attachment menu manually first.",
    );
  }

  async function attachImage(dataUrl, filename, signal) {
    const file = dataUrlToFile(dataUrl, filename);

    console.log("[GPasTe] Attaching image:", {
      name: file.name,
      type: file.type,
      size: file.size,
    });

    let input = await waitForFileInput(1500, signal).catch((error) => {
      if (signal?.aborted) throw error;
      return null;
    });

    // If ChatGPT has not created its file input yet, open the attachment menu.
    if (!input) {
      const attachButton = [...document.querySelectorAll("button")].find(
        (button) => {
          const label = [
            button.getAttribute("aria-label"),
            button.getAttribute("data-testid"),
            button.innerText,
          ]
            .filter(Boolean)
            .join(" ");

          return (
            /attach|upload|add photos|add files/i.test(label) &&
            button.getClientRects().length > 0
          );
        },
      );

      if (attachButton) {
        attachButton.click();
        await sleep(300, signal);
      }

      input = await waitForFileInput(5000, signal);
    }

    const transfer = new DataTransfer();
    transfer.items.add(file);

    input.files = transfer.files;

    input.dispatchEvent(new Event("input", { bubbles: true }));

    input.dispatchEvent(new Event("change", { bubbles: true }));

    console.log("[GPasTe] Image file supplied to ChatGPT.");

    // Give ChatGPT time to process the attachment before submitting.
    await sleep(1200, signal);

    // A preview in the composer is a useful signal that the attachment
    // was accepted. Log the DOM if you need to debug upload issues.
    const previews = document.querySelectorAll(
      '[data-testid*="attachment"], [data-testid*="upload"], img',
    );

    console.log(
      "[GPasTe] Attachment-related elements after upload:",
      previews.length,
    );
  }

  async function waitForEnabledSendButton(timeoutMs = 8000, signal) {
    const started = Date.now();

    while (Date.now() - started < timeoutMs) {
      signal?.throwIfAborted();
      const button = findSendButton();

      if (button && !button.disabled) {
        return button;
      }

      await sleep(100, signal);
    }

    console.error(
      "[GPasTe] Visible buttons:",
      [...document.querySelectorAll("button")]
        .filter((button) => button.getClientRects().length > 0)
        .map((button) => ({
          label: button.getAttribute("aria-label"),
          testId: button.getAttribute("data-testid"),
          text: button.innerText,
          disabled: button.disabled,
        })),
    );

    throw new Error("Send button did not become enabled.");
  }
  const ASSISTANT_SELECTOR = '[data-testid="generated-image-gallery"]';
  function getAssistantImages() {
    return [...document.querySelectorAll(ASSISTANT_SELECTOR)];
  }

  function isGenerating() {
    return Boolean(
      document.querySelector(
        'button[data-testid="stop-button"], button[aria-label="Stop streaming"], button[aria-label="Stop generating"]',
      ),
    );
  }

  function findGeneratedImage(baselineCount) {
    const messages = getAssistantImages();
    if (messages.length <= baselineCount) return null;

    const latest = messages[messages.length - 1];

    const imgs = [...latest.querySelectorAll("img")].filter(
      (img) =>
        img.complete && img.naturalWidth >= 256 && img.naturalHeight >= 256,
    );

    return imgs[imgs.length - 1] || null;
  }

  // Find generated images in the latest turn (ignores icons/avatars by size)
  function findGeneratedImages(baselineCount) {
    const messages = getAssistantImages();
    if (messages.length <= baselineCount) return null;

    const latest = messages[messages.length - 1];
    const turn = latest.closest("article") || latest;

    const imgs = [...turn.querySelectorAll("img")].filter(
      (img) =>
        img.complete && img.naturalWidth >= 256 && img.naturalHeight >= 256,
    );

    return imgs[imgs.length - 1] || null;
  }

  async function waitForImage({
    baselineCount,
    timeoutMs = 5 * 60 * 1000,
    stableMs = 2000,
    signal,
  }) {
    const started = Date.now();
    let lastSrc = "";
    let lastChangeAt = Date.now();

    while (Date.now() - started < timeoutMs) {
      signal?.throwIfAborted();
      const img = findGeneratedImage(baselineCount);

      if (img) {
        // src can change (blurred preview -> final), so wait until it settles
        if (img.src !== lastSrc) {
          lastSrc = img.src;
          lastChangeAt = Date.now();
        }

        if (!isGenerating() && Date.now() - lastChangeAt >= stableMs) {
          return img;
        }
      }

      await sleep(500, signal);
    }

    throw new Error("Timed out waiting for generated image.");
  }

  async function imageToPngBase64(img, signal) {
    const response = await fetch(img.src, { credentials: "include", signal });
    if (!response.ok) throw new Error(`Image fetch failed: ${response.status}`);

    const bitmap = await createImageBitmap(await response.blob());
    signal?.throwIfAborted();

    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    canvas.getContext("2d").drawImage(bitmap, 0, 0);
    bitmap.close();

    const dataUrl = canvas.toDataURL("image/png");

    return {
      dataUrl,
      base64: dataUrl.slice(dataUrl.indexOf(",") + 1),
      mimeType: "image/png",
      width: canvas.width,
      height: canvas.height,
    };
  }

  async function convertToBase64Image(img, signal) {
    const result = await imageToPngBase64(img, signal);

    console.log("[GPasTe] Image converted:", {
      mimeType: result.mimeType,
      width: result.width,
      height: result.height,
      base64Length: result.base64.length,
    });

    return result;
  }
  async function processPrompt(message, active) {
    const requestId = message.request_id;
    const { signal } = active.controller;
    const text = message.text ?? message.prompt ?? "";
    const imageDataUrl = message.image_data_url;

    try {
      if (imageDataUrl) {
        await attachImage(
          imageDataUrl,
          message.image_name || "gpaste-image.png",
          signal,
        );
      }

      const composer = findComposer();
      if (!composer) throw new Error("ChatGPT composer not found.");
      if (text.trim()) insertPrompt(composer, text);

      const sendButton = await waitForEnabledSendButton(8000, signal);
      const baselineCount = getAssistantImages().length;

      // Last chance to bail out before anything is actually submitted.
      signal.throwIfAborted();
      sendButton.click();
      active.submitted = true;
      chrome.runtime.sendMessage({ type: "prompt_sent", request_id: requestId });

      const img = await waitForImage({ baselineCount, signal });
      const info = await convertToBase64Image(img, signal);

      signal.throwIfAborted();
      console.log("[GPasTe] Image converted:", info.width, "x", info.height);
      chrome.runtime.sendMessage({
        type: "image_copied",
        request_id: requestId,
        base64: info.base64,
        mime_type: info.mimeType,
        width: info.width,
        height: info.height,
      });
    } catch (error) {
      if (signal.aborted) {
        console.log("[GPasTe] Request aborted:", requestId);
        return; // the GUI already knows; stay silent
      }
      console.error("[GPasTe] Failed:", error);
      chrome.runtime.sendMessage({
        type: "bridge_error",
        request_id: requestId,
        error: error.message || String(error),
      });
    } finally {
      // Only release the slot if an abort (or newer request) hasn't already done so.
      if (activeRequest === active) activeRequest = null;
    }
  }

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message.type === "abort_prompt") {
      const active = activeRequest;
      if (active && active.id === message.request_id) {
        activeRequest = null; // free the slot right away so a new send isn't rejected
        active.controller.abort();
        // If ChatGPT already started generating, stop it so the tab isn't left busy.
        if (active.submitted) stopGeneration();
        sendResponse({ aborted: true });
      } else {
        sendResponse({ aborted: false });
      }
      return;
    }

    if (message.type !== "send_prompt") return;

    console.log("[GPasTe] Received request:", {
      request_id: message.request_id,
      hasText: Boolean(message.text ?? message.prompt),
      hasImage: Boolean(message.image_data_url),
    });

    if (activeRequest) {
      sendResponse({
        accepted: false,
        error: "Another send operation is in progress.",
      });
      return;
    }

    if (!message.request_id) {
      sendResponse({
        accepted: false,
        error: "Missing request_id.",
      });
      return;
    }

    if (
      !String(message.text ?? message.prompt ?? "").trim() &&
      !message.image_data_url
    ) {
      sendResponse({
        accepted: false,
        error: "Provide text, an image, or both.",
      });
      return;
    }

    const active = {
      id: message.request_id,
      controller: new AbortController(),
      submitted: false,
    };
    activeRequest = active;

    sendResponse({ accepted: true });

    void processPrompt(message, active);
  });
})();
