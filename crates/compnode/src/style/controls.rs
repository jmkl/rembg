//! CompNode's widget kit, built on vanilla egui.
//!
//! Everything elegance used to provide lives here: buttons, menu rows,
//! sliders, checkboxes, selects, colour fields, callouts and window controls.
//! Each widget implements [`egui::Widget`], so call sites read
//! `ui.add(Button::new("Run").small())`. Colours come from [`pal`] only.

use std::ops::RangeInclusive;

use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Pos2, Rect, Response, RichText,
    Sense, Shadow, Stroke, StrokeKind, TextStyle, Ui, Widget, WidgetInfo, WidgetText, WidgetType,
    pos2, vec2,
};

use super::{font, icons, metrics, pal};

// ------------------------------------------------------------------ text ----

/// Secondary text at an explicit size.
pub(crate) fn muted(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text.into()).size(size).color(pal::MUTED)
}

/// Small error-coloured text.
pub(crate) fn danger_text(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).small().color(pal::DANGER)
}

/// Section heading used in dialogs.
pub(crate) fn section_title(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).strong().size(font::SECTION)
}

// ---------------------------------------------------------------- frames ----

pub(crate) fn window_frame() -> Frame {
    Frame::new()
        .fill(pal::PANEL)
        .stroke(Stroke::new(1.0, pal::NODE_STROKE))
        .corner_radius(CornerRadius::same(metrics::R_CARD))
        .inner_margin(Margin::same(14))
}

pub(crate) fn titlebar_frame() -> Frame {
    Frame::new()
        .fill(pal::PANEL)
        .inner_margin(Margin::symmetric(10, 0))
}

pub(crate) fn sidebar_frame() -> Frame {
    Frame::new()
        .fill(pal::PANEL)
        .stroke(Stroke::new(1.0, pal::NODE_STROKE))
}

pub(crate) fn canvas_frame() -> Frame {
    Frame::new().fill(pal::BG).shadow(Shadow {
        offset: [10, 10],
        blur: 12,
        spread: 2,
        color: Color32::from_black_alpha(100),
    })
}

// ---------------------------------------------------------------- button ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Primary,
    Success,
    Danger,
    Outline,
    Trans,
}

/// Filled / outline button. Defaults to the primary (accent) look.
pub(crate) struct Button {
    label: WidgetText,
    tone: Tone,
    small: bool,
    full_width: bool,
    min_width: f32,
    loading: bool,
}

impl Button {
    pub fn new(label: impl Into<WidgetText>) -> Self {
        Self {
            label: label.into(),
            tone: Tone::Primary,
            small: false,
            full_width: false,
            min_width: 0.0,
            loading: false,
        }
    }

    /// Icon-only button (draws the glyph from the icon font).
    pub fn icon(icon: char) -> Self {
        Self::new(icons::rich(icon, font::ICON)).small()
    }

