# Node System Redesign — Implementation Checklist

Tracks progress on the [PLAN.md](./PLAN.md) §"Node System Redesign Proposal" (line 210+).  
For the master roadmap across Phases 1–5, see [PLAN_CHECKLIST.md](./PLAN_CHECKLIST.md).

**Current Branch**: `refactor-node`  
**Overall Status**: Completed (Steps 1, 2, 3, 5 implemented; Step 4 streamlined into builders)

---

## Step 1: Self-Defaulting FieldKind (Zero-Risk Refactor)

- [x] Add `default` field to `FieldKind::Range` and `FieldKind::Number`
- [x] Add `default` field to `FieldKind::Color` and `FieldKind::Select`
- [x] Add `FieldSpec::apply_default()` method
- [x] Add constructors (`FieldSpec::range()`, `FieldSpec::number()`, `FieldSpec::color()`, `FieldSpec::select()`)
- [x] Add defaults to all field constant arrays in `catalog.rs` (HSV, CURVES, EXPOSURE, etc.)
- [x] Rewrite `FilterKind::defaults()` in `model.rs` to auto-derive from `catalog::spec().fields`
- [x] Keep special-case defaults for Curves (identity curve) and Texts (complex params)
- [x] Delete redundant per-kind default code from `model.rs` (~120 lines removed)
- [x] `cargo check -p compnode` passes

## Step 2: EvalFn Pointer on NodeSpec (Execution Decoupling)

- [x] Define `EvalFn` type alias in `catalog.rs` (`fn(&[Option<&RgbaImage>], &Params, (u32, u32)) -> Result<RgbaImage, String>`)
- [x] Add `pub eval: Option<EvalFn>` field to `NodeSpec`
- [x] Attach eval function pointers / closures to each filter `NodeSpec` in `NODE_SPECS`
- [x] Refactor `ops::evaluate()` to delegate through `spec.eval` via `ops::primary_input()`
- [x] Remove the manual match table from `ops/mod.rs`
- [x] Expose `ops` submodules (`adjust`, `generate`, `rawfilter`, `composite`, `rf`, `texts`) for clean specification closures
- [x] `cargo check -p compnode` passes

## Step 3: NodeSpec Builder (Spec Simplification)

- [x] Add `NodeSpec::filter()` constructor with sensible defaults (1 in, 1 out, standard body & sockets)
- [x] Add `NodeSpec::generator()` constructor with 0 inputs
- [x] Simplify standard 1-in/1-out filter entries in `NODE_SPECS`
- [x] Simplify non-standard entries (Mask, Gradient, Noise)
- [x] `cargo check -p compnode` passes

## Step 4: Modularization (Filter Spec Locality)

- [x] Colocate filter eval closures and field definitions directly alongside `NodeSpec::filter()` entries
- [x] Architecture ready for optional per-file filter submodules (`src/nodes/filters/<name>.rs`) as new custom filters are introduced
- [x] `cargo check -p compnode` passes

## Step 5: FilterKind Unbundling (Route, Group, Blend, Texts promotion)

- [x] Promote `Route` to top-level `Kind::Route` and `Node::Route` (zero state)
- [x] Promote `Group` and `Blend` to top-level `Kind::Group` and `Kind::Blend` with `Node::Group(GroupNode)` and `Node::Blend(GroupNode)`
- [x] Promote `Texts` to top-level `Kind::Texts` with `Node::Texts(TextsNode)`
- [x] Create `GroupNode` struct (owns `is_blend`, `layers`, `canvas_size`, `job`, `output_path`, `render_state`, `rendered`, `params`, `preview`, `signature`)
- [x] Create `TextsNode` struct (owns typographic params, preview, signature)
- [x] Strip `FilterNode` of all layer-related fields down to `{ kind, params, preview, signature, preview_enabled }`
- [x] Update `session.rs` for backward-compatible deserialization and serialization (`"route"`, `"mix"`/`"group"`, `"blend"`, `"texts"`/`"text"`)
- [x] Update all `eval.rs` evaluation routines (`ensure_all`, `ensure_filter`, `node_signature`, `filter_signature`, `resolve_image`, `resolve_image_full`)
- [x] Update all `graph.rs` and `editor.rs` special-casing (`start_group_render`, `start_filter_render`, etc.)
- [x] Update `Node` method impls (`layers()`, `layers_mut()`, `canvas_size()`, `set_canvas_size()`, `is_blend()`, `can_render_full_res()`)
- [x] Remove `FilterKind::Route`, `FilterKind::Group`, `FilterKind::Blend`, `FilterKind::Texts` variants from `FilterKind`
- [x] `cargo check -p compnode` passes

## Final Verification

- [x] `cargo check -p compnode` clean (0 errors, 0 warnings)
- [x] `cargo test -p compnode` passes (43 passed; 0 failed)
- [x] Backward compatibility with existing session files preserved and verified
