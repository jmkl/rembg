//! Every colour in CompNode is defined here and nowhere else.
//!
//! * [`base`] holds raw swatches, named after what they look like.
//!   Scales run dark -> light (950 = darkest).
//! * [`pal`] holds semantic tokens (what a colour is *for*). All other modules
//!   refer to `pal::*` only; none of them construct a `Color32` themselves.

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

pub(crate) mod catppuccin {
    use super::{Color32, hex};

    pub const WHITE: Color32 = hex(0xffffff);
    pub const BLACK: Color32 = hex(0x000000);

    // Neutrals: Macchiato crust -> text. Repeated values are intentional,
    // Catppuccin has fewer steps than this scale.
    pub const NEUTRAL_975: Color32 = hex(0x181926); // crust  (curve editor well)
    pub const NEUTRAL_960: Color32 = hex(0x181926); // crust  (input fields)
    pub const NEUTRAL_950: Color32 = hex(0x1e2030); // mantle (checkerboard dark)
    pub const NEUTRAL_900: Color32 = hex(0x1e2030); // mantle (graph canvas)
    pub const NEUTRAL_850: Color32 = hex(0x24273a); // base   (node body, popups)
    pub const NEUTRAL_800: Color32 = hex(0x363a4f); // surface0 (title bar, sidebar)
    pub const NEUTRAL_750: Color32 = hex(0x494d64); // surface1
    pub const NEUTRAL_700: Color32 = hex(0x494d64); // surface1 (borders)
    pub const NEUTRAL_650: Color32 = hex(0x5b6078); // surface2
    pub const NEUTRAL_600: Color32 = hex(0x494d64); // surface1
    pub const NEUTRAL_550: Color32 = hex(0x6e738d); // overlay0 (faint text)
    pub const NEUTRAL_500: Color32 = hex(0x939ab7); // overlay2 (muted text)
    pub const NEUTRAL_450: Color32 = hex(0x8087a2); // overlay1
    pub const NEUTRAL_300: Color32 = hex(0xb8c0e0); // subtext1
    pub const NEUTRAL_200: Color32 = hex(0xcad3f5); // text
    pub const NEUTRAL_100: Color32 = hex(0xcad3f5); // text (header text)
    pub const NEUTRAL_50: Color32 = hex(0xeff1f5); // latte base (light canvas)

    // Sidebar surfaces
    pub const SLATE_900: Color32 = hex(0x1e2030); // mantle
    pub const SLATE_800: Color32 = hex(0x24273a); // base
    pub const SLATE_700: Color32 = hex(0x494d64); // surface1
    pub const SLATE_600: Color32 = hex(0x6e738d); // overlay0
    pub const SLATE_400: Color32 = hex(0x939ab7); // overlay2
    pub const SLATE_BLUE: Color32 = hex(0x475170); // selected layer row

    // Accent / status colours
    pub const BLUE_500: Color32 = hex(0x8aadf4); // blue (accent)
    pub const BLUE_400: Color32 = hex(0xb7bdf8); // lavender (accent hover)
    pub const SKY_300: Color32 = hex(0x91d7e3); // sky (curve line)
    pub const AZURE_500: Color32 = hex(0x7dc4e4); // sapphire (gizmo)
    pub const GOLD_500: Color32 = hex(0xeed49f); // yellow (sockets)
    pub const GOLD_950: Color32 = hex(0x181926); // crust (socket outline)
    pub const RED_400: Color32 = hex(0xed8796); // red (danger text)
    pub const RED_600: Color32 = hex(0xed8796); // red (button)
    pub const RED_700: Color32 = hex(0xee99a0); // maroon (button hover)
    pub const GREEN_400: Color32 = hex(0xa6da95); // green (success)
    pub const GREEN_600: Color32 = hex(0xa6da95); // green (button)
    pub const GREEN_700: Color32 = hex(0x8bd5ca); // teal (button hover)

