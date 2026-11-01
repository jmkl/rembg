//! Node geometry constants shared by hit-testing, wires and renderers.
pub const HEADER_H: f32 = 26.0;
pub const ROW_H: f32 = 22.0;
pub const BODY_PAD: f32 = 6.0;
pub const DEFAULT_WIDTH: f32 = 180.0;

/// Height of a node with the given pin counts.
#[inline]
pub fn node_height(inputs: usize, outputs: usize) -> f32 {
    HEADER_H + inputs.max(outputs).max(1) as f32 * ROW_H + BODY_PAD
}

/// Vertical offset (from node top) of pin row `i`'s center.
#[inline]
pub fn pin_y(i: usize) -> f32 {
    HEADER_H + (i as f32 + 0.5) * ROW_H
}
