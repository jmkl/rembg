# compnode-editor

An egui front end for the [`compnode`](../compnode-core) node graph. It turns a
headless [`compnode_ui`](../compnode-ui) document into an interactive editor and
evaluates user-defined nodes.

The crate is the **bridge** layer of the project:

| Layer | Crate | Responsibility |
|---|---|---|
| graph model | [`compnode_core`](../compnode-core) | nodes, wires, geometry, spatial index |
| headless editor | [`compnode_ui`](../compnode-ui) | document, schemas, widgets, undo, draw list |
| egui bridge | `compnode-editor` | input, painting, node types, evaluation |

Each layer can be used on its own: `CompnodeEditor` owns a `compnode_ui::Editor`
and an evaluator, and draws it with egui.

- [Running the examples](#running-the-examples)
- [Defining a node](#defining-a-node)
- [Registering nodes](#registering-nodes)
- [Using the editor](#using-the-editor)
- [Evaluation](#evaluation)
- [Image previews and textures](#image-previews-and-textures)
- [Painting primitives](#painting-primitives)
- [Node API reference](#node-api-reference)
- [`ProcessCtx`](#processctx)
- [Related `compnode_ui` types](#related-compnode_ui-types)
- [The editor application](#the-editor-application)

## Running the examples

```text
cargo run -p compnode-editor --example simple_node   # one node, minimal host
cargo run -p compnode-editor --example user_node     # image load + previews
```

`simple_node` is the shortest complete program: it defines a `Scale` node,
registers it and draws the editor. `user_node` shows a host that owns image
pixels, uploads preview textures and handles a button.

The editor application itself is the crate's binary:

```text
cargo run --release -p compnode-editor -- [project.cnp]
cargo run --release -p compnode-editor -- --bench [NODES]
```

## Defining a node

A node is a plain Rust struct implementing [`CompnodeNode`]. The struct owns
runtime state; parameter values shown in the node body live in the document and
are mirrored into the struct through `set_param`, so they survive undo/redo and
saving.

```rust
use compnode_editor::{CompnodeNode, NodeError, Payload, ProcessCtx};
use compnode_ui::schema::{Category, SocketDef};
use compnode_ui::value::{SocketType, Value};

/// Multiplies the value on input 0 by the factor on input 1.
struct Scale {
    factor: f32,
}

impl Default for Scale {
    fn default() -> Self {
        Self { factor: 2.0 }
    }
}

impl CompnodeNode for Scale {
    fn id(&self) -> &'static str {
        "demo.scale"
    }

    fn title(&self) -> &str {
        "Scale"
    }

    fn category(&self) -> Category {
        Category::Utility
    }

    fn inputs(&self) -> Vec<SocketDef> {
        vec![
            SocketDef::new("Value", SocketType::Float),
            SocketDef::float("Factor", self.factor).range(0.0, 10.0),
        ]
    }

    fn outputs(&self) -> Vec<SocketDef> {
        vec![SocketDef::new("Result", SocketType::Float)]
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
        Ok(vec![Payload::value(Value::Float(ctx.float(0) * ctx.float(1)))])
    }

    fn set_param(&mut self, key: &str, value: Value) -> bool {
        if key == "Factor" {
            self.factor = value.as_f32().unwrap_or(self.factor);
            true
        } else {
            false
        }
    }
}
```

If you don't need a custom constructor, `#[derive(Default)]` is enough.
`register_with` accepts a custom `Fn() -> T` constructor instead.

### Adding the node to a graph

```rust
use compnode_editor::{CompnodeEditor, register_nodes};

let mut editor = register_nodes![Scale];
let uid = editor.add_node("demo.scale", Default::default());
assert!(uid.is_some());
```

The type also appears in the add-node menu (right-click or the `Add node`
button in the example host) as soon as it is registered.

## Registering nodes

| Item | Description |
|---|---|
| `register_nodes![A, B, ...]` | Create a `CompnodeEditor` with the listed `Default` node types registered. |
| `CompnodeEditor::register::<T>()` | Register `T` using `T::default`. |
| `CompnodeEditor::register_with::<T>(make)` | Register `T` with a custom `Fn() -> T` constructor. |
| `CompnodeEditor::types()` | The `NodeTypes` registry. |

Registering takes the node's schema from an instance, so the editor knows the
sockets, footer rows and title without any extra description.

`NodeTypes` (also usable on its own):

| Method | Description |
|---|---|
| `NodeTypes::new()` | An empty registry. |
| `register::<T: CompnodeNode + Default>()` | Register a default-constructed type. |
| `register_with::<T>(make)` | Register with a custom constructor. |
| `get(id)` | The `NodeType` registered under `id`. |
| `iter()` | Iterate over all registered `NodeType`s. |
| `len()`, `is_empty()` | Count of registered types. |
| `fill_registry(&mut Registry)` | Copy every schema into a `compnode_ui` registry. |

`NodeType` exposes:

- `schema: NodeSchema` — the schema used to lay out and draw the node.
- `create() -> Box<dyn CompnodeNode>` — a fresh instance.

## Using the editor

Create one with `CompnodeEditor::new()` (or `Default::default()`), register the
node types and draw it from an `eframe::App`:

```rust
use compnode_editor::{CompnodeEditor, register_nodes};

struct App {
    editor: CompnodeEditor,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            self.editor.draw(ui);
        });
    }
}
```

`draw` renders one frame, feeds the pointer and keyboard events from the egui
context into the editor, handles the add-node and text popups, and calls
`evaluate`.

### `CompnodeEditor` reference

| Method | Description |
|---|---|
| `new()` | An editor with no node types registered. |
| `register::<T>()`, `register_with::<T>(make)` | Register node types. |
| `types()` | The `NodeTypes` registry. |
| `document()`, `document_mut()` | The `compnode_ui::Document`: graph, parameters, undo history. |
| `editor()`, `editor_mut()` | The underlying `compnode_ui::Editor`, for hosts that drive it directly (connect wires, selection, view, ...). |
| `nodes()` | The live node instances (`Nodes`, keyed by uid). |
| `node(uid)`, `node_mut(uid)` | One node instance. |
| `evaluator()` | The `Evaluator` (results, errors, dirty set). |
| `canvas()`, `set_canvas((w, h))` | The size of the processed canvas. Changing it drops all results. |
| `add_node(id, pos) -> Option<u64>` | Add a registered node type at a graph position. |
| `evaluate()` | Recompute every dirty node. `draw` does this automatically. |
| `mark_dirty(uid)` | Mark a node and its descendants for recomputation after a manual `node_mut` change. |
| `press_button(uid, slot)` | Deliver a footer button press to the node. `slot` counts output sockets first, then footer rows. |
| `undo()`, `redo()` | Undo or redo one edit. |
| `open_add_menu()` | Open the add-node menu in the middle of the view. |
| `view_all()`, `view_selected()` | Frame the whole graph or the selection. |
| `set_theme(theme)` | Replace the `compnode_ui::theme::Theme`. |
| `draw(ui)` | Draw one frame. |
| `rect()` | The rectangle the graph was drawn in last frame. |
| `set_preview(uid, slot, handle, aspect)` | Show a host image in a preview row (see below). |
| `textures()`, `set_textures(map)` | Read or replace the preview texture map (handle -> `egui::TextureId`). |

Graph editing is driven by the underlying `compnode_ui::Editor`, available
through `editor_mut()`. For example, wiring two nodes:

```rust
# use compnode_editor::CompnodeEditor;
# fn f(editor: &mut CompnodeEditor, a: u64, b: u64) {
editor.editor_mut().connect((a, 0), (b, 0));
# }
```

## Evaluation

The evaluator pulls inputs on demand, caches results per node and recomputes
only what changed. A node is marked dirty when a value changes or the graph
structure changes, and the dirty set is propagated downstream.

| Method / field | Description |
|---|---|
| `Evaluator::new(canvas)` | An evaluator for a `canvas` size. |
| `canvas()`, `set_canvas((w, h))` | Read or change the canvas; changing it clears all results. |
| `clear()` | Forget every result, dirty flag and error. |
| `outputs(uid)` | The `Payload`s of an evaluated node. |
| `error(uid)` | The error message of a node's last evaluation. |
| `forget(uid)` | Drop a node's cached result (used when a node is deleted). |
| `mark_downstream(doc, uid)` | Mark `uid` and its descendants dirty. |
| `apply_changes(doc, changes)` | Feed a document change log in. |
| `evaluate_all(doc, nodes)` | Recompute every dirty node in topological order. |
| `last_evaluated: Vec<u64>` | The nodes recomputed by the most recent `evaluate_all`. |
| `sinks(doc)` | Nodes with no consumers: usually the results a host wants to show. |
| `dependencies(doc, uid)` | Every node that feeds `uid`, directly or indirectly. |
| `error_message(&NodeError)` | The message shown on a failed node. |

`Nodes` is a type alias for the map of live instances:

```rust
pub type Nodes = FxMap<u64, Box<dyn CompnodeNode>>;
```

Reading a node result from a host:

```rust
# use compnode_editor::CompnodeEditor;
# fn f(editor: &CompnodeEditor, uid: u64) {
if let Some(outputs) = editor.evaluator().outputs(uid) {
    let _first = &outputs[0];
}
# }
```

## Image previews and textures

An image preview is not a socket value: the host owns the pixels, uploads them
as an egui texture, and tells the editor which texture belongs to which image
handle. Declare the row in the node's `footer` and give it the
`WidgetKind::ImagePreview` widget:

```rust
# use compnode_ui::schema::{SocketDef, WidgetKind};
# fn footer() -> Vec<SocketDef> {
vec![
    SocketDef::image("Preview").widget(WidgetKind::ImagePreview),
    SocketDef::new("Open", compnode_ui::value::SocketType::Bool)
        .widget(WidgetKind::Button),
]
# }
```

The button row reports `on_button("Open")`; the preview row receives the image
handle from the host:

```rust
# use compnode_editor::CompnodeEditor;
# use compnode_ui::value::ImageHandle;
# use std::collections::HashMap;
# fn f(editor: &mut CompnodeEditor, uid: u64, texture: &egui::TextureHandle) {
// 1. Put the texture in the map the graph draws from. Keep `texture` alive:
//    dropping the handle frees the GPU texture.
let mut textures = HashMap::new();
textures.insert(1, texture.id());
editor.set_textures(textures);

// 2. Point a preview row at the handle.
//    `slot` = output sockets first, then footer rows.
editor.set_preview(uid, 1, Some(ImageHandle(1)), 0.75);
# }
```

**Aspect convention:** `set_preview`/`ImagePreview` take **height / width**, not
width / height. A 1024x1280 image therefore passes `1280.0 / 1024.0`.

**Keep the `TextureHandle` alive.** A bare `egui::TextureId` is only an index
into the texture manager; dropping the `TextureHandle` frees the GPU texture
and the preview shows only the checkerboard. The `user_node` example keeps one
`TextureHandle` per node and updates it with `TextureHandle::set` when a node
produces a new image.

**Cap preview resolution.** A preview row is small, so uploading the full image
wastes GPU memory. `user_node` downscales decoded images to at most **256 px
wide** (`MAX_PREVIEW_WIDTH` in that example) before uploading, while the payload
keeps the full-resolution pixels. This also avoids exhausting wgpu memory with
large images.

The UI draws the texture aspect-fit and centered over a checkerboard, using
`TextureOptions::LINEAR`.

## Painting primitives

The `paint` module converts a `compnode_ui::draw::DrawList` into egui shapes.
Most hosts only need `build` through `CompnodeEditor::draw`, but the helpers are
public for custom overlays.

| Item | Description |
|---|---|
| `Xf { origin, zoom }` | Graph-to-screen transform: `(p - origin) * zoom`. |
| `Xf::IDENTITY` | Zero origin, zoom 1. |
| `c32(Color) -> Color32` | Convert a `compnode_core::Color` to an egui colour. |
| `rounded_outline(rect, radius, segments) -> Vec<Pos2>` | Outline of a rounded rectangle. |
| `polyline(mesh, points, width, color)` | Thick, mitered polyline as a triangle strip. |
| `build(painter, draw_list, xf, textures) -> Vec<Shape>` | Build egui shapes, merging solid runs into meshes while keeping text and images in back-to-front order. |

## Node API reference

### `CompnodeNode`

```rust
pub trait CompnodeNode: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn title(&self) -> &str;
    fn category(&self) -> Category { Category::Utility }
    fn inputs(&self) -> Vec<SocketDef> { Vec::new() }
    fn outputs(&self) -> Vec<SocketDef> { Vec::new() }
    fn footer(&self) -> Vec<SocketDef> { Vec::new() }
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError>;
    fn set_param(&mut self, _key: &str, _value: Value) -> bool { false }
    fn on_button(&mut self, _key: &str) {}
}
```

| Method | Description |
|---|---|
| `id()` | Stable unique id. Written to project files, so don't change it between releases. |
| `title()` | Header title. |
| `category()` | Header colour category. |
| `inputs()` | Input sockets in pin order. Socket `i` is pin `i`. |
| `outputs()` | Output sockets in pin order. The vector returned by `process` is indexed the same way. |
| `footer()` | Widget-only body rows below the pins: previews, buttons, labels. |
| `process(ctx)` | Compute the outputs from the inputs. |
| `set_param(key, value)` | Mirror a document value into the instance. `key` is the socket name; return `true` when stored. |
| `on_button(key)` | A footer button was pressed. `key` is the button's name. Call `mark_dirty` if the button changes state used by `process` (the editor already does this for `press_button`). |

### `Payload`

A cheaply cloned, type-erased value travelling on a wire.

| Method | Description |
|---|---|
| `Payload::new<T: Any + Send + Sync>(value)` | Wrap any value. |
| `Payload::value(Value)` | Wrap a plain socket `Value`. |
| `get::<T>()` | Borrow the concrete value, if it has type `T`. |
| `cloned::<T: Clone>()` | Clone the concrete value, if it has type `T`. |
| `as_value()` | The plain `Value`, if the payload carries one. |
| `type_name()` | The concrete type name, for error messages. |
| `is::<T>()` | Whether the payload holds a `T`. |

`Payload` is `Clone` and `Debug`; cloning shares the inner value.

### `NodeError`

```rust
pub enum NodeError {
    MissingInput(usize),
    TypeMismatch { input: usize, expected: &'static str, got: &'static str },
    Failed(String),
}
```

`NodeError` implements `Display` and `std::error::Error`. The message is shown
on the node and is also returned by `Evaluator::error`.

## `ProcessCtx`

Passed to `CompnodeNode::process`. A connected input yields the payload on the
wire; an unconnected input falls back to the value stored in the document.

| Method | Description |
|---|---|
| `payloads(i)` | Every payload on input `i` in connection order. Multi-input sockets have more than one. |
| `payload(i)` | The first payload on input `i`. |
| `get::<T>(i)` | Borrow the first payload as `T`. |
| `cloned::<T: Clone>(i)` | Clone the first payload as `T`. |
| `all::<T: Clone>(i)` | Every payload on input `i` that is a `T`. |
| `value(i)` | The plain `Value` on the wire, else the widget/document value. |
| `float(i)`, `int(i)`, `bool(i)`, `color(i)`, `string(i)`, `enum_index(i)` | Typed convenience accessors with a sensible fallback. |
| `canvas()` | The `(width, height)` of the processed canvas. |
| `node_id()` | The uid of the node being processed. |

## Related `compnode_ui` types

Node authors also use these re-exported types from `compnode_ui`:

- `schema::Category` — `Input`, `Output`, `Filter`, `Color`, `Blend`,
  `Transform`, `Utility`, `Layout`.
- `schema::SocketDef` — one socket. Constructors and modifiers:

  | Item | Description |
  |---|---|
  | `SocketDef::new(name, SocketType)` | A socket with the type's default value and widget. |
  | `float(name, default)`, `int(name, default)`, `color(name, default)`, `image(name)` | Typed constructors. |
  | `with_default(Value)` | Set the default value. |
  | `widget(WidgetKind)` | Pick the body widget. |
  | `range(min, max)` | Hard limits, also the slider range when no soft range is set. |
  | `soft(min, max)` | Soft slider range. |
  | `step(f32)` | Slider step. |
  | `options(&[&str])` | Options for an enum. |
  | `multi()` | Accept any number of wires. |
  | `custom_widget(id)` | Use a host-provided widget by id. |

- `value::Value` — `Float`, `Int`, `Bool`, `Vec3`, `Color`, `String`, `Enum`,
  `Image(Option<ImageHandle>)`. Helpers: `socket_type()`, `convert(SocketType)`
  (follows the connection rules) and `as_f32()` (numeric view of float, int and
  bool).
- `value::SocketType` — `Float`, `Int`, `Bool`, `Vec3`, `Color`, `String`,
  `Enum`, `Image`, `Any`.
- `value::ImageHandle(u64)` — opaque host image id used by previews.
- `schema::NodeSchema` / `schema::Registry` — the built schema and the
  document's schema registry.
- `theme::Theme` — the editor theme accepted by `CompnodeEditor::set_theme`.

## The editor application

`src/main.rs` is a standalone egui application around `compnode_ui`:

```text
cargo run --release -p compnode-editor -- [project.cnp]
cargo run --release -p compnode-editor -- --bench [NODES]
```

- With a project path it opens that graph.
- `--bench [NODES]` runs a scripted pan/zoom over a large graph and prints
  frame times; `NODES` defaults to 1000.
- The window is a single canvas: the graph, its minimap, the add-node menu, the
  context menu and the inline text field. The application edits graphs; it does
  not process images.
