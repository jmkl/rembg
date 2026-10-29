# CompNode Architecture Refactoring — Master Checklist

Tracks progress on the overarching refactoring roadmap outlined in [PLAN.md](./PLAN.md) (Sections 1–5, lines 1–209).

> For the completed **Node System Redesign** (lines 210–483), see [CHECKLIST.md](./CHECKLIST.md).

---

## Progress Overview

| Phase        | Description                                      | Status          | Progress                                    |
| :----------- | :----------------------------------------------- | :-------------- | :------------------------------------------ |
| **Baseline** | Quick Fixes & Compiler Health                    | **Completed**   | 100%                                        |
| **Phase 1**  | Operations Reorganization & Core Data Foundation | **Completed**   | 100%                                        |
| **Phase 2**  | Graph & Evaluation Engine Decoupling             | **Completed**   | 100%                                        |
| **Phase 3**  | Worker & ML Inference Pipeline Abstraction       | **Completed**  | 100% |
| **Phase 4**  | Node UI, Catalog & Session Modernization         | **Complete**    | UI, discovery, migration and history landed  |
| **Phase 5**  | Verification, Benchmarking & Documentation       | **In Progress** | ~50% (Tests & docs current)                 |

---

## Baseline: Quick Fixes & Compiler Health

- [x] Fix pre-existing unit test failure in `tests::preview_textures_are_capped`
- [x] Clean up compiler warnings:
  - [x] Remove unused import `self` from the legacy Raw Filter wrapper
  - [x] Prefix unused variables `_t` and `_backend` in `src/ops/rf/mod.rs`
  - [x] Remove obsolete `src/ops/rawfilter.rs` and route catalog dispatch directly to `src/ops/rf/`
  - [x] Annotate/clean unused `min_pixels` in `src/ops/rf/mod.rs` and `set_u32` in `src/params.rs`
  - [x] Annotate `SocketSpec` fields
- [x] Verify zero warnings with `cargo check -p compnode`
- [x] Verify all unit tests pass with `cargo test -p compnode` (70 passed, 1 ignored benchmark)

---

## Phase 1: Operations Reorganization & Core Data Foundation

### 1.1 Unbundling Structural & Layer Nodes (Completed via Node Redesign)

- [x] Promote `Route` to top-level `Kind::Route` and zero-state `Node::Route`
- [x] Promote `Group` and `Blend` to top-level `Kind::Group` / `Kind::Blend` with dedicated `GroupNode`
- [x] Promote `Texts` to top-level `Kind::Texts` with dedicated `TextsNode`
- [x] Strip layer-stack fields (`layers`, `canvas_size`) from `FilterNode`
- [x] Ensure 100% backward compatibility for `.rembg.json` session files

### 1.2 Operations Module Reorganization (`src/ops/`)

- [x] Create domain-focused directory structure:
  - [x] `src/ops/color/`: HSV, Curves LUT, Exposure, Color Balance, Gradient Map, Invert
  - [x] `src/ops/composite/`: 16 CSS blend modes, layer stacking, bilinear transforms, alpha masks
  - [x] `src/ops/generators/`: Gradients, Perlin/Simplex/FBM noise, text renderer
  - [x] `src/ops/rf/`: Maintain dual CPU/GPU Raw Filter pipeline
- [x] Consolidate shared math & color utilities in `src/ops/math.rs`:
  - [x] Name Rec. 709, Rec. 601, and CSS blend luminosity calculations explicitly
  - [x] Share normalized color clamping and byte/float conversion helpers
  - [x] Share bilinear and bicubic RGBA sampling helpers

### 1.3 Pixel Semantics & Operation Contracts

- [x] Document encoded-sRGB versus linear-light assumptions at operation boundaries in [`PIXEL_CONTRACT.md`](./PIXEL_CONTRACT.md)
- [x] Document straight-alpha versus premultiplied-alpha expectations for compositing, masks, text, and resampling
- [x] Record rounding, clamping, and edge-sampling rules for shared pixel helpers
- [x] Define behavior for empty images, non-finite parameters, and generator dimensions
- [x] Add deterministic generator and sampler edge-case checks
- [x] Require golden cases and compatibility notes for color-space or alpha behavior changes

### 1.4 Core Node Trait & Lifecycle Abstraction

- [x] Define object-safe `NodeTrait` adapter for dynamic node metadata and parameter dispatch:
  - [x] Expose catalog metadata and socket/port definitions
  - [x] Expose parameter schemas and schema-driven field validation
  - [x] Add parameter serialization and default-preserving restore hooks; use them in session persistence
- [x] **Node Lifecycle & Content Bounding Box Contract**:
  - [x] Register all evaluable nodes (e.g. `TextsNode`) in `can_render_full_res` for disk cache persistence
  - [x] Compute instant alpha bounds crop (`content_uv`) from preview CPU pixels on frame 1
  - [x] Maintain full-res on-disk output path for downstream consumers and export nodes
- [x] Standardize `Preview` access and the 256px evaluation-image cache across image-producing nodes; structural Route and Output nodes intentionally have no pixel preview

---

## Phase 2: Graph & Evaluation Engine Decoupling

### 2.1 Graph & Evaluation Decoupling (`src/eval.rs`)

- [x] Introduce `EvalContext` trait to eliminate direct Snarl node/pin access in `eval.rs`:
  - [x] Encapsulate node input lookups and route traversal
  - [x] Keep topology reads behind the evaluation context interface
