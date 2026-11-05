//! The default widget set: number fields, slider, toggle, button, dropdown,
//! text area, color picker, image preview and label.
use crate::draw::{Align, DrawList, Layer, Prim};
use crate::input::{InputEvent, Key, PointerButton};
use crate::text::{ellipsize, format_float};
use crate::theme::{Col, Num, Theme};
use crate::value::Value;
use crate::widget::{Response, TextEditRequest, Widget};
use compnode_core::color::{hsv_to_rgb, linear_to_srgb, rgb_to_hsv, srgb_to_linear};
use compnode_core::geom::vec2;
use compnode_core::layout::ROW_H;
use compnode_core::{Color, Rect, Vec2};

const ARROW_W: f32 = 16.0;
const DRAG_PX: f32 = 3.0;
const LINE_H: f32 = 16.0;

/// Read a Float or Int value as `f64`.
pub fn num_get(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(f64::from(*f)),
        Value::Int(i) => Some(f64::from(*i)),
        _ => None,
    }
}

/// Write a Float or Int value (ints round). Returns `false` for other types.
pub fn num_set(v: &mut Value, x: f64) -> bool {
    match v {
        Value::Float(f) => {
            *f = x as f32;
            true
        }
        Value::Int(i) => {
            *i = x.round() as i32;
            true
        }
        _ => false,
    }
}

/// Short text shown for a value in a field.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Float(f) => format_float(*f),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => if *b { "On" } else { "Off" }.to_string(),
        Value::Vec3(a) => format!(
            "{}, {}, {}",
            format_float(a[0]),
            format_float(a[1]),
            format_float(a[2])
        ),
        Value::Color(c) => c.to_hex(),
        Value::String(s) => s.clone(),
        Value::Enum(i) => format!("#{i}"),
        Value::Image(Some(h)) => format!("image {}", h.0),
        Value::Image(None) => "no image".to_string(),
    }
}

fn field_fill(theme: &Theme, hover: bool, pressed: bool) -> Color {
    if pressed {
        theme.c(Col::FieldPressed)
    } else if hover {
        theme.c(Col::FieldHover)
    } else {
        theme.c(Col::FieldFill)
    }
}

fn draw_field(
    dl: &mut DrawList,
    rect: Rect,
    theme: &Theme,
    hover: bool,
    pressed: bool,
    focused: bool,
) {
    let fill = field_fill(theme, hover, pressed);
    let r = theme.n(Num::FieldRadius);
    // Subtle bottom inner shadow so the field reads as inset.
    let shade = theme.c(Col::FieldShadow);
    if shade.a > 0.0 {
        dl.rect(
            Layer::Nodes,
            Rect::new(
                vec2(rect.min.x, rect.min.y),
                vec2(rect.max.x, rect.min.y + 1.0),
            ),
            r,
            shade,
        );
    }
    dl.rect(Layer::Nodes, rect, r, fill);
    if focused {
        let glow = theme.c(Col::FieldFocus);
        dl.rect_stroked(
            Layer::Nodes,
            rect,
            r,
            Color::TRANSPARENT,
            glow,
            theme.n(Num::FieldFocusWidth),
        );
        dl.rect_stroked(
            Layer::Nodes,
            rect.expand(1.0),
            r + 1.0,
            Color::TRANSPARENT,
            glow.with_alpha(glow.a * 0.25),
            1.0,
        );
    } else {
        dl.rect_stroked(
            Layer::Nodes,
            rect,
            r,
            Color::TRANSPARENT,
            theme.c(Col::FieldBorder),
            1.0,
        );
    }
}

/// A soft drop shadow behind a floating panel (popups, menus).
fn draw_panel(dl: &mut DrawList, rect: Rect, theme: &Theme, radius: f32) {
    let shadow = theme.c(Col::NodeShadow);
    if shadow.a > 0.0 {
        dl.rect(
            Layer::Popup,
            rect.expand(2.0).translate(vec2(0.0, 2.0)),
            radius + 1.0,
            shadow.with_alpha(shadow.a * 0.35),
        );
    }
    dl.rect_stroked(
        Layer::Popup,
        rect,
        radius,
        theme.c(Col::Popup),
        theme.c(Col::PopupBorder),
        1.0,
    );
}

/// A rounded status dot (used for the button "hot" state and radio marks).
fn dot(dl: &mut DrawList, center: Vec2, r: f32, color: Color) {
    dl.rect(
        Layer::Nodes,
        Rect::from_center_size(center, vec2(r * 2.0, r * 2.0)),
        r,
        color,
    );
}

/// Split a row into its label and control areas. Blender-style sockets show
/// the socket name on the left and a compact value widget on the right.
/// Narrow rows (< ~150 graph units, e.g. the unit-test rows) keep the whole
/// row as one control so their click coordinates stay in the interactive zone.
fn split_row(rect: Rect, frac: f32) -> (Rect, Rect) {
    let f = frac.clamp(0.0, 0.85);
    if f <= 0.0 || rect.width() < 150.0 {
        return (Rect::new(rect.min, rect.min), rect);
    }
    let x = rect.min.x + rect.width() * f;
    let label = Rect::new(
        vec2(rect.min.x + 2.0, rect.min.y),
        vec2((x - 4.0).max(rect.min.x + 2.0), rect.max.y),
    );
    let control = Rect::new(vec2(x, rect.min.y), vec2(rect.max.x - 2.0, rect.max.y));
    (label, control)
}

fn draw_row_label(dl: &mut DrawList, rect: Rect, text: &str, theme: &Theme) {
    if text.is_empty() || rect.width() <= 0.0 {
        return;
    }
    let size = theme.n(Num::FontSize);
    let t = ellipsize(text, rect.width(), size);
    dl.text(
        Layer::Nodes,
        vec2(rect.min.x, rect.center().y),
        &t,
        size,
        theme.c(Col::Text),
        Align::Left,
    );
}

/// A control row consisting of a pill/field body with centered text.
/// A control row consisting of a pill/field body with centered text.
fn centered_text(dl: &mut DrawList, rect: Rect, text: &str, theme: &Theme, color: Col, inset: f32) {
    let size = theme.n(Num::FontSize);
    let t = ellipsize(text, rect.width() - 2.0 * inset, size);
    dl.text(
        Layer::Nodes,
        rect.center(),
        &t,
        size,
        theme.c(color),
        Align::Center,
    );
}

fn triangle(dl: &mut DrawList, center: Vec2, half: f32, pointing_right: bool, color: Color) {
    let s = if pointing_right { 1.0 } else { -1.0 };
    dl.push(
        Layer::Nodes,
        Prim::Triangle {
            p: [
                vec2(center.x - s * half * 0.6, center.y - half),
                vec2(center.x - s * half * 0.6, center.y + half),
                vec2(center.x + s * half, center.y),
            ],
            color,
        },
    );
}

fn is_primary_down(ev: &InputEvent) -> Option<Vec2> {
    match ev {
        InputEvent::PointerDown {
            pos,
            button: PointerButton::Primary,
            ..
        } => Some(*pos),
        _ => None,
    }
}

fn is_primary_up(ev: &InputEvent) -> Option<Vec2> {
    match ev {
        InputEvent::PointerUp {
            pos,
            button: PointerButton::Primary,
            ..
        } => Some(*pos),
        _ => None,
    }
}

// ───────────────────────────── number field ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zone {
    None,
    Dec,
    Center,
    Inc,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    start_x: f32,
    start_val: f64,
    scrubbing: bool,
}

/// Float or int field. Drag the middle to scrub (Shift = fine, Ctrl = snap
/// to the step), click the middle to type, click an arrow to step.
pub struct NumberField {
    int: bool,
    min: f32,
    max: f32,
    step: f32,
    hover: Zone,
    press: Option<Press>,
    focused: bool,
    label_frac: f32,
}

