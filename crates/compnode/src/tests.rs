//! Small, fast graph/session tests. No GUI or model loading is required.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use egui_snarl::Snarl;
use egui_snarl::ui::SnarlViewer;

use crate::app::CompNodeApp;
use crate::assets::AssetStore;
use crate::cache::{OutputCache, cache_key, cache_key_layers, cache_key_refs};
use crate::editor::GraphEditor;
use crate::gpaste_app::GPasTeApp;
use crate::graph::{
    full_res_source_path, invalidate_node, refresh_viewer, rgba_to_texture, upstream_texture,
};
use crate::image_canvas::ImageCanvas;
use crate::model::{FilterKind, Kind, Node, ViewerLayer};
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
        process.preview.texture = Some(tex.clone());
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
            kind: Kind::RemoveBg,
            texture: None,
            source_name: String::new(),
            source_path: None,
            center: egui::vec2(111.0, 222.0),
            size: egui::vec2(333.0, 444.0),
            angle: 0.25,
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
    assert_eq!(layers[0].center, egui::vec2(111.0, 222.0));
    assert_eq!(layers[0].size, egui::vec2(333.0, 444.0));
    assert_eq!(layers[0].angle, 0.25);

    // Reopening in the *same* app must not keep canvas items from the old
    // document: a later save would otherwise commit their stale transforms
    // over the restored ones.
    let stale_layers = [(
        bg,
        Kind::RemoveBg,
        String::from("stale"),
        Some(tex),
        Some(output_path.clone()),
        (egui::vec2(9.0, 9.0), egui::vec2(50.0, 50.0), 0.5),
        false,
    )];
    loaded.image_canvas.sync_layers(&ctx, &stale_layers);
    assert_eq!(loaded.image_canvas.layers().count(), 1);
    loaded.load_session(&session, &ctx).unwrap();
    assert_eq!(loaded.image_canvas.layers().count(), 0);

    // Save immediately after opening: the restored transforms must survive.
    let resaved = root.join("resaved.rembg.json");
    loaded.save_session(&resaved).unwrap();
    let mut again = app_with_cache(OutputCache::new(root.join("cache")));
    again.load_session(&resaved, &ctx).unwrap();
    let viewer = again
        .snarl
        .node_ids()
        .find_map(|(id, node)| matches!(node, Node::Output(_)).then_some(id))
        .expect("viewer restored");
    let layers = &again.snarl[viewer].viewer_node().unwrap().layers;
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].center, egui::vec2(111.0, 222.0));
    assert_eq!(layers[0].size, egui::vec2(333.0, 444.0));
    assert_eq!(layers[0].angle, 0.25);

    // Saving must flush canvas transforms immediately: they normally wait for
    // an idle timer, so a save right after a drag/resize must not miss them.
    let bg_again = again
        .snarl
        .node_ids()
        .find_map(|(id, node)| matches!(node, Node::RemoveBg(_)).then_some(id))
        .expect("remove-bg restored");
    let moved_layers = [(
        bg_again,
        Kind::RemoveBg,
        String::from("moved"),
        again.snarl[bg_again]
            .process()
            .unwrap()
            .preview
            .texture
            .clone(),
        Some(output_path.clone()),
        (egui::vec2(40.0, 60.0), egui::vec2(640.0, 480.0), 0.0),
        false,
    )];
    again.image_canvas.sync_layers(&ctx, &moved_layers);
    again.save_session(&resaved).unwrap();
    let mut third = app_with_cache(OutputCache::new(root.join("cache")));
    third.load_session(&resaved, &ctx).unwrap();
    let viewer = third
        .snarl
        .node_ids()
        .find_map(|(id, node)| matches!(node, Node::Output(_)).then_some(id))
        .expect("viewer restored");
    let layers = &third.snarl[viewer].viewer_node().unwrap().layers;
    assert_eq!(layers[0].center, egui::vec2(40.0, 60.0));
    assert_eq!(layers[0].size, egui::vec2(640.0, 480.0));
    assert_eq!(layers[0].angle, 0.0);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn filter_rerender_keeps_user_canvas_transform() {
    let root = std::env::temp_dir().join("compnode-filter-refit-test");
    let _ = fs::remove_dir_all(&root);
    let ctx = egui::Context::default();
    let mut app = app_with_cache(OutputCache::new(root.join("cache")));
    let node = app
        .snarl
        .insert_node(egui::pos2(0.0, 0.0), Node::image("curve.png"));

    // A solid texture stands in for the layer raster; only its pixel size
    // matters here (preview textures are capped, full renders are not).
    let solid = |name: &str, w: u32, h: u32| {
        let rgba = vec![255u8; (w * h * 4) as usize];
        ctx.load_texture(
            name,
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
            egui::TextureOptions::LINEAR,
        )
    };

    let sync = |app: &mut CompNodeApp, tex: &egui::TextureHandle| {
        app.image_canvas.sync_layers(
            &ctx,
            &[(
                node,
                Kind::Filter(FilterKind::Curves),
                String::from("Curves"),
                Some(tex.clone()),
                None,
                (egui::Vec2::ZERO, egui::Vec2::ZERO, 0.0),
                true,
            )],
        );
    };

    // Full-resolution render arrives first.
    let full = solid("full", 1280, 720);
    sync(&mut app, &full);
    // The user places and scales the layer.
    let (center, size) = (egui::vec2(300.0, 200.0), egui::vec2(400.0, 225.0));
    app.image_canvas
        .apply_transforms(&[(node, center, size, 0.0)]);

    // Tweaking the Curves node invalidates its output: the canvas falls back
    // to the capped preview (different pixel dims). The user transform must
    // survive that swap and the later full-res re-render.
    let preview = solid("preview", 256, 144);
    sync(&mut app, &preview);
    let layer = app.image_canvas.layers().next().expect("layer stays");
    assert_eq!(layer.center, center);
    assert_eq!(layer.size, size);

    sync(&mut app, &full);
    let layer = app.image_canvas.layers().next().expect("layer stays");
    assert_eq!(layer.center, center);
    assert_eq!(layer.size, size);

    // A layer the user never touched still follows the texture resolution.
    app.image_canvas.reset_graph();
    let square = solid("square", 1000, 1000);
    sync(&mut app, &square);
    let fitted = app.image_canvas.layers().next().unwrap().size;
    let wide = solid("wide", 200, 100);
    sync(&mut app, &wide);
    let refitted = app.image_canvas.layers().next().unwrap().size;
    assert_ne!(fitted, refitted);

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
        canvas_width: 1280.0,
        canvas_height: 720.0,
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
    assert_eq!(crate::graph::preview_dimensions(4000, 2000), (256, 128));
    assert_eq!(crate::graph::preview_dimensions(800, 600), (256, 192));
    assert_eq!(crate::graph::preview_dimensions(100, 4000), (6, 256));
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

#[test]
fn every_filter_field_has_a_default_value() {
    for spec in crate::catalog::NODE_SPECS {
        let Kind::Filter(kind) = spec.kind else {
            continue;
        };
        let defaults = kind.defaults();
        for field in spec.fields {
            assert!(
                defaults.contains_key(field.key),
                "{} is missing a default for {}",
                spec.title,
                field.key,
            );
        }
    }
}

#[test]
fn raw_filter_defaults_are_a_no_op() {
    let image = image::RgbaImage::from_fn(16, 16, |x, y| {
        image::Rgba([(x * 11) as u8, (y * 13) as u8, ((x + y) * 7) as u8, 200])
    });
    let params = FilterKind::RawFilter.defaults();
    let out = crate::ops::evaluate(
        FilterKind::RawFilter,
        &params,
        &[vec![image.clone()]],
        (16, 16),
    )
    .unwrap();
    assert_eq!(out, image);
}

#[test]
fn raw_filter_temperature_warms_the_image() {
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 128, 255]));
    let mut params = FilterKind::RawFilter.defaults();
    crate::params::set_f32(&mut params, "temp", 100.0);
    let out = crate::ops::evaluate(FilterKind::RawFilter, &params, &[vec![image]], (4, 4)).unwrap();
    let pixel = out.get_pixel(0, 0);
    assert!(pixel[0] > 128, "red should increase: {pixel:?}");
    assert!(pixel[2] < 128, "blue should decrease: {pixel:?}");
}

