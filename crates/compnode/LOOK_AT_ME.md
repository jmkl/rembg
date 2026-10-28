# LOOK_AT_ME — CompNode orientation report for LLM agents

> Written 2026-10-27 by an agent that re-read the whole crate. Target reader: a
> future LLM (or human) dropped into this repo with no prior context. Facts were
> verified against the source on that date; re-check anything marked
> "in progress".

---

## 1. What this crate is

**CompNode is a desktop, node-based image processing studio** (think a small,
single-purpose Photopea/Blender-compositor hybrid). The user wires a graph of
nodes in an egui window: load an image, run AI background removal or face
restoration, stack CPU-only color/filter adjustments, composite layers, and
inspect the result on a transformable layer canvas. Everything runs locally;
ONNX inference (via `ort`) is the only "heavy" part, executed on a background
worker thread.

It was extracted from the old `rembg studio` binary so the web/server side of
rembg does not need to depend on egui. Window title / product name: **CompNode**.

- Binary: `cargo run -p compnode` from the workspace root.
- Rust edition 2024, `eframe`/`egui` 0.36, `egui-snarl` 0.12, `image` 0.25,
  ONNX runtime via workspace `ort = 2.0.0-rc.12` (cuda feature), rayon-free
  except `ops/rf` CPU backend.
- Sibling crates it depends on: `rembg-core` (models, ONNX pipelines,
  detection) and `gpaste` (clipboard protocol).

---

## 2. Place in the workspace

```
rembg/                      (repo root, Cargo workspace)
├── crates/compnode/        ← THIS CRATE (GUI studio)
├── crates/rembg-core/      ONNX models + inference, shared
├── crates/gpaste/          clipboard paste protocol
├── crates/rembg-winit/, wgpu-playground/, cmd-craft/   unrelated
├── frontend/               web front end — CompNode mirrors its parameter
│                           names/defaults (see FilterKind::defaults in model.rs)
├── models/                 birefnet.onnx, GFPGANv1.4.onnx, det_10g.onnx, rmbg…
├── input/ output/          scratch dirs
└── Cargo.toml              workspace members + shared deps
```

Path conventions (user-visible, do not change casually):

- Result cache: `%LOCALAPPDATA%\rembg\studio-cache`
- Autosave: `%LOCALAPPDATA%\rembg\studio-autosave.rembg.json`
- Settings: `%LOCALAPPDATA%\rembg\compnode-settings.json`
- Model directory: `./models` (configurable in Settings)

---

## 3. Architecture in one screen

Three execution domains:

1. **UI thread (every frame)**
   `app.rs` drives eframe → `editor.rs` (SnarlViewer) renders the graph from
   `Snarl<Node>` (the single source of truth, `model.rs` data types).
   `eval.rs::ensure_all` re-evaluates every filter node whose _signature_
   (hash of upstream outputs + params + canvas size) changed, at
   **256 px** eval resolution (`ops::EVAL_MAX_EDGE`), and uploads preview
   textures. Node-body thumbnails are capped at **200 px** (`graph::PREVIEW_MAX_EDGE`).
   Slider/filter drags stay smooth because full-res work never happens here.

2. **Worker thread (one long-lived thread, crossbeam-channel)**
   `worker.rs` owns `Job` enum variants: `RemoveBg`, `RemoveBgV2`, `FaceRestore`
   (ONNX inference via `rembg-core`) plus `Composite` (full-res Group stack) and
   `Filter` (full-res render of any other filter). Filter inputs travel as
   `FilterInput::Path | Encoded | Pixels` — **large decodes happen on the worker**,
   never on the UI thread. Results are PNG-encoded into the content-addressed
   cache (`cache.rs`, SHA-256 keys) and posted back with a `token: u64`.
   `Node::finish_job` drops results whose token no longer matches (stale runs).

