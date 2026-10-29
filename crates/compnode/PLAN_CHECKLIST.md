# CompNode Architecture Refactoring — Master Checklist

Tracks progress on the overarching refactoring roadmap outlined in [PLAN.md](./PLAN.md) (Sections 1–5, lines 1–209).

> For the completed **Node System Redesign** (lines 210–483), see [CHECKLIST.md](./CHECKLIST.md).

---

## Progress Overview

| Phase | Description | Status | Progress |
| :--- | :--- | :--- | :--- |
| **Baseline** | Quick Fixes & Compiler Health | **Completed** | 100% |
| **Phase 1** | Operations Reorganization & Core Data Foundation | **In Progress** | ~60% (Node unbundling complete) |
| **Phase 2** | Graph & Evaluation Engine Decoupling | **Pending** | 0% |
| **Phase 3** | Worker & ML Inference Pipeline Abstraction | **Pending** | 0% |
| **Phase 4** | Node UI & Catalog Modernization | **Pending** | 25% (Builder pattern done) |
| **Phase 5** | Verification, Benchmarking & Documentation | **In Progress** | ~50% (Tests & docs current) |

---

## Baseline: Quick Fixes & Compiler Health

- [x] Fix pre-existing unit test failure in `tests::preview_textures_are_capped`
- [x] Clean up compiler warnings:
  - [x] Remove unused import `self` in `src/ops/rawfilter.rs`
  - [x] Prefix unused variables `_t` and `_backend` in `src/ops/rf/mod.rs`
  - [x] Annotate/clean dead helper functions in `src/ops/rawfilter.rs`
  - [x] Annotate/clean unused `min_pixels` in `src/ops/rf/mod.rs` and `set_u32` in `src/params.rs`
  - [x] Annotate `SocketSpec` fields
- [x] Verify zero warnings with `cargo check -p compnode`
- [x] Verify all unit tests pass with `cargo test -p compnode` (43 passed)

---

## Phase 1: Operations Reorganization & Core Data Foundation

### 1.1 Unbundling Structural & Layer Nodes (Completed via Node Redesign)
- [x] Promote `Route` to top-level `Kind::Route` and zero-state `Node::Route`
- [x] Promote `Group` and `Blend` to top-level `Kind::Group` / `Kind::Blend` with dedicated `GroupNode`
- [x] Promote `Texts` to top-level `Kind::Texts` with dedicated `TextsNode`
- [x] Strip layer-stack fields (`layers`, `canvas_size`) from `FilterNode`
- [x] Ensure 100% backward compatibility for `.rembg.json` session files

### 1.2 Operations Module Reorganization (`src/ops/`)
- [ ] Create domain-focused directory structure:
  - [ ] `src/ops/color/`: HSV, Curves LUT, Exposure, Color Balance, Gradient Map, Invert
  - [ ] `src/ops/composite/`: 16 CSS blend modes, layer stacking, bilinear transforms, alpha masks
  - [ ] `src/ops/generators/`: Gradients, Perlin/Simplex/FBM noise, text renderer
  - [ ] `src/ops/rf/`: Maintain dual CPU/GPU Raw Filter pipeline
- [ ] Consolidate shared math & color utilities:
  - [ ] Standardize luma calculations across modules (Rec. 709 vs Rec. 601)
  - [ ] Standardize color clamping and integer-float conversions
  - [ ] Shared bilinear/bicubic sampling helpers

### 1.3 Core Node Trait Abstraction (Optional / Next Milestone)
- [ ] Define `NodeTrait` for dynamic node dispatch:
  - [ ] Sockets / port definitions
  - [ ] Parameter schema and validation
  - [ ] Serialization hooks
- [ ] Standardize `Preview` structure and caching bounds across all node types

---

## Phase 2: Graph & Evaluation Engine Decoupling

### 2.1 Graph & Evaluation Decoupling (`src/eval.rs`)
- [ ] Introduce `EvalContext` trait to eliminate direct `snarl[node]` indexing in `eval.rs`:
  - [ ] Encapsulate node input lookups and route traversal
  - [ ] Provide immutable view of graph topology during evaluation
- [ ] Encapsulate signature computation:
  - [ ] Isolate hashing of inputs, parameters, and canvas dimensions
  - [ ] Make signature generation and cache invalidation unit-testable in isolation
