# CompNode Refactoring Plan

## Overview

This plan outlines a comprehensive refactoring of the CompNode crate to improve scalability and maintainability while preserving all existing functionality.

## Current State Analysis

### Strengths

- **Well-structured node catalog**: `NODE_SPECS` drives menus, styling, sockets, and dispatch uniformly
- **Clear separation of concerns**: UI thread vs worker thread vs session layer
- **Content-addressed caching**: SHA-256 based cache for instant session restore
- **Established patterns**: Node creation, evaluation, and persistence patterns are well-established

### Areas for Improvement

1. **Module fragmentation**: 30+ source files with complex interdependencies
2. **Large individual files**: `eval.rs` (25K), `editor.rs` (42K), `model.rs` (32K), `worker.rs` (36K), `image_canvas.rs` (79K)
3. **Tight coupling between graph and evaluation**: `eval.rs` directly accesses `snarl[node]` internals
4. **Node body UI scattered**: Each node's `show_body` in separate files, but logic varies in consistency
5. **Limited test coverage**: Some tests exist but comprehensive coverage is lacking
6. **No formal plugin/extension system**: All nodes are compiled in statically
7. **Pixel semantics**: Color-space, alpha, rounding, and resampling behavior should be explicit and consistent across operations.

## Refactoring Goals

1. **Modularize the codebase** into clear functional units
2. **Improve abstraction boundaries** between UI, graph, evaluation, and worker
3. **Create a plugin system** for node extensibility
4. **Enhance testability** with proper unit tests
5. **Improve type safety** and reduce downcasting
6. **Better organize operations** (ops module)
7. **Maintain backward compatibility** with existing sessions and settings

## Proposed Structure

### 1. Core Modules (refactor existing, don't split unnecessarily)

- `model/` - Graph data types (`Kind`, `Node`, `Preview`, `ViewerLayer`)
- `catalog/` - Node registry and specification matching
- `graph/` - Graph operations (connect, disconnect, invalidate, resolve routes)
- `eval/` - Filter evaluation pipeline (signature computation, input resolution, ops execution)
- `worker/` - Background inference thread and model management
- `ops/` - Organized operation implementations (currently scattered)
- `memory/` - CPU image management and bounds

### 2. New Abstractions

- **Node trait**: Introduce an object-safe contract over the current `Node` enum for metadata, ports, parameter validation, and persistence; only replace enum storage if future extension requirements justify it
- **Node Lifecycle Contract**: Formalize the 4-stage lifecycle across all node types:
  1. *Immediate UI Preview* (capped resolution, non-blocking on UI thread)
  2. *Full-Resolution Worker Render* (background thread with disk cache persistence)
  3. *Downstream Signature Invalidation* (deterministic input/parameter hashing)
  4. *Canvas Bounding Box & Gizmo Hit-Testing* (tight alpha content crop vs full canvas rectangle)
- **Resolution Negotiation & Scale Stability**: Upstream dimension propagation ensuring layers never collapse to preview dimensions during slider dragging or wire connections
- **Evaluation strategy**: Abstract filter evaluation behind a trait for CPU/GPU dispatch
- **GPU Compute Pipeline**: Standardize WGPU compute passes for heavy operations (Raw Filter, Blends, Color Grading)
- **Cache interface**: Abstract content-addressed cache for testability
- **Worker job interface**: Cleaner job submission and result handling
- **Session Versioning & Migration**: Explicit schema versioning (`schema_version: u32`) and migration pipeline for `.rembg.json` files
- **Undo/Redo History**: Command pattern for graph topology modifications and parameter adjustments

### 3. Operations Organization

Keep operations under `src/ops/` and organize them by domain:

- `color/` - HSV, curves, exposure, color balance, gradient map, invert
- `composite/` - Blend modes, layer stacking, transforms, masks
- `generators/` - Gradients, value/Perlin/Simplex noise, text rendering
- `rf/` - CPU and GPU Raw Filter implementations
- `math.rs` - Shared luma, clamping, conversion, and sampling helpers

### 4. Pixel Semantics & Operation Contracts

Make the image-processing assumptions explicit before changing algorithms:

See [`PIXEL_CONTRACT.md`](./PIXEL_CONTRACT.md) for the measured current behavior and compatibility baseline.