#[test]
fn raw_filter_preserves_alpha_with_every_control_enabled() {
    let image = image::RgbaImage::from_fn(24, 24, |x, y| {
        image::Rgba([
            (x * 9) as u8,
            (y * 10) as u8,
            ((x * y) % 255) as u8,
            ((x * 7 + y * 3) % 255) as u8,
        ])
    });
    let mut params = FilterKind::RawFilter.defaults();
    for (key, value) in [
        ("temp", 35.0),
        ("tint", -20.0),
        ("texture", 50.0),
        ("clarity", 40.0),
        ("dehaze", 30.0),
        ("sharpen", 80.0),
        ("sharpen_radius", 2.0),
        ("sharpen_detail", 60.0),
        ("noise_reduction", 50.0),
        ("noise_reduction_detail", 40.0),
    ] {
        crate::params::set_f32(&mut params, key, value);
    }

    let out = crate::ops::evaluate(
        FilterKind::RawFilter,
        &params,
        &[vec![image.clone()]],
        (24, 24),
    )
    .unwrap();
    assert_eq!(out.dimensions(), image.dimensions());
    for (source, processed) in image.pixels().zip(out.pixels()) {
        assert_eq!(source[3], processed[3], "alpha must be untouched");
    }
}

#[test]
#[ignore = "manual performance benchmark"]
fn bench_filter_evaluation() {
    use image::ImageEncoder;
    use std::time::Instant;

    let full = image::RgbaImage::from_fn(4000, 3000, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x * y) % 256) as u8, 255])
    });
    let preview = image::imageops::thumbnail(&full, 256, 192);

    let started = Instant::now();
    let resized = image::imageops::resize(&full, 256, 192, image::imageops::FilterType::Triangle);
    println!("resize 12MP -> 256 with Triangle: {:?}", started.elapsed());
    drop(resized);
    let started = Instant::now();
    let thumb = image::imageops::thumbnail(&full, 256, 192);
    println!("thumbnail 12MP -> 256: {:?}", started.elapsed());
    drop(thumb);

    let started = Instant::now();
    let png = {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                full.as_raw(),
                full.width(),
                full.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    };
    println!(
        "encode 12MP PNG: {:?} ({} bytes)",
        started.elapsed(),
        png.len()
    );
    let started = Instant::now();
    let _ = image::load_from_memory(&png).unwrap().to_rgba8();
    println!("decode 12MP PNG: {:?}", started.elapsed());

    for kind in [
        FilterKind::Hsv,
        FilterKind::Curves,
        FilterKind::Exposure,
        FilterKind::ColorBalance,
        FilterKind::GradientMap,
        FilterKind::Invert,
        FilterKind::RawFilter,
    ] {
        let mut params = kind.defaults();
        if kind == FilterKind::RawFilter {
            for (key, value) in [
                ("temp", 35.0),
                ("tint", -20.0),
                ("texture", 50.0),
                ("clarity", 40.0),
                ("dehaze", 30.0),
                ("sharpen", 80.0),
                ("sharpen_radius", 2.0),
                ("sharpen_detail", 60.0),
                ("noise_reduction", 50.0),
                ("noise_reduction_detail", 40.0),
            ] {
                crate::params::set_f32(&mut params, key, value);
            }
        }

        let started = Instant::now();
        let _ =
            crate::ops::evaluate(kind, &params, &[vec![preview.clone()]], (4000, 3000)).unwrap();
        let preview_time = started.elapsed();

        let started = Instant::now();
        let _ = crate::ops::evaluate(kind, &params, &[vec![full.clone()]], (4000, 3000)).unwrap();
        let full_time = started.elapsed();

        println!("{kind:?}: preview {preview_time:?}, full 12MP {full_time:?}");
    }
}