    pub fn success(mut self) -> Self {
        self.tone = Tone::Success;
        self
    }
    pub fn danger(mut self) -> Self {
        self.tone = Tone::Danger;
        self
    }
    pub fn outline(mut self) -> Self {
        self.tone = Tone::Outline;
        self
    }
    pub fn trans(mut self) -> Self {
        self.tone = Tone::Trans;
        self
    }
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// (fill, stroke) for rest / hover / pressed, plus the text colour.
    fn scheme(&self) -> ([(Color32, Stroke); 3], Color32) {
        let none = Stroke::NONE;
        match self.tone {
            Tone::Primary => (
                [
                    (pal::ACCENT, none),
                    (pal::hover(pal::ACCENT), none),
                    (pal::hover(pal::ACCENT), none),
                ],
                pal::WHITE,
            ),
            Tone::Success => (
                [
                    (pal::SUCCESS, none),
                    (pal::hover(pal::SUCCESS), none),
                    (pal::hover(pal::SUCCESS), none),
                ],
                pal::WHITE,
            ),
            Tone::Danger => (
                [
                    (pal::DANGER, none),
                    (pal::hover(pal::DANGER), none),
                    (pal::hover(pal::DANGER), none),
                ],
                pal::WHITE,
            ),
            Tone::Outline => (
                [
                    (Color32::TRANSPARENT, Stroke::new(1.0, pal::NODE_STROKE)),
                    (pal::hover(pal::PANEL), Stroke::new(1.0, pal::MUTED)),
                    (pal::ACTIVE_FILL, Stroke::new(1.0, pal::ACCENT)),
                ],
                pal::TEXT,
            ),
            Tone::Trans => (
                [
                    (Color32::TRANSPARENT, Stroke::new(1.0, Color32::TRANSPARENT)),
                    (
                        pal::hover(pal::PANEL),
                        Stroke::new(1.0, Color32::TRANSPARENT),
                    ),
                    (pal::ACTIVE_FILL, Stroke::new(1.0, Color32::TRANSPARENT)),
                ],
                pal::TEXT,
            ),
        }
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        let (states, text_color) = self.scheme();
        let height = if self.small {
            metrics::BTN_H_SMALL
        } else {
            metrics::BTN_H
        };
        let width = if self.full_width {
            self.min_width.max(ui.available_width())
        } else {
            self.min_width
        };
        let (small, loading) = (self.small, self.loading);
        let label = self.label;

        ui.scope(|ui| {
            let style = ui.style_mut();
            let size = if small { font::SMALL } else { font::BUTTON };
            style
                .text_styles
                .insert(TextStyle::Button, FontId::proportional(size));
            style.spacing.button_padding = vec2(if small { 7.0 } else { 10.0 }, 3.0);
            let widgets = &mut style.visuals.widgets;
            for (visual, (fill, stroke)) in [
                (&mut widgets.inactive, states[0]),
                (&mut widgets.hovered, states[1]),
                (&mut widgets.active, states[2]),
            ] {
                visual.bg_fill = fill;
                visual.weak_bg_fill = fill;
                visual.bg_stroke = stroke;
                visual.fg_stroke = Stroke::new(1.0, text_color);
                visual.expansion = 0.0;
                visual.corner_radius = CornerRadius::same(metrics::R_CONTROL);
            }
            let button = egui::Button::new(label);
            let response = ui
                .add_enabled_ui(!loading, |ui| {
                    if width > 0.0 {
                        // Fixed or full width: add_sized centers the text in the rect.
                        ui.add_sized(vec2(width, height), button)
                    } else {
                        // Hug the content; the text can't be off-center here.
                        ui.add(button.min_size(vec2(0.0, height)))
                    }
                })
                .inner;
            if loading {
                let center = pos2(response.rect.left() + 14.0, response.rect.center().y);
                egui::Spinner::new()
                    .size(12.0)
                    .color(pal::TEXT)
                    .paint_at(ui, Rect::from_center_size(center, vec2(12.0, 12.0)));
            }
            response
        })
        .inner
    }
}

// ------------------------------------------------------------- menu item ----

/// A row in a menu: optional icon, label, shortcut, check mark.
pub(crate) struct MenuItem<'a> {
    label: &'a str,
    icon: Option<char>,
    shortcut: Option<&'a str>,
    checked: bool,
    danger: bool,
    enabled: bool,
}

impl<'a> MenuItem<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            shortcut: None,
            checked: false,
            danger: false,
            enabled: true,
        }
    }
    pub fn icon(mut self, icon: char) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Widget for MenuItem<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (text_color, icon_color, hint_color) = match (self.enabled, self.danger) {
            (false, _) => (pal::FAINT, pal::FAINT, pal::FAINT),
            (true, true) => (pal::DANGER, pal::DANGER, pal::DANGER),
            (true, false) => (pal::TEXT, pal::MUTED, pal::MUTED),
        };
        let label = ui.painter().layout_no_wrap(
            self.label.to_owned(),
            FontId::proportional(font::BUTTON),
            text_color,
        );
        let shortcut = self.shortcut.map(|s| {
            ui.painter()
                .layout_no_wrap(s.to_owned(), FontId::proportional(font::SMALL), hint_color)
        });

        let pad = metrics::MENU_PAD_X;
        let icon_col = metrics::MENU_ICON_COL;
        let mut desired = pad + icon_col + 8.0 + label.size().x + pad;
        if let Some(shortcut) = &shortcut {
            desired += 24.0 + shortcut.size().x;
        }
        let width = if ui.is_sizing_pass() {
            desired
        } else {
            ui.available_width().max(desired)
        };
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, metrics::MENU_ROW_H), sense);

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if self.enabled && response.hovered() {
                painter.rect_filled(rect, CornerRadius::same(4), pal::hover(pal::PANEL));
            }
            let icon = if self.checked {
                Some(icons::CHECK)
            } else {
                self.icon
            };
            if let Some(icon) = icon {
                painter.text(
                    pos2(rect.left() + pad + icon_col * 0.5, rect.center().y),
                    Align2::CENTER_CENTER,
                    icon,
                    icons::font(font::ICON),
                    icon_color,
                );
            }
            let text_x = rect.left() + pad + icon_col + 8.0;
            painter.galley(
                pos2(text_x, rect.center().y - label.size().y * 0.5),
                label,
                text_color,
            );
            if let Some(shortcut) = shortcut {
                painter.galley(
                    pos2(
                        rect.right() - pad - shortcut.size().x,
                        rect.center().y - shortcut.size().y * 0.5,
                    ),
                    shortcut,
                    hint_color,
                );
            }
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, self.enabled, self.label));
        response
    }
}