- [x] Encapsulate signature computation:
  - [x] Isolate hashing of inputs, parameters, and canvas dimensions
  - [x] Make signature generation deterministic and unit-testable in isolation
- [x] Decouple evaluation downscaling:
  - [x] Move preview clamping into the shared resolution strategy
  - [x] Support Draft (128px), Normal (256px), and High (512px) evaluation tiers

### 2.2 Resolution Negotiation & Transform Stability

- [x] Walk upstream to compute `effective_dimensions` across filter chains
- [x] Prevent temporary preview dimension collapses (e.g. 1024 -> 256) during slider scrubbing or pin wiring
- [x] Formalize document canvas coordinate space vs individual layer transform bounds in [`GRAPH_COORDINATES.md`](./GRAPH_COORDINATES.md)

### 2.3 Graph Topology Improvements (`src/graph.rs`)

- [x] Create formal graph query API:
  - [x] `upstream_sources(node)`
  - [x] `downstream_dependents(node)`
  - [x] `find_cycle(from, to)`
- [x] Unit tests for complex topologies:
  - [x] Multi-hop `Route` chains and cycle-path detection
  - [x] Diamond dependency evaluation
  - [x] Reconnection and invalidation cascade

---

## Phase 3: Worker & ML Inference Pipeline Abstraction

### 3.1 Job Queue & Dispatcher Modernization (`src/worker.rs`)

- [x] Refactor worker job transport into structured request/response types:
  - [x] `RenderRequest` / `InferenceRequest`
  - [x] `RenderResponse` / `InferenceResponse`
- [x] Formalize token cancellation:
  - [x] Shared atomic cancellation token for stale queued worker work
  - [x] Discard invalidated jobs before inference or render work begins
  - [x] Show the active node type and job count in the title bar

### 3.2 Inference Backend Abstraction

- [x] Define a mockable `InferenceBackend` trait for background-removal models:
  - [x] Lifecycle management with configurable model cache policy and a 10-minute idle unload timeout
  - [x] Standardized execution signature: `predict(&self, input: &RgbaImage) -> Result<RgbaImage, InferenceError>`
  - [x] Decouple background-removal `rembg-core` bindings to allow mock backends in tests
- [ ] Mock worker test suite:
  - [x] Add integration test verifying UI non-blocking behavior under simulated worker load
  - [x] Test backend substitution through a mock implementation
  - [x] Test worker cancellation of queued requests
  - [x] Verify an egui frame can run while the worker is held behind simulated load

### 3.3 Cache Interface Abstraction (`src/cache.rs`)

- [x] Define `OutputCacheInterface` trait:
  - [x] `get(key: &CacheKey) -> Option<RgbaImage>`
  - [x] `put(key: &CacheKey, image: &RgbaImage) -> Result<PathBuf, CacheError>`
- [x] Enable in-memory cache backend for automated headless tests without disk writes

### 3.4 GPU Accelerated Compute Pipeline

- [x] Share the existing WGPU device and queue with color compute passes
- [x] Port expensive color grading and multi-layer composites to WGSL compute passes:
  - [x] Curves LUT
  - [x] Color Balance
  - [x] Source-over multi-layer transformed composites
  - Other blend modes retain the CPU path for parity.

---

## Phase 4: Node UI, Catalog & Session Modernization

### 4.1 UI Component Standardization (`src/nodes/`)

- [ ] Create reusable UI layout helpers:
  - [x] Standardized node header with status indicator and functional bypass button
  - [x] Standardized preview thumbnail container with zoom/pan tooltip
  - [x] Reuse the existing error banner for inference failures
- [x] Uniform vertical spacing across node bodies

### 4.2 Catalog & Node Discovery (`src/catalog.rs`, `src/graph_menu.rs`)

- [x] NodeSpec builder constructors (`NodeSpec::filter`, `NodeSpec::generator`)
- [x] Group the context menu by node category
- [x] Fuzzy search over node titles and keywords
- [x] Show recently used node suggestions
- [x] Support runtime catalog registration for existing `Kind` variants

### 4.3 Session Schema Versioning & Migration

- [x] Keep an explicit root session version
- [x] Migrate unversioned / version 0 sessions through `migrate_v0_to_v1`
- [x] Reject unsupported future versions with a clear error
- [x] Supply defaults for newly introduced optional node state

### 4.4 Graph Undo / Redo Command History

- [x] Implement bounded undo / redo history for node and image additions, deletion, duplication, and wire changes
- [x] Record parameter edits and coalesce slider scrubbing
- [x] Record canvas layer transforms and node bypass toggles
- [x] Add Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z and Edit menu commands

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

### 5.2 Automated Fuzzing & Stress Testing

- [ ] Add random graph topology fuzz testing (connecting/disconnecting arbitrary pins to ensure no panics)
- [ ] Golden image regression test suite comparing CPU and GPU outputs
  - [ ] Define operation-specific tolerances and compare alpha separately from RGB
- [ ] Add property/edge-case checks for sampling bounds, alpha preservation, empty inputs, and non-finite parameters

### 5.3 Documentation & Verification

- [x] Maintain up-to-date [README_AG.md](./README_AG.md)
- [x] Maintain up-to-date [NODE.md](./NODE.md) guide for creating nodes
- [x] Maintain [CHECKLIST.md](./CHECKLIST.md) for node system redesign
- [x] 100% backward compatibility for all existing `.rembg.json` sessions