- [ ] Decouple evaluation downscaling:
  - [ ] Move `ops::EVAL_MAX_EDGE` preview clamping into a configurable resolution strategy
  - [ ] Support custom preview quality tiers (e.g., draft 128px, normal 256px, high-res 512px)

### 2.2 Graph Topology Improvements (`src/graph.rs`)
- [ ] Create formal graph query API:
  - [ ] `upstream_sources(node)`
  - [ ] `downstream_dependents(node)`
  - [ ] `find_cycle(from, to)`
- [ ] Unit tests for complex topologies:
  - [ ] Multi-hop `Route` chains (`Node -> Route -> Route -> Route -> Viewer`)
  - [ ] Diamond dependency evaluation
  - [ ] Reconnection and invalidation cascade

---

## Phase 3: Worker & ML Inference Pipeline Abstraction

### 3.1 Job Queue & Dispatcher Modernization (`src/worker.rs`)
- [ ] Refactor `Job` enum into structured request/response types:
  - [ ] `RenderRequest` / `InferenceRequest`
  - [ ] `RenderResponse` / `InferenceResponse`
- [ ] Formalize token cancellation:
  - [ ] Explicit job cancellation token / atomic flag for aborted preview renders
  - [ ] Discard outdated jobs before model inference begins

### 3.2 Inference Backend Abstraction
- [ ] Define `InferenceBackend` trait for AI models:
  - [ ] Lifecycle management (e.g. keep-alive timeout, unload on low memory)
  - [ ] Standardized execution signature: `predict(&self, input: &RgbaImage) -> Result<RgbaImage, InferenceError>`
  - [ ] Decouple `rembg-core` bindings to allow mock backends in tests
- [ ] Mock worker test suite:
  - [ ] Add integration test verifying UI non-blocking behavior under simulated worker load
  - [ ] Test rapid parameter scrubbing and token invalidation

### 3.3 Cache Interface Abstraction (`src/cache.rs`)
- [ ] Define `OutputCacheInterface` trait:
  - [ ] `get(key: &CacheKey) -> Option<RgbaImage>`
  - [ ] `put(key: &CacheKey, image: &RgbaImage) -> Result<PathBuf, CacheError>`
- [ ] Enable in-memory cache backend for automated headless tests without disk writes

---

## Phase 4: Node UI & Catalog Modernization

### 4.1 UI Component Standardization (`src/nodes/`)
- [ ] Create reusable UI layout helpers:
  - [ ] Standardized node header with status indicator and bypass button
  - [ ] Standardized preview thumbnail container with zoom/pan tooltip
  - [ ] Standardized error / warning banner for failed inferences or missing inputs
- [ ] Uniform slider and control spacing across all node bodies

### 4.2 Catalog & Node Discovery (`src/catalog.rs`, `src/graph_menu.rs`)
- [x] NodeSpec builder constructors (`NodeSpec::filter`, `NodeSpec::generator`)
- [ ] Grouped context menu categories in `graph_menu.rs`:
  - [ ] Input / Sources (Image, GPasTe Bridge)
  - [ ] AI Tools (Remove Background, Remove Background v2, Face Restore)
  - [ ] Color & Grading (HSV, Curves, Exposure, Color Balance, Gradient Map, Invert, Raw Filter)
  - [ ] Generators (Gradient, Noise, Text)
  - [ ] Compositing & Layers (Group, Blend, Mask)
  - [ ] Utilities (Route, Output / Viewer)
- [ ] Node search improvements:
  - [ ] Fuzzy search over titles and keywords
  - [ ] Recently used node suggestions

---

## Phase 5: Polish, Benchmarking & Documentation

### 5.1 Benchmarking & Performance
- [ ] Automated image processing benchmarks:
  - [ ] Benchmark all filter operations on standard test images (1080p, 4K)
  - [ ] Benchmark GPU vs CPU Raw Filter crossover point
  - [ ] Measure composite throughput on multi-layer compositions
- [ ] Memory profiling:
  - [ ] Verify `ImageMemory` budget enforcement (512 MB cap) under 20+ open source images
  - [ ] Check for texture memory leaks across repeated node creations/deletions

### 5.2 Documentation & Verification
- [x] Maintain up-to-date [README_AG.md](./README_AG.md)
- [x] Maintain up-to-date [NODE.md](./NODE.md) guide for creating nodes
- [x] Maintain [CHECKLIST.md](./CHECKLIST.md) for node system redesign
- [x] 100% backward compatibility for all existing `.rembg.json` sessions
