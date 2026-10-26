# CompNode frontend parity

CompNode is the native egui version of the node studio. The reference UI is the
Svelte app in `frontend/src` (`App.svelte`, `core/nodes.ts`, `NodeView.svelte`,
`EditMode.svelte`, `core/graph.svelte.ts`, `core/session.ts`,
`core/workspace.ts`).

Legend:

- `[x]` present in CompNode
- `[ ]` missing from CompNode
- **Deferred** intentionally postponed

## Current status

- Native node kinds: **Image**, **Remove Background**, **Face Restore**,
  **Viewer**.
- Directly represented frontend node types: **3 of 15** (`image`,
  `facerestore`, `viewer`). **Remove Background** is an extra native node with
  different semantics.
- UI modes: node graph only. Frontend has node, side-by-side split, stacked
  split, and edit canvas.
- Image input: Image nodes accept one upstream input and pass it through until
  transforms are implemented. Selected assets are content-addressed and can be
  embedded in sessions as blobs.
- Persistence: local JSON session plus autosave. Processed result paths are
  absolute and there are no sidecar asset copies. Templates, projects,
  thumbnails, and project/template identity are still missing.
- Model residency: configurable in Settings (keep models resident between
  runs, or load and release them per job).
- Preview: processing nodes show their connected input until a result exists;
  preview textures are capped at 1024 px.
- Output: no composited export yet.

## Implemented in CompNode

### Application shell

- [x] Custom undecorated title bar with **CompNode** branding and
      File / Edit / View / Help menus.
- [x] Native window controls: minimize, maximize/restore, close.
- [x] Fullscreen shortcut (`F11`) and title-bar status area.
- [x] Status text for the current session, autosaving, inference count, and
      errors.
- [x] Timed notices/toasts for save, load, autosave failure, and help hints;
      equivalent to the frontend `graph.toast`.
- [x] Session shortcuts: `Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+Shift+S`.
- [x] Delete selected nodes with `Delete` or `Backspace` when no text field has
      focus, plus Edit → Delete selected and the node context menu.
- [x] Settings window (`Edit → Settings…`, `Ctrl+,`):
  - model-memory caching toggle
  - model directory
  - workspace root
  - JSON persistence in `%LOCALAPPDATA%\rembg\compnode-settings.json`
- [x] Applying settings reconfigures the inference worker; disabling model
      caching or changing the model directory unloads resident models.
- [x] Workspace root is created on startup/apply and used as the default session
      dialog directory.
- [x] Window close writes one final autosave.

### Graph canvas

- [x] Node canvas with pan, zoom, selection, and a Blender-style dot grid.
      Normal click selects a node; Shift/Ctrl-click extend or toggle the selection.
      Selection is tracked by `GraphEditor` because egui-snarl 0.12 does not select
      on an unmodified click.
- [x] Node header accents per node kind and rounded node frames.
- [x] Chevron collapse/expand in every node header.
- [x] Node header titles are not selectable.
- [x] Right-click canvas opens an add-node menu at the cursor.
- [x] Add-node menu filters by node title and keywords, shows node icons, and
      reports “No matching nodes”.
- [x] Filter text entry does not close the menu; clicking outside or pressing
      Escape does.
- [x] Newly created nodes are inserted at the right-click position.
- [x] Node right-click menu with Delete node.
- [x] Single-input replacement: connecting a new wire replaces the existing
      input wire.
- [x] Viewer accepts multiple input wires and renders them as layers.
- [x] Viewer layer order follows connection order and is restored from saved
      sessions.
- [x] Downstream outputs are invalidated when an upstream connection or result
      changes.
- [x] Remove Background and Face Restore preview their connected input until a
      processed result exists. Connecting/disconnecting clears stale processed
      output, and deleting a source also invalidates its consumers.
- [x] Node positions and collapsed state are persisted.

### Nodes and inference

- [x] **Image** node:
  - local file picker
  - preview
  - file name
  - reload from disk
  - error display
  - saved source path
- [x] **Remove Background** node:
  - dedicated node with one image input
  - BiRefNet inference on a background thread
  - RGBA result preview
  - run/re-run state
  - error display
  - content-addressed disk cache
  - cached result restored with sessions
- [x] **Face Restore** node:
  - one image input
  - strength slider
  - “Center face only” option
  - GFPGAN + face detection on a background thread
  - result preview
  - run/re-run state
  - error display
  - content-addressed disk cache
  - cached result restored with sessions
- [x] **Viewer** node:
  - multi-layer input
  - checkerboard preview
  - bottom-to-top layer painting
  - layer count
