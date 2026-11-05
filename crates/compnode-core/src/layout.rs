//! Node geometry constants shared by hit-testing, wires and renderers.
pub const HEADER_H: f32 = 26.0;
pub const ROW_H: f32 = 22.0;
pub const BODY_PAD: f32 = 6.0;
pub const DEFAULT_WIDTH: f32 = 180.0;

/// Height of a node with the given pin counts (uniform rows).
#[inline]
pub fn node_height(inputs: usize, outputs: usize) -> f32 {
    HEADER_H + inputs.max(outputs).max(1) as f32 * ROW_H + BODY_PAD
}

/// Vertical offset (from node top) of pin row `i`'s center (uniform rows).
#[inline]
pub fn pin_y(i: usize) -> f32 {
    HEADER_H + (i as f32 + 0.5) * ROW_H
}

/// Height of a node whose rows have the given heights.
pub fn node_height_rows(rows: &[f32]) -> f32 {
    if rows.is_empty() {
        return node_height(0, 0);
    }
    HEADER_H + rows.iter().sum::<f32>() + BODY_PAD
}

/// Vertical offset (from node top) of the center of row `i`.
/// Rows past the end continue with [`ROW_H`].
pub fn row_center(rows: &[f32], i: usize) -> f32 {
    let above: f32 = rows.iter().take(i).sum();
    let extra = i.saturating_sub(rows.len()) as f32 * ROW_H;
    let h = rows.get(i).copied().unwrap_or(ROW_H);
    HEADER_H + above + extra + h * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_rows_match_legacy_helpers() {
        let rows = [ROW_H; 3];
        assert_eq!(node_height_rows(&rows), node_height(3, 1));
        for i in 0..3 {
            assert_eq!(row_center(&rows, i), pin_y(i));
        }
    }

    #[test]
    fn tall_rows_shift_following_pins() {
        let rows = [ROW_H, 100.0, ROW_H];
        assert_eq!(row_center(&rows, 1), HEADER_H + ROW_H + 50.0);
        assert_eq!(row_center(&rows, 2), HEADER_H + ROW_H + 100.0 + ROW_H * 0.5);
        assert_eq!(
            node_height_rows(&rows),
            HEADER_H + ROW_H + 100.0 + ROW_H + BODY_PAD
        );
    }
}