#[test]
fn borrowed_cache_key_matches_the_owned_variant() {
    let first = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 4]));
    let second = image::RgbaImage::from_pixel(3, 1, image::Rgba([5, 6, 7, 8]));

    let owned = cache_key_layers("test-domain", &[first.clone(), second.clone()], b"params");
    let borrowed = cache_key_refs("test-domain", [&first, &second], b"params");

    assert_eq!(owned, borrowed);
    // Reordering the inputs must produce a different key.
    let swapped = cache_key_refs("test-domain", [&second, &first], b"params");
    assert_ne!(owned, swapped);
}

#[test]
fn full_res_inputs_never_walk_past_a_filter() {
    let mut snarl = Snarl::new();
    let source = snarl.insert_node(egui::pos2(0.0, 0.0), Node::image("C:/missing/input.png"));
    let filter = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Hsv));
    snarl.connect(
        egui_snarl::OutPinId {
            node: source,
            output: 0,
        },
        egui_snarl::InPinId {
            node: filter,
            input: 0,
        },
    );

    // An unrendered filter has no full-resolution output of its own; using the
    // upstream file instead would silently drop the filter effect.
    assert!(full_res_source_path(&snarl, filter).is_none());

    snarl[filter].set_output_path(Some(PathBuf::from("C:/tmp/filter-output.png")));
    assert_eq!(
        full_res_source_path(&snarl, filter),
        Some(PathBuf::from("C:/tmp/filter-output.png")),
    );

    // Once the filter has rendered, downstream nodes may read its output.
    assert_eq!(
        crate::graph::effective_output_path(&snarl, filter),
        Some(PathBuf::from("C:/tmp/filter-output.png")),
    );
}