impl NumberField {
    pub fn float(min: f32, max: f32, step: f32) -> NumberField {
        NumberField {
            int: false,
            min,
            max,
            step,
            hover: Zone::None,
            press: None,
            focused: false,
            label_frac: 0.35,
        }
    }
    pub fn int(min: f32, max: f32, step: f32) -> NumberField {
        NumberField {
            int: true,
            min,
            max,
            step,
            hover: Zone::None,
            press: None,
            focused: false,
            label_frac: 0.35,
        }
    }
    fn zone(&self, p: Vec2, rect: Rect) -> Zone {
        if !rect.contains(p) {
            Zone::None
        } else if p.x < rect.min.x + ARROW_W {
            Zone::Dec
        } else if p.x > rect.max.x - ARROW_W {
            Zone::Inc
        } else {
            Zone::Center
        }
    }
    fn clamp(&self, x: f64) -> f64 {
        let (lo, hi) = (f64::from(self.min), f64::from(self.max));
        if lo <= hi { x.clamp(lo, hi) } else { x }
    }
    fn step_size(&self) -> f64 {
        let s = f64::from(self.step);
        if self.int {
            s.round().max(1.0)
        } else if s > 0.0 {
            s
        } else {
            0.1
        }
    }
    fn per_px(&self) -> f64 {
        if self.int {
            self.step_size() / 8.0
        } else {
            self.step_size() * 0.25
        }
    }
    /// Set the clamped value; `true` if it changed.
    fn set(&self, value: &mut Value, x: f64) -> bool {
        let before = value.clone();
        if !num_set(value, self.clamp(x)) {
            return false;
        }
        *value != before
    }
    fn text_request(value: &Value) -> TextEditRequest {
        TextEditRequest {
            text: value_text(value),
            multiline: false,
        }
    }
}

impl Widget for NumberField {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response {
        let mut r = Response::default();
        let Some(cur) = num_get(value) else { return r };
        let (_, ctrl) = split_row(rect, self.label_frac);
        match ev {
            InputEvent::PointerMove { pos, mods } => {
                if let Some(mut p) = self.press {
                    let dx = f64::from(pos.x - p.start_x);
                    if !p.scrubbing && dx.abs() >= f64::from(DRAG_PX) {
                        p.scrubbing = true;
                        r.begin_edit = true;
                    }
                    if p.scrubbing {
                        let mut speed = self.per_px();
                        if mods.shift {
                            speed *= 0.1;
                        }
                        let mut x = p.start_val + dx * speed;
                        if mods.ctrl {
                            let s = self.step_size();
                            x = (x / s).round() * s;
                        }
                        r.changed = self.set(value, x);
                    }
                    self.press = Some(p);
                    r.consumed = true;
                } else {
                    self.hover = self.zone(*pos, ctrl);
                }
            }
            InputEvent::PointerDown {
                pos,
                button: PointerButton::Primary,
                mods,
            } => {
                let z = self.zone(*pos, ctrl);
                match z {
                    Zone::None => {}
                    Zone::Center => {
                        self.press = Some(Press {
                            start_x: pos.x,
                            start_val: cur,
                            scrubbing: false,
                        });
                        r.consumed = true;
                    }
                    Zone::Dec | Zone::Inc => {
                        let mut s = self.step_size();
                        if mods.shift && !self.int {
                            s *= 0.1;
                        }
                        let x = if z == Zone::Dec { cur - s } else { cur + s };
                        r.changed = self.set(value, x);
                        r.begin_edit = true;
                        r.end_edit = true;
                        r.consumed = true;
                    }
                }
            }
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                if let Some(p) = self.press.take() {
                    r.consumed = true;
                    if p.scrubbing {
                        r.end_edit = true;
                    } else {
                        r.text_edit = Some(NumberField::text_request(value));
                    }
                }
            }
            InputEvent::Key { key, .. } => match key {
                Key::Escape => {
                    if let Some(p) = self.press.take() {
                        if p.scrubbing {
                            r.changed = self.set(value, p.start_val);
                            r.end_edit = true;
                        }
                        r.consumed = true;
                    }
                }
                Key::Left | Key::Right if self.focused => {
                    let s = self.step_size();
                    let x = if *key == Key::Left { cur - s } else { cur + s };
                    r.changed = self.set(value, x);
                    r.begin_edit = true;
                    r.end_edit = true;
                    r.consumed = true;
                }
                Key::Enter if self.focused => {
                    r.text_edit = Some(NumberField::text_request(value));
                    r.consumed = true;
                }
                _ => {}
            },
            _ => {}
        }
        r
    }

    fn commit_text(&mut self, text: &str, value: &mut Value) -> Response {
        let mut r = Response::consumed();
        if let Ok(x) = text.trim().parse::<f64>()
            && x.is_finite()
        {
            r.changed = self.set(value, x);
        }
        r
    }

    fn captures(&self) -> bool {
        self.press.is_some()
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_label_frac(&mut self, frac: f32) {
        if frac.is_finite() {
            self.label_frac = frac.clamp(0.0, 0.85);
        }
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        let (lab, ctrl) = split_row(rect, self.label_frac);
        draw_row_label(dl, lab, label, theme);
        let scrubbing = self.press.is_some_and(|p| p.scrubbing);
        draw_field(
            dl,
            ctrl,
            theme,
            self.hover != Zone::None,
            scrubbing,
            self.focused,
        );
        // Round stepper buttons that appear on hover/focus, with a hairline
        // separator so they read as pressed-on caps rather than plain arrows.
        if self.hover != Zone::None || self.focused {
            let c = theme.c(Col::TextDim);
            let sep = theme.c(Col::FieldBorder);
            dl.line(
                Layer::Nodes,
                vec2(ctrl.min.x + ARROW_W, ctrl.min.y + 3.0),
                vec2(ctrl.min.x + ARROW_W, ctrl.max.y - 3.0),
                sep,
                1.0,
            );
            dl.line(
                Layer::Nodes,
                vec2(ctrl.max.x - ARROW_W, ctrl.min.y + 3.0),
                vec2(ctrl.max.x - ARROW_W, ctrl.max.y - 3.0),
                sep,
                1.0,
            );
            // Chevrons pointing out of the field.
            triangle(
                dl,
                vec2(ctrl.min.x + ARROW_W * 0.5 - 1.0, ctrl.center().y),
                3.2,
                false,
                c,
            );
            triangle(
                dl,
                vec2(ctrl.max.x - ARROW_W * 0.5 + 1.0, ctrl.center().y),
                3.2,
                true,
                c,
            );
        }
        centered_text(dl, ctrl, &value_text(value), theme, Col::FieldText, ARROW_W);
    }
}

// ─────────────────────────────── slider ───────────────────────────────

#[derive(Clone, Copy, Debug)]
struct SliderDrag {
    start_x: f32,
    anchor_val: f64,
    orig: f64,
}

/// Factor slider with a fill bar. Click or drag sets the value from the
/// pointer position; Shift-drag moves it relatively and slowly; Ctrl-click
/// types a value.
pub struct Slider {
    min: f32,
    max: f32,
    drag: Option<SliderDrag>,
    hover: bool,
    focused: bool,
    label_frac: f32,
}

impl Slider {
    pub fn new(min: f32, max: f32) -> Slider {
        let (min, max) = if max > min { (min, max) } else { (0.0, 1.0) };
        Slider {
            min,
            max,
            drag: None,
            hover: false,
            focused: false,
            label_frac: 0.35,
        }
    }
    fn range(&self) -> f64 {
        f64::from(self.max - self.min)
    }
    fn set(&self, value: &mut Value, x: f64) -> bool {
        let before = value.clone();
        let x = x.clamp(f64::from(self.min), f64::from(self.max));
        num_set(value, x) && *value != before
    }
    fn frac(&self, value: &Value) -> f32 {
        let v = num_get(value).unwrap_or(0.0);
        (((v - f64::from(self.min)) / self.range()) as f32).clamp(0.0, 1.0)
    }
}

