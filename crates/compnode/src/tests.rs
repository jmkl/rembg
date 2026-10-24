//! Small, fast graph/session tests. No GUI or model loading is required.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use egui_snarl::Snarl;
use egui_snarl::ui::SnarlViewer;

use crate::app::CompNodeApp;
use crate::assets::AssetStore;
use crate::cache::{OutputCache, cache_key};
use crate::editor::GraphEditor;
use crate::gpaste_app::GPasTeApp;
use crate::graph::{invalidate_node, refresh_viewer, rgba_to_texture, upstream_texture};
use crate::image_canvas::ImageCanvas;
use crate::model::{Node, ViewerLayer};
use crate::session::{self, SessionFile, SessionNodeData};
use crate::settings::Settings;
use crate::style::{install_theme, snarl_style};
use crate::worker::WorkerRequest;

fn app_with_cache(cache: OutputCache) -> CompNodeApp {
    let ctx = egui::Context::default();
    install_theme(&ctx);
    let (worker, _job_rx) = mpsc::channel::<WorkerRequest>();
    let (_done_tx, inflight) = mpsc::channel();
    let (tx, rx) = crossbeam_channel::unbounded();
    let settings = Settings::default();
    CompNodeApp {
        snarl: Snarl::new(),
        style: snarl_style(),
        editor: GraphEditor::new(worker.clone()),
        inflight,
        cache,
        session_path: None,
        notice: None,
        autosave_path: std::env::temp_dir().join("compnode-test-autosave.rembg.json"),
        last_autosave: None,
        last_autosave_graph: None,
        last_autosave_asset_revision: 0,
        autosave_due: None,
        node_filter: String::new(),
        settings_draft: settings.clone(),
        settings_open: false,
        settings,
        worker,
        url_tx: {
            let (tx, _rx) = mpsc::channel::<crate::assets::FetchedImage>();
            tx
        },
        url_rx: {
            let (_tx, rx) = mpsc::channel::<crate::assets::FetchedImage>();
            rx
        },
        url_open: false,
        url_text: String::new(),
        input_folder_open: false,
        paste_pending: false,
        ctrl_v_was_down: false,
        sidebar_width: None,
        sidebar_maximized: false,
        images_need_load: true,
        image_canvas: ImageCanvas::new(GPasTeApp::new(tx, rx)),
        sidebar_restore: false,
    }
}

fn test_editor() -> GraphEditor {
    let (jobs, _job_rx) = mpsc::channel::<WorkerRequest>();
    GraphEditor::new(jobs)
}

fn image_node(path: &str, texture: egui::TextureHandle) -> Node {
    let mut node = Node::image(path);
    if let Some(image) = node.image_node_mut() {
        image.preview.texture = Some(texture);
    }
    node
}

#[test]
fn processing_preview_follows_its_input_and_clears_with_the_wire() {
    let ctx = egui::Context::default();
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let texture = rgba_to_texture(&ctx, &image);

    let mut snarl = Snarl::new();
    let source = snarl.insert_node(
        egui::pos2(0.0, 0.0),
        image_node("C:/missing/input.png", texture),
    );
    let process = snarl.insert_node(egui::pos2(200.0, 0.0), Node::remove_bg());
    let from = egui_snarl::OutPinId {
        node: source,
        output: 0,
    };
    let to = egui_snarl::InPinId {
        node: process,
        input: 0,
    };
    snarl.connect(from, to);

    assert!(upstream_texture(&snarl, process).is_some());

    // A completed result wins over the input fallback until the wire changes.
    let result = rgba_to_texture(&ctx, &image);
    snarl[process].set_image_data(result, image);
    snarl[process].set_output_path(Some(std::path::PathBuf::from("C:/tmp/result.png")));
    assert!(snarl[process].texture().is_some());

    invalidate_node(&mut snarl, process);
    assert!(snarl[process].texture().is_none());
    assert!(snarl[process].output_path().is_none());

    snarl.disconnect(from, to);
    invalidate_node(&mut snarl, process);
    assert!(upstream_texture(&snarl, process).is_none());
}

#[test]
fn viewer_sessions_without_layer_data_still_parse() {
    let file: SessionFile = serde_json::from_str(
        r#"{
            "version": 1,
            "nodes": [{
                "id": 0,
                "pos": [0.0, 0.0],
                "collapsed": false,
                "data": { "kind": "viewer" }
            }],
            "links": []
        }"#,
    )
    .unwrap();
    match &file.nodes[0].data {
        SessionNodeData::Viewer { layers, .. } => assert!(layers.is_empty()),
        _ => unreachable!(),
    }
}