#[test]
fn route_is_transparent_to_downstream_nodes() {
    let ctx = egui::Context::default();
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let source_tex = rgba_to_texture(&ctx, &image);
    let route_tex = rgba_to_texture(&ctx, &image);

    let mut snarl = Snarl::new();
    let source = snarl.insert_node(
        egui::pos2(0.0, 0.0),
        image_node("C:/tmp/source.png", source_tex),
    );
    let route = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Route));
    let curve = snarl.insert_node(egui::pos2(400.0, 0.0), Node::new_filter(FilterKind::Curves));
    for (from, to) in [(source, route), (route, curve)] {
        snarl.connect(
            egui_snarl::OutPinId {
                node: from,
                output: 0,
            },
            egui_snarl::InPinId { node: to, input: 0 },
        );
    }

    // Downstream wires resolve as if the Route were not in the graph.
    assert_eq!(crate::graph::resolve_route(&snarl, route), source);
    assert_eq!(crate::graph::upstream(&snarl, curve), Some(source));
    assert_eq!(crate::graph::source_name(&snarl, route), "source.png");
    assert_eq!(
        crate::graph::effective_output_path(&snarl, route),
        Some(PathBuf::from("C:/tmp/source.png")),
    );
    assert_eq!(
        full_res_source_path(&snarl, route),
        Some(PathBuf::from("C:/tmp/source.png")),
    );

    // A filter behind a Route must still render before it can be consumed.
    let filter = snarl.insert_node(egui::pos2(600.0, 0.0), Node::new_filter(FilterKind::Hsv));
    snarl.connect(
        egui_snarl::OutPinId {
            node: curve,
            output: 0,
        },
        egui_snarl::InPinId {
            node: filter,
            input: 0,
        },
    );
    let route_after =
        snarl.insert_node(egui::pos2(800.0, 0.0), Node::new_filter(FilterKind::Route));
    snarl.connect(
        egui_snarl::OutPinId {
            node: filter,
            output: 0,
        },
        egui_snarl::InPinId {
            node: route_after,
            input: 0,
        },
    );
    assert!(full_res_source_path(&snarl, route_after).is_none());
    snarl[filter].set_output_path(Some(PathBuf::from("C:/tmp/filter.png")));
    assert_eq!(
        full_res_source_path(&snarl, route_after),
        Some(PathBuf::from("C:/tmp/filter.png")),
    );

    // A stale Route texture (from an older session/render) must not win over
    // the real upstream preview.
    snarl[route].filter_node_mut().unwrap().preview.texture = Some(route_tex);
    assert_eq!(
        upstream_texture(&snarl, curve).map(|tex| tex.id()),
        snarl[source].texture().map(|tex| tex.id()),
    );
}