impl Widget for Slider {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response {
        let mut r = Response::default();
        let Some(cur) = num_get(value) else { return r };
        let (_, ctrl) = split_row(rect, self.label_frac);
        let w = f64::from(ctrl.width().max(1.0));
        match ev {
            InputEvent::PointerMove { pos, mods } => {
                if let Some(d) = self.drag {
                    let x = if mods.shift {
                        d.anchor_val + f64::from(pos.x - d.start_x) / w * self.range() * 0.1
                    } else {
                        f64::from(self.min) + f64::from(pos.x - ctrl.min.x) / w * self.range()
                    };
                    r.changed = self.set(value, x);
                    r.consumed = true;
                } else {
                    self.hover = ctrl.contains(*pos);
                }
            }
            InputEvent::PointerDown {
                pos,
                button: PointerButton::Primary,
                mods,
            } if ctrl.contains(*pos) => {
                r.consumed = true;
                if mods.ctrl {
                    r.text_edit = Some(TextEditRequest {
                        text: value_text(value),
                        multiline: false,
                    });
                } else {
                    let mut anchor = cur;
                    if !mods.shift {
                        let x =
                            f64::from(self.min) + f64::from(pos.x - ctrl.min.x) / w * self.range();
                        r.changed = self.set(value, x);
                        anchor = num_get(value).unwrap_or(cur);
                    }
                    self.drag = Some(SliderDrag {
                        start_x: pos.x,
                        anchor_val: anchor,
                        orig: cur,
                    });
                    r.begin_edit = true;
                }
            }
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                if self.drag.take().is_some() {
                    r.end_edit = true;
                    r.consumed = true;
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                if let Some(d) = self.drag.take() {
                    r.changed = self.set(value, d.orig);
                    r.end_edit = true;
                    r.consumed = true;
                }
            }
            InputEvent::Key {
                key: Key::Left | Key::Right,
                ..
            } if self.focused => {
                let s = self.range() * 0.01;
                let x = if matches!(ev, InputEvent::Key { key: Key::Left, .. }) {
                    cur - s
                } else {
                    cur + s
                };
                r.changed = self.set(value, x);
                r.begin_edit = true;
                r.end_edit = true;
                r.consumed = true;
            }
            _ => {}
        }
        r
    }

    fn commit_text(&mut self, text: &str, value: &mut Value) -> Response {
        let mut r = Response::consumed();
        if let Ok(x) = text.trim().parse::<f64>()
            && x.is_finite()
        {
            r.changed = self.set(value, x);
        }
        r
    }

    fn captures(&self) -> bool {
        self.drag.is_some()
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_label_frac(&mut self, frac: f32) {
        if frac.is_finite() {
            self.label_frac = frac.clamp(0.0, 0.85);
        }
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        let (lab, ctrl) = split_row(rect, self.label_frac);
        draw_row_label(dl, lab, label, theme);
        draw_field(
            dl,
            ctrl,
            theme,
            self.hover,
            self.drag.is_some(),
            self.focused,
        );
        let f = self.frac(value);
        let r = theme.n(Num::FieldRadius);
        let inner = ctrl.expand(-1.5);
        if f > 0.0 {
            let fill = Rect::new(
                inner.min,
                vec2(inner.min.x + inner.width() * f, inner.max.y),
            );
            dl.rect(
                Layer::Nodes,
                fill,
                (r - 1.0).max(0.0),
                theme.c(Col::SliderFill),
            );
        }
        // A slim knob marks the handle position, like a modern range slider.
        let kx = inner.min.x + inner.width() * f;
        let knob = theme.c(Col::FieldText);
        dl.rect(
            Layer::Nodes,
            Rect::from_center_size(vec2(kx, inner.center().y), vec2(3.0, inner.height() - 4.0)),
            1.5,
            knob.with_alpha(0.85),
        );
        centered_text(dl, ctrl, &value_text(value), theme, Col::FieldText, 6.0);
    }
}

// ─────────────────────────────── toggle ───────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToggleStyle {
    Checkbox,
    Switch,
}

pub struct Toggle {
    style: ToggleStyle,
    hover: bool,
    focused: bool,
}

impl Toggle {
    pub fn new(style: ToggleStyle) -> Toggle {
        Toggle {
            style,
            hover: false,
            focused: false,
        }
    }
    fn flip(value: &mut Value, r: &mut Response) {
        if let Value::Bool(b) = value {
            *b = !*b;
            r.changed = true;
            r.begin_edit = true;
            r.end_edit = true;
            r.consumed = true;
        }
    }
}

impl Widget for Toggle {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response {
        let mut r = Response::default();
        match ev {
            InputEvent::PointerMove { pos, .. } => self.hover = rect.contains(*pos),
            InputEvent::PointerDown {
                pos,
                button: PointerButton::Primary,
                ..
            } if rect.contains(*pos) => {
                Toggle::flip(value, &mut r);
                r.consumed = true;
            }
            InputEvent::Key {
                key: Key::Space | Key::Enter,
                ..
            } if self.focused => Toggle::flip(value, &mut r),
            _ => {}
        }
        r
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        let on = matches!(value, Value::Bool(true));
        let size = theme.n(Num::FontSize);
        let cy = rect.center().y;
        let box_fill = if on {
            theme.c(Col::ToggleOn)
        } else if self.hover {
            theme.c(Col::FieldHover)
        } else {
            theme.c(Col::ToggleOff)
        };
        match self.style {
            ToggleStyle::Checkbox => {
                let b = Rect::from_center_size(vec2(rect.min.x + 8.0, cy), vec2(15.0, 15.0));
                if self.focused {
                    dl.rect_stroked(
                        Layer::Nodes,
                        b,
                        4.0,
                        box_fill,
                        theme.c(Col::FieldFocus),
                        1.5,
                    );
                } else {
                    dl.rect(Layer::Nodes, b, 4.0, box_fill);
                    dl.rect_stroked(
                        Layer::Nodes,
                        b,
                        4.0,
                        Color::TRANSPARENT,
                        theme.c(Col::FieldBorder),
                        1.0,
                    );
                }
                if on {
                    let c = theme.c(Col::FieldText);
                    dl.push(
                        Layer::Nodes,
                        Prim::LineStrip {
                            points: vec![
                                vec2(b.min.x + 3.5, cy + 0.5),
                                vec2(b.min.x + 6.0, cy + 3.0),
                                vec2(b.max.x - 3.5, cy - 3.5),
                            ],
                            color: c,
                            width: 1.8,
                        },
                    );
                }
                dl.text(
                    Layer::Nodes,
                    vec2(b.max.x + 7.0, cy),
                    label,
                    size,
                    theme.c(Col::Text),
                    Align::Left,
                );
            }
            ToggleStyle::Switch => {
                let pill = Rect::from_center_size(vec2(rect.max.x - 15.0, cy), vec2(30.0, 16.0));
                let track = if on { theme.c(Col::ToggleOn) } else { box_fill };
                dl.rect(Layer::Nodes, pill, 8.0, track);
                if self.focused {
                    dl.rect_stroked(
                        Layer::Nodes,
                        pill,
                        8.0,
                        Color::TRANSPARENT,
                        theme.c(Col::FieldFocus),
                        1.5,
                    );
                }
                // Knob hugs the ends and lifts slightly off the track.
                let knob_x = if on {
                    pill.max.x - 8.0
                } else {
                    pill.min.x + 8.0
                };
                dl.rect(
                    Layer::Nodes,
                    pill.expand(0.5),
                    8.5,
                    theme.c(Col::FieldShadow).with_alpha(0.25),
                );
                dl.rect(
                    Layer::Nodes,
                    Rect::from_center_size(vec2(knob_x, cy), vec2(12.0, 12.0)),
                    6.0,
                    theme.c(Col::FieldText),
                );
                dl.text(
                    Layer::Nodes,
                    vec2(rect.min.x, cy),
                    label,
                    size,
                    theme.c(Col::Text),
                    Align::Left,
                );
            }
        }
    }
}

// ─────────────────────────────── button ───────────────────────────────

pub struct Button {
    pressed: bool,
    hover: bool,
    focused: bool,
    label_frac: f32,
}

impl Button {
    pub fn new() -> Button {
        Button {
            pressed: false,
            hover: false,
            focused: false,
            label_frac: 0.35,
        }
    }
}

impl Default for Button {
    fn default() -> Self {
        Button::new()
    }
}

