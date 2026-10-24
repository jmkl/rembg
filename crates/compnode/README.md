# CompNode

CompNode is the node-based image processing studio extracted from the old
`rembg studio` binary. It is a separate crate so the web/server side of `rembg`
does not need to depend on egui.

## Run

From the workspace root:

```sh
cargo run -p compnode
```

The window title and product name are **CompNode**. Existing cache and autosave
paths are intentionally unchanged so sessions created by `rembg studio` keep
working:

- `%LOCALAPPDATA%\rembg\studio-cache`
- `%LOCALAPPDATA%\rembg\studio-autosave.rembg.json`
- `%LOCALAPPDATA%\rembg\compnode-settings.json` (new persisted settings)

## Settings

Open **Edit → Settings…** (`Ctrl+,`) to configure:

- **Keep inference models in memory** — when enabled, the first run loads the
  models and later runs reuse them. When disabled, every run loads the needed
  models and releases them afterwards. Turning the option off or changing the
  model directory unloads resident models immediately.
- **Model directory** — folder containing `birefnet.onnx`,
  `GFPGANv1.4.onnx` and `det_10g.onnx`. The default is `./models`.
- **Workspace root** — base folder for projects, exported images, masks and
  future workspace data. It is also the default location for session file
  dialogs.

`Settings` is versionless JSON with serde defaults, so new fields can be added
without breaking existing settings files.

## Processing previews

Remove Background and Face Restore show their processed result once a run
finishes. Before that, their preview mirrors the image connected to their
input. Connecting or disconnecting an input clears the node's processed result,
so it immediately shows the new input or goes back to no preview.

## Inference timing

Remove Background and Face Restore show the last successful run's:

- total worker time
- model load time
- inference time
- whether the model cache was on for that run

Run each node once to load models, then again to see the warm-cache timing.
Disable **Keep inference models in memory** to compare with per-run loading.

## Sessions

Session files are single `.rembg.json` documents. Remove Background and Face
Restore results are **not** copied into a sidecar `*.assets` folder; the JSON
stores each result's absolute path. Legacy sessions that used cache keys or an
old `*.assets` folder still load.

When Save As picks a name that already exists, ` (2)`, ` (3)`, … is appended
before the extension instead of overwriting it. Pressing Save with an existing
session path updates that file in place.

## Image memory

Preview textures are downscaled to a **1024 px longest edge** before upload,
so panning and zooming do not depend on the source image resolution. The
full-resolution bitmap is kept separately for inference.

Decoded full-resolution images are bounded by a **512 MiB** graph-wide budget.
The least recently used images are released first, keeping their preview
textures intact. If a released image is needed for a run, it is decoded again
from its source path or processed-result absolute path. Images without a
reloadable path are never released.

## Module layout

| Module | Responsibility |
| --- | --- |
| `style/` | All styling: `palette.rs` (every colour), `icons.rs` (icon font), `controls.rs` (widget kit on vanilla egui), `mod.rs` (metrics, egui theme, Snarl style) |
| `model.rs` | `Node`, `Preview`, `Process`, viewer layers, node accessors |
| `catalog.rs` | Single registry of node kinds, menu metadata and body functions |
| `nodes/` | One body UI module per node kind |
| `graph.rs` | Connect, invalidate, image loading, preview textures, viewer refresh |
| `memory.rs` | 512 MiB CPU image budget, LRU tracking, lazy reload decisions |
| `editor.rs` | `SnarlViewer` implementation, chevrons, dot grid, node menu |
| `graph_menu.rs` | Filterable right-click “add node” menu |
| `app.rs` | Window, shortcuts, autosave, session actions |
| `session.rs` | Session JSON, output paths, restore, unique filenames |
| `cache.rs` | Content-addressed result cache |
| `worker.rs` | Background inference thread and model residency policy |
| `settings.rs` | Persisted settings, model/workspace path resolution |
| `settings_window.rs` | Settings window UI |
| `title_bar.rs` | Custom undecorated title bar and menus |
| `widgets.rs` | Reusable preview, socket and error widgets |

## Selection

`egui-snarl` 0.12 only updates its selection on modified clicks and rectangle
selection, not on a normal click. `GraphEditor` therefore keeps the app-level
selection used by Delete/Edit, draws the accent outline itself, and mirrors
Snarl's rectangle selection. Because Snarl renders nodes on a transformed
layer without an `Area`, normal clicks are hit-tested by transforming the node
rect into screen space instead of using `Ui::rect_contains_pointer`. Loading or
clearing a graph gets a fresh widget ID so old pan/zoom and selection do not
leak into the new file.

## Adding a node

1. Add a `Kind` variant and a `Node` variant in `model.rs`.
2. Add the node’s data/accessors in `model.rs` (reuse `Preview`/`Process` when it
   is an inference node).
3. Add `src/nodes/<name>.rs` with `create` and `show_body` functions.
4. Register it in `catalog.rs` (`NODE_SPECS`). The graph menu, title/header
   accent, socket counts, body width and body dispatch all come from this entry.
5. If it is persistent, add a `SessionNodeData` case and snapshot/load mapping
   in `session.rs`.
6. If it runs inference, add a `Job` variant in `worker.rs` and a starter method
   in `editor.rs`.

No central `match` needs to change for the graph menu or the node body: both
are driven by `NODE_SPECS`.

## Styling

Change colours in `style/palette.rs`, and radii, typography, spacing and the egui theme in `style/mod.rs`. Widgets live in `style/controls.rs`. Node header
colours live in `pal::HEADER_*` and are assigned per node in `catalog.rs`.
Body padding is `style::BODY_MARGIN`; default widths are `BODY_W` and
`VIEWER_W`.

## Checks

```sh
cargo fmt -p compnode
cargo clippy -p compnode --all-targets -- -D warnings
cargo test -p compnode
```