// -------------------------------------------------------------- checkbox ----

pub(crate) struct Checkbox<'a> {
    value: &'a mut bool,
    label: &'a str,
}

impl<'a> Checkbox<'a> {
    pub fn new(value: &'a mut bool, label: &'a str) -> Self {
        Self { value, label }
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let galley = ui.painter().layout(
            self.label.to_owned(),
            FontId::proportional(font::BODY),
            pal::TEXT,
            (ui.available_width() - metrics::CHECK_SIZE - 8.0).max(40.0),
        );
        let size = vec2(
            metrics::CHECK_SIZE + 8.0 + galley.size().x,
            metrics::CHECK_SIZE.max(galley.size().y),
        );
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
        if response.clicked() {
            *self.value = !*self.value;
            response.mark_changed();
        }
        let checked = *self.value;
        response
            .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, checked, self.label));

        if ui.is_rect_visible(rect) {
            let boxr = Rect::from_min_size(
                pos2(rect.left(), rect.center().y - metrics::CHECK_SIZE * 0.5),
                vec2(metrics::CHECK_SIZE, metrics::CHECK_SIZE),
            );
            let (fill, stroke) = if checked {
                (pal::ACCENT, Stroke::new(1.0, pal::ACCENT))
            } else if response.hovered() {
                (pal::hover(pal::INPUT_BG), Stroke::new(1.0, pal::MUTED))
            } else {
                (pal::INPUT_BG, Stroke::new(1.0, pal::NODE_STROKE))
            };
            let painter = ui.painter();
            painter.rect(
                boxr,
                CornerRadius::same(4),
                fill,
                stroke,
                StrokeKind::Inside,
            );
            if checked {
                painter.text(
                    boxr.center(),
                    Align2::CENTER_CENTER,
                    icons::CHECK,
                    icons::font(font::ICON),
                    pal::WHITE,
                );
            }
            painter.galley(
                pos2(boxr.right() + 8.0, rect.center().y - galley.size().y * 0.5),
                galley,
                pal::TEXT,
            );
        }
        response
    }
}

// ---------------------------------------------------------------- slider ----

/// Horizontal slider: label and value above a pill track.
pub(crate) struct Slider<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: Option<&'a str>,
    suffix: &'a str,
    step: Option<f64>,
    decimals: usize,
}

impl<'a> Slider<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        Self {
            value,
            range,
            label: None,
            suffix: "",
            step: None,
            decimals: 0,
        }
    }
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }
    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = suffix;
        self
    }
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }
    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }
}