#[test]
fn route_matches_a_direct_connection_during_eval() {
    let ctx = egui::Context::default();
    let input = image::RgbaImage::from_fn(16, 12, |x, y| {
        image::Rgba([(x * 7) as u8, (y * 11) as u8, ((x + y) * 3) as u8, 255])
    });

    let build = |with_route: bool| {
        let mut snarl = Snarl::new();
        let source = snarl.insert_node(
            egui::pos2(0.0, 0.0),
            Node::image("C:/missing/route-eval.png"),
        );
        snarl[source].set_cpu(input.clone());
        let curve = snarl.insert_node(egui::pos2(400.0, 0.0), Node::new_filter(FilterKind::Curves));
        if with_route {
            let route =
                snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Route));
            snarl.connect(
                egui_snarl::OutPinId {
                    node: source,
                    output: 0,
                },
                egui_snarl::InPinId {
                    node: route,
                    input: 0,
                },
            );
            snarl.connect(
                egui_snarl::OutPinId {
                    node: route,
                    output: 0,
                },
                egui_snarl::InPinId {
                    node: curve,
                    input: 0,
                },
            );
        } else {
            snarl.connect(
                egui_snarl::OutPinId {
                    node: source,
                    output: 0,
                },
                egui_snarl::InPinId {
                    node: curve,
                    input: 0,
                },
            );
        }
        crate::eval::ensure_all(&mut snarl, &ctx);
        (snarl, curve)
    };

    let (direct, direct_curve) = build(false);
    let (routed, routed_curve) = build(true);
    assert_eq!(
        direct[direct_curve].cpu().map(|img| img.as_raw()),
        routed[routed_curve].cpu().map(|img| img.as_raw()),
    );
    assert!(routed[routed_curve].cpu().is_some());
}

#[test]
fn route_is_never_rendered() {
    let mut editor = test_editor();
    let mut snarl = Snarl::new();
    let source = snarl.insert_node(egui::pos2(0.0, 0.0), Node::image("C:/missing/source.png"));
    let route = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Route));
    snarl.connect(
        egui_snarl::OutPinId {
            node: source,
            output: 0,
        },
        egui_snarl::InPinId {
            node: route,
            input: 0,
        },
    );

    let ctx = egui::Context::default();
    assert!(!editor.start_filter_render(&mut snarl, route, 1, &ctx));
    let filter = snarl[route].filter_node().unwrap();
    assert!(filter.job.is_none());
    assert!(filter.output_path.is_none());
}

#[test]
fn viewer_layers_follow_routes_and_keep_transforms() {
    let ctx = egui::Context::default();
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([40, 50, 60, 255]));
    let texture = rgba_to_texture(&ctx, &image);

    let mut snarl = Snarl::new();
    let source = snarl.insert_node(
        egui::pos2(0.0, 0.0),
        image_node("C:/missing/source.png", texture),
    );
    let viewer = snarl.insert_node(egui::pos2(400.0, 0.0), Node::viewer());
    snarl.connect(
        egui_snarl::OutPinId {
            node: source,
            output: 0,
        },
        egui_snarl::InPinId {
            node: viewer,
            input: 0,
        },
    );
    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());

    // User move/scale on the layer.
    if let Some(node) = snarl[viewer].viewer_node_mut() {
        let layer = &mut node.layers[0];
        layer.center = egui::vec2(120.0, 80.0);
        layer.size = egui::vec2(640.0, 360.0);
        layer.angle = 0.3;
    }

    // Insert a Route between the image and the viewer.
    let route = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Route));
    snarl.disconnect(
        egui_snarl::OutPinId {
            node: source,
            output: 0,
        },
        egui_snarl::InPinId {
            node: viewer,
            input: 0,
        },
    );
    snarl.connect(
        egui_snarl::OutPinId {
            node: source,
            output: 0,
        },
        egui_snarl::InPinId {
            node: route,
            input: 0,
        },
    );
    snarl.connect(
        egui_snarl::OutPinId {
            node: route,
            output: 0,
        },
        egui_snarl::InPinId {
            node: viewer,
            input: 0,
        },
    );
    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());

    let node = snarl[viewer].viewer_node().unwrap();
    assert_eq!(node.layers.len(), 1);
    assert_eq!(node.layers[0].source, source);
    assert_eq!(node.layers[0].center, egui::vec2(120.0, 80.0));
    assert_eq!(node.layers[0].size, egui::vec2(640.0, 360.0));
    assert_eq!(node.layers[0].angle, 0.3);
    assert!(node.layers[0].texture.is_some());

    // Sessions saved before Route-aware layers may store the Route id; a
    // refresh must repair it without discarding the saved transform.
    snarl[viewer].viewer_node_mut().unwrap().layers[0].source = route;
    snarl[viewer].viewer_node_mut().unwrap().layers[0].center = egui::vec2(7.0, 9.0);
    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());

    let node = snarl[viewer].viewer_node().unwrap();
    assert_eq!(node.layers.len(), 1);
    assert_eq!(node.layers[0].source, source);
    assert_eq!(node.layers[0].center, egui::vec2(7.0, 9.0));
}

