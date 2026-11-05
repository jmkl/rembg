//! The draw-list boundary: everything the editor wants painted, as plain
//! primitives in graph space. Hosts (egui, a test, ...) turn it into pixels.
use compnode_core::{Color, Rect, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    RoundedRect {
        rect: Rect,
        radius: f32,
        fill: Color,
        stroke: Option<(Color, f32)>,
    },
    LineStrip {
        points: Vec<Vec2>,
        color: Color,
        width: f32,
    },
    Triangle {
        p: [Vec2; 3],
        color: Color,
    },
    /// `pos.x` is the anchor for `align`, `pos.y` the vertical center.
    Text {
        pos: Vec2,
        text: String,
        size: f32,
        color: Color,
        align: Align,
    },
    /// A host texture. `texture` is the id the host associated with the image.
    Image {
        rect: Rect,
        texture: u64,
        tint: Color,
    },
    /// Bilinear four-corner gradient (top-left, top-right, bottom-left, bottom-right).
    GradientQuad {
        rect: Rect,
        colors: [Color; 4],
    },
    /// Two-color checkerboard (shown behind transparency).
    Checker {
        rect: Rect,
        cell: f32,
        a: Color,
        b: Color,
    },
}

/// Stable back-to-front drawing order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Layer {
    Grid,
    Wires,
    Nodes,
    Overlay,
    Popup,
}

impl Layer {
    pub const ALL: [Layer; 5] = [
        Layer::Grid,
        Layer::Wires,
        Layer::Nodes,
        Layer::Overlay,
        Layer::Popup,
    ];
}

#[derive(Default, Clone, Debug)]
pub struct DrawList {
    layers: [Vec<Prim>; 5],
}

impl DrawList {
    pub fn new() -> DrawList {
        DrawList::default()
    }
    pub fn push(&mut self, layer: Layer, prim: Prim) {
        self.layers[layer as usize].push(prim);
    }
    pub fn layer(&self, layer: Layer) -> &[Prim] {
        &self.layers[layer as usize]
    }
    /// All primitives, back to front.
    pub fn iter(&self) -> impl Iterator<Item = (Layer, &Prim)> {
        Layer::ALL
            .into_iter()
            .flat_map(move |l| self.layers[l as usize].iter().map(move |p| (l, p)))
    }
    pub fn len(&self) -> usize {
        self.layers.iter().map(Vec::len).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn clear(&mut self) {
        for l in &mut self.layers {
            l.clear();
        }
    }

    // Small conveniences used by widgets and the node renderer.
    pub fn rect(&mut self, layer: Layer, rect: Rect, radius: f32, fill: Color) {
        self.push(
            layer,
            Prim::RoundedRect {
                rect,
                radius,
                fill,
                stroke: None,
            },
        );
    }
    pub fn rect_stroked(
        &mut self,
        layer: Layer,
        rect: Rect,
        radius: f32,
        fill: Color,
        stroke: Color,
        width: f32,
    ) {
        self.push(
            layer,
            Prim::RoundedRect {
                rect,
                radius,
                fill,
                stroke: Some((stroke, width)),
            },
        );
    }
    pub fn line(&mut self, layer: Layer, a: Vec2, b: Vec2, color: Color, width: f32) {
        self.push(
            layer,
            Prim::LineStrip {
                points: vec![a, b],
                color,
                width,
            },
        );
    }
    pub fn text(
        &mut self,
        layer: Layer,
        pos: Vec2,
        text: &str,
        size: f32,
        color: Color,
        align: Align,
    ) {
        if !text.is_empty() {
            self.push(
                layer,
                Prim::Text {
                    pos,
                    text: text.to_string(),
                    size,
                    color,
                    align,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_core::geom::vec2;

    #[test]
    fn iterates_back_to_front_regardless_of_push_order() {
        let mut dl = DrawList::new();
        let r = Rect::new(vec2(0.0, 0.0), vec2(1.0, 1.0));
        dl.rect(Layer::Popup, r, 0.0, Color::WHITE);
        dl.rect(Layer::Grid, r, 0.0, Color::BLACK);
        dl.line(
            Layer::Wires,
            vec2(0.0, 0.0),
            vec2(1.0, 1.0),
            Color::WHITE,
            1.0,
        );
        dl.rect(Layer::Nodes, r, 0.0, Color::WHITE);
        let order: Vec<Layer> = dl.iter().map(|(l, _)| l).collect();
        assert_eq!(
            order,
            vec![Layer::Grid, Layer::Wires, Layer::Nodes, Layer::Popup]
        );
        assert_eq!(dl.len(), 4);
        dl.clear();
        assert!(dl.is_empty());
    }

    #[test]
    fn empty_text_is_skipped() {
        let mut dl = DrawList::new();
        dl.text(
            Layer::Nodes,
            vec2(0.0, 0.0),
            "",
            12.0,
            Color::WHITE,
            Align::Left,
        );
        assert!(dl.is_empty());
    }
}
