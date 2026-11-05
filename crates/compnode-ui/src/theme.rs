//! The visual design system. Every color, size and font setting the editor
//! uses lives in a [`Theme`]; changing the theme (in code or from a text file)
//! changes the whole look with no other code changes.
use crate::schema::Category;
use crate::value::SocketType;
use compnode_core::Color;

macro_rules! keys {
    ($(#[$m:meta])* $name:ident { $($var:ident => $s:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum $name { $($var),* }
        impl $name {
            #[allow(clippy::redundant_static_lifetimes)]
            pub const ALL: &'static [$name] = &[$($name::$var),*];
            /// Name used in the theme text format.
            pub fn key(self) -> &'static str {
                match self { $($name::$var => $s),* }
            }
        }
    };
}

keys! {
    /// Themeable colors.
    Col {
        Background => "background",
        GridMinor => "grid_minor",
        GridMajor => "grid_major",
        NodeBody => "node_body",
        NodeBodyTop => "node_body_top",
        NodeBodyBottom => "node_body_bottom",
        NodeBorder => "node_border",
        NodeShadow => "node_shadow",
        NodeHighlight => "node_highlight",
        Text => "text",
        TextDim => "text_dim",
        HeaderText => "header_text",
        HeaderAccent => "header_accent",
        Hover => "hover",
        Selected => "selected",
        Active => "active",
        SelectionGlow => "selection_glow",
        Error => "error",
        FrameFill => "frame_fill",
        FrameBorder => "frame_border",
        RowHover => "row_hover",
        WireHighlight => "wire_highlight",
        WireShadow => "wire_shadow",
        WireCut => "wire_cut",
        WireInvalid => "wire_invalid",
        WireInsert => "wire_insert",
        FieldFill => "field_fill",
        FieldBorder => "field_border",
        FieldHover => "field_hover",
        FieldPressed => "field_pressed",
        FieldFocus => "field_focus",
        FieldShadow => "field_shadow",
        FieldText => "field_text",
        SliderFill => "slider_fill",
        Popup => "popup",
        PopupBorder => "popup_border",
        PopupHover => "popup_hover",
        ToggleOn => "toggle_on",
        ToggleOff => "toggle_off",
        PinRing => "pin_ring",
        Guide => "guide",
        BoxFill => "box_fill",
        BoxBorder => "box_border",
        CheckerA => "checker_a",
        CheckerB => "checker_b",
    }
}

keys! {
    /// Themeable numbers (sizes in graph units at zoom 1, line widths in pixels).
    Num {
        Radius => "radius",
        FontSize => "font_size",
        PinRadius => "pin_radius",
        PinRingWidth => "pin_ring_width",
        WireWidth => "wire_width",
        BorderWidth => "border_width",
        SelectedWidth => "selected_width",
        GlowWidth => "glow_width",
        ShadowOffset => "shadow_offset",
        ShadowSpread => "shadow_spread",
        HeaderAccentH => "header_accent_h",
        MutedAlpha => "muted_alpha",
        WireDimAlpha => "wire_dim_alpha",
        GridStep => "grid_step",
        GridMajorEvery => "grid_major_every",
        FieldRadius => "field_radius",
        FieldInset => "field_inset",
        FieldFocusWidth => "field_focus_width",
        WidgetLabelFrac => "widget_label_frac",
    }
}

/// How a node is drawn. Later entries win when several apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeState {
    Normal,
    Hovered,
    Selected,
    Active,
    Muted,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    colors: Vec<Color>,
    nums: Vec<f32>,
    sockets: [Color; 9],
    headers: [Color; 8],
    /// Bumped by every change so hosts can invalidate cached meshes.
    pub revision: u64,
}

fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap_or(Color::BLACK)
}

impl Theme {
    pub fn c(&self, c: Col) -> Color {
        self.colors[c as usize]
    }
    pub fn n(&self, n: Num) -> f32 {
        self.nums[n as usize]
    }
    pub fn socket(&self, t: SocketType) -> Color {
        self.sockets[t.index()]
    }
    pub fn header(&self, c: Category) -> Color {
        self.headers[c.index()]
    }
    pub fn set_c(&mut self, c: Col, v: Color) {
        self.colors[c as usize] = v;
        self.revision += 1;
    }
    pub fn set_n(&mut self, n: Num, v: f32) {
        self.nums[n as usize] = v;
        self.revision += 1;
    }
    pub fn set_socket(&mut self, t: SocketType, v: Color) {
        self.sockets[t.index()] = v;
        self.revision += 1;
    }
    pub fn set_header(&mut self, c: Category, v: Color) {
        self.headers[c.index()] = v;
        self.revision += 1;
    }

