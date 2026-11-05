//! Regression test: a widget in a node's footer row must be reachable.
//!
//! Footer rows live below the pin rows. If the node layout drops them, the
//! widget ends up outside the node body: it is neither drawn nor hit, so
//! dragging it does nothing at all.
use compnode_core::geom::vec2;
use compnode_ui::editor::{Editor, Interaction};
use compnode_ui::input::InputEvent;
use compnode_ui::schema::{Category, NodeSchema, Registry, SocketDef, WidgetKind};
use compnode_ui::value::Value;

/// A node with one output and one footer row holding a slider.
fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(
        NodeSchema::new("t.footer", "Footer", Category::Filter)
            .output(SocketDef::image("Image"))
            .footer(
                SocketDef::float("Amount", 4.0)
                    .range(0.0, 100.0)
                    .widget(WidgetKind::Slider),
            ),
    );
    r
}

fn editor() -> (Editor, u64) {
    let mut e = Editor::new(registry());
    e.set_viewport_size(vec2(800.0, 600.0));
    let uid = e.add_node("t.footer", vec2(0.0, 0.0)).unwrap();
    e.sync();
    (e, uid)
}

/// The footer widget's slot: outputs first, then the footer rows.
const FOOTER_SLOT: usize = 1;

fn footer_value(e: &Editor, uid: u64) -> f32 {
    e.doc.data(uid).unwrap().out_values[FOOTER_SLOT]
        .as_f32()
        .unwrap()
}

#[test]
fn footer_slider_sits_inside_the_node_body() {
    let (e, uid) = editor();
    let rect = e
        .widget_rect(uid, FOOTER_SLOT, true)
        .expect("footer widget rect");
    let node = e.doc.graph.node(e.doc.id_of(uid).unwrap()).unwrap();
    let body = node.rect();
    assert!(
        body.contains_rect(&rect),
        "footer widget {rect:?} outside the node body {body:?}"
    );
}

#[test]
fn footer_slider_can_be_dragged() {
    let (mut e, uid) = editor();
    let rect = e
        .widget_rect(uid, FOOTER_SLOT, true)
        .expect("footer widget rect");
    let before = footer_value(&e, uid);

    // Press inside the slider's control (the row's right-hand part), drag to
    // the right, release.
    let from = e
        .view
        .to_screen(rect.min + vec2(rect.width() * 0.6, rect.height() * 0.5));
    let to = e
        .view
        .to_screen(rect.min + vec2(rect.width() * 0.95, rect.height() * 0.5));
    e.handle_event(&InputEvent::down(from));
    assert!(
        matches!(e.interaction(), Interaction::EditWidget { uid: u, slot: FOOTER_SLOT, output: true } if *u == uid),
        "the footer slider did not capture the press: {:?}",
        e.interaction()
    );
    e.handle_event(&InputEvent::mv(to));
    e.handle_event(&InputEvent::up(to));

    let after = footer_value(&e, uid);
    assert!(
        after > before,
        "the drag did not change the value: {before} -> {after}"
    );
    assert_eq!(
        e.doc.data(uid).unwrap().out_values[FOOTER_SLOT],
        Value::Float(after)
    );
}