impl Widget for Slider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let width = ui.available_width();
        let thumb_r = metrics::SLIDER_THUMB * 0.5;

        let (head, mut response) = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (head, _) = ui.allocate_exact_size(vec2(width, 14.0), Sense::hover());
                let response = ui.allocate_response(
                    vec2(width, metrics::SLIDER_THUMB + 2.0),
                    Sense::click_and_drag(),
                );
                (head, response)
            })
            .inner;

        let track_x = (response.rect.left() + thumb_r)..=(response.rect.right() - thumb_r);
        let (x0, x1) = (*track_x.start(), *track_x.end());
        let before = *self.value;
        if let Some(pos) = response.interact_pointer_pos()
            && (response.dragged() || response.is_pointer_button_down_on())
            && x1 > x0
        {
            let frac = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0);
            let mut v = lo as f64 + frac as f64 * (hi - lo) as f64;
            if let Some(step) = self.step.filter(|s| *s > 0.0) {
                v = lo as f64 + ((v - lo as f64) / step).round() * step;
            }
            *self.value = (v as f32).clamp(lo, hi);
        }
        if *self.value != before {
            response.mark_changed();
        }

        let value = *self.value;
        response
            .widget_info(|| WidgetInfo::slider(true, value as f64, self.label.unwrap_or_default()));

        if ui.is_rect_visible(response.rect) {
            let painter = ui.painter();
            if let Some(label) = self.label {
                painter.text(
                    head.left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(font::LABEL),
                    pal::MUTED,
                );
            }
            painter.text(
                head.right_center(),
                Align2::RIGHT_CENTER,
                format!("{:.*}{}", self.decimals, value, self.suffix),
                FontId::proportional(font::LABEL),
                pal::TEXT,
            );

            let cy = response.rect.center().y;
            let track = Rect::from_min_max(
                pos2(
                    response.rect.left() + thumb_r,
                    cy - metrics::SLIDER_TRACK * 0.5,
                ),
                pos2(
                    response.rect.right() - thumb_r,
                    cy + metrics::SLIDER_TRACK * 0.5,
                ),
            );
            let radius = CornerRadius::same((metrics::SLIDER_TRACK * 0.5) as u8);
            painter.rect(
                track,
                radius,
                pal::INPUT_BG,
                Stroke::new(1.0, pal::NODE_STROKE),
                StrokeKind::Inside,
            );
            let frac = if hi > lo {
                (value - lo) / (hi - lo)
            } else {
                0.0
            };
            let thumb_x = x0 + frac.clamp(0.0, 1.0) * (x1 - x0);
            let fill = Rect::from_min_max(track.min, pos2(thumb_x, track.max.y));
            painter.rect_filled(fill, radius, pal::ACCENT);
            let thumb = pos2(thumb_x, cy);
            if response.hovered() || response.dragged() {
                painter.circle_filled(thumb, thumb_r + 4.0, pal::ACTIVE_FILL);
            }
            painter.circle(
                thumb,
                thumb_r,
                pal::HEADER_TEXT,
                Stroke::new(2.0, pal::ACCENT),
            );
        }
        response
    }
}

// ---------------------------------------------------------------- select ----

/// Drop-down with an optional label above it.
pub(crate) struct Select<'a> {
    salt: Id,
    value: &'a mut String,
    label: Option<&'a str>,
    options: Vec<(String, &'a str)>,
    width: Option<f32>,
}

impl<'a> Select<'a> {
    pub fn new(salt: impl std::hash::Hash + std::fmt::Debug, value: &'a mut String) -> Self {
        Self {
            salt: Id::new("compnode_control").with(salt),
            value,
            label: None,
            options: Vec::new(),
            width: None,
        }
    }
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }
    pub fn options(mut self, options: impl IntoIterator<Item = (String, &'a str)>) -> Self {
        self.options = options.into_iter().collect();
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl Widget for Select<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if let Some(label) = self.label {
                ui.label(muted(label, font::LABEL));
            }
            let current = self
                .options
                .iter()
                .find(|(value, _)| value == self.value)
                .map(|(_, label)| *label)
                .unwrap_or("");
            let mut changed = false;
            let combo = egui::ComboBox::from_id_salt(self.salt)
                .width(self.width.unwrap_or_else(|| ui.available_width()))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (value, label) in &self.options {
                        changed |= ui
                            .selectable_value(self.value, value.clone(), *label)
                            .changed();
                    }
                });
            let mut response = combo.response;
            if changed {
                response.mark_changed();
            }
            response
        })
        .inner
    }
}

// ----------------------------------------------------------- colour field ----

/// Colour swatch (opens egui's picker) plus an editable `#rrggbb` field.
pub(crate) struct ColorField<'a> {
    salt: Id,
    color: &'a mut Color32,
    label: Option<&'a str>,
}

