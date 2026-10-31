const app = window.require("photoshop").app;
const { entrypoints } = require("uxp");
let initialized = false;
entrypoints.setup({
  panels: {
    quick: {
      show() {},
      menuItems: [{ id: "connect", label: "Connect" }],
      invokeMenu(id) {
        handleFlyout(id);
      },
    },
  },
});

let ws = null;
let receivedMessages = [];

function connect() {
  if (ws) {
    updateStatus("already connect");
  }
}

function updateStatus(text) {
  document.getElementById("status").textContent = `${text}`;
}

function showLayerNames() {
  const allLayers = app.activeDocument.layers;
  const allLayerNames = allLayers.map((layer) => layer.name);
  const sortedNames = allLayerNames.sort((a, b) =>
    a < b ? -1 : a > b ? 1 : 0,
  );
  updateStatus(sortedNames);
}

function handleFlyout(id) {
  switch (id) {
    case "connect": {
      if (ws) {
        showLayerNames();
      } else {
        connect();
      }
    }
  }
}