    // Node headers: each accent mixed ~22-35% into base (#24273a)
    pub const MOSS: Color32 = hex(0x52665a);
    pub const STEEL_BLUE: Color32 = hex(0x48567b);
    pub const BRONZE: Color32 = hex(0x6d5552);
    pub const MAROON: Color32 = hex(0x6a495a);
    pub const SLATE_PURPLE: Color32 = hex(0x575c7c);
    pub const PLUM: Color32 = hex(0x6d5c76);
    pub const OLIVE: Color32 = hex(0x6b645d);
    pub const MULBERRY: Color32 = hex(0x5d517c);
    pub const DENIM: Color32 = hex(0x435e76);
    pub const SAND: Color32 = hex(0x6d6671);
    pub const TEAL: Color32 = hex(0x48646c);
    pub const INDIGO: Color32 = hex(0x38425f);
    pub const CLAY: Color32 = hex(0x524449);
    pub const SAGE: Color32 = hex(0x414e4e);
}

pub(crate) mod photoshop {
    use super::{Color32, hex};

    pub const WHITE: Color32 = hex(0xffffff);
    pub const BLACK: Color32 = hex(0x000000);

    pub const NEUTRAL_975: Color32 = hex(0x0f0f0f); // curve editor well
    pub const NEUTRAL_960: Color32 = hex(0x141414); // input fields
    pub const NEUTRAL_950: Color32 = hex(0x171717); // checkerboard dark
    pub const NEUTRAL_900: Color32 = hex(0x1a1a1a); // graph canvas
    pub const NEUTRAL_850: Color32 = hex(0x1f1f1f); // node body, popups
    pub const NEUTRAL_800: Color32 = hex(0x232323); // title bar, sidebar, dialogs
    pub const NEUTRAL_750: Color32 = hex(0x2a2a2a);
    pub const NEUTRAL_700: Color32 = hex(0x333333); // borders
    pub const NEUTRAL_650: Color32 = hex(0x3f3f3f);
    pub const NEUTRAL_600: Color32 = hex(0x4a4a4a);
    pub const NEUTRAL_550: Color32 = hex(0x6e6e6e); // faint text
    pub const NEUTRAL_500: Color32 = hex(0x909090); // muted text
    pub const NEUTRAL_450: Color32 = hex(0xa0a0a0);
    pub const NEUTRAL_300: Color32 = hex(0xbdbdbd); // body text
    pub const NEUTRAL_200: Color32 = hex(0xd4d4d4);
    pub const NEUTRAL_100: Color32 = hex(0xececec); // headings, header text
    pub const NEUTRAL_50: Color32 = hex(0xf5f5f5);

    // Slate (near-neutral with a faint cool tint, used by the sidebar)
    pub const SLATE_900: Color32 = hex(0x161718);
    pub const SLATE_800: Color32 = hex(0x1d1e20);
    pub const SLATE_700: Color32 = hex(0x333437);
    pub const SLATE_600: Color32 = hex(0x5a5b5e);
    pub const SLATE_400: Color32 = hex(0x909296);
    pub const SLATE_BLUE: Color32 = hex(0x232f44); // selected layer row

    // Accent / status colours
    pub const BLUE_500: Color32 = hex(0x2680eb); // Photoshop selection blue
    pub const BLUE_400: Color32 = hex(0x3b8ff0);
    pub const SKY_300: Color32 = hex(0x6cb2ff);
    pub const AZURE_500: Color32 = hex(0x0b9cff); // transform gizmo
    pub const GOLD_500: Color32 = hex(0xe8a33d); // sockets
    pub const GOLD_950: Color32 = hex(0x2a1d06);
    pub const RED_400: Color32 = hex(0xea5a60);
    pub const RED_600: Color32 = hex(0xd7373f);
    pub const RED_700: Color32 = hex(0xb72a31);
    pub const GREEN_400: Color32 = hex(0x33ab84);
    pub const GREEN_600: Color32 = hex(0x268e6c);
    pub const GREEN_700: Color32 = hex(0x1f7a5c);

    // Muted dark hues for node headers
    pub const MOSS: Color32 = hex(0x2f4033);
    pub const STEEL_BLUE: Color32 = hex(0x2b3a50);
    pub const BRONZE: Color32 = hex(0x45372a);
    pub const MAROON: Color32 = hex(0x452b2b);
    pub const SLATE_PURPLE: Color32 = hex(0x373745);
    pub const PLUM: Color32 = hex(0x433043);
    pub const OLIVE: Color32 = hex(0x424230);
    pub const MULBERRY: Color32 = hex(0x432f3a);
    pub const DENIM: Color32 = hex(0x2f3a48);
    pub const SAND: Color32 = hex(0x433b2f);
    pub const TEAL: Color32 = hex(0x2f4343);
    pub const INDIGO: Color32 = hex(0x383048);
    pub const CLAY: Color32 = hex(0x43372f);
    pub const SAGE: Color32 = hex(0x384238);
}
pub(crate) mod base {
    use super::{Color32, hex};