#[test]
fn filter_renders_are_reclaimed_after_invalidation() {
    let mut snarl = Snarl::new();
    let source = snarl.insert_node(egui::pos2(0.0, 0.0), Node::image("C:/missing/input.png"));
    let filter = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Hsv));
    snarl.connect(
        egui_snarl::OutPinId {
            node: source,
            output: 0,
        },
        egui_snarl::InPinId {
            node: filter,
            input: 0,
        },
    );

    if let Some(node) = snarl[filter].filter_node_mut() {
        node.output_path = Some(PathBuf::from("C:/tmp/filter-output.png"));
        node.rendered = Some(42);
    }
    assert!(snarl[filter].output_path().is_some());

    // Any upstream change drops the persisted output and forces the render to
    // be re-claimed, even if the signature later returns to the same value.
    invalidate_node(&mut snarl, filter);
    assert!(snarl[filter].output_path().is_none());
    assert!(snarl[filter].filter_node().unwrap().rendered.is_none());
}

fn blend_layers_for(
    snarl: &mut Snarl<Node>,
    blend: egui_snarl::NodeId,
    sources: &[egui_snarl::NodeId],
) {
    let kind = snarl[blend].kind();
    let layers = snarl[blend].layers_mut().unwrap();
    for source in sources {
        layers.push(ViewerLayer {
            source: *source,
            kind,
            texture: None,
            source_name: String::new(),
            source_path: None,
            center: egui::Vec2::ZERO,
            size: egui::Vec2::ZERO,
            angle: 0.0,
        });
    }
}

#[test]
fn blend_collects_layers_from_both_sockets() {
    let ctx = egui::Context::default();
    let mut editor = test_editor();
    let mut snarl = Snarl::new();
    let mut img = image::RgbaImage::new(4, 4);
    img.put_pixel(0, 0, image::Rgba([40, 50, 60, 255]));
    let tex = rgba_to_texture(&ctx, &img);

    let base = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("base", tex.clone()));
    let overlay = snarl.insert_node(egui::pos2(0.0, 100.0), image_node("overlay", tex));
    let blend = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Blend));

    // Socket order: Socket 0 is Top Layer (overlay), Socket 1 is Bottom Layer (base).
    for (source, input) in [(overlay, 0usize), (base, 1usize)] {
        let from = snarl.out_pin(egui_snarl::OutPinId {
            node: source,
            output: 0,
        });
        let to = snarl.in_pin(egui_snarl::InPinId { node: blend, input });
        editor.connect(&from, &to, &mut snarl);
    }

    refresh_viewer(&mut snarl, blend, &ctx, &mut AssetStore::default());
    let layers = snarl[blend].layers().unwrap();
    assert_eq!(layers.len(), 2);
    // In viewer layers (bottom to top): base feeds the bottom layer (index 0).
    assert_eq!(layers[0].source, base);
    assert_eq!(layers[1].source, overlay);
    assert!(layers.iter().all(|layer| layer.texture.is_some()));
}