- [x] Implemented non-Image nodes show an output thumbnail even when not
      connected to a Viewer: processed results for inference nodes, layer previews
      for Viewer, and connected-input fallback before processing.
- [x] Long-lived inference worker. Model residency follows the Settings cache
      toggle: models stay resident between runs, or load and release per job. The
      UI thread never blocks on inference.
- [x] Stale worker results are ignored when a node was deleted, re-run, or had
      its input wire changed.
- [x] Processing errors are shown in the node body with a hover tooltip.
- [x] Content-addressed result cache with a fallback key for outputs that could
      not be written by the worker.
- [x] Remove Background and Face Restore show a last-run benchmark: total
      worker time, model load time, inference time, and model-cache state.
- [x] Full-resolution filter output. Filters still eval at a small cap for the
      node preview, but a debounced background render re-evaluates them at
      full resolution from disk-backed inputs, persists the result in the
      output cache, and records it as the node's output path. The sidebar
      canvas and downstream nodes then use the full-resolution file; chained
      filters render in upstream order; disconnected filters are skipped.
- [x] Preview textures are downscaled to a 1024 px longest edge before upload;
      full-resolution pixels are reserved for inference.
- [x] Full-resolution CPU images share a 512 MiB least-recently-used budget.
      Released images keep their preview texture and are decoded again from their
      source or processed-result path when a run needs them.

### Persistence

- [x] Session JSON version 1 with nodes, links, positions, collapsed state,
      parameters, processed-output absolute paths, and Viewer layer order. Legacy
      cache keys remain loadable.
- [x] New / Open / Save / Save As.
- [x] Autosave with a short debounce and startup recovery.
- [x] Autosave is separate from the explicit session path, so recovery never
      silently overwrites the user's document.
- [x] Save writes only the JSON project file; processed results are referenced
      by absolute path instead of being copied into `*.assets`.
- [x] Save As auto-increments an existing filename (`name (2).rembg.json`)
      instead of overwriting it. Save updates the current path in place.
- [x] `Ctrl+S` saves in place when a session path exists, otherwise prompts for
      a name; `Ctrl+Shift+S` saves a copy.
- [x] Result files remain in the shared content cache and are referenced by
      absolute path; no files are copied beside the project.
- [x] Old `rembg studio` cache and autosave paths are preserved for
      compatibility:
  - `%LOCALAPPDATA%\rembg\studio-cache`
  - `%LOCALAPPDATA%\rembg\studio-autosave.rembg.json`
- [x] Old studio session schema remains loadable, including legacy `cache_key`
      results and old `*.assets` folders.

### Maintainability and styling

- [x] One shared `NodeSpec` registry for menu title, icon, keywords, accent,
      socket counts, body width, creation, and body rendering.
- [x] One module per node under `src/nodes/`.
- [x] Shared `Preview`, `Process`, preview widgets, error line, and socket
      widget.
- [x] Centralized colours, spacing, typography, node widths, and Snarl styling
      in `style.rs`.
- [x] Dedicated `memory.rs` for the CPU image budget and LRU tracking; preview
      downscaling lives in graph helpers.
- [x] Full-width node body controls with clean body padding.
- [x] Tests for session round-trip, legacy cache-key parsing, graph behavior,
      connected-input preview, preview downscaling, CPU image eviction, filename
      increments, settings defaults/round-trip, and catalog uniqueness.

## Missing core structures

These are the model-level pieces the frontend has and CompNode still needs.
Most of the UI work below depends on them.

- [ ] Generic field model: `FieldSpec` / `FieldKind` for
      `range`, `number`, `color`, and `select`, with min/max/step/options.
      Done when a node can declare fields in the catalog and one generic body
      renderer draws them.
- [ ] Socket model with named inputs, per-socket `multi` flag, output count,
      `single_output`, and duplicate-wire prevention. Current `NodeSpec` only has
      input/output counts and one `multi_input` flag.
- [ ] Shared node parameters: `visible`, `opacity`, `x`, `y`, `scale`,
      `rotation`, and a `transform` flag. Done when every node has these defaults
      and the evaluator applies visibility/opacity.
- [ ] Generic evaluation/cache layer for CPU effects. Current caching is for
      inference results, not a graph evaluator.
- [x] Asset store keyed by asset id, with name, image dimensions, and either a
      path or blob. Current Image node stores only a source path.
- [ ] Pixel-identity-keyed mask cache and face-restore cache with staleness
      detection. Jobs compute content keys for result storage, but results are not
      looked up by key or shared across nodes.
- [ ] Viewer dimensions (`width`, `height`) and a real composition root.
      Current Viewer is only a fixed-size preview.