impl Widget for Button {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, _value: &mut Value) -> Response {
        let mut r = Response::default();
        let (_, ctrl) = split_row(rect, self.label_frac);
        if let Some(p) = is_primary_down(ev) {
            if ctrl.contains(p) {
                self.pressed = true;
                r.consumed = true;
            }
            return r;
        }
        if let Some(p) = is_primary_up(ev) {
            if self.pressed {
                self.pressed = false;
                r.consumed = true;
                r.clicked = ctrl.contains(p);
            }
            return r;
        }
        match ev {
            InputEvent::PointerMove { pos, .. } => self.hover = ctrl.contains(*pos),
            InputEvent::Key {
                key: Key::Escape, ..
            } if self.pressed => {
                self.pressed = false;
                r.consumed = true;
            }
            InputEvent::Key {
                key: Key::Space | Key::Enter,
                ..
            } if self.focused => {
                r.clicked = true;
                r.consumed = true;
            }
            _ => {}
        }
        r
    }

    fn captures(&self) -> bool {
        self.pressed
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_label_frac(&mut self, frac: f32) {
        if frac.is_finite() {
            self.label_frac = frac.clamp(0.0, 0.85);
        }
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, _value: &Value, label: &str, theme: &Theme) {
        let (lab, ctrl) = split_row(rect, self.label_frac);
        draw_row_label(dl, lab, label, theme);
        // Modern button: a raised pill with a top highlight and a press that
        // sinks it (shifts down a hair and dims the fill).
        let r = theme.n(Num::FieldRadius);
        let body = if self.pressed {
            ctrl.translate(vec2(0.0, 1.0))
        } else {
            ctrl
        };
        let fill = if self.pressed {
            theme.c(Col::FieldPressed)
        } else if self.hover {
            theme.c(Col::FieldHover)
        } else {
            theme.c(Col::FieldFill)
        };
        let shadow = theme.c(Col::FieldShadow);
        if shadow.a > 0.0 && !self.pressed {
            dl.rect(Layer::Nodes, ctrl.translate(vec2(0.0, 1.0)), r, shadow);
        }
        dl.rect(Layer::Nodes, body, r, fill);
        let hi = theme.c(Col::NodeHighlight);
        if hi.a > 0.0 {
            dl.rect(
                Layer::Nodes,
                Rect::new(body.min, vec2(body.max.x, body.min.y + 1.0)),
                r,
                hi,
            );
        }
        if self.focused {
            dl.rect_stroked(
                Layer::Nodes,
                body,
                r,
                Color::TRANSPARENT,
                theme.c(Col::FieldFocus),
                theme.n(Num::FieldFocusWidth),
            );
        } else {
            dl.rect_stroked(
                Layer::Nodes,
                body,
                r,
                Color::TRANSPARENT,
                theme.c(Col::FieldBorder),
                1.0,
            );
        }
        centered_text(dl, body, label, theme, Col::FieldText, 10.0);
    }
}

// ────────────────────────────── dropdown ──────────────────────────────

pub struct Dropdown {
    options: Vec<String>,
    open: bool,
    hover: Option<usize>,
    field_hover: bool,
    focused: bool,
    label_frac: f32,
}

impl Dropdown {
    pub fn new(options: Vec<String>) -> Dropdown {
        Dropdown {
            options,
            open: false,
            hover: None,
            field_hover: false,
            focused: false,
            label_frac: 0.35,
        }
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    fn popup(&self, rect: Rect) -> Rect {
        Rect::from_min_size(
            vec2(rect.min.x, rect.max.y + 2.0),
            vec2(rect.width(), self.options.len() as f32 * ROW_H),
        )
    }
    fn item_at(&self, rect: Rect, p: Vec2) -> Option<usize> {
        let pr = self.popup(rect);
        if !pr.contains(p) {
            return None;
        }
        let i = ((p.y - pr.min.y) / ROW_H) as usize;
        (i < self.options.len()).then_some(i)
    }
    fn current(value: &Value) -> usize {
        match value {
            Value::Enum(i) => *i,
            _ => 0,
        }
    }
    fn select(&mut self, value: &mut Value, i: usize, r: &mut Response) {
        if let Value::Enum(cur) = value
            && *cur != i
            && i < self.options.len()
        {
            *cur = i;
            r.changed = true;
        }
        self.open = false;
        r.end_edit = true;
        r.consumed = true;
    }
}

impl Widget for Dropdown {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response {
        let mut r = Response::default();
        if !matches!(value, Value::Enum(_)) || self.options.is_empty() {
            return r;
        }
        let last = self.options.len() - 1;
        if self.open {
            match ev {
                InputEvent::PointerMove { pos, .. } => {
                    if let Some(i) = self.item_at(rect, *pos) {
                        self.hover = Some(i);
                    }
                    r.consumed = true;
                }
                InputEvent::PointerDown {
                    pos,
                    button: PointerButton::Primary,
                    ..
                } => match self.item_at(rect, *pos) {
                    Some(i) => self.select(value, i, &mut r),
                    None => {
                        self.open = false;
                        r.end_edit = true;
                        r.consumed = true;
                    }
                },
                InputEvent::Key { key, .. } => match key {
                    Key::Escape => {
                        self.open = false;
                        r.end_edit = true;
                        r.consumed = true;
                    }
                    Key::Down => {
                        self.hover = Some(
                            self.hover
                                .map_or(Dropdown::current(value), |h| (h + 1).min(last)),
                        );
                        r.consumed = true;
                    }
                    Key::Up => {
                        self.hover = Some(
                            self.hover
                                .map_or(Dropdown::current(value), |h| h.saturating_sub(1)),
                        );
                        r.consumed = true;
                    }
                    Key::Enter | Key::Space => {
                        let i = self.hover.unwrap_or(Dropdown::current(value));
                        self.select(value, i, &mut r);
                    }
                    _ => {}
                },
                _ => {}
            }
            return r;
        }
        let (_, ctrl) = split_row(rect, self.label_frac);
        match ev {
            InputEvent::PointerMove { pos, .. } => self.field_hover = ctrl.contains(*pos),
            InputEvent::PointerDown {
                pos,
                button: PointerButton::Primary,
                ..
            } if ctrl.contains(*pos) => {
                self.open = true;
                self.hover = Some(Dropdown::current(value).min(last));
                r.begin_edit = true;
                r.consumed = true;
            }
            InputEvent::Key { key, .. } if self.focused => match key {
                Key::Enter | Key::Space => {
                    self.open = true;
                    self.hover = Some(Dropdown::current(value).min(last));
                    r.begin_edit = true;
                    r.consumed = true;
                }
                Key::Down | Key::Up => {
                    let cur = Dropdown::current(value);
                    let next = if *key == Key::Down {
                        (cur + 1).min(last)
                    } else {
                        cur.saturating_sub(1)
                    };
                    if next != cur {
                        *value = Value::Enum(next);
                        r.changed = true;
                    }
                    r.begin_edit = true;
                    r.end_edit = true;
                    r.consumed = true;
                }
                _ => {}
            },
            _ => {}
        }
        r
    }

    fn captures(&self) -> bool {
        self.open
    }