#[test]
fn blend_composites_layers_on_the_document_canvas() {
    let ctx = egui::Context::default();
    let mut snarl = Snarl::new();

    let base = snarl.insert_node(
        egui::pos2(0.0, 0.0),
        Node::image("C:/missing/blend-base.png"),
    );
    snarl[base].set_cpu(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([128, 128, 128, 255]),
    ));
    let overlay = snarl.insert_node(
        egui::pos2(0.0, 100.0),
        Node::image("C:/missing/blend-overlay.png"),
    );
    snarl[overlay].set_cpu(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([255, 255, 255, 255]),
    ));
    let blend = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Blend));

    snarl.connect(
        egui_snarl::OutPinId {
            node: base,
            output: 0,
        },
        egui_snarl::InPinId {
            node: blend,
            input: 0,
        },
    );
    snarl.connect(
        egui_snarl::OutPinId {
            node: overlay,
            output: 0,
        },
        egui_snarl::InPinId {
            node: blend,
            input: 1,
        },
    );

    // Document canvas, synced by the app like the Group node's.
    if let Some(filter) = snarl[blend].filter_mut() {
        filter.canvas_size = egui::vec2(16.0, 12.0);
    }
    blend_layers_for(&mut snarl, blend, &[base, overlay]);

    crate::eval::ensure_all(&mut snarl, &ctx);

    // Unplaced layers fill the canvas preserving their aspect ratio.
    let out = snarl[blend].filter().unwrap().preview.cpu.clone().unwrap();
    assert_eq!(out.dimensions(), (16, 12));
    assert_eq!(out.get_pixel(8, 6), &image::Rgba([255, 255, 255, 255]));

    // Moving the top layer away reveals the base underneath.
    {
        let layers = snarl[blend].layers_mut().unwrap();
        layers[0].center = egui::vec2(8.0, 6.0);
        layers[0].size = egui::vec2(16.0, 12.0);
        layers[1].center = egui::vec2(40.0, 40.0);
        layers[1].size = egui::vec2(8.0, 8.0);
    }
    crate::eval::ensure_all(&mut snarl, &ctx);
    let out = snarl[blend].filter().unwrap().preview.cpu.clone().unwrap();
    assert_eq!(out.get_pixel(2, 2), &image::Rgba([128, 128, 128, 255]));
}

#[test]
fn blend_mode_applies_to_layers_above_the_base() {
    let ctx = egui::Context::default();
    let mut snarl = Snarl::new();

    let base = snarl.insert_node(
        egui::pos2(0.0, 0.0),
        Node::image("C:/missing/mode-base.png"),
    );
    snarl[base].set_cpu(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([128, 128, 128, 255]),
    ));
    let overlay = snarl.insert_node(
        egui::pos2(0.0, 100.0),
        Node::image("C:/missing/mode-overlay.png"),
    );
    snarl[overlay].set_cpu(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([255, 255, 255, 255]),
    ));
    let blend = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Blend));

    snarl.connect(
        egui_snarl::OutPinId {
            node: base,
            output: 0,
        },
        egui_snarl::InPinId {
            node: blend,
            input: 0,
        },
    );
    snarl.connect(
        egui_snarl::OutPinId {
            node: overlay,
            output: 0,
        },
        egui_snarl::InPinId {
            node: blend,
            input: 1,
        },
    );

    if let Some(filter) = snarl[blend].filter_mut() {
        filter.canvas_size = egui::vec2(8.0, 8.0);
        crate::params::set_string(&mut filter.params, "mode", "multiply");
    }
    blend_layers_for(&mut snarl, blend, &[base, overlay]);

    crate::eval::ensure_all(&mut snarl, &ctx);

    // White multiplied over mid-grey stays mid-grey (source-over would be white).
    let out = snarl[blend].filter().unwrap().preview.cpu.clone().unwrap();
    assert_eq!(out.get_pixel(4, 4), &image::Rgba([128, 128, 128, 255]));
}

#[test]
fn blend_op_canvas_size_and_top_layer_effects() {
    let mut params = crate::params::Params::new();
    crate::params::set_string(&mut params, "mode", "multiply");
    crate::params::set_f32(&mut params, "opacity", 50.0);

    // Top layer: white (255, 255, 255)
    let top = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255]));
    // Bottom layer: mid-grey (128, 128, 128)
    let bottom = image::RgbaImage::from_pixel(100, 100, image::Rgba([128, 128, 128, 255]));

    // Canvas size from settings: 800 x 600
    let canvas = (800, 600);
    let result = crate::ops::composite::blend(Some(&top), Some(&bottom), &params, canvas).unwrap();

    assert_eq!(result.dimensions(), (800, 600));

    // Mid-pixel: White multiplied by mid-grey = mid-grey (128).
    // At 50% opacity of top layer over 128:
    // composite mixes cs (white multiplied = 128) at 50% over cb (128) -> 128.
    let px = result.get_pixel(400, 300);
    assert_eq!(px[0], 128);
    assert_eq!(px[1], 128);
    assert_eq!(px[2], 128);
    assert_eq!(px[3], 255);
}

