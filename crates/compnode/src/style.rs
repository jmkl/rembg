//! Visual identity for CompNode.
//!
//! Keep colours, spacing and widget geometry here so restyling does not require
//! touching node logic. Node-specific headers are registered in [`crate::catalog`].

use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use egui_snarl::ui::{
    BackgroundPattern, PinPlacement, SelectionStyle, SnarlStyle, WireLayer, WireStyle,
};
use elegance::Theme;
/// `0xRRGGBB` -> Color32, so the palette reads like a normal hex color.
const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Raw colors. Name = what it looks like, not where it is used.
/// Scales run dark -> light (950 = darkest).
pub mod base {
    use super::hex;
    use eframe::egui::Color32;

    // Neutral grays
    pub const NEUTRAL_950: Color32 = hex(0x1b1b1b);
    pub const NEUTRAL_900: Color32 = hex(0x1e1e1e);
    pub const NEUTRAL_850: Color32 = hex(0x242424);
    pub const NEUTRAL_800: Color32 = hex(0x282828);
    pub const NEUTRAL_700: Color32 = hex(0x3b3b3b);
    pub const NEUTRAL_650: Color32 = hex(0x4a4a4a);
    pub const NEUTRAL_600: Color32 = hex(0x505050);
    pub const NEUTRAL_500: Color32 = hex(0x8b8b8b);
    pub const NEUTRAL_300: Color32 = hex(0xd8d8d8);
    pub const NEUTRAL_200: Color32 = hex(0xe0e0e0);
    pub const NEUTRAL_100: Color32 = hex(0xf2f2f2);

    // Slate (grays with a slight blue tint, used by the sidebar)
    pub const SLATE_900: Color32 = hex(0x1c1f23);
    pub const SLATE_800: Color32 = hex(0x22262b);
    pub const SLATE_700: Color32 = hex(0x3a3d41);
    pub const SLATE_600: Color32 = hex(0x5a5d61);
    pub const SLATE_400: Color32 = hex(0x8a8d93);

    // Accent / status colors
    pub const BLUE_500: Color32 = hex(0x4776d1);
    pub const GOLD_500: Color32 = hex(0xd1a84b);
    pub const GOLD_950: Color32 = hex(0x241e0b);
    pub const RED_400: Color32 = hex(0xe06c60);
    pub const GREEN_400: Color32 = hex(0x6ab87a);

    // Muted dark hues for node headers
    pub const MOSS: Color32 = hex(0x3d5a47);
    pub const STEEL_BLUE: Color32 = hex(0x394a63);
    pub const BRONZE: Color32 = hex(0x5d4833);
    pub const MAROON: Color32 = hex(0x593737);
    pub const SLATE_PURPLE: Color32 = hex(0x4a4a5a);
    pub const PLUM: Color32 = hex(0x5a3d5a);
    pub const OLIVE: Color32 = hex(0x5a5a3d);
    pub const MULBERRY: Color32 = hex(0x5a3d4a);
    pub const DENIM: Color32 = hex(0x3d4a5a);
    pub const SAND: Color32 = hex(0x5a503d);
    pub const TEAL: Color32 = hex(0x3d5a5a);
    pub const INDIGO: Color32 = hex(0x4a3d5a);
    pub const CLAY: Color32 = hex(0x5a4a3d);
    pub const SAGE: Color32 = hex(0x4a5a4a);
}
/// Muted Blender-inspired palette.
pub(crate) mod pal {
    use crate::style::base;
    use egui::Color32;

    // --- Sidebar / panel palette ---
    pub const SIDEBAR_BORDER: Color32 = base::SLATE_700; // thin dark gray border
    pub const SIDEBAR_LAYER_BG: Color32 = base::SLATE_900; // layer row background
    pub const SIDEBAR_LAYER_BG_HOVER: Color32 = base::SLATE_800; // layer row hover
    pub const SIDEBAR_TEXT: Color32 = base::NEUTRAL_200; // main text
    pub const SIDEBAR_TEXT_MUTED: Color32 = base::SLATE_400; // muted text
    pub const SIDEBAR_DRAG_HANDLE: Color32 = base::SLATE_600; // drag handle dots
    pub const SIDEBAR_ACCENT: Color32 = base::BLUE_500; // accent blue