    fn popup_rect(&self, rect: Rect) -> Option<Rect> {
        self.open.then(|| self.popup(rect))
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_label_frac(&mut self, frac: f32) {
        if frac.is_finite() {
            self.label_frac = frac.clamp(0.0, 0.85);
        }
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        let (lab, ctrl) = split_row(rect, self.label_frac);
        draw_row_label(dl, lab, label, theme);
        draw_field(
            dl,
            ctrl,
            theme,
            self.field_hover || self.open,
            self.open,
            self.focused,
        );
        let cur = Dropdown::current(value);
        let name = self.options.get(cur).map_or("?", String::as_str);
        centered_text(dl, ctrl, name, theme, Col::FieldText, ARROW_W);
        // Chevron (two strokes) instead of a filled triangle.
        let c = theme.c(Col::TextDim);
        let cx = ctrl.max.x - 12.0;
        let cy = ctrl.center().y;
        dl.push(
            Layer::Nodes,
            Prim::LineStrip {
                points: vec![
                    vec2(cx - 3.5, cy - 1.5),
                    vec2(cx, cy + 2.0),
                    vec2(cx + 3.5, cy - 1.5),
                ],
                color: c,
                width: 1.6,
            },
        );
        if self.open {
            let pr = self.popup(rect);
            draw_panel(dl, pr, theme, 5.0);
            let size = theme.n(Num::FontSize);
            for (i, o) in self.options.iter().enumerate() {
                let row = Rect::from_min_size(
                    vec2(pr.min.x, pr.min.y + i as f32 * ROW_H),
                    vec2(pr.width(), ROW_H),
                );
                if self.hover == Some(i) {
                    dl.rect(
                        Layer::Popup,
                        row.expand(-3.0),
                        4.0,
                        theme.c(Col::PopupHover),
                    );
                } else if Dropdown::current(value) == i {
                    dot(
                        dl,
                        vec2(row.min.x + 7.0, row.center().y),
                        2.5,
                        theme.c(Col::Selected),
                    );
                }
                let t = ellipsize(o, row.width() - 20.0, size);
                dl.text(
                    Layer::Popup,
                    vec2(row.min.x + 14.0, row.center().y),
                    &t,
                    size,
                    theme.c(Col::Text),
                    Align::Left,
                );
            }
        }
    }
}

// ───────────────────────────── text area ─────────────────────────────

/// Multi-line text. Editing uses the host's text field (edit overlay).
pub struct TextArea {
    min_lines: usize,
    max_lines: usize,
    hover: bool,
    editing: bool,
}

impl TextArea {
    pub fn new(min_lines: usize, max_lines: usize) -> TextArea {
        let min_lines = min_lines.max(1);
        TextArea {
            min_lines,
            max_lines: max_lines.max(min_lines),
            hover: false,
            editing: false,
        }
    }
    fn lines(value: &Value) -> usize {
        match value {
            Value::String(s) => s.split('\n').count(),
            _ => 1,
        }
    }
}

impl Widget for TextArea {
    fn measure(&self, _width: f32, value: &Value) -> f32 {
        TextArea::lines(value).clamp(self.min_lines, self.max_lines) as f32 * LINE_H + 8.0
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response {
        let mut r = Response::default();
        let Value::String(s) = value else { return r };
        match ev {
            InputEvent::PointerMove { pos, .. } => self.hover = rect.contains(*pos),
            InputEvent::PointerDown {
                pos,
                button: PointerButton::Primary,
                ..
            } if rect.contains(*pos) => {
                self.editing = true;
                r.text_edit = Some(TextEditRequest {
                    text: s.clone(),
                    multiline: true,
                });
                r.consumed = true;
            }
            _ => {}
        }
        r
    }

    fn commit_text(&mut self, text: &str, value: &mut Value) -> Response {
        self.editing = false;
        let mut r = Response::consumed();
        if let Value::String(s) = value
            && s != text
        {
            *s = text.to_string();
            r.changed = true;
        }
        r
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
    }

    fn captures(&self) -> bool {
        self.editing
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        draw_field(dl, rect, theme, self.hover, false, self.editing);
        let size = theme.n(Num::FontSize);
        let text = match value {
            Value::String(s) => s.as_str(),
            _ => "",
        };
        if text.is_empty() {
            dl.text(
                Layer::Nodes,
                vec2(rect.min.x + 6.0, rect.min.y + 4.0 + LINE_H * 0.5),
                label,
                size,
                theme.c(Col::TextDim),
                Align::Left,
            );
            return;
        }
        for (i, line) in text.split('\n').take(self.max_lines).enumerate() {
            let t = ellipsize(line, rect.width() - 12.0, size);
            let y = rect.min.y + 4.0 + LINE_H * (i as f32 + 0.5);
            dl.text(
                Layer::Nodes,
                vec2(rect.min.x + 6.0, y),
                &t,
                size,
                theme.c(Col::FieldText),
                Align::Left,
            );
        }
    }
}

// ──────────────────────────── color picker ────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PickDrag {
    Square,
    Hue,
    Alpha,
}

struct PopupLayout {
    panel: Rect,
    square: Rect,
    hue: Rect,
    alpha: Rect,
    hex: Rect,
}

fn popup_layout(rect: Rect) -> PopupLayout {
    const PAD: f32 = 8.0;
    const SQ: f32 = 150.0;
    const BAR: f32 = 16.0;
    const GAP: f32 = 6.0;
    let min = vec2(rect.min.x, rect.max.y + 2.0);
    let h = PAD + SQ + GAP + BAR + GAP + BAR + GAP + ROW_H + PAD;
    let square = Rect::from_min_size(min + vec2(PAD, PAD), vec2(SQ, SQ));
    let hue = Rect::from_min_size(square.min + vec2(0.0, SQ + GAP), vec2(SQ, BAR));
    let alpha = Rect::from_min_size(hue.min + vec2(0.0, BAR + GAP), vec2(SQ, BAR));
    let hex = Rect::from_min_size(alpha.min + vec2(0.0, BAR + GAP), vec2(SQ, ROW_H));
    PopupLayout {
        panel: Rect::from_min_size(min, vec2(SQ + 2.0 * PAD, h)),
        square,
        hue,
        alpha,
        hex,
    }
}

fn srgb_color(rgb: [f32; 3], a: f32) -> Color {
    Color::new(
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
        a,
    )
}

/// Swatch plus popup with an HSV square, a hue bar, an alpha bar and a hex
/// field. The picker works on sRGB-encoded HSV and stores linear color.
pub struct ColorPicker {
    open: bool,
    hsv: [f32; 3],
    alpha: f32,
    drag: Option<PickDrag>,
    hover: bool,
    label_frac: f32,
}

impl ColorPicker {
    pub fn new() -> ColorPicker {
        ColorPicker {
            open: false,
            hsv: [0.0, 0.0, 0.0],
            alpha: 1.0,
            drag: None,
            hover: false,
            label_frac: 0.35,
        }
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    fn load(&mut self, c: Color) {
        let enc = |l: f32| linear_to_srgb(l.clamp(0.0, 1.0));
        let [h, s, v] = rgb_to_hsv(enc(c.r), enc(c.g), enc(c.b));
        // keep the hue when it carries no information (gray or black)
        let h = if s <= 1e-6 || v <= 1e-6 {
            self.hsv[0]
        } else {
            h
        };
        self.hsv = [h, s, v];
        self.alpha = c.a.clamp(0.0, 1.0);
    }
    fn current(&self) -> Color {
        srgb_color(
            hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2]),
            self.alpha,
        )
    }
    fn apply(&self, value: &mut Value) -> bool {
        let new = Value::Color(self.current());
        if *value == new {
            false
        } else {
            *value = new;
            true
        }
    }
    fn drag_to(&mut self, p: Vec2, lay: &PopupLayout) {
        let unit = |v: f32, lo: f32, size: f32| ((v - lo) / size.max(1.0)).clamp(0.0, 1.0);
        match self.drag {
            Some(PickDrag::Square) => {
                self.hsv[1] = unit(p.x, lay.square.min.x, lay.square.width());
                self.hsv[2] = 1.0 - unit(p.y, lay.square.min.y, lay.square.height());
            }
            Some(PickDrag::Hue) => self.hsv[0] = unit(p.x, lay.hue.min.x, lay.hue.width()),
            Some(PickDrag::Alpha) => self.alpha = unit(p.x, lay.alpha.min.x, lay.alpha.width()),
            None => {}
        }
    }
}

impl Default for ColorPicker {
    fn default() -> Self {
        ColorPicker::new()
    }
}

impl Widget for ColorPicker {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response {
        let mut r = Response::default();
        let Value::Color(col) = value else { return r };
        let col = *col;
        if !self.open {
            let (_, ctrl) = split_row(rect, self.label_frac);
            match ev {
                InputEvent::PointerMove { pos, .. } => self.hover = ctrl.contains(*pos),
                InputEvent::PointerDown {
                    pos,
                    button: PointerButton::Primary,
                    ..
                } if ctrl.contains(*pos) => {
                    self.load(col);
                    self.open = true;
                    r.consumed = true;
                }
                _ => {}
            }
            return r;
        }
        let lay = popup_layout(rect);
        match ev {
            InputEvent::PointerMove { pos, .. } => {
                if self.drag.is_some() {
                    self.drag_to(*pos, &lay);
                    r.changed = self.apply(value);
                }
                r.consumed = true;
            }
            InputEvent::PointerDown {
                pos,
                button: PointerButton::Primary,
                ..
            } => {
                r.consumed = true;
                let hit = if lay.square.contains(*pos) {
                    Some(PickDrag::Square)
                } else if lay.hue.contains(*pos) {
                    Some(PickDrag::Hue)
                } else if lay.alpha.contains(*pos) {
                    Some(PickDrag::Alpha)
                } else {
                    None
                };
                if let Some(d) = hit {
                    self.drag = Some(d);
                    self.drag_to(*pos, &lay);
                    r.begin_edit = true;
                    r.changed = self.apply(value);
                } else if lay.hex.contains(*pos) {
                    r.text_edit = Some(TextEditRequest {
                        text: col.to_hex(),
                        multiline: false,
                    });
                } else if !lay.panel.contains(*pos) {
                    self.open = false;
                }
            }
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                if self.drag.take().is_some() {
                    r.end_edit = true;
                    r.consumed = true;
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                if self.drag.take().is_some() {
                    r.end_edit = true;
                }
                self.open = false;
                r.consumed = true;
            }
            _ => {}
        }
        r
    }

    fn commit_text(&mut self, text: &str, value: &mut Value) -> Response {
        let mut r = Response::consumed();
        let t = text.trim();
        if let Some(mut c) = Color::from_hex(t) {
            if t.trim_start_matches('#').len() == 6
                && let Value::Color(old) = &*value
            {
                c.a = old.a;
            }
            let new = Value::Color(c);
            if *value != new {
                *value = new;
                r.changed = true;
            }
            self.load(c);
        }
        r
    }

