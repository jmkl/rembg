const { app, core, imaging, action } = require("photoshop");
const { entrypoints } = require("uxp");

let ws = null;
let initialized = false;
let lastFaceRestoreLayerId = null;
let processing = false;
let processTimer = null;

entrypoints.setup({
  panels: {
    quick: {
      show() {
        if (!initialized) {
          initialized = true;
          setTimeout(() => connect(), 300);
          setTimeout(() => {
            const cn = document.getElementById("connectBtn");
            if (cn) cn.addEventListener("click", () => connect());
            const pb = document.getElementById("processBtn");
            if (pb) pb.addEventListener("click", () => handleGetSelection());
            const cb = document.getElementById("copyBtn");
            if (cb) cb.addEventListener("click", () => copyLayerToClipboard());
          }, 400);
        }
      },
      menuItems: [{ id: "connect", label: "Connect" }],
      invokeMenu(id) {
        handleFlyout(id);
      },
    },
  },
});

function connect() {
  if (
    ws &&
    (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)
  ) {
    updateStatus("already connected/connecting");
    return;
  }
  updateStatus("connecting...");
  ws = new WebSocket("ws://localhost:9001");
  ws.onopen = () => {
    updateStatus("connected");
    try {
      ws.send(JSON.stringify({ type: "hello", app: "photoshop" }));
    } catch (e) {}
  };
  ws.onmessage = async (ev) => {
    let msg;
    try {
      msg = JSON.parse(ev.data);
    } catch (e) {
      return;
    }
    if (msg.type === "error") {
      setProcessing(false);
      sendError(msg.message || "Rust error");
    } else if (msg.type === "get_selection") {
      await handleGetSelection();
    } else if (msg.type === "image") {
      await handlePutImage(msg);
    }
  };
  ws.onclose = () => {
    updateStatus("disconnected");
    ws = null;
  };
  ws.onerror = () => {
    updateStatus("connection error");
  };
}

function updateStatus(text) {
  const el = document.getElementById("status");
  if (el) el.textContent = String(text);
}

function setProcessing(on) {
  processing = on;
  const pb = document.getElementById("processBtn");
  const cb = document.getElementById("copyBtn");
  if (pb) pb.disabled = on;
  if (cb) cb.disabled = on;
  if (processTimer) {
    clearTimeout(processTimer);
    processTimer = null;
  }
  if (on) {
    processTimer = setTimeout(() => {
      processing = false;
      if (pb) pb.disabled = false;
      if (cb) cb.disabled = false;
      updateStatus("Timed out waiting for Rust (30s)");
      processTimer = null;
    }, 30000);
  }
}

function uint8ToBase64(bytes) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, Math.min(i + chunkSize, bytes.length));
    binary += String.fromCharCode.apply(null, chunk);
  }
  return btoa(binary);
}

function base64ToArrayBuffer(b64) {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

async function handleGetSelection() {
  try {
    await core.executeAsModal(
      async () => {
        const doc = app.activeDocument;
        if (!doc) {
          sendError("no active document");
          return;
        }
        let sourceBounds = undefined;
        let x = 0,
          y = 0;
        try {
          const b = doc.selection && doc.selection.bounds;
          if (b) {
            sourceBounds = {
              left: b.left,
              top: b.top,
              right: b.right,
              bottom: b.bottom,
            };
            x = b.left;
            y = b.top;
          }
        } catch (e) {}

        if (!sourceBounds) {
          sendError("make a selection first");
          return;
        }
        const result = await imaging.getPixels({
          documentID: doc.id,
          sourceBounds,
        });

        const imageData = result.imageData;
        const width = imageData.width;
        const height = imageData.height;
        const components = imageData.components;
        const colorSpace = imageData.colorSpace;
        const pixels = await imageData.getData();
        const b64 = uint8ToBase64(pixels);

        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(
            JSON.stringify({
              type: "selection",
              width,
              height,
              components,
              colorSpace,
              x,
              y,
              data: b64,
            }),
          );
          updateStatus("Processing...");
          setProcessing(true);
        } else {
          sendError("not connected");
        }
      },
      { commandName: "QuickRestore Get Selection" },
    );
  } catch (e) {
    setProcessing(false);
    sendError(String(e && e.message ? e.message : e));
  }
}