- [ ] `UiMode` state for node / split / split-horizontal / edit.
- [ ] Workspace structures: `WorkspaceMeta`, `WorkspaceManifest`,
      `WorkspaceKind`, thumbnails, and workspace assets/masks/restores.
- [ ] `SessionIdentity` for the current project/template, plus full session
      assets, masks, and restore results.
- [ ] Edit-mode layer model: nested layer tree, per-layer visibility, opacity,
      name, and ordering.
- [ ] **Deferred:** per-layer transform box, move/scale handles, numeric
      x/y/scale/rotation controls, and group transforms.

## Missing features

### Node library

- [x] **Route** — headerless pass-through image, one input, one output, only one
      outgoing link.
- [x] **Blend** — `base` + `blend` inputs, 16 blend modes, opacity.
- [x] **Mask** — `image` + `mask` inputs, dark/light-is-opaque mode, strength.
- [x] **Group** — multi-input stack; stored as `mix` for saved-graph
      compatibility.
- [x] **Hue / Saturation** — hue, saturation, brightness.
- [x] **Curves** — RGB/R/G/B channel curves, reset, add/drag/right-click
      remove points, monotone interpolation and LUT evaluation.
- [x] **Exposure** — EV, offset, gamma.
- [x] **Color Balance** — shadows/midtones/highlights, cyan/red,
      magenta/green, yellow/blue, preserve luminosity.
- [x] **Gradient Map** — shadow/mid/highlight colours, midpoint, reverse.
- [x] **Invert** — amount.
- [x] **Gradient** — linear/radial/solid, two colours, angle, colour
      positions, output width/height.
- [x] **Noise Texture** — fBM/multifractal/ridged, scale, detail, roughness,
      lacunarity, distortion, seed.

Done when every frontend node type appears in the add menu, has its expected
parameters, previews a result, and can be saved/restored.

### Graph editing and shortcuts

- [ ] Reject connections that would create a dependency cycle.
- [ ] `Shift+D` duplicate the selected node, including its parameters and
      generated mask where applicable.
- [ ] Prevent duplicate source → target wires on multi-input sockets.
- [ ] Enforce Route's single outgoing wire.
- [ ] `Alt+click` an edge to delete it.
- [ ] `Ctrl+drag` across wires to cut all crossed edges.
- [ ] Open the add-node menu from node and edge right-clicks as well as empty
      canvas.
- [ ] Add-node menu keyboard behavior: auto-focus the filter, arrow/Home/End
      navigation, `Enter` creates the first match, `Escape`/`Backspace` closes.
- [ ] Add-node filter also matches the stored node type string.
- [ ] `Shift+A` opens the add menu at the cursor.
- [ ] `Shift+E` toggles node/edit mode.
- [ ] Select a newly created node automatically.
- [ ] Keep exactly one Viewer, non-deletable and pinned. Create one for a new
      graph and use a fallback Viewer when loading a graph without one.
- [ ] Fit-view control and automatic refit when the canvas changes size or the
      UI mode changes.

### Image input and assets

Current: implemented in the native crate. Drag/drop, clipboard paste, URL fetching,
and replacement all store the image in the backend `input/` folder and create
content-addressed assets.