    fn captures(&self) -> bool {
        self.open
    }

    fn popup_rect(&self, rect: Rect) -> Option<Rect> {
        self.open.then(|| popup_layout(rect).panel)
    }

    fn set_label_frac(&mut self, frac: f32) {
        if frac.is_finite() {
            self.label_frac = frac.clamp(0.0, 0.85);
        }
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        let Value::Color(c) = value else { return };
        let (lab, ctrl) = split_row(rect, self.label_frac);
        draw_row_label(dl, lab, label, theme);
        draw_field(dl, ctrl, theme, self.hover || self.open, self.open, false);
        let size = theme.n(Num::FontSize);
        let swatch_w = ctrl.width().clamp(24.0, 90.0);
        let sw = Rect::new(
            vec2(ctrl.max.x - swatch_w - 2.0, ctrl.min.y + 2.0),
            vec2(ctrl.max.x - 2.0, ctrl.max.y - 2.0),
        );
        dl.push(
            Layer::Nodes,
            Prim::Checker {
                rect: sw,
                cell: 4.0,
                a: theme.c(Col::CheckerA),
                b: theme.c(Col::CheckerB),
            },
        );
        dl.rect(Layer::Nodes, sw, 3.0, *c);
        let left_half = Rect::new(sw.min, vec2(sw.center().x, sw.max.y));
        dl.rect(Layer::Nodes, left_half, 3.0, c.with_alpha(1.0));
        dl.rect_stroked(
            Layer::Nodes,
            sw,
            3.0,
            Color::TRANSPARENT,
            theme.c(Col::FieldBorder),
            1.0,
        );
        dl.text(
            Layer::Nodes,
            vec2(ctrl.min.x + 8.0, ctrl.center().y),
            &c.to_hex(),
            size,
            theme.c(Col::FieldText),
            Align::Left,
        );

        if self.open {
            let lay = popup_layout(rect);
            draw_panel(dl, lay.panel, theme, 6.0);
            let hue_col = srgb_color(hsv_to_rgb(self.hsv[0], 1.0, 1.0), 1.0);
            dl.push(
                Layer::Popup,
                Prim::GradientQuad {
                    rect: lay.square,
                    colors: [Color::WHITE, hue_col, Color::BLACK, Color::BLACK],
                },
            );
            let cur = vec2(
                lay.square.min.x + self.hsv[1] * lay.square.width(),
                lay.square.min.y + (1.0 - self.hsv[2]) * lay.square.height(),
            );
            let knob = |dl: &mut DrawList, p: Vec2| {
                dl.rect_stroked(
                    Layer::Popup,
                    Rect::from_center_size(p, vec2(11.0, 11.0)),
                    5.5,
                    Color::TRANSPARENT,
                    Color::BLACK.with_alpha(0.4),
                    3.0,
                );
                dl.rect_stroked(
                    Layer::Popup,
                    Rect::from_center_size(p, vec2(11.0, 11.0)),
                    5.5,
                    Color::TRANSPARENT,
                    Color::WHITE,
                    1.8,
                );
            };
            knob(dl, cur);
            let n = 6;
            let seg = lay.hue.width() / n as f32;
            for i in 0..n {
                let (h0, h1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
                let (c0, c1) = (
                    srgb_color(hsv_to_rgb(h0, 1.0, 1.0), 1.0),
                    srgb_color(hsv_to_rgb(h1, 1.0, 1.0), 1.0),
                );
                let seg_rect = Rect::from_min_size(
                    vec2(lay.hue.min.x + seg * i as f32, lay.hue.min.y),
                    vec2(seg, lay.hue.height()),
                );
                dl.push(
                    Layer::Popup,
                    Prim::GradientQuad {
                        rect: seg_rect,
                        colors: [c0, c1, c0, c1],
                    },
                );
            }
            knob(
                dl,
                vec2(
                    lay.hue.min.x + self.hsv[0] * lay.hue.width(),
                    lay.hue.center().y,
                ),
            );
            let opaque = srgb_color(hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2]), 1.0);
            dl.push(
                Layer::Popup,
                Prim::Checker {
                    rect: lay.alpha,
                    cell: 4.0,
                    a: theme.c(Col::CheckerA),
                    b: theme.c(Col::CheckerB),
                },
            );
            let clear = opaque.with_alpha(0.0);
            dl.push(
                Layer::Popup,
                Prim::GradientQuad {
                    rect: lay.alpha,
                    colors: [clear, opaque, clear, opaque],
                },
            );
            knob(
                dl,
                vec2(
                    lay.alpha.min.x + self.alpha * lay.alpha.width(),
                    lay.alpha.center().y,
                ),
            );
            draw_field(dl, lay.hex, theme, true, false, false);
            dl.text(
                Layer::Popup,
                lay.hex.center(),
                &c.to_hex(),
                size,
                theme.c(Col::FieldText),
                Align::Center,
            );
        }
    }
}

// ──────────────────────────── image preview ────────────────────────────

/// Shows an image the host provided (by texture id), aspect-fit on a
/// checkerboard so transparency is visible.
pub struct ImagePreview {
    aspect: f32,
}

impl ImagePreview {
    pub fn new() -> ImagePreview {
        ImagePreview { aspect: 1.0 }
    }
    /// Where the image lands inside `rect` (aspect-fit, centered).
    pub fn fit_rect(&self, rect: Rect) -> Rect {
        let (rw, rh) = (rect.width(), rect.height());
        let (w, h) = if rh / rw.max(1e-6) > self.aspect {
            (rw, rw * self.aspect)
        } else {
            (rh / self.aspect, rh)
        };
        Rect::from_center_size(rect.center(), vec2(w, h))
    }
}

impl Default for ImagePreview {
    fn default() -> Self {
        ImagePreview::new()
    }
}

impl Widget for ImagePreview {
    fn measure(&self, width: f32, _value: &Value) -> f32 {
        (width * self.aspect).clamp(ROW_H * 2.0, 320.0)
    }

    fn event(&mut self, _ev: &InputEvent, _rect: Rect, _value: &mut Value) -> Response {
        Response::default()
    }

    fn set_aspect(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.0 {
            self.aspect = aspect;
        }
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, _label: &str, theme: &Theme) {
        match value {
            Value::Image(Some(h)) => {
                let fit = self.fit_rect(rect);
                dl.push(
                    Layer::Nodes,
                    Prim::Checker {
                        rect: fit,
                        cell: 8.0,
                        a: theme.c(Col::CheckerA),
                        b: theme.c(Col::CheckerB),
                    },
                );
                dl.push(
                    Layer::Nodes,
                    Prim::Image {
                        rect: fit,
                        texture: h.0,
                        tint: Color::WHITE,
                    },
                );
            }
            _ => {
                dl.rect(
                    Layer::Nodes,
                    rect,
                    theme.n(Num::FieldRadius),
                    theme.c(Col::FieldPressed),
                );
                dl.text(
                    Layer::Nodes,
                    rect.center(),
                    "No image",
                    theme.n(Num::FontSize),
                    theme.c(Col::TextDim),
                    Align::Center,
                );
            }
        }
    }
}

// ─────────────────────────────── label ───────────────────────────────

/// Static or computed text with ellipsis truncation. Never interactive.
pub struct Label {
    computed: Option<fn(&Value) -> String>,
}

impl Label {
    pub fn new() -> Label {
        Label { computed: None }
    }
    pub fn computed(f: fn(&Value) -> String) -> Label {
        Label { computed: Some(f) }
    }
}

impl Default for Label {
    fn default() -> Self {
        Label::new()
    }
}

impl Widget for Label {
    fn measure(&self, _width: f32, _value: &Value) -> f32 {
        ROW_H
    }

    fn event(&mut self, _ev: &InputEvent, _rect: Rect, _value: &mut Value) -> Response {
        Response::default()
    }

    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme) {
        let text = match (self.computed, value) {
            (Some(f), v) => f(v),
            (None, Value::String(s)) if !s.is_empty() => s.clone(),
            _ => label.to_string(),
        };
        let size = theme.n(Num::FontSize);
        let t = ellipsize(&text, rect.width(), size);
        dl.text(
            Layer::Nodes,
            vec2(rect.min.x, rect.center().y),
            &t,
            size,
            theme.c(Col::TextDim),
            Align::Left,
        );
    }
}

