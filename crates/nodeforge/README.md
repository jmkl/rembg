# nodeforge

Headless, renderer-agnostic node-graph **engine** in Rust, built so per-frame
cost scales with what is on screen rather than with graph size.

**Status: engine only.** There is no renderer or UI yet, so this is *not* a
Blender-geometry-nodes-class editor. It is the foundation that such an editor
needs, with the hot paths measured. See "Not done yet".

## What's here
- `arena`  generational ids (ABA-safe)       - `grid`  incremental spatial grid
- `graph`  nodes/wires, O(degree) edits, DAG enforcement (incremental
  topological order), O(V+E) `load_edges` for file loading
- `wire`   Blender-style bezier wires, tolerance-driven flattening, hit
  testing, per-wire polyline cache that only holds visible wires
- `view`   pan/zoom, zoom-about-cursor, level-of-detail, zoom-relaxed tolerance

## Measured (release, single thread, this sandbox; not a GPU/frame benchmark)
`cargo run --release --example bench`

| graph | build | frame @ zoom 1.0 (cull + wires) | frame @ zoom 0.05 (all visible) |
|---|---|---|---|
| 1k nodes / 1.4k wires | 0.9 ms | ~10 us | ~0.13 ms |
| 5k / 7.3k | 5 ms | ~10 us | ~0.93 ms |
| 20k / 29.7k | 20 ms | ~10 us | ~4.2 ms |

Node drag and hover hit-testing are sub-microsecond at all sizes.
These numbers cover CPU-side work only (culling, wire geometry, indexing).
Drawing, text and widgets are not included and will dominate real frame time.

## Not done yet
1. Renderer: wgpu/instanced wires, node/text batching, egui or custom UI shell
2. Interaction layer: selection, box-select, wire dragging, undo/redo, snapping
3. Whole-graph zoomed-out frames still cost O(N); needs GPU-side wire batching
4. Visual design system / theming
5. Not run under clippy/rustfmt here (not installed); built with rustc 1.84

Rules of thumb: bulk-load with `load_edges`, not repeated `connect`, when node
order isn't dependency order.