3. **Persistence**
   `session.rs` — one self-contained `.rembg.json`: nodes (serde-tagged
   `kind` snake_case), links, embedded base64 source assets (`assets.rs`,
   content-addressed), layer transforms, and **absolute paths** to cached
   outputs. `FilterKind::restore` fills missing params with defaults, so old
   sessions survive new fields. `settings.rs` — versionless JSON with serde
   defaults (keep-models-resident, model dir, workspace root).

Filter full-res renders are **debounced** (`FilterNode.render_state` +
`rendered` claim token): while a slider rests, the worker re-renders the chain
in upstream order at full resolution; downstream inference nodes then read that
file instead of the unfiltered upstream image.

### Image size caps (memorize these)

| Stage                                | Cap                          | Where                           |
| ------------------------------------ | ---------------------------- | ------------------------------- |
| Filter preview evaluation            | 256 px longest edge          | `ops::EVAL_MAX_EDGE`            |
| Node-body thumbnails                 | 200 px                       | `graph::PREVIEW_MAX_EDGE`       |
| Output-canvas textures / layer cache | 4096 px                      | `layer_cache::TEXTURE_MAX_EDGE` |
| Layer-cache CPU budget               | 256 MiB                      | `layer_cache::CPU_BUDGET`       |
| Full-res CPU images, graph-wide LRU  | 512 MiB                      | `memory::DEFAULT_LIMIT`         |
| Full-res processing                  | uncapped (12 MP benchmarked) | `worker.rs`                     |
| Settings canvas drag range           | 64…8192                      | `settings_window.rs`            |

`memory.rs` releases LRU full-res images that still have a reloadable path
(`Node::can_reload_image`); preview textures survive release.

---

## 4. Module map (`src/`)

| Module                              | Responsibility                                                                                                                                                                                                                                                                                   |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `main.rs`, `lib.rs`                 | binary entry; crate doc-comment map                                                                                                                                                                                                                                                              |
| `app.rs`                            | eframe app shell: window, menus, shortcuts (Ctrl+O/N/S/, V, F11, Delete), autosave, `ensure_all`/`poll_*` pumps                                                                                                                                                                                  |
| `editor.rs`                         | `SnarlViewer` impl, app-level selection (see §7), chevrons, node menu, full-res render scheduling, `filter_input`                                                                                                                                                                                |
| `catalog.rs`                        | **THE node registry** — `NODE_SPECS: &[NodeSpec]`; menu, header accents, socket counts, body dispatch all derive from it                                                                                                                                                                         |
| `model.rs`                          | `Kind`, `FilterKind` (+`defaults()`), `Node` enum, `Preview`/`Process`/`FilterNode`/`ViewerLayer`, accessors, `finish_job`, `invalidate_output`                                                                                                                                                  |
| `graph.rs`                          | connect/disconnect, `invalidate_node`, image loading, preview textures, viewer refresh                                                                                                                                                                                                           |
| `eval.rs`                           | per-frame filter evaluation, signature hashing, `resolve_inputs`/`resolve_image`, `canvas_size`                                                                                                                                                                                                  |
| `memory.rs`                         | 512 MiB full-res CPU LRU budget + reload decisions                                                                                                                                                                                                                                               |
| `layer_cache.rs`                    | per-canvas decoded-layer cache (4096 px textures, background decode)                                                                                                                                                                                                                             |
| `image_canvas.rs`                   | self-contained layer canvas widget: gizmos (move/scale/rotate, marquee, multi-select), mirrors viewer layers                                                                                                                                                                                     |
| `nodes/`                            | one body-UI module per node kind: `image, remove_bg, remove_bgv2, face_restore, output, route, filter` (generic body for all Filter kinds)                                                                                                                                                       |
| `ops/`                              | pure CPU image ops behind filters: `adjust` (hsv/curves/exposure/color_balance/gradient_map/invert), `composite` (stack/blend/mask), `generate` (gradient/noise), `rawfilter` (dispatch → `rf`), `rf/` (raw develop: `cpu` rayon backend + `gpu` wgpu WGSL behind `gpu` cargo feature, ≥512² px) |
| `params.rs`                         | untyped `Params = serde_json::Map` bag + typed getters with defaults (mirrors frontend `Record<string,any>`)                                                                                                                                                                                     |
| `session.rs`                        | session JSON schema, save/load/restore, unique filenames (` (2)`, ` (3)`…)                                                                                                                                                                                                                       |
| `cache.rs`                          | content-addressed output cache under `%LOCALAPPDATA%\rembg\studio-cache`; `cache_key*` SHA-256 helpers                                                                                                                                                                                           |
| `assets.rs`                         | content-addressed source assets, base64-embedded in sessions                                                                                                                                                                                                                                     |
| `worker.rs`                         | inference thread, model residency policy (keep in memory vs per-run load), PNG cache writes                                                                                                                                                                                                      |
| `settings.rs`, `settings_window.rs` | persisted settings + its window                                                                                                                                                                                                                                                                  |
| `title_bar.rs`                      | custom undecorated title bar + menus                                                                                                                                                                                                                                                             |
| `graph_menu.rs`                     | filterable right-click "add node" menu (catalog-driven)                                                                                                                                                                                                                                          |
| `widgets.rs`                        | reusable preview/socket/error/timing widgets                                                                                                                                                                                                                                                     |
| `gpaste_app.rs`                     | clipboard image paste (arboard + gpaste protocol)                                                                                                                                                                                                                                                |
| `style/`                            | `palette.rs` (every colour), `icons.rs` (icon font), `controls.rs` (widget kit: Button, Slider, Checkbox, Select, ColorField…), `mod.rs` (metrics/theme), `tests.rs` (headless widget tests)                                                                                                     |
| `tests.rs`                          | crate-level headless tests + (temporarily) an ignored release benchmark                                                                                                                                                                                                                          |
| `examples/blank.rs`                 | minimal egui-snarl demo, not part of the app                                                                                                                                                                                                                                                     |
| `assets/fonts/`                     | Inter, Roboto Condensed, Plus Jakarta Sans, Symbols/Lucide                                                                                                                                                                                                                                       |