#[test]
fn processing_nodes_replace_their_single_input() {
    let ctx = egui::Context::default();
    let mut editor = test_editor();
    let mut snarl = Snarl::new();
    let mut img = image::RgbaImage::new(4, 4);
    img.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
    let tex = rgba_to_texture(&ctx, &img);

    let a = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("a", tex.clone()));
    let b = snarl.insert_node(egui::pos2(0.0, 100.0), image_node("b", tex));
    let bg = snarl.insert_node(egui::pos2(200.0, 0.0), Node::remove_bg());

    let from_a = snarl.out_pin(egui_snarl::OutPinId { node: a, output: 0 });
    let to = snarl.in_pin(egui_snarl::InPinId { node: bg, input: 0 });
    editor.connect(&from_a, &to, &mut snarl);

    let from_b = snarl.out_pin(egui_snarl::OutPinId { node: b, output: 0 });
    let to = snarl.in_pin(egui_snarl::InPinId { node: bg, input: 0 });
    editor.connect(&from_b, &to, &mut snarl);

    let remotes = snarl
        .in_pin(egui_snarl::InPinId { node: bg, input: 0 })
        .remotes;
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].node, b);
}

#[test]
fn viewer_stacks_multiple_inputs_in_connection_order() {
    let ctx = egui::Context::default();
    let mut editor = test_editor();
    let mut snarl = Snarl::new();
    let mut img = image::RgbaImage::new(4, 4);
    img.put_pixel(0, 0, image::Rgba([40, 50, 60, 255]));
    let tex = rgba_to_texture(&ctx, &img);

    let first = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("first", tex.clone()));
    let second = snarl.insert_node(egui::pos2(0.0, 100.0), image_node("second", tex));
    let viewer = snarl.insert_node(egui::pos2(200.0, 0.0), Node::viewer());

    for source in [second, first] {
        let from = snarl.out_pin(egui_snarl::OutPinId {
            node: source,
            output: 0,
        });
        let to = snarl.in_pin(egui_snarl::InPinId {
            node: viewer,
            input: 0,
        });
        editor.connect(&from, &to, &mut snarl);
    }

    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
    let layers = &snarl[viewer].viewer_node().unwrap().layers;
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].source, second);
    assert_eq!(layers[1].source, first);
    assert!(layers.iter().all(|layer| layer.texture.is_some()));
}

