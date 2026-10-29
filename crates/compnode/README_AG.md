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
- **Shared Node Contract**: `NodeTrait` exposes catalog metadata, ports, field schemas, validation, and parameter persistence over the current node enum.
- **Pixel Behavior Contract**: [`PIXEL_CONTRACT.md`](./PIXEL_CONTRACT.md) records current color-space, alpha, luma, quantization, and sampling behavior.
- **Graph Coordinate Contract**: [`GRAPH_COORDINATES.md`](./GRAPH_COORDINATES.md) defines document-canvas units and how layer transforms scale with render targets.

---

## 2. Workspace Context & Dependencies

```
rembg/                                 (Repository Root)
├── crates/compnode/                   ← CompNode GUI Studio
│   ├── src/ops/color/                 Color adjustments
│   ├── src/ops/composite/             Blend, mask, stack, and layer transforms
│   ├── src/ops/generators/            Gradients, noise, and text rasterization
│   ├── src/ops/rf/                    CPU/WGSL Raw Filter pipeline
│   ├── src/ops/math.rs                Shared pixel math and samplers
│   ├── assets/                        Fonts (Anton, Inter, Uni Sans, Roboto, Icons)
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
│  - Preview evaluation (src/eval.rs, 128/256/512px tiers)   │
│  - Transform canvas & gizmos (src/image_canvas.rs)          │
└──────────────────────────┬──────────────────────────────────┘
                           │ WorkerRequest (typed render / inference)
                           │ WorkerResponse / token matching
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
- Graph evaluation has Draft (128px), Normal (256px), and High (512px) resolution tiers; the app currently uses Normal. Texts raster previews stay at document-canvas resolution to keep their layer bounds and alpha crop aligned.
- Uses hash-based change detection (`eval::filter_signature`): nodes only re-evaluate when their upstream dependencies, parameters, or canvas dimensions mutate.
- Source graph thumbnails are capped at `graph::PREVIEW_MAX_EDGE` (256px).
- Image-producing nodes share `Preview`; its cached evaluation copy is bounded to 256px on the longest edge. Full-resolution CPU images remain available for layer compositing and worker jobs.
- Heavy pixel processing or model loading is **never** executed directly on the UI thread.

### 2. Inference & Render Worker (`src/worker.rs`)
- Single long-lived worker thread accepting typed `RenderRequest` and `InferenceRequest` envelopes:
  - Background-removal models share a mockable `InferenceBackend::predict` interface; the worker retains responsibility for model loading and cache policy.
  - `RemoveBg` (BiRefNet) & `RemoveBgV2` (RMBG-1.4)
  - `FaceRestore` (SCRFD alignment + GFPGAN)
  - `Composite` (full-resolution Group / Blend multi-layer rendering)
  - `Filter` (full-resolution offline filter evaluation)
- Decodes large images on the worker thread (`FilterInput::Path`, `Encoded`, `Pixels`).
- Emits results with a monotonically increasing `token: u64`. The UI matches tokens to discard stale results if a user modified parameters while a job was in flight.
- Background-removal models use a mockable prediction trait. Cached model instances unload after ten idle minutes, when caching is disabled, or when the model directory changes.
- Worker cache writes use `OutputCacheInterface`; tests can use its in-memory implementation without disk output.

### 3. Persistence, Cache & Memory
- **Output Cache (`src/cache.rs`)**: Processed results are saved as PNGs under `%LOCALAPPDATA%\rembg\studio-cache\` keyed by SHA-256 hashes of inputs and parameters.
- **CPU Memory Management (`src/memory.rs`)**: Full-resolution source images held in RAM are budgeted (512 MB cap) and dynamically evicted/reloaded from disk or asset store on demand.
- **Session Serialization (`src/session.rs`)**: Saved graphs (`.rembg.json`) store a root version, node coordinates, connection links, embedded base64 assets, parameter dictionaries, bypass state, and output paths. Unversioned/version 0 files migrate through `migrate_v0_to_v1`; unsupported future versions fail clearly. Optional bypass state defaults to false for older files.
- **Parameter restore**: Generic parameter maps merge saved values into constructor defaults and preserve unknown keys. Validation is explicit; session loading does not silently alter saved values.
- **Worker jobs**: Render and inference work use separate request/response types. Invalidated queued jobs carry shared cancellation flags and are skipped before execution; stale completion tokens remain guarded by the UI.

---

## 4. Module Map & Responsibilities

| Module | Purpose | Key Types / Functions |
| :--- | :--- | :--- |
| `src/model.rs` | Core domain model, shared node contract, and preview state | `Kind`, `FilterKind`, `Node`, `NodeTrait`, `FilterNode`, `GroupNode`, `TextsNode`, `Preview`, `Process` |
| `src/catalog.rs` | Node registry, metadata and categories | `NODE_SPECS`, `NodeCatalog`, `NodeCategory`, `NodeSpec`, `FieldSpec`, `FieldKind`, `EvalFn`, `spec()` |
| `src/editor.rs` | Snarl node graph viewer implementation | `GraphEditor`, `SnarlViewer` impl, wire cutting (`Ctrl`+drag), pin layout |
| `src/graph.rs` | Graph topological algorithms | `resolve_route()`, `refresh_viewer()`, `upstream()`, `preview_dimensions()` |
| `src/eval.rs` | Preview pipeline evaluation | `ensure_all()`, `ensure_filter()`, `filter_signature()`, `resolve_image()` |
| `src/image_canvas.rs`| Interactive canvas viewer & gizmos | `ImageCanvas`, layer hit-testing, uniform scale, rotation snap, guides, marquee |
| `src/worker.rs` | Inference & async job runner | `RenderRequest`, `InferenceRequest`, `WorkerResponse`, `spawn_worker()`, `InferenceConfig` |
| `src/assets.rs` | Content-addressed asset repository | `AssetStore`, `Asset`, embedded base64 encoding/decoding |
| `src/nodes/` | Individual node body UI renderers | UI logic for `image`, `remove_bg`, `face_restore`, `filter`, `texts`, `route`, etc. |
| `src/ops/color/` | Color adjustment algorithms | HSV, curves, exposure, balance, gradient map, invert |
| `src/ops/composite/` | Layer and mask operations | CSS blend modes, stacking, transforms, alpha masks |
| `src/ops/generators/` | Source-free pixel generation | Gradients, value/Perlin/Simplex noise, text rasterization |
| `src/ops/rf/` | Raw Filter backends | Shared settings, Rayon CPU pipeline, optional WGSL GPU pipeline |
| `src/ops/math.rs` | Shared pixel math | Luma definitions, clamping, conversions, bilinear/bicubic samplers |
| `src/ops/effects.rs` | Text and image effect primitives | Masks, shadows, outlines |
| `src/session.rs` | JSON session serialization | `SessionFile`, `SessionNode`, `SessionLink`, save / load logic |
| `src/settings.rs` | App preferences | Model paths, caching preferences, autosave intervals |
| `src/style/` | Visual theme, controls, and colors | Theme definitions, sliders, buttons, socket widgets, color palettes |
| `src/gpaste_app.rs` | GPasTe protocol bridge | TCP client/server exchanging prompt requests & generated images |

---

## 5. Key Design Concepts

### Unbundled Node Archetypes
- **Generic Filters & Generators (`Kind::Filter(FilterKind)`)**: Catalog-driven pure operations registered with `NodeSpec::filter` or `NodeSpec::generator`. Their `FilterNode` stores parameters, a shared preview, signature, and full-resolution render state.
- **Pass-Through Route Nodes (`Kind::Route`)**: `Node::Route` has zero state and produces no image of its own. It organizes complex graphs visually; downstream nodes and layer extractors resolve incoming wires through Route nodes via `graph::resolve_route()` until reaching the authoritative content-producing node.
- **Layer Containers (`Kind::Group`, `Kind::Blend`)**: Dedicated `GroupNode` structs that aggregate multiple upstream sources as transformable layers (`ViewerLayer`). Layers retain individual coordinates (`center`), dimensions (`size`), and rotation (`angle`). They can be manipulated interactively inside `ImageCanvas` or composited into flattened images.
- **Typography Overlays (`Kind::Texts`)**: Dedicated `TextsNode` handling rich multi-layer vector text rendering, font asset bindings, outlines, and drop shadows.

### Self-Defaulting Fields & Pure Function Pointers
- **Single Source of Truth for Defaults**: Parameter default values are embedded directly into `FieldSpec` (`FieldKind::Range`, `FieldKind::Number`, `FieldKind::Color`, `FieldKind::Select`). Node parameter defaults are auto-derived directly from catalog metadata.
- **Decoupled Evaluation (`EvalFn`)**: Filter specs attach pure `eval` function pointers / closures. `ops::evaluate()` delegates directly without requiring large manual match tables across the codebase.
- **Builder Pattern (`NodeSpec::filter`, `NodeSpec::generator`)**: Standard filter specifications are declared cleanly with boilerplate-free constructors.

### NodeTrait and Session Parameters
- `NodeTrait` adapts the static `Node` enum with dynamic metadata, port, and parameter access. The catalog supports runtime registration or replacement for `Kind` values; adding a wholly new node kind still requires an enum variant and Rust implementation.
- Its field validator clamps numeric fields to their catalog range and replaces invalid colors/selections with field defaults. Validation is explicit and is not applied automatically during session restore.
- Parameter serialization hooks are used by session snapshots and restoration. Typed node state, including inference settings and layer transforms, remains handled by the session codec.

### Phase 4 Editor Behavior
- Shared body spacing, preview slots, error banners and status markers live in `src/widgets.rs`. The graph header shows node status and a functional bypass control for evaluable nodes.
- Bypass passes the first connected input image through and persists in session files.
- The graph menu groups nodes by category, searches titles and keywords with fuzzy subsequence matching, and shows five recently used node kinds.
- Undo/redo has a bounded 100-step history for graph structure, image changes, parameters, bypass and canvas transforms. Rapid edits to the same node coalesce for slider scrubbing.

### Pixel Semantics
- `image::RgbaImage` is the RGBA8 interchange format. Current operations treat RGB as encoded sRGB-like values and do not apply ICC profiles or linearize RGB.
- Alpha, luma, rounding, and sampling rules—and compatibility expectations for changing them—are documented in [`PIXEL_CONTRACT.md`](./PIXEL_CONTRACT.md).

### Dual-Backend Raw Filter (`src/ops/rf/`)
The Raw Filter provides photographer-style adjustments: White Balance (temperature/tint), Texture, Clarity, Dehaze, Unsharp Mask Sharpening, and Noise Reduction.
- **GPU Path (`rf/gpu.rs`)**: Uses a shared WGPU device/queue for Raw Filter, WGSL Curves LUT and Color Balance, and source-over multi-layer transformed composites. Compute passes run at 512 × 512 and above; smaller images and unavailable/oversized GPU paths fall back to CPU. Other composite blend modes remain on CPU for parity.
- **CPU Path (`rf/cpu.rs`)**: Rayon-parallelized fallback using float buffers and separable Gaussian blur passes.

---

## 6. Codebase Health & Future Work

- **Compilation and tests**: `cargo check -p compnode` and `cargo check -p compnode --no-default-features` pass without warnings. The latest full runs passed 77 tests (1 ignored) with default features and 76 tests (1 ignored) without default features.
- **Node System Redesign**: Completed and tracked in [`CHECKLIST.md`](./CHECKLIST.md) and [`PLAN.md`](./PLAN.md).
