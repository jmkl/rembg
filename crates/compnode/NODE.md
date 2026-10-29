# Guide: How to Add a New Node to CompNode

This guide describes the current workflow for adding generic filters and generators, structural/container nodes, dedicated text nodes, and inference nodes. The `Node` enum remains the storage model; `NodeTrait` provides an object-safe interface to node metadata, ports, parameter validation, and parameter persistence.

---

## Table of Contents
1. [Node Archetypes Overview](#1-node-archetypes-overview)
2. [Step-by-Step: Adding a Generic Filter Node](#2-step-by-step-adding-a-generic-filter-node)
3. [Walkthrough Example: Creating a Sepia Filter](#3-walkthrough-example-creating-a-sepia-filter)
4. [Advanced: Adding a Dedicated / AI Inference Node](#4-advanced-adding-a-dedicated--ai-inference-node)
5. [Key Invariants & Common Pitfalls](#5-key-invariants--common-pitfalls)

---

## 1. Node Archetypes Overview

Nodes in CompNode fall into the following distinct categories:

| Feature | Generic Filter / Generator (`Kind::Filter(FilterKind)`) | Layer Container (`Kind::Group`, `Kind::Blend`) | Typography (`Kind::Texts`) | Pass-Through (`Kind::Route`) | Inference / IO (`Kind::Image`, `Kind::RemoveBg`, etc.) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Typical Use** | Color grading, blurs, procedural textures, layer masks. | Canvas-oriented layer blending, 16 blend modes, full 2D gizmo transforms. | Custom typography, fonts, drop shadows, multi-outlines. | Visual wire routing and graph organization. | Local image loading, BiRefNet, RMBG, GFPGAN, Viewers. |
| **State Struct** | `FilterNode` (kind, params, shared preview, signature, job and full-resolution render state). | `GroupNode` (blend flag, params, layers, canvas size, preview and render state). | `TextsNode` (typographic params, shared preview and render state). | Unit variant `Node::Route` (zero state). | Dedicated structs (`ImageNode`, `Process`, `FaceRestoreNode`, `ViewerNode`). |
| **UI Rendering** | Automatic via `nodes::filter::show_body` driven by `FieldSpec` metadata. | Custom layer UI in `nodes::filter::show_body` or canvas. | Dedicated UI under `nodes::texts::show_body`. | Minimal pass-through UI in `nodes::route`. | Custom UI module under `src/nodes/<name>.rs`. |
| **Evaluation** | Pure `EvalFn` on `NodeSpec`; capped preview evaluation and full-resolution `Job::Filter`. | Full-resolution multi-layer composite via `Job::Composite`. | Text rasterization in `ops/generators/text.rs`; full-resolution `Job::Texts`. | Bypassed; wires resolved via `graph::resolve_route()`. | Custom worker job and model dispatch in `worker.rs`. |
| **Files to Edit** | 3 core files: `model.rs`, an operation under `ops/`, and `catalog.rs`; add tests as appropriate. | Core graph, model, and session files. | `nodes/texts.rs`, `ops/generators/text.rs`, and model/session code when changing state. | Graph topology in `graph.rs`. | Model, catalog, node UI, worker, session, and tests. |

### Shared Node Contract

`NodeTrait` is implemented by the `Node` enum and exposes its catalog definition, input sockets, output count, parameter schema, validation, and parameter serialization/restoration. It adapts the current enum; it is not a plugin registry. Generic session restore merges saved values into constructor defaults and retains unknown keys. Validation is explicit, so loading a session does not silently clamp saved values.

---

## 2. Step-by-Step: Adding a Generic Filter Node

Adding a standard image filter (e.g., *Sepia*, *Vignette*, *Pixelate*, *Threshold*) requires editing only **3 files**:

```
crates/compnode/
├── src/
│   ├── model.rs                ← 1. Register FilterKind, stable id, and from_id
│   ├── ops/color/adjust.rs     ← 2. Implement the pure pixel operation
│   └── catalog.rs              ← 3. Define fields and register NodeSpec::filter()
```

> [!NOTE]
> Current catalog behavior:
> - Parameter default values are embedded directly into `FieldSpec` (`FieldKind::Range`, `FieldKind::Number`, `FieldKind::Color`, `FieldKind::Select`). You do **not** need to manually define default maps in `model.rs`.
> - Evaluation functions are attached directly to `NodeSpec` as function pointers / closures. You do **not** need to touch `ops/mod.rs` to add a match arm to `ops::evaluate()`.

### File 1: `src/model.rs`
1. Add a variant to the `FilterKind` enum:
   ```rust
   pub(crate) enum FilterKind {
       // ...
       Sepia,
   }
   ```
2. Return a stable snake_case identifier in `FilterKind::id()`:
   ```rust
   pub(crate) fn id(self) -> &'static str {
       match self {
           // ...
           Self::Sepia => "sepia",
       }
   }
   ```
3. Parse that identifier back in `FilterKind::from_id()`:
   ```rust
   pub(crate) fn from_id(id: &str) -> Option<Self> {
       Some(match id {
           // ...
           "sepia" => Self::Sepia,
           _ => return None,
       })
   }
   ```

### File 2: `src/ops/<module>.rs`
Implement the image transformation. Keep this pure (no `egui` or graph imports):
```rust
pub(crate) fn sepia(src: &RgbaImage, params: &Params) -> RgbaImage {
    let amount = (params::f32(params, "amount", 100.0) / 100.0).clamp(0.0, 1.0);
    // Pixel transformations...
}
```

### File 3: `src/catalog.rs`
1. Define the UI controls using `FieldSpec` constructors (defaults are embedded directly):
   ```rust
   const SEPIA_FIELDS: &[FieldSpec] = &[FieldSpec::range(
       "amount",
       "Amount",
       0.0,
       100.0,
       100.0,   // ← Embedded default value
       1.0,
       " %",
   )];
   ```
2. Add an entry to the static `NODE_SPECS` slice using the `NodeSpec::filter()` builder with an inline `eval` closure:
   ```rust
   NodeSpec::filter(
       FilterKind::Sepia,
       "Sepia",
       icons::FILTER,
       pal::node_header::SAND,
       &["sepia", "vintage", "retro", "color", "tone"],
       SEPIA_FIELDS,
       |inputs, params, _| Ok(ops::adjust::sepia(ops::primary_input(inputs)?, params)),
   ),
   ```

### Source-free generators

Use `NodeSpec::generator()` for a procedural node with no image inputs. Its evaluation closure receives the document canvas size. The existing Gradient and Noise nodes are generic `FilterKind` nodes and do not need a new top-level `Kind` or `Node` variant:

```rust
NodeSpec::generator(
    FilterKind::Noise,
    "Noise",
    icons::FILTER,
    pal::node_header::OLIVE,
    &["noise", "perlin", "simplex", "texture"],
    NOISE_FIELDS,
    |_, params, canvas| Ok(ops::generate::noise(params, canvas)),
),
```

`FilterKind::defaults()` derives ordinary defaults from `FieldSpec`; special structured defaults (such as the identity Curves channels) stay in the appropriate constructor/default logic.

---

## 3. Walkthrough Example: Creating a Sepia Filter

Here is the exact code to add a working **Sepia** adjustment filter.

### 1. Register in `src/model.rs`
```rust
// 1. In `enum FilterKind`:
pub(crate) enum FilterKind {
    // ...
    Sepia,
}

// 2. In `impl FilterKind::id`:
Self::Sepia => "sepia",

// 3. In `impl FilterKind::from_id`:
"sepia" => Self::Sepia,
```

### 2. Implement the pixel operation in `src/ops/color/adjust.rs`
```rust
/// Apply a sepia tone blend with original pixels based on `amount` (0..100%).
pub(crate) fn sepia(src: &RgbaImage, params: &Params) -> RgbaImage {
    let amount = (params::f32(params, "amount", 100.0) / 100.0).clamp(0.0, 1.0);
    if amount <= 0.0 {
        return src.clone();
    }
    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        let r = pixel[0] as f32;
        let g = pixel[1] as f32;
        let b = pixel[2] as f32;
        let a = pixel[3];

        let sr = (r * 0.393 + g * 0.769 + b * 0.189).min(255.0);
        let sg = (r * 0.349 + g * 0.686 + b * 0.168).min(255.0);
        let sb = (r * 0.272 + g * 0.534 + b * 0.131).min(255.0);

        pixel[0] = (r * (1.0 - amount) + sr * amount).round() as u8;
        pixel[1] = (g * (1.0 - amount) + sg * amount).round() as u8;
        pixel[2] = (b * (1.0 - amount) + sb * amount).round() as u8;
        pixel[3] = a;
    }
    out
}
```

### 3. Register in `src/catalog.rs`
```rust
// 1. Declare field specs with embedded defaults:
const SEPIA_FIELDS: &[FieldSpec] = &[FieldSpec::range(
    "amount",
    "Amount",
    0.0,
    100.0,
    100.0,
    1.0,
    " %",
)];

// 2. Add to `NODE_SPECS` with builder constructor:
pub(crate) static NODE_SPECS: &[NodeSpec] = &[
    // ...
    NodeSpec::filter(
        FilterKind::Sepia,
        "Sepia",
        icons::FILTER,
        pal::node_header::SAND,
        &["sepia", "tone", "vintage", "color"],
        SEPIA_FIELDS,
        |inputs, params, _| Ok(ops::adjust::sepia(ops::primary_input(inputs)?, params)),
    ),
];
```

---

## 4. Advanced: Adding a Dedicated / AI Inference Node

When adding a node that relies on background ONNX models (like super-resolution, depth estimation, or matting), follow these steps:

### 1. Model & Types (`src/model.rs`)
- Add variant to `Kind`: `Kind::SuperResolution`
- Define node state struct or wrap `Process`:
  ```rust
  #[derive(Clone, Default)]
  pub(crate) struct SuperResNode {
      pub scale: u32,
      pub process: Process,
  }
  ```
- Add variant to `Node`: `Node::SuperResolution(SuperResNode)`
- Implement accessors on `Node` (`process()`, `process_mut()`, `texture()`, `cpu()`).

### 2. Custom Node Body (`src/nodes/<name>.rs`)
Create `src/nodes/super_resolution.rs`:
```rust
pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: Kind) {
    snarl.insert_node(pos, Node::SuperResolution(SuperResNode::default()));
}

pub(crate) fn show_body(
    editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    width: f32,
) {
    // Show run button, scale slider, and dispatch worker job via editor.jobs.send(...)
}
```

### 3. Worker Thread Execution (`src/worker.rs`)
- Add a job variant to `enum Job`:
  ```rust
  pub(crate) enum Job {
      // ...
      SuperResolution {
          token: u64,
          node: NodeId,
          input: image::RgbaImage,
          scale: u32,
          source_name: String,
      },
  }
  ```
- In the worker loop, handle model inference, write result to `OutputCache`, and send `JobDone` back over the inflight channel.

### 4. Session Persistence (`src/session.rs`)
- Add variant to `SessionNodeData`:
  ```rust
  pub(crate) enum SessionNodeData {
      // ...
      SuperResolution {
          scale: u32,
          #[serde(default, skip_serializing_if = "Option::is_none")]
          path: Option<String>,
      },
  }
  ```
- Map typed state to and from `SessionNodeData` in the `snapshot()` and `load()` paths. Keep legacy aliases in the load mapping when supporting old session IDs.

---

## 5. Key Invariants & Common Pitfalls

### Invariant 1: Self-Defaulting Fields
Every field defined in `spec.fields` automatically provides its own default value via `FieldSpec::apply_default()`.
The test `tests::every_filter_field_has_a_default_value` verifies that all fields have corresponding defaults populated in `FilterKind::defaults()`.

### Invariant 2: Unique Catalog Titles
`spec.title` must be unique across all nodes registered in `NODE_SPECS`. This is enforced by `tests::every_node_kind_has_a_unique_catalog_entry`.

### Invariant 3: Session `id()` Stability
The string returned by `FilterKind::id()` and matched in `FilterKind::from_id()` is stored directly in `.rembg.json` files. Once a node is released, never change this string or saved graphs will fail to parse that node.

### Invariant 4: Pure Operations in `src/ops/`
Operation modules stay independent of graph and windowing state. Catalog closures connect these operations to capped previews and worker renders.

### Invariant 5: Shared Preview Cache
Image-producing nodes use the common `Preview` type. Whenever CPU image data changes, use `set_cpu()` or `Preview::with_image()` so the cached Normal-tier evaluation copy stays bounded to 256px. Graph evaluation also supports Draft (128px) and High (512px) tiers through `PreviewQuality`. Text raster previews remain at document-canvas resolution so the Viewer does not stretch low-resolution glyphs across the canvas. Route and Output intentionally have no pixel preview.

### Invariant 6: Node Invalidation on Change
In custom node bodies, whenever a parameter or property changes, call:
```rust
crate::graph::invalidate_node(snarl, node);
```
This clears cached preview signatures, notifies downstream nodes to recompute, and updates Viewer layers.
