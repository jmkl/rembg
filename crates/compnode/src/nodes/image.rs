//! Image source node.

use egui::Ui;
use egui_snarl::{InPinId, NodeId, Snarl};
use rfd::FileDialog;

use crate::editor::GraphEditor;
use crate::graph::{effective_dimensions, effective_texture, invalidate_downstream, upstream};
use crate::model::{Kind, Node};
use crate::nodes::IMAGE_EXTENSIONS;
use crate::style::controls::{Button, muted};
use crate::style::font;
use crate::widgets::{error_line, preview};

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: Kind) {
    snarl.insert_node(pos, Node::image(""));
}

pub(crate) fn show_body(
    editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    width: f32,
) {
    let texture = effective_texture(snarl, node);
    if snarl[node].preview_enabled() {
        preview(ui, texture.as_ref(), egui::vec2(width, 140.0));
    }

    let upstream_connected = upstream(snarl, node).is_some();
    let (source_label, dimensions) = if upstream_connected {
        let dims = effective_dimensions(snarl, node, &editor.assets);
        ("Source: upstream input (takes priority)".to_owned(), dims)
    } else if let Some(image) = snarl[node].image_node() {
        let dims = if let Some(asset_id) = &image.asset_id {
            editor.assets.dimensions(asset_id)
        } else if !image.path.is_empty() {
            image::image_dimensions(&image.path).ok()
        } else {
            image.preview.cpu.as_ref().map(|cpu| cpu.dimensions())
        };
        let label = if !image.name.is_empty() {
            format!("Source: {} (selected asset)", image.name)
        } else if !image.path.is_empty() {
            format!(
                "Source: {} (selected asset)",
                std::path::Path::new(&image.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_owned())
            )
        } else {
            "Source: no image selected".to_owned()
        };
        (label, dims)
    } else {
        ("Source: no image selected".to_owned(), None)
    };

    ui.add(
        egui::Label::new(muted(source_label, font::SMALL))
            .selectable(false)
            .truncate(),
    );
    if let Some((width, height)) = dimensions {
        ui.add(
            egui::Label::new(muted(format!("{width} × {height} px"), font::TINY))
            .selectable(false),
        );
    }
    error_line(ui, snarl[node].error());

    let mut pick = false;
    let mut reload = false;
    ui.columns(2, |cols| {
        pick = cols[0]
            .add(Button::new("Replace").success().small().full_width())
            .clicked();
        reload = cols[1]
            .add(Button::new("Reload").danger().small().full_width())
            .clicked();
    });

    if pick
        && let Some(path) = pick_image()
        && let Ok(asset_id) = editor.assets.insert_path(path)
    {
        snarl.drop_inputs(InPinId { node, input: 0 });
        if let Some(image) = snarl[node].image_node_mut() {
            image.asset_id = Some(asset_id.clone());
            image.name = editor
                .assets
                .name(&asset_id)
                .unwrap_or("image.png")
                .to_owned();
            image.path = editor
                .assets
                .path(&asset_id)
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            image.preview.clear();
        }
        invalidate_downstream(snarl, node);
        editor.assets.retain_used(snarl);
    }

    if reload {
        if let Some(image) = snarl[node].image_node_mut() {
            image.preview.clear();
        }
        invalidate_downstream(snarl, node);
    }
}

fn pick_image() -> Option<String> {
    FileDialog::new()
        .add_filter("images", IMAGE_EXTENSIONS)
        .pick_file()
        .map(|path| path.display().to_string())
}

pub(crate) fn pick_images() -> Option<Vec<String>> {
    FileDialog::new()
        .add_filter("images", IMAGE_EXTENSIONS)
        .pick_files()
        .map(|paths| {
            paths
                .into_iter()
                .map(|path| path.display().to_string())
                .collect()
        })
}