- Document whether RGB values are treated as encoded sRGB or linear light at each operation boundary.
- Document straight-alpha versus premultiplied-alpha expectations, including text rendering, masks, resize, and compositing.
- Name the luma definition used by each operation (Rec. 709, Rec. 601, or CSS blend luminosity) and preserve CSS blend behavior.
- Define rounding and clamping rules for float-to-byte conversion and sampling at image edges.
- Record behavior for empty images, non-finite parameters, and extreme dimensions; keep operations deterministic for a fixed input and seed.
- Require any behavior-changing color-space or alpha correction to include before/after golden cases and a session compatibility note.

### 5. Node System Improvements

- Standardize `show_body` signatures and patterns
- Create base trait for node bodies
- Add node registry hooks for auto-discovery
- Improve preview management consistency

### 6. Testing Framework

- Add integration test suite
- Create mock worker for UI tests
- Test evaluation pipeline with known inputs/outputs, including alpha and edge cases
- Test cache behavior
- Compare CPU and GPU output with documented per-operation tolerances
- Check deterministic generator output for fixed parameters and seeds

## Migration Plan

### Phase 1: Foundation (Weeks 1-2)

1. Refactor `ops/` module into organized submodules
2. Create `model/trait.rs` with core node traits
3. Add unit tests for core types
4. Ensure `cargo check` passes throughout

### Phase 2: Graph & Evaluation (Weeks 3-4)

1. Refactor `graph.rs` to use new abstractions
2. Restructure `eval.rs` with strategy pattern
3. Improve test coverage for evaluation pipeline
4. Maintain backward compat with session format

### Phase 3: Worker & Inference (Weeks 5-6)

1. Refactor `worker.rs` with cleaner job interface
2. Add model management abstractions
3. Test inference pipeline end-to-end

### Phase 4: Node System (Weeks 7-8)

1. Standardize node body UI patterns
2. Create node trait and registry improvements
3. Add plugin discovery mechanism (optional)

### Phase 5: Polish & Documentation (Weeks 9-10)

1. Add comprehensive examples
2. Ensure all existing tests pass
3. Performance benchmarking

---

## Actionable Task Checklist (TODO)

> Detailed standalone checklists:
> - Master Architectural Roadmap Checklist: [`PLAN_CHECKLIST.md`](./PLAN_CHECKLIST.md)
> - Node System Redesign Checklist (Steps 1–5): [`CHECKLIST.md`](./CHECKLIST.md)

### Quick Fixes & Baseline Health

- [x] Fix pre-existing unit test failure in `tests::preview_textures_are_capped` (assert dimension check 256 vs 200).
- [x] Clean up compiler warnings:
  - [x] Remove unused import `self` from the legacy Raw Filter wrapper.
  - [x] Remove unused variables `_t` and `_backend` in `src/ops/rf/mod.rs`.
  - [x] Remove the obsolete `src/ops/rawfilter.rs` wrapper and route catalog dispatch to `src/ops/rf/`.
  - [x] Remove unused `min_pixels` in `src/ops/rf/mod.rs` and `eval_image` in `src/model.rs`.

### Phase 1: Operations & Core Data Foundation

- [x] **Ops Reorganization (`src/ops/`)**:
  - [x] Split `src/ops/` into clear submodules:
    - [x] `ops/color/`: HSV, Curves LUT, Exposure, Color Balance, Gradient Map, Invert
    - [x] `ops/composite/`: 16 CSS blend modes, layer stacking, bilinear transforms, alpha masks
    - [x] `ops/generators/`: Gradients, Perlin/Simplex/FBM noise, text renderer
    - [x] `ops/rf/`: White balance, dehaze, texture, clarity, sharpen, noise reduction (CPU & WGSL)
  - [x] Consolidate common math & color utilities in `src/ops/math.rs` (luma, clamping, color space conversions).
- [x] **Core Node Abstraction (`src/model/`)**:
  - [x] Define object-safe `NodeTrait` over current node instances for metadata, ports, parameter definitions/validation, and parameter persistence hooks.
  - [x] **Promote `Route`, `Group`, and `Texts` out of `FilterKind` into top-level `model::Kind`**:
    - [x] Make `Route` a dedicated structural pass-through variant (`Kind::Route`, `Node::Route`).
    - [x] Make `Group` a dedicated layer container node (`Kind::Group`, `Node::Group(GroupNode)`), removing `layers`, `canvas_size`, and debounce state from `FilterNode`.
    - [x] Make `Texts` a dedicated generator node (`Kind::Texts`, `Node::Texts(TextsNode)`) with its typographic model.
    - [x] Ensure backward compatibility with session parser (`SessionNodeData::Filter` mapping for legacy sessions).
  - [x] **Node Lifecycle & Bounding Box Contract**:
    - [x] Ensure all evaluable nodes (including unbundled generator nodes like `TextsNode`) support background full-resolution renders (`can_render_full_res`) with disk caching.
    - [x] Ensure instant tight alpha bounding box calculation (`alpha_content_uv`) from both immediate UI preview CPU pixels and cached worker outputs.
  - [x] Standardize image-producing nodes on `Preview` and its bounded 256px evaluation cache; structural Route and Output nodes have no pixel preview.
  - [ ] Standardize `Preview` structure and caching bounds across all node types.