    pub const WHITE: Color32 = hex(0xffffff);
    pub const BLACK: Color32 = hex(0x000000);

    // Neutral grays
    pub const NEUTRAL_975: Color32 = hex(0x181818);
    pub const NEUTRAL_960: Color32 = hex(0x1a1a1a);
    pub const NEUTRAL_950: Color32 = hex(0x1b1b1b);
    pub const NEUTRAL_900: Color32 = hex(0x1e1e1e);
    pub const NEUTRAL_850: Color32 = hex(0x242424);
    pub const NEUTRAL_800: Color32 = hex(0x282828);
    pub const NEUTRAL_750: Color32 = hex(0x3c3c3c);
    pub const NEUTRAL_700: Color32 = hex(0x3b3b3b);
    pub const NEUTRAL_650: Color32 = hex(0x4a4a4a);
    pub const NEUTRAL_600: Color32 = hex(0x505050);
    pub const NEUTRAL_550: Color32 = hex(0x6c6c6c);
    pub const NEUTRAL_500: Color32 = hex(0x8b8b8b);
    pub const NEUTRAL_450: Color32 = hex(0x787878);
    pub const NEUTRAL_300: Color32 = hex(0xd8d8d8);
    pub const NEUTRAL_200: Color32 = hex(0xe0e0e0);
    pub const NEUTRAL_100: Color32 = hex(0xf2f2f2);
    pub const NEUTRAL_50: Color32 = hex(0xf5f5f5);

    // Slate (grays with a slight blue tint, used by the sidebar)
    pub const SLATE_900: Color32 = hex(0x1c1f23);
    pub const SLATE_800: Color32 = hex(0x22262b);
    pub const SLATE_700: Color32 = hex(0x3a3d41);
    pub const SLATE_600: Color32 = hex(0x5a5d61);
    pub const SLATE_400: Color32 = hex(0x8a8d93);
    pub const SLATE_BLUE: Color32 = hex(0x2a354a);

    // Accent / status colours
    pub const BLUE_500: Color32 = hex(0x4776d1);
    pub const BLUE_400: Color32 = hex(0x5a89e0);
    pub const SKY_300: Color32 = hex(0x7ab8ff);
    pub const AZURE_500: Color32 = hex(0x008cff);
    pub const GOLD_500: Color32 = hex(0xd1a84b);
    pub const GOLD_950: Color32 = hex(0x241e0b);
    pub const RED_400: Color32 = hex(0xe06c60);
    pub const RED_600: Color32 = hex(0xdc2626);
    pub const RED_700: Color32 = hex(0xb91c1c);
    pub const GREEN_400: Color32 = hex(0x6ab87a);
    pub const GREEN_600: Color32 = hex(0x16a34a);
    pub const GREEN_700: Color32 = hex(0x15803d);

    // Muted dark hues for node headers
    pub const MOSS: Color32 = hex(0x283b2e);
    pub const STEEL_BLUE: Color32 = hex(0x253040);
    pub const BRONZE: Color32 = hex(0x3c2f21);
    pub const MAROON: Color32 = hex(0x3a2424);
    pub const SLATE_PURPLE: Color32 = hex(0x30303b);
    pub const PLUM: Color32 = hex(0x3b283b);
    pub const OLIVE: Color32 = hex(0x3b3b28);
    pub const MULBERRY: Color32 = hex(0x3b2830);
    pub const DENIM: Color32 = hex(0x28303b);
    pub const SAND: Color32 = hex(0x3b3428);
    pub const TEAL: Color32 = hex(0x283b3b);
    pub const INDIGO: Color32 = hex(0x30283b);
    pub const CLAY: Color32 = hex(0x3b3028);
    pub const SAGE: Color32 = hex(0x303b30);
}

/// Semantic tokens (Blender-inspired muted dark palette).
pub(crate) mod pal {
    use super::{Color32, mix, photoshop as base};

