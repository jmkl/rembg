//! Every colour in CompNode is defined here and nowhere else.
//!
//! * [`base`] holds the raw swatches of the active theme: a conventional UI
//!   palette — a neutral ramp (50 = lightest .. 950 = darkest), four accent
//!   hues and eight muted node-header hues.
//! * [`catppuccin`] and [`photoshop`] are alternate themes that declare
//!   *exactly the same swatch names*, so switching theme is one `use` line
//!   in [`pal`].
//! * [`pal`] holds the semantic tokens (what a colour is *for*) plus the
//!   [`pal::node_header`] hues. All other modules refer to `pal::*` only;
//!   none of them construct a `Color32` themselves. Hover variants are
//!   derived with [`pal::hover`] instead of being stored as swatches.

use egui::Color32;

/// `0xRRGGBB` -> Color32, so the palette reads like a normal hex colour.
pub(crate) const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Opaque blend of `a` towards `b`; `pct` is how much of `b` (0..=100).
/// `const`, so derived tokens can live next to the raw ones.
pub(crate) const fn mix(a: Color32, b: Color32, pct: u32) -> Color32 {
    const fn ch(x: u8, y: u8, pct: u32) -> u8 {
        ((x as u32 * (100 - pct) + y as u32 * pct + 50) / 100) as u8
    }
    Color32::from_rgb(
        ch(a.r(), b.r(), pct),
        ch(a.g(), b.g(), pct),
        ch(a.b(), b.b(), pct),
    )
}

// ------------------------------------------------------------------ themes ----
// Every theme declares the same swatch names, so `pal` never cares which one
// is active.

/// The active theme: a conventional dark-UI palette (neutral ramp + primaries).
pub(crate) mod base {
    use super::{Color32, hex};

    pub const WHITE: Color32 = hex(0xffffff);
    pub const BLACK: Color32 = hex(0x000000);

    // Neutral ramp, light -> dark.
    pub const NEUTRAL_50: Color32 = hex(0xfafafa); // light image canvas
    pub const NEUTRAL_100: Color32 = hex(0xf5f5f5); // header text
    pub const NEUTRAL_300: Color32 = hex(0xd4d4d4); // body text
    pub const NEUTRAL_400: Color32 = hex(0xa3a3a3); // muted text
    pub const NEUTRAL_500: Color32 = hex(0x737373); // faint text, handles
    pub const NEUTRAL_700: Color32 = hex(0x404040); // borders, neutral header
    pub const NEUTRAL_800: Color32 = hex(0x262626); // panels, node bodies
    pub const NEUTRAL_900: Color32 = hex(0x171717); // graph canvas
    pub const NEUTRAL_950: Color32 = hex(0x0a0a0a); // wells, input fields

    // Accent / status primaries.
    pub const BLUE: Color32 = hex(0x3b82f6); // accent, selection, gizmo
    pub const GREEN: Color32 = hex(0x16a34a); // success
    pub const RED: Color32 = hex(0xef4444); // danger
    pub const AMBER: Color32 = hex(0xf59e0b); // sockets, warnings

    // Muted node-header hues; node kinds share these (see `pal::node_header`).
    pub const MOSS: Color32 = hex(0x2e4a38);
    pub const STEEL_BLUE: Color32 = hex(0x2c3c56);
    pub const BRONZE: Color32 = hex(0x4a3a28);
    pub const MAROON: Color32 = hex(0x4a2f33);
    pub const PLUM: Color32 = hex(0x453049);
    pub const OLIVE: Color32 = hex(0x45452e);
    pub const TEAL: Color32 = hex(0x2c4747);
    pub const SAND: Color32 = hex(0x4a4234);
}

/// Alternate theme, kept for a future theme switcher: swap the `use` in `pal`.
#[allow(dead_code)]
pub(crate) mod catppuccin {
    use super::{Color32, hex};

    pub const WHITE: Color32 = hex(0xffffff);
    pub const BLACK: Color32 = hex(0x000000);

    // Catppuccin Macchiato, packed onto the same ramp.
    pub const NEUTRAL_50: Color32 = hex(0xeff1f5);
    pub const NEUTRAL_100: Color32 = hex(0xcad3f5);
    pub const NEUTRAL_300: Color32 = hex(0xb8c0e0);
    pub const NEUTRAL_400: Color32 = hex(0x939ab7);
    pub const NEUTRAL_500: Color32 = hex(0x6e738d);
    pub const NEUTRAL_700: Color32 = hex(0x494d64);
    pub const NEUTRAL_800: Color32 = hex(0x24273a);
    pub const NEUTRAL_900: Color32 = hex(0x1e2030);
    pub const NEUTRAL_950: Color32 = hex(0x181926);

    pub const BLUE: Color32 = hex(0x8aadf4);
    pub const GREEN: Color32 = hex(0xa6da95);
    pub const RED: Color32 = hex(0xed8796);
    pub const AMBER: Color32 = hex(0xeed49f);