### Phase 2: Graph & Evaluation Engine

- [ ] **Decouple Graph from Evaluation (`src/eval.rs`)**:
  - [x] Introduce an evaluation context trait to keep node and pin access out of evaluation logic.
  - [x] Isolate deterministic signature hashing and test it independently of graph traversal.
  - [x] Move preview bounds out of `ops` into configurable Draft/Normal/High resolution tiers.
- [ ] **Resolution Negotiation & Canvas Transform Stability**:
  - [x] Propagate upstream effective dimensions (`effective_dimensions`) to prevent layers from collapsing to preview bounds on connect or slider tweak.
  - [x] Formalize document canvas coordinate space vs individual layer transforms in [`GRAPH_COORDINATES.md`](./GRAPH_COORDINATES.md).
- [ ] **Graph Topology Improvements (`src/graph.rs`)**:
  - [x] Add graph queries for upstream sources, downstream dependents, and cycle paths.
  - [x] Test multi-hop `Route` traversal, cycle-path detection, diamond evaluation, and reconnection invalidation.

### Phase 3: Worker & ML Inference Pipeline

- [x] **Job Queue & Dispatcher (`src/worker.rs`)**:
  - [x] Split worker transport into typed Render/Inference requests and responses.
  - [x] Cancel invalidated queued jobs before execution; preserve stale-result token matching.
  - [x] Show the active node type and job count in the title bar.
  - [x] Add a mockable `InferenceBackend::predict` boundary for background removal.
  - [x] Keep cached models resident while active and unload them after ten idle minutes or when caching is disabled.
  - [x] Add `OutputCacheInterface` with disk and in-memory implementations.
  - [x] Test mock backend substitution and worker cancellation of queued requests.
  - [x] Add an integration test verifying UI responsiveness under simulated worker load.
- [x] **GPU Compute Acceleration Pipeline**:
  - [x] Reuse the existing WGPU device and queue for color compute passes.
  - [x] Port Curves LUT and Color Balance to WGSL with CPU fallbacks.
  - [x] Port source-over transformed multi-layer composites to WGSL with CPU fallback.
  - Other blend modes continue on CPU to preserve their current output.

### Phase 4: Node UI, Catalog & Session Modernization

- [ ] **UI Body Standardization (`src/nodes/`)**:
  - [ ] Create standardized component builders for node bodies (consistent headers, preview boxes, sliders).
  - [ ] Standardize error/status badge displays across AI inference nodes (`RemoveBg`, `FaceRestore`).
- [ ] **Catalog & Extensibility (`src/catalog.rs`)**:
  - [ ] Refactor `NodeSpec` to support dynamic registration and auto-discovery.
  - [ ] Ensure category tags / grouping in the right-click node spawn menu (`graph_menu.rs`).
- [ ] **Session Schema Versioning & Forward Migration (`src/session.rs`)**:
  - [ ] Introduce explicit `schema_version` to `.rembg.json` root.
  - [ ] Create structured migration handlers (`migrate_v0_to_v1`, etc.) replacing scattered ad-hoc string aliases.
- [ ] **Graph Undo / Redo Command History**:
  - [ ] Implement command pattern stack for node additions, wire changes, parameter scrubbing, and canvas layer transforms.

### Phase 5: Verification, Benchmarking & Documentation

- [x] **Regression & Compatibility Testing**:
  - [x] Run full test suite (`cargo test -p compnode`) with 100% pass rate.
  - [x] Verify backward compatibility by loading older `.rembg.json` session files without errors.
- [ ] **Automated Fuzzing & Stress Testing**:
  - [ ] Add random graph topology fuzz testing (connecting/disconnecting arbitrary pins to ensure no panics).
  - [ ] Golden image regression test suite comparing CPU and GPU outputs.
