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
      return;
    }
    switch (msg.type) {
      case "get_selection":
        await handleGetSelection();
        break;
      case "selection_tool":
        await handleTool("selection");
        break;
      case "image":
        await handlePutImage(msg);
        break;
      case "copy":
        await copyLayerToClipboard();
        break;
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

async function handleTool(tool) {
  switch (tool) {
    case "selection":
      break;
  }

  await core.executeAsModal(
    async () => {
      const doc = app.activeDocument;
      if (!doc) {
        sendError("no active document");
        return;
      }
      await action.batchPlay(
        [
          {
            _obj: "select",
            _target: [
              {
                _ref: "marqueeRectTool",
              },
            ],
          },
        ],
        {},
      );
      try {
        await doc.selection.deselect();
      } catch (e) {}
      updateStatus("tool selected");
    },
    { commandName: "Tool Selection" },
  );
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

function sendToServer(data) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(data);
    setProcessing(true);
  } else {
    sendError("not connected");
  }
}

function sendStatus(text) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    try {
      ws.send(JSON.stringify({ type: "status", message: text }));
    } catch (e) {}
  }
  updateStatus(text);
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
        if (msg.path) {
          // Place as a linked Smart Object instead of pasting bitmap pixels.
          const uxp = require("uxp");
          const fs = uxp.storage.localFileSystem;
          const norm = msg.path.replace(/\\\\/g, "/");
          let fileEntry = null;
          try {
            fileEntry = await fs.getEntryForPath(norm);
          } catch (e) {
            // try next
          }
          if (!fileEntry) {
            try {
              fileEntry = await fs.getEntryForPath(msg.path);
            } catch (e) {}
          }
          if (!fileEntry) {
            try {
              fileEntry = await fs.getEntryWithUrl("file:///" + norm);
            } catch (e) {}
          }
          if (!fileEntry) {
            try {
              fileEntry = await fs.getEntryWithUrl("file://" + norm);
            } catch (e) {}
          }
          if (!fileEntry) {
            sendError("cannot access file: " + msg.path);
            setProcessing(false);
            return;
          }
          let token = null;
          try {
            token = fs.createSessionToken(fileEntry);
          } catch (e) {
            sendError("token: " + (e && e.message ? e.message : e));
            setProcessing(false);
            return;
          }
          const cx = msg.x + msg.width / 2;
          const cy = msg.y + msg.height / 2;
          const docCx = doc.width / 2;
          const docCy = doc.height / 2;
          try {
            await action.batchPlay(
              [
                {
                  _obj: "placeEvent",
                  null: { _path: token, _kind: "local" },
                  freeTransformCenterState: {
                    _enum: "freeTransformCenterState",
                    _value: "QCSAverage",
                  },
                  offset: {
                    _obj: "offset",
                    horizontal: {
                      _unit: "pixelsUnit",
                      _value: cx - docCx,
                    },
                    vertical: {
                      _unit: "pixelsUnit",
                      _value: cy - docCy,
                    },
                  },
                },
              ],
              { synchronousExecution: true },
            );
          } catch (e) {
            sendError("placeEvent: " + (e && e.message ? e.message : e));
            setProcessing(false);
            return;
          }
          // Convert the placed (embedded) layer into a linked Smart Object.
          try {
            await action.batchPlay(
              [
                {
                  _obj: "placedLayerConvertToLinked",
                  _target: [
                    { _ref: "layer", _enum: "ordinal", _value: "targetEnum" },
                  ],
                  using: { _path: token, _kind: "local" },
                },
              ],
              { synchronousExecution: true },
            );
          } catch (e) {
            // Non-fatal: layer still has pixels from placeEvent.
          }
          try {
            if (doc.activeLayers && doc.activeLayers[0]) {
              const layer = doc.activeLayers[0];
              layer.name = msg.name || "face_restore";
              const constants = require("photoshop").constants;
              try {
                const b = layer.bounds;
                const cw = b.right - b.left;
                const ch = b.bottom - b.top;
                if (cw > 0 && ch > 0) {
                  const sx = (msg.width / cw) * 100;
                  const sy = (msg.height / ch) * 100;
                  if (Math.abs(sx - 100) > 0.01 || Math.abs(sy - 100) > 0.01) {
                    await layer.scale(sx, sy, constants.AnchorPosition.TOPLEFT);
                  }
                }
              } catch (e) {}
              try {
                const b2 = layer.bounds;
                await layer.translate(msg.x - b2.left, msg.y - b2.top);
              } catch (e) {}
              if (msg.name === "face_restore")
                lastFaceRestoreLayerId = layer.id;
            }
          } catch (e) {}
          try {
            fileEntry = null;
          } catch (e) {}

          setProcessing(false);
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
            updateStatus("mask");
            sendStatus("mask");
            // Resize the restored layer back to the original selection size.
            try {
              const _layer = doc.layers.find(
                (l) => l.id === lastFaceRestoreLayerId,
              );
              if (_layer) {
                const constants = require("photoshop").constants;
                const ow = msg.origWidth || Math.round(msg.width / 2);
                const oh = msg.origHeight || Math.round(msg.height / 2);
                const b = _layer.bounds;
                const cw = b.right - b.left;
                const ch = b.bottom - b.top;
                if (cw > 0 && ch > 0) {
                  const sx = (ow / cw) * 100;
                  const sy = (oh / ch) * 100;
                  if (Math.abs(sx - 100) > 0.01 || Math.abs(sy - 100) > 0.01) {
                    await _layer.scale(
                      sx,
                      sy,
                      constants.AnchorPosition.TOPLEFT,
                    );
                  }
                }
                const b2 = _layer.bounds;
                await _layer.translate(msg.x - b2.left, msg.y - b2.top);
              }
            } catch (e) {}
            updateStatus("scale to original");
            sendStatus("scale to original");
            sendToServer(JSON.stringify({ type: "done", message: "DONE" }));
            updateStatus("DONE");
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
            updateStatus("mask");
            sendStatus("mask");
            // Resize the restored layer back to the original selection size.
            try {
              const _layer = doc.layers.find(
                (l) => l.id === lastFaceRestoreLayerId,
              );
              if (_layer) {
                const constants = require("photoshop").constants;
                const ow = msg.origWidth || Math.round(msg.width / 2);
                const oh = msg.origHeight || Math.round(msg.height / 2);
                const b = _layer.bounds;
                const cw = b.right - b.left;
                const ch = b.bottom - b.top;
                if (cw > 0 && ch > 0) {
                  const sx = (ow / cw) * 100;
                  const sy = (oh / ch) * 100;
                  if (Math.abs(sx - 100) > 0.01 || Math.abs(sy - 100) > 0.01) {
                    await _layer.scale(
                      sx,
                      sy,
                      constants.AnchorPosition.TOPLEFT,
                    );
                  }
                }
                const b2 = _layer.bounds;
                await _layer.translate(msg.x - b2.left, msg.y - b2.top);
              }
            } catch (e) {}
            updateStatus("scale to original");
            sendStatus("scale to original");
            sendToServer(JSON.stringify({ type: "done", message: "DONE" }));
            updateStatus("DONE");
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
        // Select the entire canvas
        await doc.selection.selectAll();
        // Copy Merged: copies all visible layers flattened, like Edit > Copy Merged
        await action.batchPlay([{ _obj: "copyMerged" }], {});
        // Restore previous state
        try {
          await doc.selection.deselect();
        } catch (e) {}
        updateStatus("canvas copied to clipboard");
      },
      { commandName: "Copy canvas to clipboard" },
    );
  } catch (e) {
    sendError(String(e && e.message ? e.message : e));
  }
}