    /// Outline color and width (pixels) for a node state, if it has one.
    pub fn state_outline(&self, s: NodeState) -> Option<(Color, f32)> {
        match s {
            NodeState::Normal | NodeState::Muted => None,
            NodeState::Hovered => Some((self.c(Col::Hover), self.n(Num::BorderWidth))),
            NodeState::Selected => Some((self.c(Col::Selected), self.n(Num::SelectedWidth))),
            NodeState::Active => Some((self.c(Col::Active), self.n(Num::SelectedWidth))),
            NodeState::Error => Some((self.c(Col::Error), self.n(Num::SelectedWidth))),
        }
    }

    /// Wire width in graph units so that it is `WireWidth` pixels at zoom 1
    /// and grows/shrinks gently with zoom (clamped to 0.4x..1.5x).
    pub fn wire_width_graph(&self, zoom: f32) -> f32 {
        self.n(Num::WireWidth) * zoom.clamp(0.4, 1.5) / zoom
    }

    fn base(name: &str) -> Theme {
        Theme {
            name: name.to_string(),
            colors: vec![Color::BLACK; Col::ALL.len()],
            nums: vec![0.0; Num::ALL.len()],
            sockets: [Color::BLACK; 9],
            headers: [Color::BLACK; 8],
            revision: 0,
        }
    }

    fn apply_common_numbers(&mut self) {
        self.nums[Num::Radius as usize] = 6.0;
        self.nums[Num::FontSize as usize] = 12.0;
        self.nums[Num::PinRadius as usize] = 4.5;
        self.nums[Num::PinRingWidth as usize] = 1.5;
        self.nums[Num::WireWidth as usize] = 2.0;
        self.nums[Num::BorderWidth as usize] = 1.0;
        self.nums[Num::SelectedWidth as usize] = 2.0;
        self.nums[Num::GlowWidth as usize] = 3.0;
        self.nums[Num::ShadowOffset as usize] = 3.0;
        self.nums[Num::ShadowSpread as usize] = 1.0;
        self.nums[Num::HeaderAccentH as usize] = 1.0;
        self.nums[Num::MutedAlpha as usize] = 0.45;
        self.nums[Num::WireDimAlpha as usize] = 0.3;
        self.nums[Num::GridStep as usize] = 20.0;
        self.nums[Num::GridMajorEvery as usize] = 5.0;
        self.nums[Num::FieldRadius as usize] = 4.0;
        self.nums[Num::FieldInset as usize] = 12.0;
        self.nums[Num::FieldFocusWidth as usize] = 1.5;
        self.nums[Num::WidgetLabelFrac as usize] = 0.35;
    }

    /// The default dark theme, in the spirit of Blender's node editor.
    pub fn blender_dark() -> Theme {
        let mut t = Theme::base("blender-dark");
        t.apply_common_numbers();
        let set = |t: &mut Theme, c: Col, h: &str| t.colors[c as usize] = hex(h);
        set(&mut t, Col::Background, "#1d1d1d");
        set(&mut t, Col::GridMinor, "#232323");
        set(&mut t, Col::GridMajor, "#2b2b2b");
        set(&mut t, Col::NodeBody, "#303030");
        set(&mut t, Col::NodeBodyTop, "#343434");
        set(&mut t, Col::NodeBodyBottom, "#2d2d2d");
        set(&mut t, Col::NodeBorder, "#0f0f0f");
        set(&mut t, Col::NodeShadow, "#000000");
        set(&mut t, Col::NodeHighlight, "#ffffff14");
        set(&mut t, Col::Text, "#e6e6e6");
        set(&mut t, Col::TextDim, "#a0a0a0");
        set(&mut t, Col::HeaderText, "#ffffff");
        set(&mut t, Col::HeaderAccent, "#ffffff30");
        set(&mut t, Col::Hover, "#8c8c8c");
        set(&mut t, Col::Selected, "#ed5700");
        set(&mut t, Col::Active, "#ffffff");
        set(&mut t, Col::SelectionGlow, "#ed5700");
        set(&mut t, Col::Error, "#e04a4a");
        set(&mut t, Col::FrameFill, "#2a2a2a80");
        set(&mut t, Col::FrameBorder, "#4a4a4a");
        set(&mut t, Col::RowHover, "#ffffff0d");
        set(&mut t, Col::WireHighlight, "#ffffff");
        set(&mut t, Col::WireShadow, "#000000");
        set(&mut t, Col::WireCut, "#ff4040");
        set(&mut t, Col::WireInvalid, "#d04040");
        set(&mut t, Col::WireInsert, "#4da3ff");
        set(&mut t, Col::FieldFill, "#545454");
        set(&mut t, Col::FieldBorder, "#303030");
        set(&mut t, Col::FieldHover, "#656565");
        set(&mut t, Col::FieldPressed, "#3d3d3d");
        set(&mut t, Col::FieldFocus, "#4772b3");
        set(&mut t, Col::FieldShadow, "#00000033");
        set(&mut t, Col::FieldText, "#ffffff");
        set(&mut t, Col::SliderFill, "#4772b3");
        set(&mut t, Col::Popup, "#1d1d1d");
        set(&mut t, Col::PopupBorder, "#000000");
        set(&mut t, Col::PopupHover, "#4772b3");
        set(&mut t, Col::ToggleOn, "#4772b3");
        set(&mut t, Col::ToggleOff, "#545454");
        set(&mut t, Col::PinRing, "#101010");
        set(&mut t, Col::Guide, "#ed5700");
        set(&mut t, Col::BoxFill, "#ffffff12");
        set(&mut t, Col::BoxBorder, "#ffffffa0");
        set(&mut t, Col::CheckerA, "#7a7a7a");
        set(&mut t, Col::CheckerB, "#9a9a9a");
        let sock = [
            (SocketType::Float, "#a1a1a1"),
            (SocketType::Int, "#598c5c"),
            (SocketType::Bool, "#cca6d6"),
            (SocketType::Vec3, "#6363c7"),
            (SocketType::Color, "#c7c729"),
            (SocketType::String, "#70b2ff"),
            (SocketType::Enum, "#d68f4f"),
            (SocketType::Image, "#f4a9a8"),
            (SocketType::Any, "#808080"),
        ];
        for (ty, h) in sock {
            t.sockets[ty.index()] = hex(h);
        }
        let heads = [
            (Category::Input, "#83314a"),
            (Category::Output, "#4d3a6b"),
            (Category::Filter, "#3b5d7a"),
            (Category::Color, "#6b6b2a"),
            (Category::Blend, "#2f6b5a"),
            (Category::Transform, "#6b4a2f"),
            (Category::Utility, "#4a4a4a"),
            (Category::Layout, "#3a3a3a"),
        ];
        for (c, h) in heads {
            t.headers[c.index()] = hex(h);
        }
        t
    }