- [ ] **Performance Benchmarks**:
  - [ ] Re-run release benchmarks for all filter nodes on 4K/12MP images.
  - [ ] Profile memory usage of `ImageMemory` under multi-image workloads.
- [x] **Documentation**:
  - [x] Update `README_AG.md` with final architectural changes.
  - [x] Verify `NODE.md` reflects any updated traits or catalog registration APIs.

## Risk Mitigation

- **Backward compatibility**: Session format remains unchanged (JSON with same keys)
- **Settings persistence**: Versionless JSON, new fields added without breaking old files
- **Incremental migration**: Each phase is independently verifiable
- **Feature flags**: Can gate new abstractions behind feature flags if needed
- **Rollback plan**: Git branches per phase, easy to revert

## Success Metrics

1. `cargo check -p compnode` passes cleanly
2. All existing tests continue to pass
3. No behavior changes for end users
4. New modules are independently testable
5. Code organization makes it easier to add new node types
6. Reduced cyclomatic complexity in main files

## Notes

- The `NODE_SPECS` constant in `catalog.rs` is the single source of truth for node registry - keep it as the canonical registration point
- Maintain the `Kind` enum in `model.rs` for session compatibility
- The `eval.rs` signature hash must remain consistent for cache compatibility
- Worker job tokens must continue to guard against stale results
- Preview dimensions and caching strategy remain unchanged

---

## Node System Redesign Proposal (Streamlined Node Creation)

### 1. Problem Statement & Motivation

Currently, adding a single generic filter node to CompNode requires "shotgun surgery" across **4 separate files and 5 distinct locations**:
1. `src/model.rs` — Declare `FilterKind` variant, string mapping in `id()`, deserializer in `from_id()`, and default parameter map in `defaults()`.
2. `src/ops/<submodule>.rs` — Implement the pixel manipulation function.
3. `src/ops/mod.rs` — Add a match arm to `ops::evaluate()`.
4. `src/catalog.rs` — Define `FieldSpec` array and construct `NodeSpec` with repetitive boilerplate (`inputs: 1, outputs: 1, body_width: NODE_W`, standard socket array, `create` callback, `body` callback).

#### Core Pain Points
- **Parameter Duplication**: Parameter keys and default values are defined twice (in `catalog.rs` for UI sliders and in `model.rs` for default maps). A dedicated test is required just to verify they stay in sync.
- **Scattered Logic**: The UI controls, metadata, and pixel operations live in different directories.
- **Boilerplate Fatigue**: Standard single-input, single-output filters repeat ~15 lines of identical structure in `NODE_SPECS`.

---

### 2. Proposed Architecture

The redesigned node system establishes a **single source of truth** and **locality of behavior** while maintaining 100% backward compatibility with existing saved sessions.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                       Self-Contained Node Module                           │
│                       (e.g., src/nodes/filters/sepia.rs)                    │
│                                                                             │
│  1. Metadata (id, title, icon, accent, keywords)                           │
│  2. Fields with embedded defaults (min, max, default, step, suffix)        │
│  3. Pixel operation (pure function pointer)                                 │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ exports FilterSpec
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Catalog / Registry (catalog.rs)                    │
│  - Generates NodeSpec with sensible defaults (1 in, 1 out, standard body)   │
│  - Automatically derives FilterKind::defaults() directly from fields        │
│  - Directly executes eval function pointer (no match table in ops/mod.rs)   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

### 3. Key Design Components

#### Component A: Self-Defaulting `FieldKind`
Embed the default value directly into each field definition so `FilterKind::defaults()` is auto-derived:

```rust
pub(crate) enum FieldKind {
    Range {
        min: f32,
        max: f32,
        default: f32, // ← Single source of truth for defaults
        step: f64,
        suffix: &'static str,
    },
    Number {
        min: f32,
        max: f32,
        default: f32,
        step: f64,
    },
    Color {
        default: &'static str,
    },
    Select {
        options: &'static [SelectOption],
        default: &'static str,
    },
}

impl FieldSpec {
    pub fn apply_default(&self, params: &mut Params) {
        match &self.kind {
            FieldKind::Range { default, .. } | FieldKind::Number { default, .. } => {
                params::set_f32(params, self.key, *default);
            }
            FieldKind::Color { default } | FieldKind::Select { default, .. } => {
                params::set_string(params, self.key, *default);
            }
        }
    }
}
```

#### Component B: Direct `EvalFn` Pointer on `NodeSpec`
Eliminate the manual switchboard in `ops::evaluate()`. Instead, attach the evaluation function pointer to the specification:

```rust
pub(crate) type EvalFn = fn(
    inputs: &[Option<&RgbaImage>],
    params: &Params,
    canvas: (u32, u32),
) -> Result<RgbaImage, String>;

pub(crate) struct NodeSpec {
    // ... metadata fields ...
    pub eval: Option<EvalFn>,
}
```

Now `ops::evaluate()` becomes generic:
```rust
pub(crate) fn evaluate(
    kind: FilterKind,
    params: &Params,
    inputs: &[Vec<RgbaImage>],
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    let spec = catalog::spec(Kind::Filter(kind));
    let eval_fn = spec.eval.ok_or_else(|| "Node has no eval function".to_owned())?;
    let primary_inputs = [
        inputs.first().and_then(|l| l.first()),
        inputs.get(1).and_then(|l| l.first()),
    ];
    eval_fn(&primary_inputs, params, canvas)
}
```

#### Component C: NodeSpec Builder Pattern
Provide smart defaults for filter nodes so standard 1-in/1-out filters require only essential attributes:

```rust
impl NodeSpec {
    pub const fn filter(
        kind: FilterKind,
        title: &'static str,
        icon: char,
        accent: Color32,
        keywords: &'static [&'static str],
        fields: &'static [FieldSpec],
        eval: EvalFn,
    ) -> Self {
        Self {
            kind: Kind::Filter(kind),
            title,
            icon,
            keywords,
            accent,
            inputs: 1,
            outputs: 1,
            body_width: NODE_W,
            multi_input: false,
            hidden: false,
            fields,
            sockets: &[SocketSpec { label: "", multi: false }],
            create: nodes::filter::create,
            body: nodes::filter::show_body,
            eval: Some(eval),
        }
    }
}
```

#### Component D: Declarative Macro (`define_filter!`)
For rapid node development, provide an optional declarative macro that generates the complete node definition:

```rust
define_filter! {
    Sepia {
        id: "sepia",
        title: "Sepia",
        icon: icons::FILTER,
        accent: pal::node_header::SAND,
        keywords: &["sepia", "vintage", "retro"],
        fields: [
            FieldSpec::range("amount", "Amount", 0.0, 100.0, 100.0, 1.0, " %"),
        ],
        eval: |inputs, params, _canvas| {
            let src = inputs[0].ok_or("No input image")?;
            let amount = params::f32(params, "amount", 100.0) / 100.0;
            Ok(apply_sepia(src, amount))
        }
    }
}
```

#### Component E: Unbundling `FilterKind` — Promoting `Route`, `Group`, and `Texts` to `model::Kind`

Currently, `FilterKind` contains nodes that are fundamentally **not** generic pixel filters:
- **`Route`** is a structural graph pass-through utility. It has no pixel manipulation, no preview texture, no parameters, and produces no image of its own. Downstream evaluation and layer extractors completely bypass it.
- **`Group`** (and `Blend`) is a **layer compositing container**, not an image filter. Treating it as a `FilterNode` forces `FilterNode` to carry `layers: Vec<ViewerLayer>`, `canvas_size: egui::Vec2`, `job`, `output_path`, `render_state`, and `rendered` fields that 90% of filter nodes never use.
- **`Texts`** is an overlay generator with custom typography, font asset binding, and complex vector layout (drop shadows, dual outlines, padding, alignment), carrying a dedicated UI module (`src/nodes/texts.rs`) rather than generic `FieldSpec` sliders.

##### Proposed Architectural Separation

Move these three variants directly into top-level `Kind` and give them clean, purpose-built `Node` structs:

```rust
// In src/model.rs:
pub(crate) enum Kind {
    Image,
    RemoveBg,
    RemoveBgV2,
    FaceRestore,
    Output,
    Route,       // ← Promoted from FilterKind
    Group,       // ← Promoted from FilterKind
    Texts,       // ← Promoted from FilterKind
    Filter(FilterKind),
}

pub(crate) enum Node {
    Image(ImageNode),
    RemoveBg(Process),
    RemoveBgV2(Process),
    FaceRestore(FaceRestoreNode),
    Output(ViewerNode),
    Route,                       // ← Zero state!
    Group(GroupNode),            // ← Owns layers, canvas_size, render_state
    Texts(TextsNode),            // ← Owns typographic state, font, effects
    Filter(FilterNode),          // ← Clean, lightweight struct!
}
```