- [x] Drag and drop images onto the canvas.
- [x] Drag and drop an image onto an Image node to replace its source.
- [x] Paste an image from the clipboard.
- [x] Paste or drop an image URL and fetch it.
- [x] Image that drop/paste/download will copied into `input\` folder for further use.
- [x] Replace an Image node's asset without creating a new node when pasting or dropping into selected Image Node.

### Remove Background parity

The frontend treats background removal as an **Image node action** that creates
a reusable mask. CompNode currently treats it as a separate inference node that
produces a new RGBA image.

- [ ] Generate a mask from the Image node rather than requiring a separate
      Remove Background node.
- [ ] Add a `useMask` toggle on the Image node.
- [ ] Apply the mask with `destination-in` so RGB and alpha are preserved.
- [ ] Key the mask to the exact source pixels; mark it stale when the image or
      upstream input changes.
- [ ] Reuse one mask across downstream consumers without re-running inference.
- [ ] Decide how to migrate the existing Remove Background node/session data
      if the frontend semantics become canonical.

### Face Restore parity

Current: the preview mirrors the upstream input, but the node does not pass the
input through as its output when no result exists.

- [ ] Pass the input image through when there is no result or the result is
      stale.
- [ ] Show Restored / Original segmented toggle when a result exists.
- [ ] Surface state as empty / not run / ready / stale.
- [ ] Label the action “Restore faces”, “Re-run face restore”, or “Restore
      again” to match the state.
- [ ] Invalidate the cached restore when the input, strength, or center-only
      option changes. Input-wire changes already clear the result; parameter
      changes still need this.
- [ ] Keep the restore result keyed by the exact input and parameters.
- [ ] Persist restore results together with their source key and parameters so
      staleness survives a reload.

### Viewer and composition

- [ ] Viewer width/height parameters (frontend default 1280 × 720, range
      16–4096).
- [ ] Use the Viewer dimensions as the composition size for effects and
      transforms.
- [ ] Use the Viewer result as the node-mode canvas backdrop, like Blender's
      compositor.
- [ ] Add Group stacking so several inputs compose without a Viewer.
- [ ] Reorder Viewer layers and persist the custom order.
- [ ] Apply per-layer visibility and opacity during composition.

### UI modes and panes

- [ ] Mode buttons for Node, Side-by-side split, Stacked split, and Edit
      canvas.
- [ ] Side-by-side and stacked panes with a draggable splitter.
- [ ] Double-click the splitter to reset to 50%.
- [ ] Persist split positions across restarts.
- [ ] Refit the graph when entering/leaving split mode, but not while dragging
      the splitter.
- [ ] Move the current node canvas inside the shared node pane rather than
      making it the whole window.

### Edit canvas and layers

- [ ] Editor canvas with the composited result and checkerboard background.
- [ ] Fit-to-pane, wheel zoom, zoom buttons, and double-click reset.
- [ ] Layer tree listed top-to-bottom, nested through input connections.
- [ ] Route nodes skipped in the layer tree while following their source.
- [ ] Select a layer and show its inspector.
- [ ] Layer visibility toggle.
- [ ] Layer opacity control.
- [ ] Layer name display and delete.
- [ ] Drag layers to reorder stacking.
- [ ] Copy one layer image to the clipboard.
- [ ] Copy the full composited canvas to the clipboard.
- [ ] **Deferred:** transform box, drag to move, corner-handle scale, numeric
      x/y/scale/rotation, and group move/scale.

Done when the editor can browse, reorder, hide, delete, and copy layers, even
while transform controls remain deferred.

### Templates and projects

- [ ] Template list with save current graph, load, rename, and delete.
- [ ] Project list with save, save as, open, rename, and delete.
- [ ] Current template/project name and identity kept in session state.
- [ ] Thumbnail per saved template/project.
- [ ] Node count per saved template/project.
- [ ] Confirm before replacing the current graph with a saved workspace.
- [ ] Serve/store workspace assets, masks, and restore results with the
      workspace, not only in the global cache.
- [ ] Decide whether to use the existing backend workspace API
      (`/api/workspaces/...`) or an equivalent native store.

### Session and autosave parity

- [ ] Persist source assets as blobs/data alongside the graph, not as absolute
      paths.
- [ ] Persist AI masks and face-restore source keys/staleness metadata.
- [ ] Persist project/template identity with the autosaved session.
- [ ] Restore the unsaved session with its project/template identity after a
      backend restart.
- [ ] Clear the recovery draft after an explicit save baseline, matching the
      frontend behavior.
- [ ] Include asset and processed-data revisions in the autosave signature so
      mask/restore changes trigger a save.

### Export and clipboard

- [ ] Export the composited Viewer result as PNG.
- [ ] Use Viewer dimensions for the exported image.
- [ ] Choose/derive a useful output filename.
- [ ] Add an Export action to the UI. The frontend has the export function but
      currently hides its toolbar button.
- [ ] Copy canvas/layer PNGs to the clipboard from edit mode.

### Styling and UX parity

- [ ] Add the missing node accent colours from the frontend registry.
- [ ] Show accent dots beside node names in the add-node menu.
- [ ] Use one consistent generic field layout for all non-custom node bodies.
- [ ] Add mode and workspace buttons to the title bar, matching the frontend
      HUD layout.
- [ ] Add empty/error states for the new node types and the edit layer tree.
- [ ] Keep controls full-width and node bodies clean as new nodes are added.
- [ ] Keep headers non-selectable and use the existing chevron collapse
      behavior for every new node.

## Suggested order

1. Generic sockets/fields/parameters and the evaluator.
2. Add the 12 missing node types.
3. Image pass-through, asset store, masks, and session blobs.
4. Viewer dimensions, composition, and Group.
5. Graph editing shortcuts and edge operations.
6. UI modes, split panes, and fit view.
7. Edit canvas and layer panel, without transform controls.
8. Templates/projects and workspace persistence.
9. Export/clipboard polish.

Per-layer transform controls remain deferred until the rest of the editor is in
place.