impl<'a> ColorField<'a> {
    pub fn new(salt: impl std::hash::Hash + std::fmt::Debug, color: &'a mut Color32) -> Self {
        Self {
            salt: Id::new("compnode_control").with(salt),
            color,
            label: None,
        }
    }
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }
}

fn parse_hex(text: &str) -> Option<Color32> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(super::hex)
}

impl Widget for ColorField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let text_id = self.salt.with("hex_text");
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if let Some(label) = self.label {
                ui.label(muted(label, font::LABEL));
            }
            ui.horizontal(|ui| {
                let mut swatch = egui::color_picker::color_edit_button_srgba(
                    ui,
                    self.color,
                    egui::color_picker::Alpha::Opaque,
                );
                let canonical = format!(
                    "#{:02x}{:02x}{:02x}",
                    self.color.r(),
                    self.color.g(),
                    self.color.b()
                );
                // While the field has focus keep the user's partial input.
                let mut text: String = ui
                    .data(|d| d.get_temp(text_id))
                    .unwrap_or_else(|| canonical.clone());
                let field = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .font(TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
                if field.has_focus() {
                    ui.data_mut(|d| d.insert_temp(text_id, text.clone()));
                    if field.changed()
                        && let Some(parsed) = parse_hex(&text)
                    {
                        *self.color = parsed;
                        swatch.mark_changed();
                    }
                } else {
                    ui.data_mut(|d| d.remove_temp::<String>(text_id));
                }
                swatch
            })
            .inner
        })
        .inner
    }
}

// --------------------------------------------------------------- callout ----

/// Error banner under a node. Renders nothing for `None`.
pub(crate) fn error_line(ui: &mut Ui, error: Option<&str>) {
    let Some(error) = error else { return };
    Frame::new()
        .fill(pal::DANGER_TINT)
        .stroke(Stroke::new(1.0, pal::CALLOUT_DANGER_BORDER))
        .corner_radius(CornerRadius::same(metrics::R_CONTROL))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(icons::rich(icons::TRIANGLE_ALERT, 12.0).color(pal::DANGER));
                ui.add(egui::Label::new(
                    RichText::new(error).size(font::TINY).color(pal::TEXT),
                ));
            });
        });
}

// ------------------------------------------------------------ title bar ----

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowGlyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Borderless window-control button, drawn with vector strokes so it needs no
/// font. Close turns red on hover.
pub(crate) fn window_button(ui: &mut Ui, glyph: WindowGlyph, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            let fill = if glyph == WindowGlyph::Close {
                pal::DANGER
            } else {
                pal::hover(pal::PANEL)
            };
            ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
        }
        let color = if response.hovered() && glyph == WindowGlyph::Close {
            pal::WHITE
        } else {
            pal::TEXT
        };
        let stroke = Stroke::new(1.2, color);
        let c = rect.center();
        let r = 4.5;
        let painter = ui.painter();
        let square =
            |center: Pos2, half: f32| Rect::from_center_size(center, vec2(half * 2.0, half * 2.0));
        match glyph {
            WindowGlyph::Minimize => {
                painter.line_segment([pos2(c.x - r, c.y), pos2(c.x + r, c.y)], stroke);
            }
            WindowGlyph::Maximize => {
                painter.rect_stroke(square(c, r), 1.0, stroke, StrokeKind::Middle);
            }
            WindowGlyph::Restore => {
                let front = square(pos2(c.x - 1.0, c.y + 1.0), r - 1.0);
                painter.rect_stroke(front, 1.0, stroke, StrokeKind::Middle);
                painter.line_segment(
                    [
                        pos2(front.left() + 2.0, front.top() - 2.0),
                        pos2(c.x + r, front.top() - 2.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        pos2(c.x + r, front.top() - 2.0),
                        pos2(c.x + r, front.bottom() - 2.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        pos2(front.right(), front.bottom() - 2.0),
                        pos2(c.x + r, front.bottom() - 2.0),
                    ],
                    stroke,
                );
            }
            WindowGlyph::Close => {
                painter.line_segment([pos2(c.x - r, c.y - r), pos2(c.x + r, c.y + r)], stroke);
                painter.line_segment([pos2(c.x + r, c.y - r), pos2(c.x - r, c.y + r)], stroke);
            }
        }
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, tooltip));
    response.on_hover_text(tooltip)
}