async function copyPixelsToClipboard(colorArrayView, width, height) {
  await core.executeAsModal(
    async () => {
      // 1. Temp document sized to the image
      const doc = await app.documents.add({
        width,
        height,
        resolution: 72,
        mode: "RGBColorMode",
        fill: "transparent",
      });

      // 2. Build ImageData from your raw RGBA(or RGB) buffer
      const imageData = await imaging.createImageDataFromBuffer(
        colorArrayView,
        {
          width,
          height,
          components: 4, // 3 if no alpha
          colorSpace: "RGB",
          colorProfile: "sRGB IEC61966-2.1",
          chunky: true, // interleaved RGBARGBA...; false if planar
        },
      );

      // 3. Write to a layer
      const layer = await doc.createLayer({ name: "clip" });
      await imaging.putPixels({
        layerID: layer.id,
        imageData,
        replace: true,
      });
      imageData.dispose();

      // 4. Select all + copy
      await action.batchPlay(
        [
          {
            _obj: "set",
            _target: [{ _ref: "channel", _property: "selection" }],
            to: { _enum: "ordinal", _value: "allEnum" },
          },
          { _obj: "copy" },
        ],
        {},
      );

      // 5. Clean up
      await doc.closeWithoutSaving();
    },
    { commandName: "Copy image to clipboard" },
  );
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