    // --- General UI ---
    pub const BG: Color32 = base::NEUTRAL_900;
    pub const PANEL: Color32 = base::NEUTRAL_800;
    pub const NODE: Color32 = base::NEUTRAL_850;
    pub const NODE_STROKE: Color32 = base::NEUTRAL_700;
    pub const HEADER_TEXT: Color32 = base::NEUTRAL_100;
    pub const TEXT: Color32 = base::NEUTRAL_300;
    pub const MUTED: Color32 = base::NEUTRAL_500;
    pub const ACCENT: Color32 = base::BLUE_500;
    pub const SOCKET: Color32 = base::GOLD_500;
    pub const SOCKET_STROKE: Color32 = base::GOLD_950;
    pub const CHECK_A: Color32 = base::NEUTRAL_950;
    pub const CHECK_B: Color32 = base::NEUTRAL_850;
    pub const DANGER: Color32 = base::RED_400;
    pub const SUCCESS: Color32 = base::GREEN_400;

    // --- Node header colours. Register these in `catalog::NODE_SPECS`. ---
    pub const HEADER_IMAGE: Color32 = base::MOSS;
    pub const HEADER_REMOVE_BG: Color32 = base::STEEL_BLUE;
    pub const HEADER_FACE_RESTORE: Color32 = base::BRONZE;
    pub const HEADER_VIEWER: Color32 = base::MAROON;
    pub const HEADER_ROUTE: Color32 = base::NEUTRAL_650;
    pub const HEADER_GROUP: Color32 = base::SLATE_PURPLE;
    pub const HEADER_BLEND: Color32 = base::PLUM;
    pub const HEADER_MASK: Color32 = base::OLIVE;
    pub const HEADER_HSV: Color32 = base::MULBERRY;
    pub const HEADER_CURVES: Color32 = base::DENIM;
    pub const HEADER_EXPOSURE: Color32 = base::SAND;
    pub const HEADER_COLOR_BALANCE: Color32 = base::TEAL;
    pub const HEADER_GRADIENT_MAP: Color32 = base::INDIGO;
    pub const HEADER_INVERT: Color32 = base::NEUTRAL_600;
    pub const HEADER_GRADIENT: Color32 = base::CLAY;
    pub const HEADER_NOISE: Color32 = base::SAGE;
}
/// Condensed grotesque UI face (Apache 2.0 / OFL, from google/fonts).
const ROBOTO_CONDENSED: &[u8] = include_bytes!("../assets/fonts/RobotoCondensed-Variable.ttf");

/// Register Roboto Condensed as the primary proportional font.
///
/// Uses `Context::add_font` with the highest priority so elegance's icon
/// font (installed with the lowest priority as a fallback) keeps covering the
/// private-use-area glyphs.
fn install_fonts(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "RobotoCondensed",
        egui::FontData::from_static(ROBOTO_CONDENSED),
        vec![egui::epaint::text::InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

/// Uniform body width for all nodes.
pub(crate) const NODE_W: f32 = 196.0;

/// Install the elegance theme and app-wide egui spacing.
pub(crate) fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    install_fonts(ctx);

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

    theme.typography.body = 12.5;
    theme.typography.button = 12.0;
    theme.typography.label = 11.5;
    theme.typography.small = 10.5;
    theme.typography.heading = 14.5;
    theme.typography.monospace = 11.0;

    theme.control_radius = 6.0;
    theme.card_radius = 8.0;
    theme.control_padding_x = 10.0;
    theme.control_padding_y = 5.0;
    theme.install(ctx);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(7.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.slider_width = 120.0;
        style.spacing.menu_margin = egui::Margin::same(6);
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
