//! Visual identity for CompNode.
//!
//! This module owns *all* styling so restyling never touches node logic:
//!
//! * [`palette`] – every colour (`base` swatches, `pal` semantic tokens).
//! * [`icons`]   – the icon font and glyph codepoints.
//! * [`controls`] – the widget kit (buttons, menu rows, sliders, ...), built
//!   on vanilla egui. There is no third-party widget library.
//! * this file – metrics, font sizes, the egui theme and the Snarl style.
//!
//! Node-specific header colours are registered in [`crate::catalog`].

pub(crate) mod controls;
pub(crate) mod icons;
mod palette;
#[cfg(test)]
mod tests;

use egui::{
    Color32, CornerRadius, FontData, FontId, Frame, Margin, Stroke, TextStyle, Vec2,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
    style::{Selection, WidgetVisuals, Widgets},
    vec2,
};
use egui_snarl::ui::{
    BackgroundPattern, PinPlacement, SelectionStyle, SnarlStyle, WireLayer, WireStyle,
};

pub(crate) use palette::{hex, pal};

/// Font sizes (points).
pub(crate) mod font {
    pub const TINY: f32 = 9.0;
    pub const SMALL: f32 = 10.5;
    pub const LABEL: f32 = 11.5;
    pub const BUTTON: f32 = 12.0;
    pub const BODY: f32 = 12.5;
    pub const SECTION: f32 = 13.0;
    pub const HEADING: f32 = 14.5;
    pub const MONO: f32 = 11.0;
    /// Icon glyphs inside menus, buttons and checkboxes.
    pub const ICON: f32 = 13.0;
}

/// Shared geometry.
pub(crate) mod metrics {
    pub const R_CONTROL: u8 = 6;
    pub const R_CARD: u8 = 8;
    pub const R_NODE: u8 = 4;

    pub const BTN_H: f32 = 28.0;
    pub const BTN_H_SMALL: f32 = 22.0;
    pub const MENU_ROW_H: f32 = 24.0;
    pub const MENU_PAD_X: f32 = 8.0;
    pub const MENU_ICON_COL: f32 = 16.0;
    pub const CHECK_SIZE: f32 = 16.0;
    pub const SLIDER_THUMB: f32 = 14.0;
    pub const SLIDER_TRACK: f32 = 6.0;
    pub const TITLEBAR_H: f32 = 32.0;
}

/// Uniform body width for all nodes.
pub(crate) const NODE_W: f32 = 196.0;

/// Shared node body padding. Keeping it here makes all nodes line up.
pub(crate) const BODY_MARGIN: Margin = Margin::symmetric(10, 8);

// ------------------------------------------------------------------ fonts ----

const UI_FONT: &[u8] = include_bytes!("../../assets/fonts/Inter_18pt-Medium.ttf");
/// DejaVu subset (arrows, ✓, ✗, …) plus Lucide icons.
const SYMBOLS_FONT: &[u8] = include_bytes!("../../assets/fonts/Symbols.ttf");

fn install_fonts(ctx: &egui::Context) {
    ctx.add_font(FontInsert::new(
        "Inter",
        FontData::from_static(UI_FONT),
        vec![InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: FontPriority::Highest,
        }],
    ));
    ctx.add_font(FontInsert::new(
        "Symbols",
        FontData::from_static(SYMBOLS_FONT),
        vec![
            // Dedicated family for icons, so the UI font can't shadow them.
            InsertFontFamily {
                family: icons::family(),
                priority: FontPriority::Highest,
            },
            // Fallback so literal arrows / check marks in text still render.
            InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: FontPriority::Lowest,
            },
            InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: FontPriority::Lowest,
            },
        ],
    ));
}

// ------------------------------------------------------------------ theme ----

/// Install fonts and the app-wide egui style (spacing, text, visuals).
pub(crate) fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    install_fonts(ctx);
    ctx.all_styles_mut(apply_style);
}

fn widget(fill: Color32, stroke: Stroke, fg: Stroke, expansion: f32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(metrics::R_CONTROL),
        fg_stroke: fg,
        expansion,
    }
}