    // --- Surfaces ---
    pub const BG: Color32 = base::NEUTRAL_900; // graph canvas
    pub const PANEL: Color32 = base::NEUTRAL_800; // title bar, side panels, dialogs
    pub const NODE: Color32 = base::NEUTRAL_850; // node body, popups, cards
    pub const NODE_STROKE: Color32 = base::NEUTRAL_700; // borders and separators
    pub const INPUT_BG: Color32 = base::NEUTRAL_960; // text fields, controls at rest
    pub const FAINT_BG: Color32 = mix(NODE, base::WHITE, 2); // striped rows
    pub const HOVER_BG: Color32 = mix(PANEL, base::WHITE, 8); // menu rows, ghost buttons
    pub const HOVER_FILL: Color32 = mix(INPUT_BG, base::WHITE, 4); // controls on hover
    pub const ACTIVE_FILL: Color32 = mix(INPUT_BG, ACCENT, 15); // controls pressed

    // --- Text ---
    pub const HEADER_TEXT: Color32 = base::NEUTRAL_100;
    pub const TEXT: Color32 = base::NEUTRAL_300;
    pub const MUTED: Color32 = base::NEUTRAL_500;
    pub const FAINT: Color32 = base::NEUTRAL_550;
    pub const ON_ACCENT: Color32 = base::WHITE; // text on filled buttons

    // --- Accent & status ---
    pub const ACCENT: Color32 = base::BLUE_500;
    pub const ACCENT_HOVER: Color32 = base::BLUE_400;
    pub const SUCCESS: Color32 = base::GREEN_400;
    pub const DANGER: Color32 = base::RED_400;
    pub const DANGER_SOFT: Color32 = mix(NODE, DANGER, 25); // hover tint behind danger icons
    pub const CALLOUT_DANGER_BG: Color32 = mix(NODE, DANGER, 14);
    pub const CALLOUT_DANGER_BORDER: Color32 = mix(NODE, DANGER, 55);

    // --- Filled buttons ---
    pub const BTN_PRIMARY: Color32 = ACCENT;
    pub const BTN_PRIMARY_HOVER: Color32 = ACCENT_HOVER;
    pub const BTN_SUCCESS: Color32 = base::GREEN_600;
    pub const BTN_SUCCESS_HOVER: Color32 = base::GREEN_700;
    pub const BTN_DANGER: Color32 = base::RED_600;
    pub const BTN_DANGER_HOVER: Color32 = base::RED_700;

    // --- Node graph ---
    pub const SOCKET: Color32 = base::GOLD_500;
    pub const SOCKET_STROKE: Color32 = base::GOLD_950;
    pub const CHECK_A: Color32 = base::NEUTRAL_950; // preview checkerboard
    pub const CHECK_B: Color32 = base::NEUTRAL_850;
    pub const NO_TINT: Color32 = base::WHITE; // multiplier for textures

    // --- Curve editor ---
    pub const CURVE_BG: Color32 = base::NEUTRAL_975;
    pub const CURVE_GRID: Color32 = base::NEUTRAL_800;
    pub const CURVE_DIAGONAL: Color32 = base::NEUTRAL_750;
    pub const CURVE_LINE: Color32 = base::SKY_300;
    pub const CURVE_HANDLE: Color32 = base::WHITE;
    pub const CURVE_HANDLE_STROKE: Color32 = base::BLACK;

    // --- Sidebar / image canvas ---
    pub const SIDEBAR_BORDER: Color32 = base::SLATE_700;
    pub const SIDEBAR_LAYER_BG: Color32 = base::SLATE_900;
    pub const SIDEBAR_LAYER_BG_HOVER: Color32 = base::SLATE_800;
    pub const SIDEBAR_LAYER_SELECTED: Color32 = base::SLATE_BLUE;
    pub const SIDEBAR_TEXT: Color32 = base::NEUTRAL_200;
    pub const SIDEBAR_TEXT_MUTED: Color32 = base::SLATE_400;
    pub const SIDEBAR_DRAG_HANDLE: Color32 = base::SLATE_600;
    pub const SIDEBAR_ACCENT: Color32 = base::BLUE_500;
    pub const CANVAS_BG: Color32 = base::NEUTRAL_50;
    pub const CANVAS_BORDER: Color32 = base::NEUTRAL_450;
    pub const GIZMO: Color32 = base::AZURE_500; // transform handles
    pub const GIZMO_HANDLE: Color32 = base::WHITE;

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
