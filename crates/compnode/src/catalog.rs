//! The node registry.

use egui::{Color32, Pos2, Ui};
use egui_snarl::{NodeId, Snarl};

use crate::editor::GraphEditor;
use crate::model::{FilterKind, Kind, Node};
use crate::nodes;
use crate::style::{NODE_W, NODE_W_S, icons, pal};

pub(crate) type BodyFn = fn(&mut GraphEditor, NodeId, &mut Ui, &mut Snarl<Node>, f32);
pub(crate) type CreateFn = fn(&mut Snarl<Node>, Pos2, Kind);

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
        step: f64,
        suffix: &'static str,
    },
    Number {
        min: f32,
        max: f32,
        step: f64,
    },
    Color,
    Select(&'static [SelectOption]),
}

#[derive(Clone, Copy)]
pub(crate) struct FieldSpec {
    pub key: &'static str,
    #[allow(dead_code)]
    pub label: &'static str,
    pub kind: FieldKind,
}

#[derive(Clone, Copy)]
pub(crate) struct SocketSpec {
    #[allow(dead_code)]
    pub label: &'static str,
    #[allow(dead_code)]
    pub multi: bool,
}

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
}

impl NodeSpec {
    pub(crate) fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim();
        if filter.is_empty() {
            return true;
        }
        let filter = filter.to_ascii_lowercase();
        self.title.to_ascii_lowercase().contains(&filter)
            || self
                .keywords
                .iter()
                .any(|keyword| keyword.to_ascii_lowercase().contains(&filter))
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
    FieldSpec {
        key: "hue",
        label: "Hue",
        kind: FieldKind::Range {
            min: 0.0,
            max: 360.0,
            step: 1.0,
            suffix: "°",
        },
    },
    FieldSpec {
        key: "sat",
        label: "Saturation",
        kind: FieldKind::Range {
            min: 0.0,
            max: 200.0,
            step: 1.0,
            suffix: " %",
        },
    },
    FieldSpec {
        key: "val",
        label: "Value",
        kind: FieldKind::Range {
            min: 0.0,
            max: 200.0,
            step: 1.0,
            suffix: " %",
        },
    },
];

const CURVES_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "channel",
    label: "Channel",
    kind: FieldKind::Select(&[
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
    ]),
}];

const EXPOSURE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "ev",
        label: "Exposure (EV)",
        kind: FieldKind::Range {
            min: -5.0,
            max: 5.0,
            step: 0.1,
            suffix: "",
        },
    },
    FieldSpec {
        key: "offset",
        label: "Offset",
        kind: FieldKind::Range {
            min: -50.0,
            max: 50.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "gamma",
        label: "Gamma",
        kind: FieldKind::Range {
            min: 0.1,
            max: 3.0,
            step: 0.01,
            suffix: "",
        },
    },
];

const COLOR_BALANCE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "range",
        label: "Range",
        kind: FieldKind::Select(&[
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
        ]),
    },
    FieldSpec {
        key: "cyanRed",
        label: "Cyan / Red",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "magentaGreen",
        label: "Magenta / Green",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "yellowBlue",
        label: "Yellow / Blue",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "preserve",
        label: "Preserve Luma",
        kind: FieldKind::Select(&[
            SelectOption {
                label: "Yes",
                value: "yes",
            },
            SelectOption {
                label: "No",
                value: "no",
            },
        ]),
    },
];

const GRADIENT_MAP_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "color1",
        label: "Shadow",
        kind: FieldKind::Color,
    },
    FieldSpec {
        key: "color2",
        label: "Midtone",
        kind: FieldKind::Color,
    },
    FieldSpec {
        key: "color3",
        label: "Highlight",
        kind: FieldKind::Color,
    },
    FieldSpec {
        key: "midpoint",
        label: "Midpoint",
        kind: FieldKind::Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: " %",
        },
    },
    FieldSpec {
        key: "reverse",
        label: "Reverse",
        kind: FieldKind::Select(&[
            SelectOption {
                label: "No",
                value: "no",
            },
            SelectOption {
                label: "Yes",
                value: "yes",
            },
        ]),
    },
];