async function handlePutImage(msg) {
  try {
    await core.executeAsModal(
      async () => {
        const doc = app.activeDocument;
        if (!doc) {
          sendError("no active document");
          return;
        }
        if (msg.name === "remove_bg" && lastFaceRestoreLayerId) {
          const rgba = new Uint8Array(base64ToArrayBuffer(msg.data));
          const gray = new Uint8Array(msg.width * msg.height);
          for (let i = 0; i < gray.length; i++) {
            gray[i] = rgba[i * 4 + 3];
          }
          const maskData = await imaging.createImageDataFromBuffer(gray, {
            width: msg.width,
            height: msg.height,
            components: 1,
            colorSpace: "Grayscale",
          });
          try {
            await imaging.putLayerMask({
              documentID: doc.id,
              layerID: lastFaceRestoreLayerId,
              imageData: maskData,
              targetBounds: {
                left: msg.x,
                top: msg.y,
                right: msg.x + msg.width,
                bottom: msg.y + msg.height,
              },
            });
            try {
              maskData.dispose();
            } catch (e) {}
            updateStatus("applied remove_bg as mask");
            setProcessing(false);
          } catch (e) {
            try {
              const _layer = doc.layers.find(
                (l) => l.id === lastFaceRestoreLayerId,
              );
              if (_layer) doc.activeLayers = [_layer];
              await action.batchPlay(
                [
                  {
                    _obj: "make",
                    new: { _class: "channel" },
                    at: { _ref: "channel", _enum: "channel", _value: "mask" },
                    using: { _enum: "userMaskEnabled", _value: "revealAll" },
                  },
                ],
                { synchronousExecution: true },
              );
            } catch (e2) {
              updateStatus(
                "mask create failed: " + (e2 && e2.message ? e2.message : e2),
              );
            }
            await imaging.putLayerMask({
              documentID: doc.id,
              layerID: lastFaceRestoreLayerId,
              imageData: maskData,
              targetBounds: {
                left: msg.x,
                top: msg.y,
                right: msg.x + msg.width,
                bottom: msg.y + msg.height,
              },
            });
            try {
              maskData.dispose();
            } catch (e) {}
            updateStatus("applied remove_bg as mask");
            setProcessing(false);
          }
        } else {
          const buffer = new Uint8Array(base64ToArrayBuffer(msg.data));
          const imageData = await imaging.createImageDataFromBuffer(buffer, {
            width: msg.width,
            height: msg.height,
            components: msg.components,
            colorSpace: msg.colorSpace || "RGB",
          });
          const layer = await doc.createLayer();
          if (msg.name) {
            try {
              layer.name = msg.name;
            } catch (e) {}
          }
          await imaging.putPixels({
            documentID: doc.id,
            layerID: layer.id,
            imageData,
            targetBounds: {
              left: msg.x,
              top: msg.y,
              right: msg.x + msg.width,
              bottom: msg.y + msg.height,
            },
          });
          if (msg.name === "face_restore") lastFaceRestoreLayerId = layer.id;
          try {
            imageData.dispose();
          } catch (e) {}
          updateStatus(`added ${msg.name || "image"}`);
        }
      },
      { commandName: "QuickRestore Put Image" },
    );
  } catch (e) {
    setProcessing(false);
    sendError(String(e && e.message ? e.message : e));
  }
}

async function copyLayerToClipboard() {
  try {
    await core.executeAsModal(
      async () => {
        const doc = app.activeDocument;
        if (!doc) {
          sendError("no active document");
          return;
        }
        // Get composite pixels of the whole document
        const pixels = await imaging.getPixels({ documentID: doc.id });
        const imageData = pixels.imageData;
        const width = imageData.width;
        const height = imageData.height;
        const components = imageData.components;
        const src = await imageData.getData();
        console.log(src.byteLength);
        await navigator.clipboard.write([
          new ClipboardItem({
            [imageBlob.type]: imageData,
          }),
        ]);
        let b64;
        if (components === 4) {
          // keep RGBA
          b64 = uint8ToBase64(src);
        } else if (components === 3) {
          b64 = uint8ToBase64(src);
        } else {
          // grayscale -> replicate
          const dst = new Uint8Array(width * height * 3);
          for (let i = 0; i < width * height; i++) {
            const g = src[i];
            dst[i * 3] = g;
            dst[i * 3 + 1] = g;
            dst[i * 3 + 2] = g;
          }
          b64 = uint8ToBase64(dst);
          // adjust components for rust
        }
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(
            JSON.stringify({
              type: "clipboard",
              width,
              height,
              components,
              data: b64,
            }),
          );
          updateStatus("Sent canvas to Rust for clipboard...");
        } else {
          sendError("not connected");
        }
        try {
          imageData.dispose();
        } catch (e) {}
      },
      { commandName: "QuickRestore Copy Canvas" },
    );
  } catch (e) {
    sendError(String(e && e.message ? e.message : e));
  }
}

function sendError(message) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    try {
      ws.send(JSON.stringify({ type: "error", message }));
    } catch (e) {}
  }
  updateStatus("error: " + message);
}

function handleFlyout(id) {
  switch (id) {
    case "connect": {
      if (ws && ws.readyState === WebSocket.OPEN) {
        updateStatus("already connected");
      } else {
        connect();
      }
      break;
    }
  }
}