#[test]
fn session_roundtrip_restores_graph_and_cached_output() {
    let root = std::env::temp_dir().join("compnode-session-test");
    let _ = fs::remove_dir_all(&root);
    let cache = OutputCache::new(root.join("cache"));
    let mut app = app_with_cache(cache.clone());

    let mut img = image::RgbaImage::new(32, 24);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba([x as u8 * 4, y as u8 * 4, 90, 200]);
    }
    let key = cache_key("test-v1", &img, &[]);
    let output_path = cache.store_named(&key, "input", "test", &img).unwrap();

    let ctx = egui::Context::default();
    let tex = rgba_to_texture(&ctx, &img);
    let image_node = app
        .snarl
        .insert_node(egui::pos2(12.0, 34.0), Node::image("C:/missing/input.png"));
    let bg_node = app
        .snarl
        .insert_node(egui::pos2(250.0, 80.0), Node::remove_bg());
    if let Some(process) = app.snarl[bg_node].process_mut() {
        process.preview.texture = Some(tex);
        process.preview.cpu = Some(img.clone());
        process.output_path = Some(output_path.clone());
    }
    let viewer = app
        .snarl
        .insert_node(egui::pos2(500.0, 120.0), Node::viewer());
    app.snarl.connect(
        egui_snarl::OutPinId {
            node: image_node,
            output: 0,
        },
        egui_snarl::InPinId {
            node: bg_node,
            input: 0,
        },
    );
    app.snarl.connect(
        egui_snarl::OutPinId {
            node: bg_node,
            output: 0,
        },
        egui_snarl::InPinId {
            node: viewer,
            input: 0,
        },
    );
    if let Some(node) = app.snarl[viewer].viewer_node_mut() {
        node.layers.push(ViewerLayer {
            source: bg_node,
            texture: None,
            source_name: String::new(),
            source_path: None,
            center: egui::Vec2::ZERO,
            size: egui::Vec2::ZERO,
            angle: 0.0,
        });
    }

    let session = root.join("roundtrip.rembg.json");
    app.save_session(&session).unwrap();
    assert!(session.exists());
    assert!(!root.join("roundtrip.rembg.assets").exists());

    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&session).unwrap()).unwrap();
    let stored = raw["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["data"]["kind"] == "remove_bg")
        .expect("remove-bg node saved");
    assert_eq!(
        stored["data"]["path"].as_str(),
        Some(output_path.to_string_lossy().as_ref())
    );
    assert!(stored["data"].get("cache_key").is_none());

    let mut loaded = app_with_cache(cache);
    loaded.load_session(&session, &ctx).unwrap();
    assert_eq!(loaded.snarl.node_ids().count(), 3);

    let bg = loaded
        .snarl
        .node_ids()
        .find_map(|(id, node)| matches!(node, Node::RemoveBg(_)).then_some(id))
        .expect("remove-bg node restored");
    let process = loaded.snarl[bg].process().expect("remove-bg process");
    assert_eq!(
        process.preview.cpu.as_ref().map(|img| img.dimensions()),
        Some((32, 24))
    );
    assert!(process.preview.texture.is_some());
    assert_eq!(process.output_path.as_deref(), Some(output_path.as_path()));
    assert!(process.preview.error.is_none());
    assert_eq!(
        loaded.snarl.get_node_info(bg).unwrap().pos,
        egui::pos2(250.0, 80.0)
    );
    assert_eq!(
        loaded
            .snarl
            .out_pin(egui_snarl::OutPinId {
                node: bg,
                output: 0
            })
            .remotes
            .len(),
        1
    );

    let viewer = loaded
        .snarl
        .node_ids()
        .find_map(|(id, node)| matches!(node, Node::Output(_)).then_some(id))
        .expect("viewer restored");
    let layers = &loaded.snarl[viewer].viewer_node().unwrap().layers;
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].source, bg);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn settings_missing_fields_fall_back_to_defaults() {
    let settings: Settings = serde_json::from_str(r#"{"cache_models": false}"#).unwrap();
    assert!(!settings.cache_models);
    assert!(settings.model_dir.is_none());
    assert_eq!(
        settings.workspace_root,
        crate::settings::default_workspace_root()
    );
}

#[test]
fn settings_roundtrip_preserves_paths() {
    let settings = Settings {
        cache_models: false,
        model_dir: Some(std::path::PathBuf::from("D:/models")),
        workspace_root: std::path::PathBuf::from("D:/workspace"),
    };
    let text = serde_json::to_string(&settings).unwrap();
    let restored: Settings = serde_json::from_str(&text).unwrap();
    assert_eq!(restored, settings);
}

#[test]
fn legacy_cache_key_sessions_still_parse() {
    let data: SessionNodeData =
        serde_json::from_str(r#"{"kind":"remove_bg","cache_key":"abc123"}"#).unwrap();
    match data {
        SessionNodeData::RemoveBg { path, cache_key } => {
            assert!(path.is_none());
            assert_eq!(cache_key.as_deref(), Some("abc123"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn preview_textures_are_capped() {
    assert_eq!(crate::graph::preview_dimensions(4000, 2000), (200, 100));
    assert_eq!(crate::graph::preview_dimensions(800, 600), (200, 150));
    assert_eq!(crate::graph::preview_dimensions(100, 4000), (5, 200));
}

#[test]
fn memory_budget_releases_only_reloadable_images() {
    let root = std::env::temp_dir().join("compnode-memory-test");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.png");
    let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([10, 20, 30, 255]));
    image.save(&path).unwrap();

    let mut snarl = Snarl::new();
    let source = snarl.insert_node(
        egui::pos2(0.0, 0.0),
        Node::image(path.display().to_string()),
    );
    snarl[source].set_cpu(image.clone());
    let output = snarl.insert_node(egui::pos2(200.0, 0.0), Node::remove_bg());
    snarl[output].set_cpu(image);

    let mut memory = crate::memory::ImageMemory::new(1);
    memory.trim(&mut snarl, &AssetStore::default());

    assert!(snarl[source].cpu().is_none());
    assert!(snarl[output].cpu().is_some());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn session_names_auto_increment_when_taken() {
    let root = std::env::temp_dir().join("compnode-name-test");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let first = root.join("untitled.rembg.json");
    fs::write(&first, "{}").unwrap();
    let second = session::unique_session_path(&first);
    assert_eq!(second.file_name().unwrap(), "untitled (2).rembg.json");
    fs::write(&second, "{}").unwrap();
    assert_eq!(
        session::unique_session_path(&first).file_name().unwrap(),
        "untitled (3).rembg.json"
    );

    let plain = root.join("project.json");
    fs::write(&plain, "{}").unwrap();
    assert_eq!(
        session::unique_session_path(&plain).file_name().unwrap(),
        "project (2).json"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn every_node_kind_has_a_unique_catalog_entry() {
    use std::collections::HashSet;

    let mut titles = HashSet::new();
    for spec in crate::catalog::NODE_SPECS {
        assert_eq!(crate::catalog::spec(spec.kind).title, spec.title);
        assert!(titles.insert(spec.title), "duplicate title: {}", spec.title);
        assert!(spec.matches(spec.title));
    }
}