const INVERT_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "amount",
    label: "Amount",
    kind: FieldKind::Range {
        min: 0.0,
        max: 100.0,
        step: 1.0,
        suffix: " %",
    },
}];

const RAW_FILTER_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "temp",
        label: "Temperature",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "tint",
        label: "Tint",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "texture",
        label: "Texture",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "clarity",
        label: "Clarity",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "dehaze",
        label: "Dehaze",
        kind: FieldKind::Range {
            min: -100.0,
            max: 100.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "sharpen",
        label: "Sharpen",
        kind: FieldKind::Range {
            min: 0.0,
            max: 150.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "sharpen_radius",
        label: "Sharpen Radius",
        kind: FieldKind::Range {
            min: 0.5,
            max: 3.0,
            step: 0.1,
            suffix: "",
        },
    },
    FieldSpec {
        key: "sharpen_detail",
        label: "Sharpen Detail",
        kind: FieldKind::Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: " %",
        },
    },
    FieldSpec {
        key: "noise_reduction",
        label: "Noise Reduction",
        kind: FieldKind::Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: " %",
        },
    },
    FieldSpec {
        key: "noise_reduction_detail",
        label: "Noise Reduction Detail",
        kind: FieldKind::Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: " %",
        },
    },
];

const BLEND_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "mode",
        label: "Mode",
        kind: FieldKind::Select(BLEND_MODES),
    },
    FieldSpec {
        key: "opacity",
        label: "Opacity",
        kind: FieldKind::Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: " %",
        },
    },
];

const MASK_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "invert",
        label: "Mask Source",
        kind: FieldKind::Select(&[
            SelectOption {
                label: "Dark is opaque",
                value: "dark",
            },
            SelectOption {
                label: "Light is opaque",
                value: "light",
            },
        ]),
    },
    FieldSpec {
        key: "strength",
        label: "Strength",
        kind: FieldKind::Range {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            suffix: " %",
        },
    },
];

const GRADIENT_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "mode",
        label: "Mode",
        kind: FieldKind::Select(&[
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
        ]),
    },
    FieldSpec {
        key: "color1",
        label: "Color 1",
        kind: FieldKind::Color,
    },
    FieldSpec {
        key: "color2",
        label: "Color 2",
        kind: FieldKind::Color,
    },
    FieldSpec {
        key: "angle",
        label: "Angle",
        kind: FieldKind::Range {
            min: 0.0,
            max: 360.0,
            step: 1.0,
            suffix: "°",
        },
    },
    FieldSpec {
        key: "color1Position",
        label: "Color 1 Position",
        kind: FieldKind::Range {
            min: -200.0,
            max: 200.0,
            step: 1.0,
            suffix: " %",
        },
    },
    FieldSpec {
        key: "color2Position",
        label: "Color 2 Position",
        kind: FieldKind::Range {
            min: -200.0,
            max: 200.0,
            step: 1.0,
            suffix: " %",
        },
    },
    FieldSpec {
        key: "width",
        label: "Width",
        kind: FieldKind::Number {
            min: 100.0,
            max: 2000.0,
            step: 1.0,
        },
    },
    FieldSpec {
        key: "height",
        label: "Height",
        kind: FieldKind::Number {
            min: 100.0,
            max: 2000.0,
            step: 1.0,
        },
    },
];

