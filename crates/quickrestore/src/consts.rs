//! Single source of truth for the app palette.
//!
//! Three base colors — [`BG`], [`FG`] and [`TEXT`] — plus a handful of derived
//! shades. Everything (the hand-painted HUD and every stock egui widget) reads
//! from here, so the app has one color scheme.

use eframe::egui::style::{Selection, WidgetVisuals};
use eframe::egui::{Color32, CornerRadius, Shadow, Stroke, Visuals};

pub const WIN_W: f32 = 350.0;
pub const WIN_H: f32 = 580.0;
pub const WIN_W_COMPACT: f32 = 200.0;
pub const WIN_H_COMPACT: f32 = 30.0;

/// Frame-rate ceiling for the UI, in frames per second.
pub const MAX_FPS: u32 = 60;
pub const BTN_W: f32 = 30.0;
pub const BTN_H: f32 = 30.0;
pub const GAP: f32 = 4.0;

// --- base palette ---------------------------------------------------------

/// Window / panel background.
pub const BG: Color32 = Color32::from_rgb(19, 19, 19);
/// Foreground: accent used for interaction (hover, focus, selection).
pub const FG: Color32 = Color32::from_rgb(124, 140, 255);
/// Primary text and icon color.
pub const TEXT: Color32 = Color32::from_rgb(214, 220, 235);

// --- derived shades (kept in the same hue family) -------------------------

/// Raised surfaces: buttons, text edits, menus.
pub const SURFACE: Color32 = Color32::from_rgb(26, 29, 38);
/// Surface under the pointer.
pub const HOVER: Color32 = Color32::from_rgb(36, 40, 52);
/// Surface while pressed.
pub const PRESSED: Color32 = Color32::from_rgb(46, 52, 70);
pub const BORDER: Color32 = Color32::from_rgb(40, 45, 58);
/// Secondary text and idle icons.
pub const MUTED: Color32 = Color32::from_rgb(138, 146, 166);
pub const DANGER: Color32 = Color32::from_rgb(232, 72, 85);
/// Positive / "busy and healthy" indicator (the status dot).
pub const SUCCESS: Color32 = Color32::from_rgb(82, 201, 122);
pub const INFO: Color32 = Color32::from_rgb(100, 100, 232);

const RADIUS: u8 = 4;

fn widget(bg_fill: Color32, weak_bg_fill: Color32, border: Color32, fg: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill,
        weak_bg_fill,
        bg_stroke: Stroke::new(1.0, border),
        corner_radius: CornerRadius::same(RADIUS),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

/// The visuals applied to every egui widget.
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();

    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = SURFACE;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = SURFACE;
    v.text_edit_bg_color = Some(SURFACE);

    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(MUTED);
    v.hyperlink_color = FG;
    v.warn_fg_color = FG;
    v.error_fg_color = DANGER;

    v.window_corner_radius = CornerRadius::same(RADIUS);
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;

    v.selection = Selection {
        bg_fill: FG.gamma_multiply(0.35),
        stroke: Stroke::new(1.0, FG),
    };
    v.text_cursor.stroke = Stroke::new(1.0, FG);

    v.widgets.noninteractive = widget(BG, BG, BORDER, TEXT);
    v.widgets.inactive = widget(SURFACE, SURFACE, BORDER, TEXT);
    v.widgets.hovered = widget(HOVER, HOVER, FG, TEXT);
    v.widgets.active = widget(PRESSED, PRESSED, FG, TEXT);
    v.widgets.open = widget(PRESSED, PRESSED, FG, TEXT);

    v
}
