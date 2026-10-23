const HOST_NAME = "hello.dcsms.gpaste";

let nativePort = null;
let connected = false;
const pendingRequests = new Map();
const incomingImageChunks = new Map();

function connectNative() {
  if (nativePort && connected) return true;

  try {
    console.log("[GPasTe] Connecting to native host:", HOST_NAME);
    const port = chrome.runtime.connectNative(HOST_NAME);
    nativePort = port;
    connected = true;

    port.onMessage.addListener((message) => {
      console.log("[GPasTe] Message from Rust:", message);

      if (message.type === "send_prompt") {
        void routePrompt(message).catch((error) =>
          reportRoutingError(message, error),
        );
        return;
      }

      if (message.type === "send_prompt_start") {
        incomingImageChunks.set(message.request_id, {
          request_id: message.request_id,
          text: message.text ?? "",
          image_name: message.image_name,
          prefix: message.image_data_url_prefix,
          chunks: [],
        });
        return;
      }

      if (message.type === "send_prompt_chunk") {
        const pending = incomingImageChunks.get(message.request_id);
        if (
          !pending ||
          !Number.isInteger(message.index) ||
          typeof message.data !== "string"
        ) {
          console.warn("[GPasTe] Ignoring unexpected image chunk:", message);
          return;
        }
        pending.chunks[message.index] = message.data;
        return;
      }

      if (message.type === "send_prompt_end") {
        const pending = incomingImageChunks.get(message.request_id);
        incomingImageChunks.delete(message.request_id);

        const expectedChunks = message.chunk_count;
        const chunksComplete =
          pending &&
          Number.isInteger(expectedChunks) &&
          pending.chunks.length === expectedChunks &&
          Array.from(
            { length: expectedChunks },
            (_, index) => typeof pending.chunks[index] === "string",
          ).every(Boolean);

        if (!chunksComplete) {
          reportRoutingError(
            message,
            new Error("Image transfer was incomplete."),
          );
          return;
        }

        const completeMessage = {
          type: "send_prompt",
          request_id: pending.request_id,
          text: pending.text,
          image_name: pending.image_name,
          image_data_url: pending.prefix + pending.chunks.join(""),
        };
        void routePrompt(completeMessage).catch((error) =>
          reportRoutingError(completeMessage, error),
        );
        return;
      }

      if (message.type === "bridge_status") {
        console.log("[GPasTe] Native host status:", message);
      }
    });

    port.onDisconnect.addListener(() => {
      const error = chrome.runtime.lastError?.message;
      console.error(
        "[GPasTe] Native host disconnected:",
        error || "No error details",
      );
      if (nativePort === port) {
        nativePort = null;
        connected = false;
      }
    });

    return true;
  } catch (error) {
    nativePort = null;
    connected = false;
    console.error("[GPasTe] Native connection failed:", error);
    return false;
  }
}

function sendNative(message) {
  if (!nativePort || !connected) {
    console.error(
      "[GPasTe] Cannot send to Rust: native host is disconnected.",
      message,
    );
    return false;
  }
  try {
    nativePort.postMessage(message);
    console.log("[GPasTe] Sent to Rust:", message);
    return true;
  } catch (error) {
    console.error("[GPasTe] Sending to Rust failed:", error);
    return false;
  }
}

function reportRoutingError(message, error) {
  console.error("[GPasTe] Routing failed:", error);
  sendNative({
    type: "bridge_error",
    request_id: message.request_id ?? null,
    error: error.message || String(error),
  });
}

async function routePrompt(message) {
  const requestId = message.request_id;
  const text = message.text ?? message.prompt ?? "";
  const imageDataUrl = message.image_data_url;

  if (!requestId) throw new Error("Missing request_id in the Rust message.");
  if (typeof text !== "string")
    throw new Error("Prompt text must be a string.");
  if (!text.trim() && !imageDataUrl)
    throw new Error("Provide prompt text, an image, or both.");
  if (pendingRequests.has(requestId))
    throw new Error(`Duplicate request_id: ${requestId}`);

  // const tabs = await chrome.tabs.query({
  //   active: true,
  //   lastFocusedWindow: true,
  // });
  //  // const tab = tabs.find(
  //   (item) =>
  //     item.id != null &&
  //     (item.url?.startsWith("https://chatgpt.com/") ||
  //       item.url?.startsWith("https://chat.openai.com/")),
  // );

  const tabs = await chrome.tabs.query({
    url: ["https://chatgpt.com/*", "https://chat.openai.com/*"],
  });

  const tab = tabs.find((item) => item.id != null);

  if (tab?.id == null) {
    throw new Error("No ChatGPT tab found. Open ChatGPT first.");
  }

  pendingRequests.set(requestId, { tabId: tab.id, startedAt: Date.now() });
  try {
    const response = await chrome.tabs.sendMessage(tab.id, {
      type: "send_prompt",
      request_id: requestId,
      text,
      image_data_url: imageDataUrl,
      image_name: message.image_name,
    });

    console.log("[GPasTe] Content script acknowledgement:", response);
    if (!response?.accepted) {
      throw new Error(
        response?.error || "Content script did not accept the request.",
      );
    }
  } catch (error) {
    pendingRequests.delete(requestId);
    throw new Error(
      `Could not send request to content.js. Reload the ChatGPT tab and try again. Details: ${error.message}`,
    );
  }
}

chrome.runtime.onMessage.addListener((message, sender) => {
  const supportedTypes = ["prompt_sent", "assistant_response", "bridge_error"];
  if (message.type === "image_copied") {
    const pending = pendingRequests.get(message.request_id);

    if (!pending || pending.tabId !== sender.tab?.id) {
      console.warn("[GPasTe] No matching image request:", message.request_id);
      return;
    }

    sendNative(message);
    return;
  }
  if (!supportedTypes.includes(message.type)) return;

  if (sender.tab?.id == null) {
    console.warn("[GPasTe] Ignoring message without a tab:", message);
    return;
  }

  const requestId = message.request_id;
  const pending = pendingRequests.get(requestId);

  if (!pending || pending.tabId !== sender.tab.id) {
    console.warn("[GPasTe] No matching pending request:", message);
    return;
  }

  if (message.type === "prompt_sent") {
    // The prompt was submitted; ChatGPT may still be generating.
    pending.submittedAt = Date.now();

    console.log("[GPasTe] Prompt submitted:", requestId);
    sendNative(message);
    return;
  }

  if (message.type === "assistant_response") {
    // ChatGPT finished responding.
    pendingRequests.delete(requestId);

    console.log("[GPasTe] Assistant response received:", {
      request_id: requestId,
      length: message.text?.length ?? 0,
    });

    sendNative(message);
    return;
  }

  if (message.type === "bridge_error") {
    pendingRequests.delete(requestId);

    console.error("[GPasTe] Request failed:", message);
    sendNative(message);
  }
});

connectNative();
console.log("[GPasTe] Background service worker started.");
