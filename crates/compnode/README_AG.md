# CompNode — Architecture & Codebase Overview

> Documented on 2026-10-29. Comprehensive codebase breakdown and architectural reference for the `compnode` crate.

---

## 1. Executive Summary & Purpose

**CompNode** is a desktop, node-based image processing and composition studio (combining workflows similar to Photopea, DaVinci Resolve Fusion, and Blender's compositor).

The application allows users to build non-destructive visual graphs combining:
- **Source Assets**: Local images loaded and tracked via content-addressed hashes.
- **AI Inference**: Neural-network-driven background removal ([`BiRefNet`](../rembg-core), [`RMBG-1.4`](../rembg-core)) and face enhancement ([`GFPGAN`](../rembg-core) + [`SCRFD`](../rembg-core)).
- **Image Filters & Color Grading**: Adjustments (HSV, Curves, Exposure, Color Balance, Gradient Map, Invert), procedural generators (Gradients, Noise, Text overlays), and Camera Raw adjustments.
- **Multi-Layer Compositing**: Canvas-oriented layer blending with 16 CSS blend modes, opacity, and full 2D geometric transforms (translation, rotation, uniform & non-uniform scaling).
- **Interactive 2D Canvas**: Visual inspection canvas featuring interactive gizmos, multi-selection marquee, and alignment guides.
- **Persistence & Interop**: Lossless session save/load (`.rembg.json`), disk-level SHA-256 caching, and a TCP bridge to [`gpaste`](../gpaste) for AI prompt-based image generation.

---

## 2. Workspace Context & Dependencies

```
rembg/                                 (Repository Root)
├── crates/compnode/                   ← CompNode GUI Studio
│   ├── src/                           Source modules
│   ├── assets/                        Fonts (Anton, Inter, Uni Sans, Roboto, Icons)
│   ├── examples/                      Sample binaries
│   └── Cargo.toml                     Crate dependencies & features
├── crates/rembg-core/                 Shared ONNX inference models (BiRefNet, RMBG, GFPGAN)
├── crates/gpaste/                     Clipboard & AI generation protocol
├── models/                            ONNX weights directory (configurable in settings)
└── Cargo.toml                         Workspace definition
```

### Core Technologies
- **Language**: Rust (Edition 2024)
- **GUI & Graphics**: [`eframe`](https://crates.io/crates/eframe) & [`egui`](https://crates.io/crates/egui) `0.36`
- **Graph Framework**: [`egui-snarl`](https://crates.io/crates/egui-snarl) `0.12`
- **GPU Compute**: Optional/default `gpu` feature via [`wgpu`](https://crates.io/crates/wgpu) `29` + WGSL compute shaders for raw filtering
- **Inference**: [`rembg-core`](../rembg-core) backed by ONNX Runtime (`ort`)

---

## 3. Architecture & Execution Domains

The application is structured around strict separation between the UI event loop, background compute, and disk persistence:

```
┌─────────────────────────────────────────────────────────────┐
│                         UI Thread                           │
│  - Main event loop & shell (src/app.rs)                     │
│  - Graph layout & interaction (src/editor.rs)               │
│  - Fast preview evaluation (src/eval.rs, <= 256px max edge) │
│  - Transform canvas & gizmos (src/image_canvas.rs)          │
└──────────────────────────┬──────────────────────────────────┘
                           │ WorkerRequest (crossbeam / mpsc channel)
                           │ JobDone / token matching
┌──────────────────────────▼──────────────────────────────────┐
│                   Background Worker Thread                  │
│  - Long-lived background thread (src/worker.rs)             │
│  - ML Inference: BiRefNet, RMBG-1.4, GFPGAN                 │
│  - Full-resolution rendering (Filter & Composite jobs)      │
│  - Content-addressed disk caching (SHA-256 PNGs)            │
└─────────────────────────────────────────────────────────────┘
```

### 1. UI Thread (`src/app.rs`, `src/editor.rs`, `src/eval.rs`)
- Runs at monitor refresh rate (60+ FPS).
- Graph evaluation for previews is capped at `ops::EVAL_MAX_EDGE` (256px).
- Uses hash-based change detection (`eval::filter_signature`): nodes only re-evaluate when their upstream dependencies, parameters, or canvas dimensions mutate.
- Graph thumbnails in node bodies are capped at `graph::PREVIEW_MAX_EDGE` (256px / 200px).
- Heavy pixel processing or model loading is **never** executed directly on the UI thread.

### 2. Inference & Render Worker (`src/worker.rs`)
- Single long-lived worker thread accepting `Job` variants:
  - `RemoveBg` (BiRefNet) & `RemoveBgV2` (RMBG-1.4)
  - `FaceRestore` (SCRFD alignment + GFPGAN)
  - `Composite` (full-resolution Group / Blend multi-layer rendering)
  - `Filter` (full-resolution offline filter evaluation)
- Decodes large images on the worker thread (`FilterInput::Path`, `Encoded`, `Pixels`).
- Emits results with a monotonically increasing `token: u64`. The UI matches tokens to discard stale results if a user modified parameters while a job was in flight.

### 3. Persistence, Cache & Memory
- **Output Cache (`src/cache.rs`)**: Processed results are saved as PNGs under `%LOCALAPPDATA%\rembg\studio-cache\` keyed by SHA-256 hashes of inputs and parameters.
- **CPU Memory Management (`src/memory.rs`)**: Full-resolution source images held in RAM are budgeted (512 MB cap) and dynamically evicted/reloaded from disk or asset store on demand.
- **Session Serialization (`src/session.rs`)**: Saved graphs (`.rembg.json`) store node coordinates, connection links, embedded base64 assets, parameter dictionaries, and absolute output paths. Backward compatibility with older session files is maintained (e.g., mapping `"mix"` to `Kind::Group`, `"route"` to `Kind::Route`, `"texts"` to `Kind::Texts`).

---

## 4. Module Map & Responsibilities

| Module | Purpose | Key Types / Functions |
| :--- | :--- | :--- |
| `src/model.rs` | Core domain model & data structures | `Kind`, `FilterKind`, `Node`, `FilterNode`, `GroupNode`, `TextsNode`, `Preview`, `Process`, `ViewerLayer`, `RunTiming` |
| `src/catalog.rs` | Node registry and static metadata | `NODE_SPECS`, `NodeSpec`, `FieldSpec`, `FieldKind`, `EvalFn`, `spec()` |
| `src/editor.rs` | Snarl node graph viewer implementation | `GraphEditor`, `SnarlViewer` impl, wire cutting (`Ctrl`+drag), pin layout |
| `src/graph.rs` | Graph topological algorithms | `resolve_route()`, `refresh_viewer()`, `upstream()`, `preview_dimensions()` |
| `src/eval.rs` | Preview pipeline evaluation | `ensure_all()`, `ensure_filter()`, `filter_signature()`, `resolve_image()` |
| `src/image_canvas.rs`| Interactive canvas viewer & gizmos | `ImageCanvas`, layer hit-testing, uniform scale, rotation snap, guides, marquee |
| `src/worker.rs` | Inference & async job runner | `Job`, `spawn_worker()`, `WorkerRequest`, `JobDone`, `InferenceConfig` |
| `src/assets.rs` | Content-addressed asset repository | `AssetStore`, `Asset`, embedded base64 encoding/decoding |
| `src/nodes/` | Individual node body UI renderers | UI logic for `image`, `remove_bg`, `face_restore`, `filter`, `texts`, `route`, etc. |
| `src/ops/` | Pure image manipulation algorithms | Adjustments, blend modes, text layout, and raw camera filters |
| `src/session.rs` | JSON session serialization | `SessionFile`, `SessionNode`, `SessionLink`, save / load logic |
| `src/settings.rs` | App preferences | Model paths, caching preferences, autosave intervals |
| `src/style/` | Visual theme, controls, and colors | Theme definitions, sliders, buttons, socket widgets, color palettes |
| `src/gpaste_app.rs` | GPasTe protocol bridge | TCP client/server exchanging prompt requests & generated images |

---

## 5. Key Design Concepts

### Unbundled Node Archetypes
- **Generic Filters & Generators (`Kind::Filter(FilterKind)`)**: Operate strictly on pixel arrays or procedural parameters. Backed by lightweight `FilterNode` structs containing only parameter map, preview cache, and signature.
- **Pass-Through Route Nodes (`Kind::Route`)**: `Node::Route` has zero state and produces no image of its own. It organizes complex graphs visually; downstream nodes and layer extractors resolve incoming wires through Route nodes via `graph::resolve_route()` until reaching the authoritative content-producing node.
- **Layer Containers (`Kind::Group`, `Kind::Blend`)**: Dedicated `GroupNode` structs that aggregate multiple upstream sources as transformable layers (`ViewerLayer`). Layers retain individual coordinates (`center`), dimensions (`size`), and rotation (`angle`). They can be manipulated interactively inside `ImageCanvas` or composited into flattened images.
- **Typography Overlays (`Kind::Texts`)**: Dedicated `TextsNode` handling rich multi-layer vector text rendering, font asset bindings, outlines, and drop shadows.

### Self-Defaulting Fields & Pure Function Pointers
- **Single Source of Truth for Defaults**: Parameter default values are embedded directly into `FieldSpec` (`FieldKind::Range`, `FieldKind::Number`, `FieldKind::Color`, `FieldKind::Select`). Node parameter defaults are auto-derived directly from catalog metadata.
- **Decoupled Evaluation (`EvalFn`)**: Filter specs attach pure `eval` function pointers / closures. `ops::evaluate()` delegates directly without requiring large manual match tables across the codebase.
- **Builder Pattern (`NodeSpec::filter`, `NodeSpec::generator`)**: Standard filter specifications are declared cleanly with boilerplate-free constructors.

### Dual-Backend Raw Filter (`src/ops/rf/`)
The Raw Filter provides photographer-style adjustments: White Balance (temperature/tint), Texture, Clarity, Dehaze, Unsharp Mask Sharpening, and Noise Reduction.
- **GPU Path (`rf/gpu.rs`)**: Uses WGSL compute shaders (`raw_filter.wgsl`) via `wgpu`. Automatically chosen when image dimensions exceed `GPU_MIN_PIXELS` (default 512×512).
- **CPU Path (`rf/cpu.rs`)**: Rayon-parallelized fallback using float buffers and box-blur approximations.

---

## 6. Codebase Health & Future Work

- **Compilation**: Passes cleanly (`cargo check -p compnode` with 0 warnings).
- **Test Suite**: 43 passing tests in `src/tests.rs`, `src/style/tests.rs`, and `src/image_canvas.rs`.
- **Node System Redesign**: Completed and tracked in [`CHECKLIST.md`](./CHECKLIST.md) and [`PLAN.md`](./PLAN.md).