// ─────────────────────── footer / preview spacer ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;
    use crate::value::ImageHandle;

    fn rect() -> Rect {
        Rect::from_min_size(vec2(0.0, 0.0), vec2(120.0, 18.0))
    }
    fn at(x: f32) -> Vec2 {
        vec2(x, 9.0)
    }
    fn fv(v: &Value) -> f32 {
        v.as_f32().unwrap()
    }
    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn float_field_scrub_round_trip_and_gesture_flags() {
        let mut w = NumberField::float(0.0, 10.0, 0.1);
        let mut v = Value::Float(5.0);
        assert!(
            w.event(&InputEvent::down(at(60.0)), rect(), &mut v)
                .consumed
        );
        assert!(w.captures());
        let r = w.event(&InputEvent::mv(at(100.0)), rect(), &mut v); // dx = 40 -> +1.0
        assert!(r.begin_edit && r.changed && !r.end_edit);
        assert!(approx(fv(&v), 6.0), "{v:?}");
        let r = w.event(&InputEvent::up(at(100.0)), rect(), &mut v);
        assert!(r.end_edit && r.text_edit.is_none());
        assert!(!w.captures());
        assert_eq!(w.measure(120.0, &v), ROW_H);
    }

    #[test]
    fn float_field_modifiers_clamp_escape_and_typing() {
        let mut w = NumberField::float(0.0, 10.0, 0.1);
        let mut v = Value::Float(5.0);
        // Shift = fine (x0.1)
        w.event(&InputEvent::down(at(60.0)), rect(), &mut v);
        w.event(
            &InputEvent::mv(at(100.0)).with_mods(Modifiers::SHIFT),
            rect(),
            &mut v,
        );
        assert!(approx(fv(&v), 5.1), "{v:?}");
        // Escape restores the value from before the drag
        let r = w.event(&InputEvent::key(Key::Escape), rect(), &mut v);
        assert!(r.changed && r.end_edit);
        assert!(approx(fv(&v), 5.0));
        // Ctrl = snap to step
        w.event(&InputEvent::down(at(60.0)), rect(), &mut v);
        w.event(
            &InputEvent::mv(at(77.0)).with_mods(Modifiers::CTRL),
            rect(),
            &mut v,
        ); // 5 + 0.425
        assert!(approx(fv(&v), 5.4), "{v:?}");
        w.event(&InputEvent::up(at(77.0)), rect(), &mut v);
        // click without moving asks for text entry
        w.event(&InputEvent::down(at(60.0)), rect(), &mut v);
        let r = w.event(&InputEvent::up(at(60.0)), rect(), &mut v);
        assert_eq!(
            r.text_edit,
            Some(TextEditRequest {
                text: "5.4".into(),
                multiline: false
            })
        );
        // typed values: parsed, clamped, junk ignored
        assert!(w.commit_text("7.25", &mut v).changed);
        assert!(approx(fv(&v), 7.25));
        assert!(!w.commit_text("abc", &mut v).changed);
        assert!(w.commit_text("100", &mut v).changed);
        assert!(approx(fv(&v), 10.0));
        // arrows step, and the hard maximum stops them
        let r = w.event(&InputEvent::down(at(4.0)), rect(), &mut v);
        assert!(r.changed && r.begin_edit && r.end_edit);
        assert!(approx(fv(&v), 9.9));
        let mut top = Value::Float(10.0);
        assert!(
            !w.event(&InputEvent::down(at(116.0)), rect(), &mut top)
                .changed
        );
    }

    #[test]
    fn int_field_steps_in_whole_numbers() {
        let mut w = NumberField::int(0.0, 100.0, 1.0);
        let mut v = Value::Int(10);
        w.event(&InputEvent::down(at(60.0)), rect(), &mut v);
        w.event(&InputEvent::mv(at(84.0)), rect(), &mut v); // 24 px * 1/8 = +3
        assert_eq!(v, Value::Int(13));
        w.event(&InputEvent::up(at(84.0)), rect(), &mut v);
        w.event(&InputEvent::down(at(116.0)), rect(), &mut v);
        assert_eq!(v, Value::Int(14));
        assert!(w.commit_text("42.6", &mut v).changed);
        assert_eq!(v, Value::Int(43));
        // wrong value type is ignored
        let mut s = Value::String("x".into());
        assert_eq!(
            w.event(&InputEvent::down(at(60.0)), rect(), &mut s),
            Response::default()
        );
    }

    #[test]
    fn slider_sets_from_position_and_restores_on_escape() {
        let mut w = Slider::new(0.0, 1.0);
        let mut v = Value::Float(0.9);
        let r = w.event(&InputEvent::down(at(30.0)), rect(), &mut v);
        assert!(r.begin_edit && r.changed);
        assert!(approx(fv(&v), 0.25));
        w.event(&InputEvent::mv(at(90.0)), rect(), &mut v);
        assert!(approx(fv(&v), 0.75));
        w.event(&InputEvent::mv(at(500.0)), rect(), &mut v); // clamps at max
        assert!(approx(fv(&v), 1.0));
        let r = w.event(&InputEvent::key(Key::Escape), rect(), &mut v);
        assert!(r.end_edit);
        assert!(approx(fv(&v), 0.9));
        // up ends the gesture
        w.event(&InputEvent::down(at(60.0)), rect(), &mut v);
        assert!(w.event(&InputEvent::up(at(60.0)), rect(), &mut v).end_edit);
        // Shift-click does not jump; Ctrl-click types
        let mut v = Value::Float(0.5);
        let r = w.event(
            &InputEvent::down(at(10.0)).with_mods(Modifiers::SHIFT),
            rect(),
            &mut v,
        );
        assert!(!r.changed && approx(fv(&v), 0.5));
        w.event(&InputEvent::up(at(10.0)), rect(), &mut v);
        let r = w.event(
            &InputEvent::down(at(10.0)).with_mods(Modifiers::CTRL),
            rect(),
            &mut v,
        );
        assert!(r.text_edit.is_some());
        assert!(w.commit_text("0.2", &mut v).changed);
        assert!(approx(fv(&v), 0.2));
        // int slider rounds
        let mut iv = Value::Int(0);
        let mut w = Slider::new(0.0, 10.0);
        w.event(&InputEvent::down(at(60.0)), rect(), &mut iv);
        assert_eq!(iv, Value::Int(5));
    }

    #[test]
    fn toggle_flips_on_click_and_ignores_other_types() {
        let mut w = Toggle::new(ToggleStyle::Switch);
        let mut v = Value::Bool(false);
        let r = w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        assert!(r.changed && r.begin_edit && r.end_edit);
        assert_eq!(v, Value::Bool(true));
        assert!(
            !w.event(&InputEvent::down(at(500.0)), rect(), &mut v)
                .changed
        );
        // keyboard needs focus
        assert!(
            !w.event(&InputEvent::key(Key::Space), rect(), &mut v)
                .changed
        );
        w.set_focused(true);
        assert!(
            w.event(&InputEvent::key(Key::Space), rect(), &mut v)
                .changed
        );
        assert_eq!(v, Value::Bool(false));
        let mut f = Value::Float(0.0);
        assert!(!w.event(&InputEvent::down(at(10.0)), rect(), &mut f).changed);
    }

    #[test]
    fn button_clicks_only_when_released_inside() {
        let mut w = Button::new();
        let mut v = Value::Bool(false);
        w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        assert!(w.captures());
        assert!(w.event(&InputEvent::up(at(10.0)), rect(), &mut v).clicked);
        assert!(!w.captures());
        w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        assert!(!w.event(&InputEvent::up(at(500.0)), rect(), &mut v).clicked);
        assert_eq!(v, Value::Bool(false)); // the widget never edits the value
    }

    #[test]
    fn dropdown_pointer_and_keyboard_selection() {
        let opts: Vec<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        let mut w = Dropdown::new(opts);
        let mut v = Value::Enum(0);
        let r = w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        assert!(r.begin_edit && w.captures() && w.is_open());
        let pr = w.popup_rect(rect()).expect("popup");
        assert_eq!(pr.height(), 3.0 * ROW_H);
        let third = vec2(10.0, pr.min.y + 2.0 * ROW_H + 5.0);
        let r = w.event(&InputEvent::down(third), rect(), &mut v);
        assert!(r.changed && r.end_edit);
        assert_eq!(v, Value::Enum(2));
        assert!(!w.captures() && w.popup_rect(rect()).is_none());
        // keyboard: open, Up, Enter picks "B"
        w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        w.event(&InputEvent::key(Key::Up), rect(), &mut v);
        let r = w.event(&InputEvent::key(Key::Enter), rect(), &mut v);
        assert!(r.changed);
        assert_eq!(v, Value::Enum(1));
        // click outside closes without changing
        w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        let r = w.event(&InputEvent::down(vec2(900.0, 900.0)), rect(), &mut v);
        assert!(!r.changed && r.end_edit && !w.is_open());
        assert_eq!(v, Value::Enum(1));
        // focused arrow keys change the value without opening
        w.set_focused(true);
        w.event(&InputEvent::key(Key::Down), rect(), &mut v);
        assert_eq!(v, Value::Enum(2));
    }

    #[test]
    fn text_area_height_grows_with_lines_and_edits_through_overlay() {
        let mut w = TextArea::new(2, 4);
        let one = Value::String("a".into());
        let three = Value::String("a\nb\nc".into());
        let many = Value::String("1\n2\n3\n4\n5\n6".into());
        assert_eq!(w.measure(100.0, &one), 2.0 * LINE_H + 8.0);
        assert_eq!(w.measure(100.0, &three), 3.0 * LINE_H + 8.0);
        assert_eq!(w.measure(100.0, &many), 4.0 * LINE_H + 8.0);
        let mut v = three.clone();
        let r = w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        assert_eq!(
            r.text_edit,
            Some(TextEditRequest {
                text: "a\nb\nc".into(),
                multiline: true
            })
        );
        assert!(w.captures());
        let r = w.commit_text("hello", &mut v);
        assert!(r.changed && !w.captures());
        assert_eq!(v, Value::String("hello".into()));
        w.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        w.cancel_edit();
        assert!(!w.captures());
        assert!(!w.commit_text("hello", &mut v).changed);
    }

    #[test]
    fn color_picker_hue_square_alpha_and_hex() {
        let red = Color::from_hex("#ff0000").unwrap();
        let mut v = Value::Color(red);
        let mut w = ColorPicker::new();
        let r = w.event(&InputEvent::down(at(60.0)), rect(), &mut v);
        assert!(r.consumed && w.is_open());
        assert_eq!(v, Value::Color(red)); // opening changes nothing
        let lay = popup_layout(rect());
        assert_eq!(w.popup_rect(rect()), Some(lay.panel));

        // hue bar middle: cyan
        let r = w.event(
            &InputEvent::down(vec2(lay.hue.center().x, lay.hue.center().y)),
            rect(),
            &mut v,
        );
        assert!(r.begin_edit && r.changed);
        assert!(
            w.event(&InputEvent::up(lay.hue.center()), rect(), &mut v)
                .end_edit
        );
        match &v {
            Value::Color(c) => assert_eq!(c.to_hex(), "#00ffff"),
            other => panic!("{other:?}"),
        }
        // square: left edge is white at the top
        w.event(&InputEvent::down(lay.square.min), rect(), &mut v);
        w.event(&InputEvent::up(lay.square.min), rect(), &mut v);
        match &v {
            Value::Color(c) => assert_eq!(c.to_hex(), "#ffffff"),
            other => panic!("{other:?}"),
        }
        // alpha bar left edge: transparent
        w.event(
            &InputEvent::down(lay.alpha.min + vec2(0.0, 1.0)),
            rect(),
            &mut v,
        );
        w.event(&InputEvent::up(lay.alpha.min), rect(), &mut v);
        match &v {
            Value::Color(c) => assert!(c.a < 0.01),
            other => panic!("{other:?}"),
        }
        // hex field asks for text; a 6-digit hex keeps the alpha
        let r = w.event(&InputEvent::down(lay.hex.center()), rect(), &mut v);
        assert!(r.text_edit.is_some());
        let r = w.commit_text("#00ff00", &mut v);
        assert!(r.changed);
        match &v {
            Value::Color(c) => {
                assert_eq!(c.to_srgb8()[..3], [0, 255, 0]);
                assert!(c.a < 0.01);
            }
            other => panic!("{other:?}"),
        }
        assert!(!w.commit_text("nonsense", &mut v).changed);
        // clicking outside closes
        w.event(&InputEvent::down(vec2(900.0, 900.0)), rect(), &mut v);
        assert!(!w.is_open());
    }

    #[test]
    fn color_picker_keeps_hue_through_gray() {
        let mut w = ColorPicker::new();
        w.hsv = [0.6, 1.0, 1.0];
        w.load(Color::rgb(0.5, 0.5, 0.5));
        assert_eq!(w.hsv[0], 0.6);
        assert_eq!(w.hsv[1], 0.0);
    }

    #[test]
    fn image_preview_fits_aspect_and_draws_image_prim() {
        let mut w = ImagePreview::new();
        assert_eq!(w.measure(100.0, &Value::Image(None)), 100.0);
        w.set_aspect(0.5);
        assert_eq!(w.measure(100.0, &Value::Image(None)), 50.0);
        w.set_aspect(f32::NAN); // ignored
        assert_eq!(w.measure(100.0, &Value::Image(None)), 50.0);
        let area = Rect::from_min_size(vec2(0.0, 0.0), vec2(100.0, 100.0));
        let fit = w.fit_rect(area);
        assert_eq!((fit.width(), fit.height()), (100.0, 50.0));
        assert_eq!(fit.min.y, 25.0);
        let mut dl = DrawList::new();
        w.draw(
            &mut dl,
            area,
            &Value::Image(Some(ImageHandle(7))),
            "",
            &Theme::blender_dark(),
        );
        assert!(
            dl.layer(Layer::Nodes)
                .iter()
                .any(|p| matches!(p, Prim::Checker { .. }))
        );
        assert!(
            dl.layer(Layer::Nodes)
                .iter()
                .any(|p| matches!(p, Prim::Image { texture: 7, .. }))
        );
        let mut dl = DrawList::new();
        w.draw(
            &mut dl,
            area,
            &Value::Image(None),
            "",
            &Theme::blender_dark(),
        );
        assert!(
            !dl.layer(Layer::Nodes)
                .iter()
                .any(|p| matches!(p, Prim::Image { .. }))
        );
        // never interactive
        let mut v = Value::Image(None);
        assert_eq!(
            w.event(&InputEvent::down(at(5.0)), area, &mut v),
            Response::default()
        );
    }

    #[test]
    fn label_static_computed_and_truncated() {
        let theme = Theme::blender_dark();
        let r = Rect::from_min_size(vec2(0.0, 0.0), vec2(40.0, 18.0));
        let texts = |w: &Label, v: &Value, label: &str| {
            let mut dl = DrawList::new();
            w.draw(&mut dl, r, v, label, &theme);
            dl.layer(Layer::Nodes)
                .iter()
                .filter_map(|p| {
                    if let Prim::Text { text, .. } = p {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            texts(&Label::new(), &Value::String("hi".into()), ""),
            vec!["hi".to_string()]
        );
        assert_eq!(
            texts(&Label::new(), &Value::Float(0.0), "name"),
            vec!["name".to_string()]
        );
        let c = Label::computed(|v| format!("<{}>", value_text(v)));
        assert_eq!(texts(&c, &Value::Int(3), ""), vec!["<3>".to_string()]);
        let long = texts(
            &Label::new(),
            &Value::String("a very long label indeed".into()),
            "",
        );
        assert!(long[0].ends_with('\u{2026}'));
        assert_eq!(Label::new().measure(10.0, &Value::Int(0)), ROW_H);
    }

    #[test]
    fn widgets_draw_something_for_every_state() {
        let theme = Theme::blender_dark();
        let mut dl = DrawList::new();
        let mut d = Dropdown::new(vec!["A".into(), "B".into()]);
        let mut v = Value::Enum(0);
        d.event(&InputEvent::down(at(10.0)), rect(), &mut v);
        d.draw(&mut dl, rect(), &v, "Mode", &theme);
        assert!(!dl.layer(Layer::Popup).is_empty());
        let mut dl = DrawList::new();
        let mut c = ColorPicker::new();
        let mut cv = Value::Color(Color::WHITE);
        c.event(&InputEvent::down(at(60.0)), rect(), &mut cv);
        c.draw(&mut dl, rect(), &cv, "Color", &theme);
        assert!(
            dl.layer(Layer::Popup)
                .iter()
                .filter(|p| matches!(p, Prim::GradientQuad { .. }))
                .count()
                >= 8
        );
    }
}
