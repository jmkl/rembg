//! The node registry.

use std::sync::{OnceLock, RwLock};

use egui::{Color32, Pos2, Ui};
use egui_snarl::{NodeId, Snarl};
use image::RgbaImage;

use crate::editor::GraphEditor;
use crate::model::{FilterKind, Kind, Node};
use crate::nodes;
use crate::ops;
use crate::params::{self, Params};
use crate::style::{NODE_W, NODE_W_S, icons, pal};

pub(crate) type BodyFn = fn(&mut GraphEditor, NodeId, &mut Ui, &mut Snarl<Node>, f32);
pub(crate) type CreateFn = fn(&mut Snarl<Node>, Pos2, Kind);
pub(crate) type EvalFn =
    fn(inputs: &[Vec<RgbaImage>], params: &Params, canvas: (u32, u32)) -> Result<RgbaImage, String>;

#[derive(Clone, Copy)]
pub(crate) struct SelectOption {
    pub label: &'static str,
    pub value: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) enum FieldKind {
    Range {
        min: f32,
        max: f32,
        default: f32,
        step: f64,
        suffix: &'static str,
    },
    Number {
        min: f32,
        max: f32,
        default: f32,
        step: f64,
    },
    Color {
        default: &'static str,
    },
    Select {
        options: &'static [SelectOption],
        default: &'static str,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
}

impl FieldSpec {
    pub const fn range(
        key: &'static str,
        label: &'static str,
        min: f32,
        max: f32,
        default: f32,
        step: f64,
        suffix: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            kind: FieldKind::Range {
                min,
                max,
                default,
                step,
                suffix,
            },
        }
    }

    pub const fn number(
        key: &'static str,
        label: &'static str,
        min: f32,
        max: f32,
        default: f32,
        step: f64,
    ) -> Self {
        Self {
            key,
            label,
            kind: FieldKind::Number {
                min,
                max,
                default,
                step,
            },
        }
    }

    pub const fn color(key: &'static str, label: &'static str, default: &'static str) -> Self {
        Self {
            key,
            label,
            kind: FieldKind::Color { default },
        }
    }

    pub const fn select(
        key: &'static str,
        label: &'static str,
        options: &'static [SelectOption],
        default: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            kind: FieldKind::Select { options, default },
        }
    }

    pub fn apply_default(&self, params: &mut Params) {
        match &self.kind {
            FieldKind::Range { default, .. } | FieldKind::Number { default, .. } => {
                params::set_f32(params, self.key, *default);
            }
            FieldKind::Color { default } | FieldKind::Select { default, .. } => {
                params::set_string(params, self.key, *default);
            }
        }
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct SocketSpec {
    pub label: &'static str,
    pub multi: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct NodeSpec {
    pub kind: Kind,
    pub title: &'static str,
    pub icon: char,
    pub keywords: &'static [&'static str],
    pub accent: Color32,
    pub inputs: usize,
    pub outputs: usize,
    pub body_width: f32,
    pub multi_input: bool,
    pub hidden: bool,
    pub fields: &'static [FieldSpec],
    pub sockets: &'static [SocketSpec],
    pub create: CreateFn,
    pub body: BodyFn,
    pub eval: Option<EvalFn>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NodeCategory {
    Sources,
    Ai,
    Color,
    Generators,
    Compositing,
    Utilities,
}

impl NodeCategory {
    pub(crate) const ALL: [Self; 6] = [
        Self::Sources,
        Self::Ai,
        Self::Color,
        Self::Generators,
        Self::Compositing,
        Self::Utilities,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sources => "Input / Sources",
            Self::Ai => "AI Tools",
            Self::Color => "Color & Grading",
            Self::Generators => "Generators",
            Self::Compositing => "Compositing & Layers",
            Self::Utilities => "Utilities",
        }
    }
}

#[derive(Default)]
pub(crate) struct NodeCatalog {
    specs: Vec<&'static NodeSpec>,
}

impl NodeCatalog {
    fn builtins() -> Self {
        Self {
            specs: NODE_SPECS.iter().collect(),
        }
    }

    pub(crate) fn register(&mut self, spec: NodeSpec) -> Result<(), String> {
        if let Some(existing) = self
            .specs
            .iter_mut()
            .find(|existing| existing.kind == spec.kind)
        {
            *existing = Box::leak(Box::new(spec));
        } else {
            self.specs.push(Box::leak(Box::new(spec)));
        }
        Ok(())
    }

    pub(crate) fn specs(&self) -> &[&'static NodeSpec] {
        &self.specs
    }
}

static CATALOG: OnceLock<RwLock<NodeCatalog>> = OnceLock::new();

fn catalog() -> &'static RwLock<NodeCatalog> {
    CATALOG.get_or_init(|| RwLock::new(NodeCatalog::builtins()))
}