---

## 5. Complete node catalog (18 nodes)

Single source of truth: `catalog.rs::NODE_SPECS`. Filters share `nodes/filter.rs`
for their body; UI is generated from `FieldSpec`s.

### Sources & viewers

| Node       | Kind           | In→Out    | Notes                                                                                                               |
| ---------- | -------------- | --------- | ------------------------------------------------------------------------------------------------------------------- |
| **Image**  | `Kind::Image`  | 1→1       | Loads a source file as a content-addressed asset; pass-through (its output path is the source file). Hidden fields. |
| **Output** | `Kind::Output` | *multi*→0 | Layer viewer canvas (hidden from the add menu; created per graph). Layers get transforms; feeds `image_canvas.rs`.  |

### Inference nodes (background worker, ONNX)

| Node                       | Kind                | In→Out | Params                                                                      |
| -------------------------- | ------------------- | ------ | --------------------------------------------------------------------------- |
| **Remove Background**      | `Kind::RemoveBg`    | 1→1    | none (BiRefNet)                                                             |
| **Remove Background (v2)** | `Kind::RemoveBgV2`  | 1→1    | none (RMBG-1.4 logic, no width clamp)                                       |
| **Face Restore**           | `Kind::FaceRestore` | 1→1    | `strength` (default 100 %), `only_center` bool (GFPGAN + SCRFD face detect) |

### CPU filters (`FilterKind`, params bag keyed as below)