#[test]
fn delete_connected_node_does_not_panic_in_refresh_viewer() {
    let ctx = egui::Context::default();
    let image_data = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let tex = rgba_to_texture(&ctx, &image_data);
    let mut snarl: Snarl<Node> = Snarl::new();
    let image = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("test", tex));
    let viewer = snarl.insert_node(egui::pos2(200.0, 0.0), Node::viewer());

    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: image,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: viewer,
            input: 0,
        }).id,
    );

    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
    assert_eq!(snarl[viewer].layers().unwrap().len(), 1);

    // Delete the connected node.
    snarl.remove_node(image);

    // refresh_viewer must not panic and must cleanly remove the dangling layer.
    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
    assert_eq!(snarl[viewer].layers().unwrap().len(), 0);
}

#[test]
fn delete_connected_route_node_does_not_panic() {
    let ctx = egui::Context::default();
    let image_data = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let tex = rgba_to_texture(&ctx, &image_data);
    let mut snarl: Snarl<Node> = Snarl::new();
    let image = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("test", tex));
    let route = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Route));
    let viewer = snarl.insert_node(egui::pos2(400.0, 0.0), Node::viewer());

    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: image,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: route,
            input: 0,
        }).id,
    );
    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: route,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: viewer,
            input: 0,
        }).id,
    );

    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
    assert_eq!(snarl[viewer].layers().unwrap().len(), 1);

    // Delete the intermediate Route node.
    snarl.remove_node(route);

    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
    assert_eq!(snarl[viewer].layers().unwrap().len(), 0);
}

#[test]
fn delete_connected_node_does_not_panic_in_eval() {
    let ctx = egui::Context::default();
    let image_data = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let tex = rgba_to_texture(&ctx, &image_data);
    let mut snarl: Snarl<Node> = Snarl::new();
    let base = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("base", tex.clone()));
    let overlay = snarl.insert_node(egui::pos2(0.0, 100.0), image_node("overlay", tex));
    let blend = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Blend));

    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: base,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: blend,
            input: 1,
        }).id,
    );
    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: overlay,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: blend,
            input: 0,
        }).id,
    );

    refresh_viewer(&mut snarl, blend, &ctx, &mut AssetStore::default());
    crate::eval::ensure_all(&mut snarl, &ctx);

    // Delete one of the connected inputs.
    snarl.remove_node(overlay);

    // Neither evaluation nor refresh_viewer should panic with "invalid key".
    crate::eval::ensure_all(&mut snarl, &ctx);
    refresh_viewer(&mut snarl, blend, &ctx, &mut AssetStore::default());
    assert_eq!(snarl[blend].layers().unwrap().len(), 1);
}

#[test]
fn delete_selected_invalidates_downstream_and_avoids_panic() {
    let ctx = egui::Context::default();
    let image_data = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
    let tex = rgba_to_texture(&ctx, &image_data);
    let mut snarl: Snarl<Node> = Snarl::new();
    let image = snarl.insert_node(egui::pos2(0.0, 0.0), image_node("test", tex));
    let filter = snarl.insert_node(egui::pos2(200.0, 0.0), Node::new_filter(FilterKind::Hsv));
    let viewer = snarl.insert_node(egui::pos2(400.0, 0.0), Node::viewer());

    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: image,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: filter,
            input: 0,
        }).id,
    );
    snarl.connect(
        snarl.out_pin(egui_snarl::OutPinId {
            node: filter,
            output: 0,
        }).id,
        snarl.in_pin(egui_snarl::InPinId {
            node: viewer,
            input: 0,
        }).id,
    );

    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
    crate::eval::ensure_all(&mut snarl, &ctx);

    let mut editor = test_editor();
    editor.selected = vec![image];
    editor.delete_selected(&mut snarl);

    assert!(snarl.get_node(image).is_none());
    assert!(editor.selected.is_empty());

    // Both eval and refresh_viewer succeed cleanly.
    crate::eval::ensure_all(&mut snarl, &ctx);
    refresh_viewer(&mut snarl, viewer, &ctx, &mut AssetStore::default());
}