#[allow(dead_code)]
pub(crate) fn register(spec: NodeSpec) -> Result<(), String> {
    catalog()
        .write()
        .map_err(|_| "node catalog lock poisoned".to_owned())?
        .register(spec)
}

const SINGLE_SOCKET: &[SocketSpec] = &[SocketSpec {
    label: "",
    multi: false,
}];

impl NodeSpec {
    pub(crate) fn match_score(&self, filter: &str) -> Option<usize> {
        let filter = filter.trim();
        if filter.is_empty() {
            return Some(0);
        }
        let filter = filter.to_ascii_lowercase();
        std::iter::once(self.title)
            .chain(self.keywords.iter().copied())
            .filter_map(|candidate| fuzzy_score(candidate, &filter))
            .min()
    }

    pub(crate) fn category(&self) -> NodeCategory {
        match self.kind {
            Kind::Image => NodeCategory::Sources,
            Kind::RemoveBg | Kind::RemoveBgV2 | Kind::FaceRestore => NodeCategory::Ai,
            Kind::Filter(
                FilterKind::Hsv
                | FilterKind::Curves
                | FilterKind::Exposure
                | FilterKind::ColorBalance
                | FilterKind::GradientMap
                | FilterKind::Invert
                | FilterKind::RawFilter,
            ) => NodeCategory::Color,
            Kind::Filter(FilterKind::Gradient | FilterKind::Noise) | Kind::Texts => {
                NodeCategory::Generators
            }
            Kind::Group | Kind::Blend | Kind::Filter(FilterKind::Mask) => NodeCategory::Compositing,
            Kind::Output | Kind::Route => NodeCategory::Utilities,
        }
    }

    pub const fn filter(
        kind: FilterKind,
        title: &'static str,
        icon: char,
        accent: Color32,
        keywords: &'static [&'static str],
        fields: &'static [FieldSpec],
        eval: EvalFn,
    ) -> Self {
        Self {
            kind: Kind::Filter(kind),
            title,
            icon,
            keywords,
            accent,
            inputs: 1,
            outputs: 1,
            body_width: NODE_W,
            multi_input: false,
            hidden: false,
            fields,
            sockets: SINGLE_SOCKET,
            create: nodes::filter::create,
            body: nodes::filter::show_body,
            eval: Some(eval),
        }
    }

    pub const fn generator(
        kind: FilterKind,
        title: &'static str,
        icon: char,
        accent: Color32,
        keywords: &'static [&'static str],
        fields: &'static [FieldSpec],
        eval: EvalFn,
    ) -> Self {
        Self {
            kind: Kind::Filter(kind),
            title,
            icon,
            keywords,
            accent,
            inputs: 0,
            outputs: 1,
            body_width: NODE_W,
            multi_input: false,
            hidden: false,
            fields,
            sockets: NO_SOCKETS,
            create: nodes::filter::create,
            body: nodes::filter::show_body,
            eval: Some(eval),
        }
    }

    pub fn apply_defaults(&self, params: &mut Params) {
        for field in self.fields {
            field.apply_default(params);
        }
    }
}

