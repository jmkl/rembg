//! Visual identity for CompNode.
//!
//! Keep colours, spacing and widget geometry here so restyling does not require
//! touching node logic. Node-specific headers are registered in [`crate::catalog`].

use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use egui_snarl::ui::{
    BackgroundPattern, PinPlacement, SelectionStyle, SnarlStyle, WireLayer, WireStyle,
};
use elegance::Theme;

/// Muted Blender-inspired palette.
pub(crate) mod pal {
    use egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    pub const GRID: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
    pub const PANEL: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
    pub const NODE: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
    pub const NODE_STROKE: Color32 = Color32::from_rgb(0x3b, 0x3b, 0x3b);
    pub const HEADER_TEXT: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf2);
    pub const TEXT: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);
    pub const MUTED: Color32 = Color32::from_rgb(0x8b, 0x8b, 0x8b);
    pub const ACCENT: Color32 = Color32::from_rgb(0x47, 0x76, 0xd1);
    pub const SOCKET: Color32 = Color32::from_rgb(0xd1, 0xa8, 0x4b);
    pub const SOCKET_STROKE: Color32 = Color32::from_rgb(0x24, 0x1e, 0x0b);
    pub const CHECK_A: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1b);
    pub const CHECK_B: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
    pub const DANGER: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x60);
    pub const SUCCESS: Color32 = Color32::from_rgb(0x6a, 0xb8, 0x7a);

    // Node header colours. Register these in `catalog::NODE_SPECS`.
    pub const HEADER_IMAGE: Color32 = Color32::from_rgb(0x3d, 0x5a, 0x47);
    pub const HEADER_REMOVE_BG: Color32 = Color32::from_rgb(0x39, 0x4a, 0x63);
    pub const HEADER_FACE_RESTORE: Color32 = Color32::from_rgb(0x5d, 0x48, 0x33);
    pub const HEADER_VIEWER: Color32 = Color32::from_rgb(0x59, 0x37, 0x37);
}

/// Default body width for image/processing nodes.
pub(crate) const BODY_W: f32 = 196.0;
/// Wider body for the multi-layer viewer.
pub(crate) const VIEWER_W: f32 = 320.0;

/// Install the elegance theme and app-wide egui spacing.
pub(crate) fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);

    // Keep the Blender-ish greys, but let elegance drive every widget's shape,
    // typography, focus ring and interaction states.
    let mut theme = Theme::charcoal();
    theme.palette.bg = pal::PANEL;
    theme.palette.card = pal::NODE;
    theme.palette.input_bg = Color32::from_rgb(0x1a, 0x1a, 0x1a);
    theme.palette.border = pal::NODE_STROKE;
    theme.palette.text = pal::TEXT;
    theme.palette.text_muted = pal::MUTED;
    theme.palette.text_faint = Color32::from_rgb(0x6c, 0x6c, 0x6c);
    theme.palette.blue = pal::ACCENT;
    theme.palette.blue_hover = Color32::from_rgb(0x5a, 0x89, 0xe0);
    theme.palette.focus = pal::ACCENT;
    theme.palette.success = pal::SUCCESS;
    theme.palette.danger = pal::DANGER;
    theme.palette.warning = pal::SOCKET;

    theme.typography.body = 12.0;
    theme.typography.button = 11.5;
    theme.typography.label = 11.0;
    theme.typography.small = 10.0;
    theme.typography.heading = 14.0;
    theme.typography.monospace = 11.0;

    theme.control_radius = 5.0;
    theme.card_radius = 8.0;
    theme.control_padding_x = 9.0;
    theme.control_padding_y = 4.0;
    theme.install(ctx);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.slider_width = 120.0;
        // Node headers and labels are drag handles, so never let dragging
        // turn into text selection.
        style.interaction.selectable_labels = false;
    });
}

/// Configure egui-snarl's canvas, nodes, wires and selection.
pub(crate) fn snarl_style() -> SnarlStyle {
    let mut style = SnarlStyle::new();
    style.min_scale = Some(0.25);
    style.max_scale = Some(2.0);
    // The default triangle is replaced by an elegance chevron in
    // `editor::GraphEditor::show_header`.
    style.collapsible = Some(false);
    style.header_drag_space = Some(egui::vec2(0.0, 0.0));
    // NOTE: `crisp_magnified_text` multiplies the whole egui style by
    // `max_scale`, so enabling it makes nodes render at 2x/2.5x. Keep it off
    // and let the layer transform do the zooming.
    style.crisp_magnified_text = Some(false);

    // Wires: Blender-ish smooth curves behind the nodes.
    style.wire_style = Some(WireStyle::Bezier5);
    style.wire_width = Some(2.0);
    style.wire_layer = Some(WireLayer::BehindNodes);

    // Sockets sit on the node border.
    style.pin_size = Some(10.0);
    style.pin_placement = Some(PinPlacement::Edge);
    style.pin_fill = Some(pal::SOCKET);
    style.pin_stroke = Some(Stroke::new(1.0, pal::SOCKET_STROKE));

    // Canvas: flat dark fill, our custom dot grid is drawn by the viewer.
    style.bg_frame = Some(Frame::new().fill(pal::BG));
    style.bg_pattern = Some(BackgroundPattern::NoPattern);

    // Frames are also overridden per node, these are the fallbacks.
    style.node_frame = Some(
        Frame::new()
            .fill(pal::NODE)
            .stroke(Stroke::new(1.0, pal::NODE_STROKE))
            .corner_radius(CornerRadius::same(8)),
    );
    style.header_frame = Some(Frame::new().fill(pal::NODE).corner_radius(CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    }));

    // Rectangle selection uses the accent blue, like Blender's selection box.
    // NOTE: `select_stoke` is misspelled in egui-snarl's public API.
    style.select_stoke = Some(Stroke::new(2.0, pal::ACCENT));
    style.select_fill = Some(pal::ACCENT.gamma_multiply(0.08));
    // Node outlines are drawn by `GraphEditor` instead, because egui-snarl
    // does not select a node on a normal click. Keep Snarl's own node
    // selection invisible so the two highlight sources never disagree.
    style.select_style = Some(SelectionStyle {
        fill: Color32::TRANSPARENT,
        stroke: Stroke::NONE,
        ..Default::default()
    });
    style
}

/// Shared node body padding. Keeping it here makes all nodes line up.
pub(crate) const BODY_MARGIN: Margin = Margin::symmetric(10, 8);