    /// A light variant, mainly to show that the theme is data.
    pub fn light() -> Theme {
        let mut t = Theme::blender_dark();
        t.name = "light".to_string();
        let set = |t: &mut Theme, c: Col, h: &str| t.colors[c as usize] = hex(h);
        set(&mut t, Col::Background, "#e4e4e4");
        set(&mut t, Col::GridMinor, "#dadada");
        set(&mut t, Col::GridMajor, "#cfcfcf");
        set(&mut t, Col::NodeBody, "#f2f2f2");
        set(&mut t, Col::NodeBodyTop, "#fafafa");
        set(&mut t, Col::NodeBodyBottom, "#eaeaea");
        set(&mut t, Col::NodeBorder, "#9a9a9a");
        set(&mut t, Col::NodeShadow, "#00000028");
        set(&mut t, Col::NodeHighlight, "#00000010");
        set(&mut t, Col::Text, "#202020");
        set(&mut t, Col::TextDim, "#606060");
        set(&mut t, Col::HeaderText, "#161616");
        set(&mut t, Col::HeaderAccent, "#00000030");
        set(&mut t, Col::Selected, "#e07000");
        set(&mut t, Col::Active, "#f09020");
        set(&mut t, Col::SelectionGlow, "#e07000");
        set(&mut t, Col::Error, "#c02020");
        set(&mut t, Col::RowHover, "#00000008");
        set(&mut t, Col::WireShadow, "#00000030");
        set(&mut t, Col::FieldFill, "#d2d2d2");
        set(&mut t, Col::FieldBorder, "#b0b0b0");
        set(&mut t, Col::FieldHover, "#c4c4c4");
        set(&mut t, Col::FieldPressed, "#b4b4b4");
        set(&mut t, Col::FieldFocus, "#e07000");
        set(&mut t, Col::FieldShadow, "#0000001f");
        set(&mut t, Col::FieldText, "#101010");
        set(&mut t, Col::Popup, "#f8f8f8");
        set(&mut t, Col::PopupBorder, "#808080");
        set(&mut t, Col::FrameFill, "#c8c8c880");
        set(&mut t, Col::FrameBorder, "#8a8a8a");
        set(&mut t, Col::WireHighlight, "#000000");
        set(&mut t, Col::PinRing, "#ffffff");
        set(&mut t, Col::Guide, "#e07000");
        set(&mut t, Col::BoxBorder, "#000000a0");
        set(&mut t, Col::CheckerA, "#c0c0c0");
        set(&mut t, Col::CheckerB, "#e0e0e0");
        t.revision = 0;
        t
    }