const NO_FIELDS: &[FieldSpec] = &[];
const NO_SOCKETS: &[SocketSpec] = &[];

const BLEND_MODES: &[SelectOption] = &[
    SelectOption {
        label: "Source Over",
        value: "source-over",
    },
    SelectOption {
        label: "Normal",
        value: "normal",
    },
    SelectOption {
        label: "Multiply",
        value: "multiply",
    },
    SelectOption {
        label: "Screen",
        value: "screen",
    },
    SelectOption {
        label: "Overlay",
        value: "overlay",
    },
    SelectOption {
        label: "Darken",
        value: "darken",
    },
    SelectOption {
        label: "Lighten",
        value: "lighten",
    },
    SelectOption {
        label: "Color Dodge",
        value: "color-dodge",
    },
    SelectOption {
        label: "Color Burn",
        value: "color-burn",
    },
    SelectOption {
        label: "Hard Light",
        value: "hard-light",
    },
    SelectOption {
        label: "Soft Light",
        value: "soft-light",
    },
    SelectOption {
        label: "Difference",
        value: "difference",
    },
    SelectOption {
        label: "Exclusion",
        value: "exclusion",
    },
    SelectOption {
        label: "Hue",
        value: "hue",
    },
    SelectOption {
        label: "Saturation",
        value: "saturation",
    },
    SelectOption {
        label: "Color",
        value: "color",
    },
    SelectOption {
        label: "Luminosity",
        value: "luminosity",
    },
];

const HSV_FIELDS: &[FieldSpec] = &[
    FieldSpec::range("hue", "Hue", 0.0, 360.0, 0.0, 1.0, "°"),
    FieldSpec::range("sat", "Saturation", 0.0, 200.0, 100.0, 1.0, " %"),
    FieldSpec::range("val", "Value", 0.0, 200.0, 100.0, 1.0, " %"),
];

const CURVES_CHANNELS: &[SelectOption] = &[
    SelectOption {
        label: "RGB",
        value: "rgb",
    },
    SelectOption {
        label: "Red",
        value: "red",
    },
    SelectOption {
        label: "Green",
        value: "green",
    },
    SelectOption {
        label: "Blue",
        value: "blue",
    },
];

const CURVES_FIELDS: &[FieldSpec] = &[FieldSpec::select(
    "channel",
    "Channel",
    CURVES_CHANNELS,
    "rgb",
)];

const EXPOSURE_FIELDS: &[FieldSpec] = &[
    FieldSpec::range("ev", "Exposure (EV)", -5.0, 5.0, 0.0, 0.1, ""),
    FieldSpec::range("offset", "Offset", -50.0, 50.0, 0.0, 1.0, ""),
    FieldSpec::range("gamma", "Gamma", 0.1, 3.0, 1.0, 0.01, ""),
];

const COLOR_BALANCE_RANGES: &[SelectOption] = &[
    SelectOption {
        label: "Shadows",
        value: "shadows",
    },
    SelectOption {
        label: "Midtones",
        value: "midtones",
    },
    SelectOption {
        label: "Highlights",
        value: "highlights",
    },
];

const COLOR_BALANCE_PRESERVE: &[SelectOption] = &[
    SelectOption {
        label: "Yes",
        value: "yes",
    },
    SelectOption {
        label: "No",
        value: "no",
    },
];