const NOISE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "kind",
        label: "Kind",
        kind: FieldKind::Select(&[
            SelectOption {
                label: "fBM",
                value: "fbm",
            },
            SelectOption {
                label: "Multifractal",
                value: "multifractal",
            },
            SelectOption {
                label: "Ridged",
                value: "ridged",
            },
        ]),
    },
    FieldSpec {
        key: "scale",
        label: "Scale",
        kind: FieldKind::Range {
            min: 0.1,
            max: 50.0,
            step: 0.1,
            suffix: "",
        },
    },
    FieldSpec {
        key: "detail",
        label: "Detail",
        kind: FieldKind::Range {
            min: 1.0,
            max: 8.0,
            step: 1.0,
            suffix: "",
        },
    },
    FieldSpec {
        key: "roughness",
        label: "Roughness",
        kind: FieldKind::Range {
            min: 0.0,
            max: 1.0,
            step: 0.01,
            suffix: "",
        },
    },
    FieldSpec {
        key: "lacunarity",
        label: "Lacunarity",
        kind: FieldKind::Range {
            min: 1.0,
            max: 10.0,
            step: 0.1,
            suffix: "",
        },
    },
    FieldSpec {
        key: "distortion",
        label: "Distortion",
        kind: FieldKind::Range {
            min: 0.0,
            max: 10.0,
            step: 0.1,
            suffix: "",
        },
    },
    FieldSpec {
        key: "seed",
        label: "Seed",
        kind: FieldKind::Number {
            min: 0.0,
            max: 10000.0,
            step: 1.0,
        },
    },
];

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
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::image::create,
        body: nodes::image::show_body,
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
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::remove_bg::create,
        body: nodes::remove_bg::show_body,
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
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::remove_bgv2::create,
        body: nodes::remove_bgv2::show_body,
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
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::face_restore::create,
        body: nodes::face_restore::show_body,
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
        sockets: &[SocketSpec {
            label: "",
            multi: true,
        }],
        create: nodes::output::create,
        body: nodes::output::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Route),
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
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::route::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Group),
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
        sockets: &[SocketSpec {
            label: "",
            multi: true,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Blend),
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
        sockets: &[
            SocketSpec {
                label: "",
                multi: false,
            },
            SocketSpec {
                label: "",
                multi: false,
            },
        ],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
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
        sockets: &[
            SocketSpec {
                label: "",
                multi: false,
            },
            SocketSpec {
                label: "",
                multi: false,
            },
        ],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Hsv),
        title: "Hue / Saturation",
        icon: icons::FILTER,
        keywords: &["hue", "saturation", "value", "color"],
        accent: pal::node_header::TEAL,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: HSV_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Curves),
        title: "Curves",
        icon: icons::PENCIL,
        keywords: &["curves", "levels", "lut"],
        accent: pal::node_header::SAND,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: CURVES_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Exposure),
        title: "Exposure",
        icon: icons::ZOOM_IN,
        keywords: &["exposure", "brightness", "gamma"],
        accent: pal::node_header::BRONZE,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: EXPOSURE_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::ColorBalance),
        title: "Color Balance",
        icon: icons::FILTER,
        keywords: &["color", "balance", "tone"],
        accent: pal::node_header::TEAL,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: COLOR_BALANCE_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::GradientMap),
        title: "Gradient Map",
        icon: icons::FILTER,
        keywords: &["gradient", "map", "duotone"],
        accent: pal::node_header::OLIVE,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: GRADIENT_MAP_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Invert),
        title: "Invert",
        icon: icons::REFRESH,
        keywords: &["invert", "negative"],
        accent: pal::node_header::MAROON,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: INVERT_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::RawFilter),
        title: "Raw Filter",
        icon: icons::FILTER,
        keywords: &[
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
        accent: pal::node_header::BRONZE,
        inputs: 1,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: RAW_FILTER_FIELDS,
        sockets: &[SocketSpec {
            label: "",
            multi: false,
        }],
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Gradient),
        title: "Gradient",
        icon: icons::DOWNLOAD,
        keywords: &["gradient", "background", "generate"],
        accent: pal::node_header::SAND,
        inputs: 0,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: GRADIENT_FIELDS,
        sockets: NO_SOCKETS,
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
    NodeSpec {
        kind: Kind::Filter(FilterKind::Noise),
        title: "Noise",
        icon: icons::REFRESH,
        keywords: &["noise", "texture", "procedural"],
        accent: pal::node_header::MOSS,
        inputs: 0,
        outputs: 1,
        body_width: NODE_W,
        multi_input: false,
        hidden: false,
        fields: NOISE_FIELDS,
        sockets: NO_SOCKETS,
        create: nodes::filter::create,
        body: nodes::filter::show_body,
    },
];

pub(crate) fn spec(kind: Kind) -> &'static NodeSpec {
    NODE_SPECS
        .iter()
        .find(|spec| spec.kind == kind)
        .expect("every node kind must be registered in NODE_SPECS")
}
