# Guide: How to Add a New Node to CompNode

This guide explains the complete architecture and workflow for adding new nodes to the `compnode` crate, covering both **Generic Filter Nodes** (the standard path for image adjustments, procedural generators, and effects) and **Dedicated Inference Nodes** (for background AI models).

---

## Table of Contents
1. [Node Archetypes Overview](#1-node-archetypes-overview)
2. [Step-by-Step: Adding a Generic Filter Node](#2-step-by-step-adding-a-generic-filter-node)
3. [Walkthrough Example: Creating a Sepia Filter](#3-walkthrough-example-creating-a-sepia-filter)
4. [Advanced: Adding a Dedicated / AI Inference Node](#4-advanced-adding-a-dedicated--ai-inference-node)
5. [Key Invariants & Common Pitfalls](#5-key-invariants--common-pitfalls)

---

## 1. Node Archetypes Overview

Nodes in CompNode fall into one of two categories:

| Feature | Generic Filter Node (`FilterKind`) | Dedicated / Inference Node (`Kind`) |
| :--- | :--- | :--- |
| **Typical Use** | Color grading, blurs, procedural textures, layer masks, distortions. | ONNX neural networks (e.g. background removal, face restoration), source loaders. |
| **UI Rendering** | Automatic via `nodes::filter::show_body` driven by `FieldSpec` metadata. | Custom UI module under `src/nodes/<name>.rs`. |
| **Evaluation** | Evaluated on UI thread during preview (`<= 256px`), full-res automatically scheduled via worker `Job::Filter`. | Custom worker `Job` variant + ONNX runtime dispatch in `worker.rs`. |
| **Persistence** | Automatically serialized as `SessionNodeData::Filter` with string `id()`. | Dedicated variant in `SessionNodeData` in `session.rs`. |
| **Files to Edit** | 4 core files (`model.rs`, `ops/<op>.rs`, `ops/mod.rs`, `catalog.rs`). | 6+ files (`model.rs`, `catalog.rs`, `nodes/<name>.rs`, `worker.rs`, `session.rs`, `app.rs`). |

---

## 2. Step-by-Step: Adding a Generic Filter Node

To add a standard image processing node (e.g. *Sepia*, *Vignette*, *Pixelate*, *Threshold*), modify these four files:

```
crates/compnode/
├── src/
│   ├── model.rs          ← 1. Register FilterKind variant, id, and parameter defaults
│   ├── ops/
│   │   ├── adjust.rs     ← 2. Implement pure pixel manipulation function
│   │   └── mod.rs        ← 3. Dispatch kind in `ops::evaluate()`
│   └── catalog.rs        ← 4. Define UI fields, socket count, and register in `NODE_SPECS`
```

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
4. Define default values in `FilterKind::defaults()`:
   ```rust
   Self::Sepia => {
       params::set_f32(&mut params, "amount", 100.0);
   }
   ```

### File 2: `src/ops/<module>.rs`
Implement the image transformation. Keep this pure (no egui or graph imports):
```rust
pub(crate) fn sepia(src: &RgbaImage, params: &Params) -> RgbaImage {
    let amount = params::f32(params, "amount", 100.0).clamp(0.0, 100.0) / 100.0;
    // Pixel transformations...
}
```

### File 3: `src/ops/mod.rs`
Wire the function into `ops::evaluate()`:
```rust
match kind {
    // ...
    FilterKind::Sepia => Ok(adjust::sepia(first(0)?, params)),
}
```

### File 4: `src/catalog.rs`
1. Define the UI controls using `FieldSpec`:
   ```rust
   const SEPIA_FIELDS: &[FieldSpec] = &[FieldSpec {
       key: "amount",
       label: "Amount",
       kind: FieldKind::Range {
           min: 0.0,
           max: 100.0,
           step: 1.0,
           suffix: " %",
       },
   }];
   ```
2. Add an entry to the static `NODE_SPECS` slice:
   ```rust
   NodeSpec {
       kind: Kind::Filter(FilterKind::Sepia),
       title: "Sepia",
       icon: icons::FILTER,
       keywords: &["sepia", "vintage", "retro", "color", "tone"],
       accent: pal::node_header::SAND,
       inputs: 1,
       outputs: 1,
       body_width: NODE_W,
       multi_input: false,
       hidden: false,
       fields: SEPIA_FIELDS,
       sockets: &[SocketSpec { label: "", multi: false }],
       create: nodes::filter::create,
       body: nodes::filter::show_body,
   },
   ```

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

// 4. In `impl FilterKind::defaults`:
Self::Sepia => {
    params::set_f32(&mut params, "amount", 100.0);
}
```

### 2. Implement the pixel operation in `src/ops/adjust.rs`
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

### 3. Connect to dispatch in `src/ops/mod.rs`
```rust
pub(crate) fn evaluate(
    kind: FilterKind,
    params: &Params,
    inputs: &[Vec<RgbaImage>],
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    // ...
    match kind {
        // ...
        FilterKind::Sepia => Ok(adjust::sepia(first(0)?, params)),
    }
}
```

### 4. Register in `src/catalog.rs`
```rust
// 1. Declare field specs:
const SEPIA_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "amount",
    label: "Amount",
    kind: FieldKind::Range {
        min: 0.0,
        max: 100.0,
        step: 1.0,
        suffix: " %",
    },
}];

// 2. Add to `NODE_SPECS`:
pub(crate) static NODE_SPECS: &[NodeSpec] = &[
    // ...
    NodeSpec {
        kind: Kind::Filter(FilterKind::Sepia),
        title: "Sepia",
        icon: icons::FILTER,
        keywords: &["sepia", "tone", "vintage", "color"],
        accent: pal::node_header::SAND,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: SEPIA_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
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
- Map to/from `SessionNodeData` in `SessionNode::to_session` and `from_session`.

---

## 5. Key Invariants & Common Pitfalls

### Invariant 1: Field Defaults Match
Every field defined in `spec.fields` **must** have an entry in `kind.defaults()`:
```rust
// Verified by `tests::every_filter_field_has_a_default_value`
assert!(defaults.contains_key(field.key));
```
If a `FieldSpec` key does not exist in `FilterKind::defaults()`, the test will fail and sliders will panic or fail to initialize on graph load.

### Invariant 2: Unique Catalog Titles
`spec.title` must be unique across all nodes registered in `NODE_SPECS`. This is enforced by `tests::every_node_kind_has_a_unique_catalog_entry`.

### Invariant 3: Session `id()` Stability
The string returned by `FilterKind::id()` and matched in `FilterKind::from_id()` is stored directly in `.rembg.json` files. Once a node is released, never change this string or saved graphs will fail to parse that node.

### Invariant 4: Pure Operations in `src/ops/`
Functions in `src/ops/` must remain pure pixel manipulation functions with no dependencies on `egui`, `egui-snarl`, or windowing state. This ensures they can be safely called both by the UI thread for preview thumbnails and by the background thread for full-resolution exports.

### Invariant 5: Node Invalidation on Change
In custom node bodies, whenever a parameter or property changes, call:
```rust
crate::graph::invalidate_node(snarl, node);
```
This clears cached preview signatures, notifies downstream nodes to recompute, and updates Viewer layers.