##### Benefits:
1. **Lightweight `FilterNode`**: `FilterNode` is stripped down to `{ kind, params, preview, signature, preview_enabled }`. No more dead layer-stack fields on every blur or contrast node.
2. **Eliminates Filter Special-Casing**: Removes awkward checks throughout `eval.rs` and `editor.rs`, such as:
   - `if filter.kind == FilterKind::Route { return false; }`
   - `if kind.is_layer_kind() { ... }`
3. **Session Backward Compatibility**: Handled seamlessly in `session.rs` deserialization:
   ```rust
   // Legacy session compatibility in SessionNodeData::Filter:
   "route" => Kind::Route,
   "mix" | "group" => Kind::Group,
   "texts" | "text" => Kind::Texts,
   other => Kind::Filter(FilterKind::from_id(other)?),
   ```

---

### 4. Before vs. After Comparison

#### Before (Adding a Node Today)
- **Edit 1 (`model.rs`)**: Add `Sepia` to `FilterKind`.
- **Edit 2 (`model.rs`)**: Add `"sepia"` to `FilterKind::id()`.
- **Edit 3 (`model.rs`)**: Add `"sepia"` to `FilterKind::from_id()`.
- **Edit 4 (`model.rs`)**: Add `amount: 100.0` to `FilterKind::defaults()`.
- **Edit 5 (`ops/adjust.rs`)**: Implement `pub fn sepia(...)`.
- **Edit 6 (`ops/mod.rs`)**: Add match arm `FilterKind::Sepia => Ok(adjust::sepia(...))` in `ops::evaluate()`.
- **Edit 7 (`catalog.rs`)**: Define `const SEPIA_FIELDS: &[FieldSpec] = ...`.
- **Edit 8 (`catalog.rs`)**: Add 17-line boilerplate `NodeSpec` in `NODE_SPECS`.

#### After (With Proposed Redesign)
- **Edit 1 (`model.rs`)**: Add `Sepia` to `FilterKind` (kept for serde session typing).
- **Edit 2 (`src/nodes/sepia.rs` or `src/ops/sepia.rs`)**: Create one self-contained specification:
  ```rust
  pub(crate) static SPEC: NodeSpec = NodeSpec::filter(
      FilterKind::Sepia,
      "Sepia",
      icons::FILTER,
      pal::node_header::SAND,
      &["sepia", "vintage"],
      &[FieldSpec::range("amount", "Amount", 0.0, 100.0, 100.0, 1.0, " %")],
      |inputs, params, _| Ok(adjust::sepia(inputs[0].ok_or("No input")?, params)),
  );
  ```
- **Edit 3 (`catalog.rs`)**: Add `&sepia::SPEC` to `NODE_SPECS`.

*Result: Total files reduced from 4 to 2, boilerplate reduced by >75%, zero duplication of defaults, zero chance of missing-default regressions.*

---

### 5. Implementation Roadmap & Current Status

The implementation is tracked in detail in [`CHECKLIST.md`](./CHECKLIST.md).

- [x] **Step 1 (Self-Defaulting FieldKind)**: Added `default` to `FieldKind::Range`, `FieldKind::Number`, `FieldKind::Color`, and `FieldKind::Select`. Rewrote `FilterKind::defaults()` to derive values dynamically from `spec.fields`. Deleted ~120 lines of redundant code in `model.rs`.
- [x] **Step 2 (Execution Decoupling)**: Added `pub eval: Option<EvalFn>` to `NodeSpec`. Refactored `ops::evaluate()` to delegate through `spec.eval`. Removed manual match table from `ops/mod.rs`.
- [x] **Step 3 (Spec Builder)**: Added `NodeSpec::filter(...)` and `NodeSpec::generator(...)` constructors. Simplified all existing entries in `catalog.rs`.
- [x] **Step 4 (Modularization)**: Colocated filter eval closures and field definitions with builder specs in `catalog.rs`, ready for optional per-file extraction as new filters are created.
- [x] **Step 5 (FilterKind Unbundling)**: Promoted `Route`, `Group`, `Blend`, and `Texts` out of `FilterKind` into top-level `Kind` and `Node` variants. Created dedicated `GroupNode` and `TextsNode` structs. Stripped `FilterNode` of layer-stack fields (`layers`, `canvas_size`). Preserved 100% backward compatibility for `.rembg.json` sessions.
- [x] **Verification**: 62 library tests pass (1 ignored benchmark); `cargo check -p compnode` passes.