| Node                 | id (session)              | In→Out          | Fields (key: default, range)                                                                                                                                                                                                         |
| -------------------- | ------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Route**            | `route`                   | 1→1             | none (pass-through, narrow body `NODE_W_S`)                                                                                                                                                                                          |
| **Group**            | `mix` ⚠                   | *multi*→1       | none; stacks inputs bottom-up like a viewer; has layers + canvas size + full-res `Composite` render                                                                                                                                  |
| **Blend**            | `blend`                   | 2→1             | `mode`: 17 blend modes (default `source-over`), `opacity`: 100 % (0–100)                                                                                                                                                             |
| **Mask**             | `mask`                    | 2→1             | `invert`: `dark`\|`light` (default dark), `strength`: 100 % (0–100)                                                                                                                                                                  |
| **Hue / Saturation** | `hsv`                     | 1→1             | `hue`: 0° (0–360), `sat`: 100 % (0–200), `val`: 100 % (0–200)                                                                                                                                                                        |
| **Curves**           | `curves`                  | 1→1             | `channel`: rgb/red/green/blue + interactive per-channel LUT curve editor (custom body)                                                                                                                                               |
| **Exposure**         | `exposure`                | 1→1             | `ev`: 0 (−5…5, step .1), `offset`: 0 (−50…50), `gamma`: 1.0 (0.1…3)                                                                                                                                                                  |
| **Color Balance**    | `colorbalance`            | 1→1             | `range`: shadows/midtones/highlights (default midtones), `cyanRed`/`magentaGreen`/`yellowBlue`: 0 (−100…100), `preserve` luma: yes/no                                                                                                |
| **Gradient Map**     | `gradientmap`             | 1→1             | `color1/2/3`: #000000/#808080/#ffffff, `midpoint`: 50 %, `reverse`: no                                                                                                                                                               |
| **Invert**           | `invert`                  | 1→1             | `amount`: 100 % (0–100)                                                                                                                                                                                                              |
| **Raw Filter**       | `rawfilter` (alias `raw`) | 1→1             | `temp` 0, `tint` 0, `texture` 0, `clarity` 0, `dehaze` 0 (all −100…100), `sharpen` 0 (0–150), `sharpen_radius` 1.0 (0.5–3.0, step .1), `sharpen_detail` 25 (0–100), `noise_reduction` 0 (0–100), `noise_reduction_detail` 50 (0–100) |
| **Gradient**         | `gradient`                | 0→1 (generator) | `mode`: linear/radial/solid (default linear), `color1/2`: #000000/#ffffff, `angle`: 90°, `color1Position`: −200 %, `color2Position`: 200 %, `width`: 1280 (100–2000), `height`: 720 (100–2000)                                       |
| **Noise**            | `noise`                   | 0→1 (generator) | `kind`: fbm/multifractal/ridged (default fbm), `scale` 5 (0.1–50), `detail` 4 (1–8), `roughness` 0.5 (0–1), `lacunarity` 2 (1–10), `distortion` 0 (0–10), `seed` 0 (0–10000)                                                         |

⚠ `Group`'s session id is `"mix"` for legacy parity — `from_id` also accepts
`"group"`. RawFilter accepts `"rawfilter"`/`"raw"`.

Defaults live in **one place**: `FilterKind::defaults()` (`model.rs`), mirroring
`frontend/src/core/nodes.ts`. They feed both node creation/restore and the
sliders' double-click-to-reset.

### Pipeline order inside Raw Filter

noise reduction → white balance → dehaze → texture → clarity → sharpen;
alpha preserved. Dispatch: `ops::evaluate` → `ops/rawfilter.rs::raw_filter` →
`ops/rf::raw_filter` (rayon CPU backend; optional wgpu GPU backend behind the
`gpu` cargo feature for images ≥ 512² px, override via
`RAW_FILTER_GPU_MIN_PIXELS`). The legacy helpers in `ops/rawfilter.rs`
(`sharpen`, `noise_reduction`, …) are currently dead code pending cleanup.

---

## 6. Recipes

### Adding a node (the canonical checklist, from the README + model.rs docs)

