//! The node registry.
//!
//! This is the one place that lists every node type for the graph menu and the
//! Snarl view. Adding a node is mostly adding one entry here plus its body
//! module under `src/nodes/`.
//!
//! `create` inserts a node for the context menu. `body` draws the node body.

use egui::{Color32, Pos2, Ui};
use egui_snarl::{NodeId, Snarl};
use elegance::glyphs;

use crate::editor::GraphEditor;
use crate::model::{Kind, Node};
use crate::nodes;
use crate::style::{BODY_W, VIEWER_W, pal};

pub(crate) type BodyFn = fn(&mut GraphEditor, NodeId, &mut Ui, &mut Snarl<Node>, f32);
pub(crate) type CreateFn = fn(&mut Snarl<Node>, Pos2);

pub(crate) struct NodeSpec {
    pub kind: Kind,
    pub title: &'static str,
    pub icon: char,
    /// Extra search terms shown in the graph menu.
    pub keywords: &'static [&'static str],
    pub accent: Color32,
    pub inputs: usize,
    pub outputs: usize,
    pub body_width: f32,
    /// Viewer accepts several wires on one socket; processing nodes replace.
    pub multi_input: bool,
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

pub(crate) static NODE_SPECS: &[NodeSpec] = &[
    NodeSpec {
        kind: Kind::Image,
        title: "Image",
        icon: glyphs::FOLDER_OPEN,
        keywords: &[
            "load", "file", "picture", "texture", "source", "asset", "input",
        ],
        accent: pal::HEADER_IMAGE,
        inputs: 1,
        outputs: 1,
        body_width: BODY_W,
        multi_input: false,
        create: nodes::image::create,
        body: nodes::image::show_body,
    },
    NodeSpec {
        kind: Kind::RemoveBg,
        title: "Remove Background",
        icon: glyphs::NETWORK,
        keywords: &["background", "matte", "birefnet", "cutout", "alpha"],
        accent: pal::HEADER_REMOVE_BG,
        inputs: 1,
        outputs: 1,
        body_width: BODY_W,
        multi_input: false,
        create: nodes::remove_bg::create,
        body: nodes::remove_bg::show_body,
    },
    NodeSpec {
        kind: Kind::FaceRestore,
        title: "Face Restore",
        icon: glyphs::PENCIL,
        keywords: &["face", "restore", "gfpgan", "upscale", "enhance"],
        accent: pal::HEADER_FACE_RESTORE,
        inputs: 1,
        outputs: 1,
        body_width: BODY_W,
        multi_input: false,
        create: nodes::face_restore::create,
        body: nodes::face_restore::show_body,
    },
    NodeSpec {
        kind: Kind::Output,
        title: "Output",
        icon: glyphs::EYE,
        keywords: &["output", "view", "layers", "preview"],
        accent: pal::HEADER_VIEWER,
        inputs: 1,
        outputs: 0,
        body_width: VIEWER_W,
        multi_input: true,
        create: nodes::viewer::create,
        body: nodes::viewer::show_body,
    },
];

pub(crate) fn spec(kind: Kind) -> &'static NodeSpec {
    NODE_SPECS
        .iter()
        .find(|spec| spec.kind == kind)
        .expect("every node kind must be registered in NODE_SPECS")
}