    pub const MOSS: Color32 = hex(0x52665a);
    pub const STEEL_BLUE: Color32 = hex(0x48567b);
    pub const BRONZE: Color32 = hex(0x6d5552);
    pub const MAROON: Color32 = hex(0x6a495a);
    pub const PLUM: Color32 = hex(0x6d5c76);
    pub const OLIVE: Color32 = hex(0x6b645d);
    pub const TEAL: Color32 = hex(0x48646c);
    pub const SAND: Color32 = hex(0x6d6671);
}

/// Alternate theme, kept for a future theme switcher: swap the `use` in `pal`.
#[allow(dead_code)]
pub(crate) mod photoshop {
    use super::{Color32, hex};

    pub const WHITE: Color32 = hex(0xffffff);
    pub const BLACK: Color32 = hex(0x000000);

    pub const NEUTRAL_50: Color32 = hex(0xf5f5f5);
    pub const NEUTRAL_100: Color32 = hex(0xececec);
    pub const NEUTRAL_300: Color32 = hex(0xbdbdbd);
    pub const NEUTRAL_400: Color32 = hex(0xa0a0a0);
    pub const NEUTRAL_500: Color32 = hex(0x6e6e6e);
    pub const NEUTRAL_700: Color32 = hex(0x333333);
    pub const NEUTRAL_800: Color32 = hex(0x232323);
    pub const NEUTRAL_900: Color32 = hex(0x1a1a1a);
    pub const NEUTRAL_950: Color32 = hex(0x0f0f0f);

    pub const BLUE: Color32 = hex(0x2680eb);
    pub const GREEN: Color32 = hex(0x33ab84);
    pub const RED: Color32 = hex(0xea5a60);
    pub const AMBER: Color32 = hex(0xe8a33d);

    pub const MOSS: Color32 = hex(0x2f4033);
    pub const STEEL_BLUE: Color32 = hex(0x2b3a50);
    pub const BRONZE: Color32 = hex(0x45372a);
    pub const MAROON: Color32 = hex(0x452b2b);
    pub const PLUM: Color32 = hex(0x433043);
    pub const OLIVE: Color32 = hex(0x424230);
    pub const TEAL: Color32 = hex(0x2f4343);
    pub const SAND: Color32 = hex(0x433b2f);
}

/// Semantic tokens (Blender-inspired muted dark palette).
///
/// 21 tokens; everything else in the UI derives from these via [`hover`] or
/// [`mix`]. Point `base` at another theme module to re-skin the whole app.
pub(crate) mod pal {
    use super::{Color32, base, mix};

    // --- Surfaces ---
    pub const BG: Color32 = base::NEUTRAL_900; // graph canvas, layer rows
    pub const PANEL: Color32 = base::NEUTRAL_800; // title bar, side panels, dialogs
    pub const NODE: Color32 = base::NEUTRAL_800; // node body, popups, cards
    pub const NODE_STROKE: Color32 = base::NEUTRAL_700; // borders and separators
    pub const INPUT_BG: Color32 = base::NEUTRAL_950; // text fields, wells
    pub const FAINT_BG: Color32 = mix(NODE, base::WHITE, 4); // striped rows
    pub const ACTIVE_FILL: Color32 = mix(NODE, ACCENT, 18); // pressed / selected

    // --- Text ---
    pub const HEADER_TEXT: Color32 = base::NEUTRAL_100;
    pub const TEXT: Color32 = base::NEUTRAL_300;
    pub const MUTED: Color32 = base::NEUTRAL_400;
    pub const FAINT: Color32 = base::NEUTRAL_500;
    pub const WHITE: Color32 = base::WHITE; // glyphs, untinted textures
    pub const BLACK: Color32 = base::BLACK; // strokes on light fills

    // --- Accent & status ---
    pub const ACCENT: Color32 = base::BLUE;
    pub const SUCCESS: Color32 = base::GREEN;
    pub const DANGER: Color32 = base::RED;
    pub const DANGER_TINT: Color32 = mix(NODE, DANGER, 18); // error callout fill, danger-icon hover
    pub const CALLOUT_DANGER_BORDER: Color32 = mix(NODE, DANGER, 55);

    // --- Node graph & canvas ---
    pub const SOCKET: Color32 = base::AMBER;
    pub const SOCKET_STROKE: Color32 = mix(SOCKET, BLACK, 75);
    pub const CANVAS_BG: Color32 = base::NEUTRAL_50; // light image canvas

    /// Hover fill for a surface (or a filled button): blend it 8% to white.
    pub(crate) const fn hover(surface: Color32) -> Color32 {
        mix(surface, base::WHITE, 8)
    }

    /// Node header colours. Register these in `catalog::NODE_SPECS`.
    pub(crate) mod node_header {
        use super::{Color32, base};

        /// Neutral header for utility nodes (Route).
        pub const NEUTRAL: Color32 = base::NEUTRAL_700;
        pub const MOSS: Color32 = base::MOSS;
        pub const STEEL_BLUE: Color32 = base::STEEL_BLUE;
        pub const BRONZE: Color32 = base::BRONZE;
        pub const MAROON: Color32 = base::MAROON;
        pub const PLUM: Color32 = base::PLUM;
        pub const OLIVE: Color32 = base::OLIVE;
        pub const TEAL: Color32 = base::TEAL;
        pub const SAND: Color32 = base::SAND;
    }
}