1. `Kind` variant (+`FilterKind` variant + `id()`/`from_id()`/`defaults()`) in `model.rs`, and a `Node` variant (reuse `Process` if it's an inference node).
2. Add `src/nodes/<name>.rs` with `create` + `show_body`.
3. Register in `catalog.rs::NODE_SPECS` (menu/title/accent/sockets/width/body dispatch come from this entry — no central `match` to touch for menu or body).
4. Persistent? Add a `SessionNodeData` case + snapshot/load mapping in `session.rs`.
5. Inference? Add a `Job` variant in `worker.rs` + a starter method in `editor.rs`.
6. Filter ops go in `ops/` as pure pixel functions; `ops::evaluate` dispatches.

### Checks

```sh
cargo fmt -p compnode
cargo clippy -p compnode --all-targets -- -D warnings
cargo test -p compnode
cargo test -p compnode --lib            # headless UI + logic tests
cargo test -p compnode --release --lib bench_filter_evaluation -- --ignored --nocapture
```

---

## 7. Quirks & invariants (things that will bite you)

- **Selection is app-level.** `egui-snarl` 0.12 only updates selection on
  modified clicks/rect-select, so `editor.rs` keeps its own selection for
  Delete/Edit, draws the accent outline itself, and hit-tests nodes in screen
  space (Snarl renders on a transformed layer without an `Area`).
- **Widget IDs must be fresh** after loading/clearing a graph, or old pan/zoom
  and selection leak into the new file.
- **`Preview { … }` literals must include the `eval` field** (or use
  `Preview::with_image`) — it carries the cached 256 px eval copy that keeps
  filter dragging fast. Introduced by the task2710 performance work.
- **Session `Group` id is `"mix"`**; don't "fix" the asymmetry without a
  migration.
- **Tokens guard stale results**: every worker result passes through
  `Node::finish_job(token, …)`; connect/disconnect/invalidate resets tokens.
- **Filters keep their old preview while re-evaluating** (`invalidate_output`
  only clears `signature`) to avoid a one-frame flash; full-res Group renders
  are dropped together with their pending job.
- **Styling is centralized**: colours in `style/palette.rs`, metrics/theme in
  `style/mod.rs`, widgets in `style/controls.rs`, node header colours from
  `pal::HEADER_*` assigned per node in `catalog.rs`.
- **Unrelated pre-existing rustfmt/clippy warnings** exist in the tree; don't
  assume you caused them.
- `README.md` is user-facing but slightly stale: it still says preview textures
  are capped at 1024 px (actual: 4096, raised for RemoveBgV2).

---

## 8. Work in progress at time of writing (2026-10-27)

See `task2710.md` (fresh work log) and `TODO.md`:

- **Raw Filter performance optimization is mid-flight.** Done: `Preview.eval`
  cached eval image. Pending: switch `eval.rs::resolve_image` to
  `Node::eval_cpu()`, fix the `session.rs` `Preview` literal, rewrite blur
  helpers (box blur + luma-only channels), re-benchmark, then remove the
  temporary `bench_filter_evaluation` test. Expect `cargo check` breakage
  described in task2710.md "Active" section if you arrive mid-task.
- **Raw Filter GPU backend** (`ops/rf/gpu.rs` + `raw_filter.wgsl`) exists behind
  the `gpu` cargo feature; CPU rayon backend is the default path.
- Recent addition (2026-10-27): **sliders double-click back to their default**
  (`style/controls.rs::Slider::default(...)`; filter sliders get defaults from
  `FilterKind::defaults()`, Face Restore strength resets to 100 %). Covered by
  `style::tests::slider_double_click_resets_to_default`.
- Known pre-existing failure: `tests::preview_textures_are_capped` fails on the
  unmodified tree (verified by stashing all local changes) — not caused by
  recent slider work.
- All of the above is **uncommitted** local work.

---

## 9. One-paragraph conclusion

CompNode is a local-first, egui-based node editor for image post-production:
source/generator nodes feed pure-CPU filter nodes and three ONNX inference
nodes, all previewed at 256 px for interactivity while a single background
worker renders full-resolution results through a SHA-256 content-addressed
cache; the graph persists as a portable JSON session with embedded source
assets, and every node kind is described once in `catalog.rs::NODE_SPECS` from
which menus, styling, sockets, and dispatch are derived. When changing it, add
nodes through the catalog checklist, keep defaults in `FilterKind::defaults()`,
keep the UI thread free of full-resolution decodes, and read `task2710.md`
before touching `model.rs`, `eval.rs`, `ops/rawfilter.rs`, or `session.rs` —
that work is actively in progress.