    /// Serialize to the text format read by [`Theme::from_text`].
    pub fn to_text(&self) -> String {
        let mut s = format!("theme = {}\n", self.name);
        for &c in Col::ALL {
            s.push_str(&format!("col.{} = {}\n", c.key(), self.c(c).to_hex()));
        }
        for &n in Num::ALL {
            s.push_str(&format!("num.{} = {}\n", n.key(), self.n(n)));
        }
        for t in SocketType::ALL {
            s.push_str(&format!(
                "socket.{} = {}\n",
                t.name(),
                self.socket(t).to_hex()
            ));
        }
        for c in Category::ALL {
            s.push_str(&format!(
                "header.{} = {}\n",
                c.name(),
                self.header(c).to_hex()
            ));
        }
        s
    }

    /// Parse `key = value` lines (`#` starts a comment only at the start of
    /// a line, since colors begin with `#`). Starts from the dark theme, so a
    /// file may override just a few keys. Unknown keys are an error.
    pub fn from_text(text: &str) -> Result<Theme, String> {
        let mut t = Theme::blender_dark();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty()
                || line.starts_with("//")
                || (line.starts_with('#') && !line.contains('='))
            {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .map(|(k, v)| (k.trim(), v.trim()))
                .ok_or_else(|| format!("line {}: expected `key = value`", n + 1))?;
            let bad_color = || format!("line {}: bad color {v:?}", n + 1);
            let bad_num = || format!("line {}: bad number {v:?}", n + 1);
            if k == "theme" {
                t.name = v.to_string();
            } else if let Some(name) = k.strip_prefix("col.") {
                let c = Col::ALL.iter().copied().find(|c| c.key() == name);
                let c = c.ok_or_else(|| format!("line {}: unknown key {k:?}", n + 1))?;
                t.colors[c as usize] = Color::from_hex(v).ok_or_else(bad_color)?;
            } else if let Some(name) = k.strip_prefix("num.") {
                let c = Num::ALL.iter().copied().find(|c| c.key() == name);
                let c = c.ok_or_else(|| format!("line {}: unknown key {k:?}", n + 1))?;
                t.nums[c as usize] = v.parse::<f32>().map_err(|_| bad_num())?;
            } else if let Some(name) = k.strip_prefix("socket.") {
                let ty = SocketType::from_name(name)
                    .ok_or_else(|| format!("line {}: unknown key {k:?}", n + 1))?;
                t.sockets[ty.index()] = Color::from_hex(v).ok_or_else(bad_color)?;
            } else if let Some(name) = k.strip_prefix("header.") {
                let c = Category::ALL.iter().copied().find(|c| c.name() == name);
                let c = c.ok_or_else(|| format!("line {}: unknown key {k:?}", n + 1))?;
                t.headers[c.index()] = Color::from_hex(v).ok_or_else(bad_color)?;
            } else {
                return Err(format!("line {}: unknown key {k:?}", n + 1));
            }
        }
        t.revision = 0;
        Ok(t)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::blender_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip_preserves_every_field() {
        for t in [Theme::blender_dark(), Theme::light()] {
            let back = Theme::from_text(&t.to_text()).expect("parses");
            assert_eq!(back, t);
        }
    }

    #[test]
    fn partial_override_and_errors() {
        let t = Theme::from_text("col.background = #ff0000\nnum.radius = 10\n").unwrap();
        assert_eq!(t.c(Col::Background).to_srgb8(), [255, 0, 0, 255]);
        assert_eq!(t.n(Num::Radius), 10.0);
        assert_eq!(t.c(Col::Text), Theme::blender_dark().c(Col::Text));
        assert!(Theme::from_text("col.nope = #000000").is_err());
        assert!(Theme::from_text("num.radius = abc").is_err());
        assert!(Theme::from_text("col.text = red").is_err());
        assert!(Theme::from_text("garbage").is_err());
    }

    #[test]
    fn setters_bump_revision_and_change_output() {
        let mut t = Theme::blender_dark();
        let r0 = t.revision;
        t.set_header(Category::Filter, Color::WHITE);
        assert!(t.revision > r0);
        assert_eq!(t.header(Category::Filter), Color::WHITE);
    }

    #[test]
    fn wire_width_is_pixel_stable_in_mid_zoom() {
        let t = Theme::blender_dark();
        let base = t.n(Num::WireWidth);
        for z in [0.5_f32, 1.0, 1.4] {
            let px = t.wire_width_graph(z) * z;
            assert!((px - base * z.clamp(0.4, 1.5)).abs() < 1e-5);
        }
        // zoomed far out, the on-screen width stops shrinking at 0.4x
        assert!((t.wire_width_graph(0.1) * 0.1 - base * 0.4).abs() < 1e-5);
    }

    #[test]
    fn states_have_expected_outlines() {
        let t = Theme::blender_dark();
        assert!(t.state_outline(NodeState::Normal).is_none());
        assert!(
            t.state_outline(NodeState::Selected).unwrap().1
                > t.state_outline(NodeState::Hovered).unwrap().1
        );
    }
}