fn apply_style(style: &mut egui::Style) {
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(font::HEADING));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(font::BODY));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(font::BUTTON));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(font::SMALL));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(font::MONO));

    let sp = &mut style.spacing;
    sp.item_spacing = vec2(7.0, 6.0);
    sp.button_padding = vec2(10.0, 4.0);
    sp.interact_size = vec2(24.0, 24.0);
    sp.icon_width = 16.0;
    sp.icon_width_inner = 10.0;
    sp.icon_spacing = 6.0;
    sp.combo_width = 120.0;
    sp.text_edit_width = 180.0;
    sp.slider_width = 120.0;
    sp.window_margin = Margin::same(10);
    sp.menu_margin = Margin::same(6);
    sp.indent = 16.0;

    style.interaction.tooltip_delay = 0.35;
    style.interaction.tooltip_grace_time = 0.2;
    // Node headers and labels are drag handles, so never let dragging turn
    // into text selection.
    style.interaction.selectable_labels = false;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = None;
    v.weak_text_color = Some(pal::MUTED);
    v.panel_fill = pal::PANEL;
    v.window_fill = pal::NODE;
    v.window_stroke = Stroke::new(1.0, pal::NODE_STROKE);
    v.window_corner_radius = CornerRadius::same(metrics::R_CARD);
    v.menu_corner_radius = CornerRadius::same(metrics::R_CARD);
    v.extreme_bg_color = pal::INPUT_BG;
    v.faint_bg_color = pal::FAINT_BG;
    v.code_bg_color = pal::INPUT_BG;
    v.hyperlink_color = pal::ACCENT;
    v.warn_fg_color = pal::SOCKET;
    v.error_fg_color = pal::DANGER;
    v.button_frame = true;
    v.striped = false;
    v.selection = Selection {
        bg_fill: Color32::from_rgba_unmultiplied(
            pal::ACCENT.r(),
            pal::ACCENT.g(),
            pal::ACCENT.b(),
            70,
        ),
        stroke: Stroke::new(1.0, pal::ACCENT),
    };
    let border = Stroke::new(1.0, pal::NODE_STROKE);
    v.widgets = Widgets {
        noninteractive: widget(pal::NODE, border, Stroke::new(1.0, pal::TEXT), 0.0),
        inactive: widget(pal::INPUT_BG, border, Stroke::new(1.0, pal::TEXT), 0.0),
        hovered: widget(
            pal::hover(pal::INPUT_BG),
            Stroke::new(1.0, pal::MUTED),
            Stroke::new(1.5, pal::TEXT),
            1.0,
        ),
        active: widget(
            pal::ACTIVE_FILL,
            Stroke::new(1.0, pal::ACCENT),
            Stroke::new(1.5, pal::TEXT),
            1.0,
        ),
        open: widget(
            pal::INPUT_BG,
            Stroke::new(1.0, pal::ACCENT),
            Stroke::new(1.0, pal::TEXT),
            0.0,
        ),
    };
}

// ------------------------------------------------------------------ snarl ----

/// Per-node body frame; the outline turns accent-coloured when selected.
pub(crate) fn node_frame(default: Frame, selected: bool) -> Frame {
    let stroke = if selected {
        Stroke::new(2.0, pal::ACCENT)
    } else {
        Stroke::new(1.0, pal::NODE_STROKE)
    };
    default
        .fill(pal::NODE)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(metrics::R_NODE))
}

/// Per-node header frame, tinted with the node's registered header colour.
pub(crate) fn node_header_frame(default: Frame, header: Color32) -> Frame {
    default
        .fill(header)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius {
            nw: metrics::R_NODE,
            ne: metrics::R_NODE,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin::symmetric(6, 3))
}

/// Configure egui-snarl's canvas, nodes, wires and selection.
pub(crate) fn snarl_style() -> SnarlStyle {
    let mut style = SnarlStyle::new();
    style.min_scale = Some(0.25);
    style.max_scale = Some(2.0);
    // The built-in triangle is replaced by our own chevron in
    // `editor::GraphEditor::show_header`.
    style.collapsible = Some(false);
    style.header_drag_space = Some(Vec2::ZERO);
    // NOTE: `crisp_magnified_text` multiplies the whole egui style by
    // `max_scale`, so enabling it makes nodes render at 2x/2.5x. Keep it off
    // and let the layer transform do the zooming.
    style.crisp_magnified_text = Some(false);

    // Wires: Blender-ish smooth curves behind the nodes.
    style.wire_style = Some(WireStyle::Bezier3);
    style.wire_width = Some(2.0);
    style.wire_layer = Some(WireLayer::BehindNodes);

    // Sockets sit on the node border.
    style.pin_size = Some(10.0);
    style.pin_placement = Some(PinPlacement::Edge);
    style.pin_fill = Some(pal::SOCKET);
    style.pin_stroke = Some(Stroke::new(1.0, pal::SOCKET_STROKE));

    // Canvas: flat dark fill; the viewer draws no grid.
    style.bg_frame = Some(controls::canvas_frame());
    style.bg_pattern = Some(BackgroundPattern::NoPattern);

    // Frames are also overridden per node (see `node_frame` /
    // `node_header_frame`); these are the fallbacks.
    style.node_frame = Some(node_frame(Frame::new(), false));
    style.header_frame =
        Some(node_header_frame(Frame::new(), pal::NODE).inner_margin(Margin::ZERO));

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