const COLOR_BALANCE_FIELDS: &[FieldSpec] = &[
    FieldSpec::select("range", "Range", COLOR_BALANCE_RANGES, "midtones"),
    FieldSpec::range("cyanRed", "Cyan / Red", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::range(
        "magentaGreen",
        "Magenta / Green",
        -100.0,
        100.0,
        0.0,
        1.0,
        "",
    ),
    FieldSpec::range("yellowBlue", "Yellow / Blue", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::select("preserve", "Preserve Luma", COLOR_BALANCE_PRESERVE, "yes"),
];

const GRADIENT_MAP_REVERSE: &[SelectOption] = &[
    SelectOption {
        label: "No",
        value: "no",
    },
    SelectOption {
        label: "Yes",
        value: "yes",
    },
];

const GRADIENT_MAP_FIELDS: &[FieldSpec] = &[
    FieldSpec::color("color1", "Shadow", "#000000"),
    FieldSpec::color("color2", "Midtone", "#808080"),
    FieldSpec::color("color3", "Highlight", "#ffffff"),
    FieldSpec::range("midpoint", "Midpoint", 0.0, 100.0, 50.0, 1.0, " %"),
    FieldSpec::select("reverse", "Reverse", GRADIENT_MAP_REVERSE, "no"),
];

const INVERT_FIELDS: &[FieldSpec] = &[FieldSpec::range(
    "amount", "Amount", 0.0, 100.0, 100.0, 1.0, " %",
)];

const RAW_FILTER_FIELDS: &[FieldSpec] = &[
    FieldSpec::range("temp", "Temperature", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::range("tint", "Tint", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::range("texture", "Texture", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::range("clarity", "Clarity", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::range("dehaze", "Dehaze", -100.0, 100.0, 0.0, 1.0, ""),
    FieldSpec::range("sharpen", "Sharpen", 0.0, 150.0, 0.0, 1.0, ""),
    FieldSpec::range("sharpen_radius", "Sharpen Radius", 0.5, 3.0, 1.0, 0.1, ""),
    FieldSpec::range(
        "sharpen_detail",
        "Sharpen Detail",
        0.0,
        100.0,
        25.0,
        1.0,
        " %",
    ),
    FieldSpec::range(
        "noise_reduction",
        "Noise Reduction",
        0.0,
        100.0,
        0.0,
        1.0,
        " %",
    ),
    FieldSpec::range(
        "noise_reduction_detail",
        "Noise Reduction Detail",
        0.0,
        100.0,
        50.0,
        1.0,
        " %",
    ),
];

const BLEND_FIELDS: &[FieldSpec] = &[
    FieldSpec::select("mode", "Mode", BLEND_MODES, "source-over"),
    FieldSpec::range("opacity", "Opacity", 0.0, 100.0, 100.0, 1.0, " %"),
];

const MASK_SOURCES: &[SelectOption] = &[
    SelectOption {
        label: "Dark is opaque",
        value: "dark",
    },
    SelectOption {
        label: "Light is opaque",
        value: "light",
    },
];

const MASK_FIELDS: &[FieldSpec] = &[
    FieldSpec::select("invert", "Mask Source", MASK_SOURCES, "dark"),
    FieldSpec::range("strength", "Strength", 0.0, 100.0, 100.0, 1.0, " %"),
];

const GRADIENT_MODES: &[SelectOption] = &[
    SelectOption {
        label: "Linear",
        value: "linear",
    },
    SelectOption {
        label: "Radial",
        value: "radial",
    },
    SelectOption {
        label: "Solid",
        value: "solid",
    },
];

const GRADIENT_FIELDS: &[FieldSpec] = &[
    FieldSpec::select("mode", "Mode", GRADIENT_MODES, "linear"),
    FieldSpec::color("color1", "Color 1", "#000000"),
    FieldSpec::color("color2", "Color 2", "#ffffff"),
    FieldSpec::range("angle", "Angle", 0.0, 360.0, 90.0, 1.0, "°"),
    FieldSpec::range(
        "color1Position",
        "Color 1 Position",
        -200.0,
        200.0,
        -200.0,
        1.0,
        " %",
    ),
    FieldSpec::range(
        "color2Position",
        "Color 2 Position",
        -200.0,
        200.0,
        200.0,
        1.0,
        " %",
    ),
    FieldSpec::number("width", "Width", 100.0, 2000.0, 1280.0, 1.0),
    FieldSpec::number("height", "Height", 100.0, 2000.0, 720.0, 1.0),
];

const NOISE_KINDS: &[SelectOption] = &[
    SelectOption {
        label: "fBM",
        value: "fbm",
    },
    SelectOption {
        label: "Perlin",
        value: "perlin",
    },
    SelectOption {
        label: "Simplex",
        value: "simplex",
    },
    SelectOption {
        label: "Multifractal",
        value: "multifractal",
    },
    SelectOption {
        label: "Ridged",
        value: "ridged",
    },
];

const NOISE_FIELDS: &[FieldSpec] = &[
    FieldSpec::select("kind", "Kind", NOISE_KINDS, "fbm"),
    FieldSpec::range("scale", "Scale", 0.1, 50.0, 5.0, 0.1, ""),
    FieldSpec::range("detail", "Detail", 1.0, 8.0, 4.0, 1.0, ""),
    FieldSpec::range("roughness", "Roughness", 0.0, 1.0, 0.5, 0.01, ""),
    FieldSpec::range("lacunarity", "Lacunarity", 1.0, 10.0, 2.0, 0.1, ""),
    FieldSpec::range("distortion", "Distortion", 0.0, 10.0, 0.0, 0.1, ""),
    FieldSpec::number("seed", "Seed", 0.0, 10000.0, 0.0, 1.0),
];

const TWO_SOCKETS: &[SocketSpec] = &[
    SocketSpec {
        label: "",
        multi: false,
    },
    SocketSpec {
        label: "",
        multi: false,
    },
];

const MULTI_SOCKET: &[SocketSpec] = &[SocketSpec {
    label: "",
    multi: true,
}];

pub(crate) static NODE_SPECS: &[NodeSpec] = &[
    NodeSpec {
        kind: Kind::Image,
        title: "Image",
        icon: icons::FOLDER_OPEN,
        keywords: &[
            "load", "file", "picture", "texture", "source", "asset", "input",
        ],
        accent: pal::node_header::MOSS,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: NO_FIELDS,
        sockets: SINGLE_SOCKET,
        create: nodes::image::create,
        body: nodes::image::show_body,
        eval: None,
    },
    NodeSpec {
        kind: Kind::RemoveBg,
        title: "Remove Background",
        icon: icons::NETWORK,
        keywords: &["background", "matte", "birefnet", "cutout", "alpha"],
        accent: pal::node_header::STEEL_BLUE,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: NO_FIELDS,
        sockets: SINGLE_SOCKET,
        create: nodes::remove_bg::create,
        body: nodes::remove_bg::show_body,
        eval: None,
    },
    NodeSpec {
        kind: Kind::RemoveBgV2,
        title: "Remove Background (v2)",
        icon: icons::NETWORK,
        keywords: &["background", "matte", "rmbg", "cutout", "alpha", "v2"],
        accent: pal::node_header::STEEL_BLUE,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: NO_FIELDS,
        sockets: SINGLE_SOCKET,
        create: nodes::remove_bgv2::create,
        body: nodes::remove_bgv2::show_body,
        eval: None,
    },
    NodeSpec {
        kind: Kind::FaceRestore,
        title: "Face Restore",
        icon: icons::PENCIL,
        keywords: &["face", "restore", "gfpgan", "upscale", "enhance"],
        accent: pal::node_header::BRONZE,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: NO_FIELDS,
        sockets: SINGLE_SOCKET,
        create: nodes::face_restore::create,
        body: nodes::face_restore::show_body,
        eval: None,
    },
    NodeSpec {
        kind: Kind::Output,
        title: "Output",
        icon: icons::EYE,
        keywords: &["output", "view", "layers", "preview"],
        accent: pal::node_header::MAROON,
        inputs: 1,
        outputs: 0,
        body_width: NODE_W,
        multi_input: true,
        hidden: true,
        fields: NO_FIELDS,
        sockets: MULTI_SOCKET,
        create: nodes::output::create,
        body: nodes::output::show_body,
        eval: None,
    },
    NodeSpec {
        kind: Kind::Route,
        title: "Route",
        icon: icons::ARROW_RIGHT,
        keywords: &["route", "pass", "passthrough", "identity"],
        accent: pal::node_header::NEUTRAL,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W_S,
        multi_input: false,
        hidden: false,
        fields: NO_FIELDS,
        sockets: SINGLE_SOCKET,
        create: nodes::route::create,
        body: nodes::route::show_body,
        eval: Some(|inputs, _, _| Ok(ops::primary_input(inputs)?.clone())),
    },
    NodeSpec {
        kind: Kind::Group,
        title: "Group",
        icon: icons::NETWORK,
        keywords: &["group", "mix", "stack", "composite"],
        accent: pal::node_header::PLUM,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: true,
        hidden: false,
        fields: NO_FIELDS,
        sockets: MULTI_SOCKET,
        create: nodes::filter::create,
        body: nodes::filter::show_body,
        eval: Some(|inputs, _, _| {
            ops::composite::stack(inputs.first().map(Vec::as_slice).unwrap_or(&[]))
        }),
    },
    NodeSpec {
        kind: Kind::Blend,
        title: "Blend",
        icon: icons::COPY,
        keywords: &["blend", "mix", "composite", "overlay"],
        accent: pal::node_header::PLUM,
        inputs: 2,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: BLEND_FIELDS,
        sockets: TWO_SOCKETS,
        create: nodes::filter::create,
        body: nodes::filter::show_body,
        eval: Some(|inputs, params, canvas| {
            ops::composite::blend(
                inputs.first().and_then(|list| list.first()),
                inputs.get(1).and_then(|list| list.first()),
                params,
                canvas,
            )
        }),
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Mask),
        title: "Mask",
        icon: icons::EYE_OFF,
        keywords: &["mask", "alpha", "cutout"],
        accent: pal::node_header::OLIVE,
        inputs: 2,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: MASK_FIELDS,
        sockets: TWO_SOCKETS,
        create: nodes::filter::create,
        body: nodes::filter::show_body,
        eval: Some(|inputs, params, _| {
            Ok(ops::composite::apply_mask(
                ops::primary_input(inputs)?,
                inputs.get(1).and_then(|list| list.first()),
                params,
            ))
        }),
    },
    NodeSpec::filter(
        FilterKind::Hsv,
        "Hue / Saturation",
        icons::FILTER,
        pal::node_header::TEAL,
        &["hue", "saturation", "value", "color"],
        HSV_FIELDS,
        |inputs, params, _| Ok(ops::adjust::hsv(ops::primary_input(inputs)?, params)),
    ),
    NodeSpec::filter(
        FilterKind::Curves,
        "Curves",
        icons::PENCIL,
        pal::node_header::SAND,
        &["curves", "levels", "lut"],
        CURVES_FIELDS,
        |inputs, params, _| Ok(ops::adjust::curves(ops::primary_input(inputs)?, params)),
    ),
    NodeSpec::filter(
        FilterKind::Exposure,
        "Exposure",
        icons::ZOOM_IN,
        pal::node_header::BRONZE,
        &["exposure", "brightness", "gamma"],
        EXPOSURE_FIELDS,
        |inputs, params, _| Ok(ops::adjust::exposure(ops::primary_input(inputs)?, params)),
    ),
    NodeSpec::filter(
        FilterKind::ColorBalance,
        "Color Balance",
        icons::FILTER,
        pal::node_header::TEAL,
        &["color", "balance", "tone"],
        COLOR_BALANCE_FIELDS,
        |inputs, params, _| {
            Ok(ops::adjust::color_balance(
                ops::primary_input(inputs)?,
                params,
            ))
        },
    ),
    NodeSpec::filter(
        FilterKind::GradientMap,
        "Gradient Map",
        icons::FILTER,
        pal::node_header::OLIVE,
        &["gradient", "map", "duotone"],
        GRADIENT_MAP_FIELDS,
        |inputs, params, _| {
            Ok(ops::adjust::gradient_map(
                ops::primary_input(inputs)?,
                params,
            ))
        },
    ),
    NodeSpec::filter(
        FilterKind::Invert,
        "Invert",
        icons::REFRESH,
        pal::node_header::MAROON,
        &["invert", "negative"],
        INVERT_FIELDS,
        |inputs, params, _| Ok(ops::adjust::invert(ops::primary_input(inputs)?, params)),
    ),
    NodeSpec::filter(
        FilterKind::RawFilter,
        "Raw Filter",
        icons::FILTER,
        pal::node_header::BRONZE,
        &[
            "raw",
            "develop",
            "temperature",
            "tint",
            "texture",
            "clarity",
            "dehaze",
            "sharpen",
            "noise reduction",
        ],
        RAW_FILTER_FIELDS,
        |inputs, params, _| Ok(ops::rf::raw_filter(ops::primary_input(inputs)?, params)),
    ),
    NodeSpec::generator(
        FilterKind::Gradient,
        "Gradient",
        icons::DOWNLOAD,
        pal::node_header::SAND,
        &["gradient", "background", "generate"],
        GRADIENT_FIELDS,
        |_, params, _| Ok(ops::generate::gradient(params)),
    ),
    NodeSpec::generator(
        FilterKind::Noise,
        "Noise",
        icons::REFRESH,
        pal::node_header::MOSS,
        &["noise", "texture", "procedural"],
        NOISE_FIELDS,
        |_, params, canvas| Ok(ops::generate::noise(params, canvas)),
    ),
    NodeSpec {
        kind: Kind::Texts,
        title: "Texts",
        icon: icons::MENU,
        keywords: &["text", "texts", "font", "caption", "title", "typography"],
        accent: pal::node_header::SAND,
        inputs: 0,
        outputs: 1,
        body_width: NODE_W * 1.5,
        multi_input: false,
        hidden: false,
        fields: NO_FIELDS,
        sockets: NO_SOCKETS,
        create: nodes::texts::create,
        body: nodes::texts::show_body,
        eval: Some(|_, params, canvas| ops::texts::texts(params, canvas)),
    },
];

pub(crate) fn spec(kind: Kind) -> &'static NodeSpec {
    catalog()
        .read()
        .expect("node catalog lock poisoned")
        .specs()
        .iter()
        .find(|spec| spec.kind == kind)
        .copied()
        .expect("every node kind must be registered in the node catalog")
}

pub(crate) fn all_specs() -> Vec<&'static NodeSpec> {
    catalog()
        .read()
        .expect("node catalog lock poisoned")
        .specs()
        .to_vec()
}

fn fuzzy_score(candidate: &str, filter: &str) -> Option<usize> {
    let candidate = candidate.to_ascii_lowercase();
    if let Some(index) = candidate.find(filter) {
        return Some(index);
    }
    let mut chars = candidate.char_indices();
    let mut start = None;
    let mut end = 0;
    for needle in filter.chars() {
        let (index, _) = chars.find(|(_, ch)| *ch == needle)?;
        start.get_or_insert(index);
        end = index;
    }
    Some(100 + end - start.unwrap_or(end))
}

#[cfg(test)]
mod catalog_tests {
    use super::{NodeCatalog, NodeCategory, fuzzy_score};

    #[test]
    fn catalog_accepts_dynamic_specs_and_replaces_matching_kind() {
        let mut catalog = NodeCatalog::default();
        let image = *super::spec(crate::model::Kind::Image);
        catalog.register(image).unwrap();
        let mut updated = image;
        updated.title = "Updated Image";
        catalog.register(updated).unwrap();
        assert_eq!(catalog.specs().len(), 1);
        assert_eq!(catalog.specs()[0].title, "Updated Image");
    }

    #[test]
    fn fuzzy_search_matches_subsequences_and_categories_are_ordered() {
        assert!(fuzzy_score("Curves", "crvs").is_some());
        assert!(fuzzy_score("Curves", "zzz").is_none());
        assert_eq!(NodeCategory::ALL.len(), 6);
    }
}
