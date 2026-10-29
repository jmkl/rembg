//! Application shell: window, menus, shortcuts, autosave and session state.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::{App, CreationContext};
use egui::Color32;
use egui_snarl::{NodeId, Snarl, ui::SnarlWidget};
use gpaste::protocol::Message;

use crate::assets::{self, AssetStore, FetchedImage};
use crate::cache::{OutputCache, output_cache_key};
use crate::editor::GraphEditor;
use crate::gpaste_app::GPasTeApp;
use crate::graph::{ensure_image_loaded, refresh_viewer, rgba_to_texture, source_name};
use crate::graph_menu;
use crate::image_canvas::ImageCanvas;
use crate::model::{Kind, Node};
use crate::session;
use crate::settings::{self, Settings};
use crate::settings_window::{self, SettingsAction};
use crate::style::controls::{Button, sidebar_frame};
use crate::style::{install_theme, pal, snarl_style};
use crate::title_bar::{self, Actions};
use crate::worker::{InferenceConfig, WorkerRequest, WorkerResponse, spawn_worker};
use image::ImageEncoder;
use std::sync::mpsc;

pub(crate) struct Notice {
    text: String,
    error: bool,
    until: Instant,
}

pub struct CompNodeApp {
    pub(crate) snarl: Snarl<Node>,
    pub(crate) style: egui_snarl::ui::SnarlStyle,
    pub(crate) editor: GraphEditor,
    pub(crate) inflight: Receiver<WorkerResponse>,
    pub(crate) cache: OutputCache,
    pub(crate) session_path: Option<PathBuf>,
    pub(crate) notice: Option<Notice>,
    pub(crate) autosave_path: PathBuf,
    pub(crate) last_autosave: Option<session::SessionFile>,
    pub(crate) last_autosave_graph: Option<session::SessionFile>,
    pub(crate) last_autosave_asset_revision: u64,
    pub(crate) autosave_due: Option<Instant>,
    pub(crate) node_filter: String,
    pub(crate) recent_node_kinds: Vec<Kind>,
    pub(crate) settings: Settings,
    pub(crate) settings_draft: Settings,
    pub(crate) settings_open: bool,
    pub(crate) worker: Sender<WorkerRequest>,
    pub(crate) url_tx: Sender<FetchedImage>,
    pub(crate) url_rx: Receiver<FetchedImage>,
    pub(crate) paste_pending: bool,
    pub(crate) ctrl_v_was_down: bool,
    #[allow(dead_code)]
    pub(crate) sidebar_width: Option<f32>,
    #[allow(dead_code)]
    pub(crate) sidebar_maximized: bool,
    pub(crate) images_need_load: bool,
    pub(crate) url_open: bool,
    pub(crate) url_text: String,
    pub(crate) input_folder_open: bool,
    pub(crate) image_canvas: ImageCanvas,
    #[allow(dead_code)]
    pub(crate) sidebar_restore: bool,
}
impl CompNodeApp {
    pub(crate) fn new(
        cx: &CreationContext,
        tx: crossbeam_channel::Sender<Message>,
        rx: crossbeam_channel::Receiver<Message>,
    ) -> Self {
        install_theme(&cx.egui_ctx);
        let settings = Settings::load();
        if let Err(err) = settings.ensure_workspace() {
            eprintln!("compnode: {err}");
        }
        let cache_dir = settings.workspace_root().join("cache");
        let cache = OutputCache::new(cache_dir);
        let config = InferenceConfig {
            cache_models: settings.cache_models,
            model_dir: settings.model_dir(),
            cache_dir: settings.cache_dir(),
        };
        let (worker, inflight) = spawn_worker(cx.egui_ctx.clone(), cache.clone(), config);
        let (url_tx, url_rx) = mpsc::channel::<FetchedImage>();
        let autosave_path = settings.autosave_path();
        let gpaste_app = GPasTeApp::new(tx, rx);
        let mut app = Self {
            snarl: Snarl::new(),
            style: snarl_style(),
            editor: GraphEditor::new(worker.clone()),
            inflight,
            cache,
            session_path: None,
            notice: None,
            autosave_path: autosave_path.clone(),
            last_autosave: None,
            last_autosave_graph: None,
            last_autosave_asset_revision: 0,
            autosave_due: None,
            node_filter: String::new(),
            recent_node_kinds: Vec::new(),
            settings_draft: settings.clone(),
            settings_open: false,
            settings,
            worker,
            url_tx,
            url_rx,
            url_open: false,
            url_text: String::new(),
            input_folder_open: false,
            paste_pending: false,
            ctrl_v_was_down: false,
            sidebar_width: None,
            sidebar_maximized: false,
            images_need_load: true,
            image_canvas: ImageCanvas::new(gpaste_app),
            sidebar_restore: false,
        };
        app.image_canvas
            .set_canvas_size(app.settings.canvas_width, app.settings.canvas_height);

        if autosave_path.exists() {
            match session::load(
                &autosave_path,
                &app.cache,
                &cx.egui_ctx,
                &mut app.editor.assets,
            ) {
                Ok(snarl) => {
                    app.snarl = snarl;
                    // The autosave is a recovery file, not the user's document:
                    // keep Save/Save As pointed at an explicit path.
                    app.session_path = None;
                    app.set_notice("Restored autosave", false);
                }
                Err(err) => eprintln!("compnode: could not restore autosave: {err}"),
            }
        } else {
            crate::graph::ensure_output_node(&mut app.snarl);
        }

        app.last_autosave = Some(app.snapshot());
        app.last_autosave_graph = Some(session::snapshot(&app.snarl, &AssetStore::default()));
        app.last_autosave_asset_revision = app.editor.assets.revision();
        app
    }

    /// Collect finished inference jobs. Stale results (node deleted or
    /// superseded by a newer run) are dropped by [`Node::finish_job`].
    fn poll_jobs(&mut self, ctx: &egui::Context) {
        while let Ok(response) = self.inflight.try_recv() {
            let done = response.into_job_done();
            if !self.snarl.node_ids().any(|(id, _)| id == done.node) {
                continue;
            }
            let is_blend = self.snarl[done.node].is_blend();
            let node = &mut self.snarl[done.node];
            match done.result {
                Ok(output) => {
                    // Blend node uses full-resolution texture so its canvas preview
                    // does not drop back down to the 256px thumbnail.
                    let texture = if is_blend {
                        crate::eval::upload(ctx, done.node, &output.image)
                    } else {
                        rgba_to_texture(ctx, &output.image)
                    };
                    node.finish_job(done.token, Ok((texture, output)));
                }
                Err(message) => node.finish_job(done.token, Err(message)),
            }
            self.editor.touch_image(done.node);
            self.autosave_due = Some(Instant::now() + Duration::from_millis(700));
        }
    }

    /// Debounced scheduler for full-resolution filter renders. A render is
    /// submitted once the node's eval signature has been stable for a short
    /// time, so dragging sliders/transforms does not spam background jobs.
    fn poll_filter_renders(&mut self, ctx: &egui::Context) {
        const FILTER_RENDER_DEBOUNCE: Duration = Duration::from_millis(600);
        let selected: Vec<NodeId> = self.editor.selected_nodes().to_vec();
        let needed = filters_reaching_output(&self.snarl, &selected);
        let filter_ids: Vec<NodeId> = self
            .snarl
            .node_ids()
            .filter(|(id, node)| node.can_render_full_res() && needed.contains(id))
            .map(|(id, _)| id)
            .collect();

        for id in filter_ids {
            let (job, render_state, signature, rendered, ready) = {
                let node = &self.snarl[id];
                let sig = match node {
                    crate::model::Node::Filter(f) => f.signature,
                    crate::model::Node::Group(g) | crate::model::Node::Blend(g) => g.signature,
                    crate::model::Node::Texts(t) => t.signature,
                    _ => None,
                };
                (
                    node.job(),
                    node.render_state(),
                    sig,
                    node.rendered(),
                    node.cpu().is_some() && node.error().is_none(),
                )
            };
            if job.is_some() || !ready {
                continue;
            }
            let Some(signature) = signature else {
                // No eval output yet (missing inputs or error): reset debounce.
                self.snarl[id].set_render_state(None);
                continue;
            };
            if rendered == Some(signature) {
                continue;
            }
            match render_state {
                Some(state) if state.signature == signature => {
                    if state.since.elapsed() >= FILTER_RENDER_DEBOUNCE {
                        // On failure (e.g. an upstream filter has not rendered
                        // yet) reset the debounce so we retry after another
                        // window.
                        if !self
                            .editor
                            .start_filter_render(&mut self.snarl, id, signature, ctx)
                        {
                            self.snarl[id].set_render_state(None);
                        }
                    }
                }
                _ => {
                    self.snarl[id].set_render_state(Some(crate::model::RenderState {
                        signature,
                        since: Instant::now(),
                    }));
                }
            }
        }
    }

    fn poll_urls(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.url_rx.try_recv() {
            match result.result {
                Ok(bytes) => {
                    let target = result
                        .target
                        .filter(|id| self.snarl.get_node_info(*id).is_some());
                    let name = assets::url_name(&result.url);
                    let asset_id = assets::save_to_input(&name, &bytes)
                        .ok()
                        .and_then(|path| self.editor.assets.insert_path(path).ok())
                        .or_else(|| {
                            self.editor
                                .assets
                                .insert_bytes(name.clone(), bytes.clone(), None)
                                .ok()
                        });
                    if let Some(asset_id) = asset_id {
                        if let Some(target) = target {
                            self.replace_image_asset(target, &asset_id);
                        } else {
                            self.add_image_asset(ctx, &asset_id, result.pos);
                        }
                        self.set_notice("Image URL loaded", false);
                    } else {
                        self.set_notice("Could not store downloaded image", true);
                    }
                    self.images_need_load = true;
                }
                Err(err) => self.set_notice(err, true),
            }
            ctx.request_repaint();
        }
    }

    /// Make sure every output currently in memory has a result file and an
    /// absolute path. The worker normally does this, but this covers results
    /// produced before path tracking existed, or a failed cache write.
    fn cache_pending_outputs(&mut self) {
        let ids: Vec<NodeId> = self
            .snarl
            .node_ids()
            .filter(|(_, node)| {
                node.cpu().is_some()
                    && node.output_path().is_none_or(|path| !path.exists())
                    && matches!(node.kind(), Kind::RemoveBg | Kind::FaceRestore)
            })
            .map(|(id, _)| id)
            .collect();

        for id in ids {
            let image = self.snarl[id].cpu().cloned();
            let Some(image) = image else { continue };
            let key = output_cache_key(&image);
            let suffix = match self.snarl[id].kind() {
                Kind::RemoveBg => "remove-bg",
                Kind::FaceRestore => "face-restore",
                _ => "result",
            };
            let name = source_name(&self.snarl, id);
            match self.cache.store_named(&key, &name, suffix, &image) {
                Ok(path) => self.snarl[id].set_output_path(Some(path)),
                Err(err) => eprintln!("compnode: {err}"),
            }
        }
    }

    /// Write the current graph as a single JSON file. The snapshot is returned
    /// so the autosave path can remember exactly what it wrote.
    fn write_session(&mut self, path: &Path) -> Result<session::SessionFile, String> {
        // Canvas transforms are normally flushed on an idle timer. A save (or
        // autosave/quit) can happen before that timer fires, so always copy
        // the current canvas state first: the snapshot must match the screen.
        self.commit_canvas_layers();
        self.cache_pending_outputs();
        self.editor.assets.retain_used(&self.snarl);
        let snapshot = self.snapshot();
        session::write(path, &snapshot)?;
        Ok(snapshot)
    }

    fn snapshot(&self) -> session::SessionFile {
        session::snapshot(&self.snarl, &self.editor.assets)
    }

    pub(crate) fn save_session(&mut self, path: &Path) -> Result<(), String> {
        self.write_session(path)?;
        self.session_path = Some(path.to_path_buf());
        self.set_notice(
            format!(
                "Saved {}",
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            ),
            false,
        );
        Ok(())
    }

    /// Persist the recovery snapshot. This intentionally does not touch
    /// `session_path`, so pressing Save still targets the user's chosen
    /// session (or opens Save As when there is none).
    fn autosave_now(&mut self) {
        let path = self.autosave_path.clone();
        match self.write_session(&path) {
            Ok(snapshot) => {
                self.last_autosave = Some(snapshot);
                self.last_autosave_graph =
                    Some(session::snapshot(&self.snarl, &AssetStore::default()));
                self.last_autosave_asset_revision = self.editor.assets.revision();
                self.autosave_due = None;
            }
            Err(err) => {
                eprintln!("compnode: autosave failed: {err}");
                // Do not retry every frame; the next edit will schedule another
                // attempt, and the title bar status reports the failure.
                self.last_autosave = Some(self.snapshot());
                self.last_autosave_graph =
                    Some(session::snapshot(&self.snarl, &AssetStore::default()));
                self.last_autosave_asset_revision = self.editor.assets.revision();
                self.autosave_due = None;
                self.set_notice("Autosave failed", true);
            }
        }
    }

    fn autosave_tick(&mut self, ctx: &egui::Context) {
        let has_edit_activity = ctx.input(|input| {
            input.pointer.any_pressed()
                || input.pointer.any_released()
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
        });
        if self.autosave_due.is_none() && !has_edit_activity && self.last_autosave_graph.is_some() {
            return;
        }

        let graph_snapshot = session::snapshot(&self.snarl, &AssetStore::default());
        let asset_revision = self.editor.assets.revision();
        if self.last_autosave_graph.as_ref() == Some(&graph_snapshot)
            && self.last_autosave_asset_revision == asset_revision
        {
            self.autosave_due = None;
            return;
        }

        let due = *self
            .autosave_due
            .get_or_insert_with(|| Instant::now() + Duration::from_millis(700));
        let now = Instant::now();
        if now >= due {
            self.autosave_now();
        } else {
            ctx.request_repaint_after(due.saturating_duration_since(now));
        }
    }

    fn save_session_dialog(&mut self, save_as: bool) {
        let existing = if save_as {
            None
        } else {
            self.session_path.clone()
        };
        let path = match existing {
            Some(path) => path,
            None => {
                let directory = self
                    .session_path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .filter(|path| path.is_dir())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| settings::dialog_directory(&self.settings));
                let base_name = self
                    .session_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_os_string())
                    .unwrap_or_else(|| "session.rembg.json".into());
                let suggested = session::unique_session_path(&directory.join(base_name));
                let default_name = suggested
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "session.rembg.json".to_owned());

                match rfd::FileDialog::new()
                    .add_filter("rembg session", &["json"])
                    .set_directory(directory)
                    .set_file_name(default_name)
                    .save_file()
                {
                    Some(path) => session::unique_session_path(&path),
                    None => return,
                }
            }
        };

        if let Err(err) = self.save_session(&path) {
            self.set_notice(err, true);
        }
    }

    fn open_session_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("rembg session", &["json"])
            .set_directory(settings::dialog_directory(&self.settings))
            .pick_file()
        else {
            return;
        };

        if let Err(err) = self.load_session(&path, ctx) {
            self.set_notice(err, true);
        }
    }

    pub(crate) fn load_session(&mut self, path: &Path, ctx: &egui::Context) -> Result<(), String> {
        let mut assets = AssetStore::default();
        let snarl = session::load(path, &self.cache, ctx, &mut assets)?;
        self.editor.assets = assets;
        self.editor.clear_history();
        self.images_need_load = true;
        self.snarl = snarl;
        // Drop canvas items left over from the previous document so they can't
        // shadow the restored layer transforms.
        self.image_canvas.reset_graph();
        self.editor.reset_view();
        self.session_path = Some(path.to_path_buf());
        self.set_notice(
            format!(
                "Opened {}",
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            ),
            false,
        );
        ctx.request_repaint();
        Ok(())
    }

    pub(crate) fn add_image_asset(
        &mut self,
        ctx: &egui::Context,
        asset_id: &str,
        graph_pos: egui::Pos2,
    ) {
        let before = self.editor.graph_snapshot(&self.snarl);
        let name = self
            .editor
            .assets
            .name(asset_id)
            .unwrap_or("image.png")
            .to_owned();
        let path = self
            .editor
            .assets
            .path(asset_id)
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let id = self.snarl.insert_node(
            graph_pos,
            Node::Image(crate::model::ImageNode::from_asset(
                asset_id.to_owned(),
                name,
                path,
            )),
        );
        ensure_image_loaded(&mut self.snarl, id, ctx, &mut self.editor.assets);
        self.editor.record_graph_change(before, &self.snarl);
        self.images_need_load = true;
        self.set_notice("Image added", false);
    }

    pub(crate) fn replace_image_asset(&mut self, node: egui_snarl::NodeId, asset_id: &str) {
        if self.snarl.get_node_info(node).is_none() {
            return;
        }
        let before = self.editor.graph_snapshot(&self.snarl);
        // Replacing the source disconnects any upstream pass-through so the
        // new asset is what the node outputs.
        self.snarl
            .drop_inputs(egui_snarl::InPinId { node, input: 0 });
        if let Some(image) = self.snarl[node].image_node_mut() {
            image.asset_id = Some(asset_id.to_owned());
            image.name = self
                .editor
                .assets
                .name(asset_id)
                .unwrap_or("image.png")
                .to_owned();
            image.path = self
                .editor
                .assets
                .path(asset_id)
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            image.preview.clear();
        }
        crate::graph::invalidate_node(&mut self.snarl, node);
        self.editor.assets.retain_used(&self.snarl);
        self.editor.record_graph_change(before, &self.snarl);
        self.set_notice("Image replaced", false);
    }

    pub(crate) fn add_images_from_paths(
        &mut self,
        ctx: &egui::Context,
        paths: Vec<PathBuf>,
        base_pos: egui::Pos2,
    ) {
        for (index, path) in paths.into_iter().enumerate() {
            match self.editor.assets.insert_path(path) {
                Ok(asset_id) => {
                    let pos = base_pos + egui::vec2(index as f32 * 24.0, index as f32 * 24.0);
                    self.add_image_asset(ctx, &asset_id, pos);
                }
                Err(err) => self.set_notice(err, true),
            }
        }
    }

    pub(crate) fn handle_paste(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }
        if !ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::V)) {
            return;
        }
        self.paste_from_clipboard(ctx);
    }

    pub(crate) fn paste_from_clipboard(&mut self, ctx: &egui::Context) {
        let pointer = ctx
            .input(|input| input.pointer.hover_pos())
            .unwrap_or(egui::pos2(120.0, 120.0));
        let target = self
            .selected_image_node()
            .or_else(|| self.editor.node_at_screen(pointer))
            .filter(|node| {
                self.snarl.get_node_info(*node).is_some()
                    && self.snarl[*node].image_node().is_some()
            });
        let graph_pos = self.editor.to_global.inverse()
            * ctx.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .unwrap_or(egui::pos2(120.0, 120.0))
            });

        match assets::clipboard_image() {
            Ok(Some((name, image))) => {
                let mut buffer = Vec::new();
                let _ = image::codecs::png::PngEncoder::new(&mut buffer).write_image(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    image::ExtendedColorType::Rgba8,
                );
                let asset_id = assets::save_to_input(&name, &buffer)
                    .ok()
                    .and_then(|path| self.editor.assets.insert_path(path).ok())
                    .or_else(|| self.editor.assets.insert_rgba(name, &image).ok());
                if let Some(asset_id) = asset_id {
                    if let Some(target) = target {
                        self.replace_image_asset(target, &asset_id);
                    } else {
                        self.add_image_asset(ctx, &asset_id, graph_pos);
                    }
                } else {
                    self.set_notice("Could not store clipboard image", true);
                }
                self.images_need_load = true;
            }
            Ok(None) => {
                if let Some(text) = assets::clipboard_text() {
                    if assets::looks_like_url(&text) {
                        assets::fetch_url(
                            ctx.clone(),
                            self.url_tx.clone(),
                            text.trim().to_owned(),
                            target,
                            graph_pos,
                        );
                    } else {
                        self.set_notice("Clipboard has no image or URL", true);
                    }
                } else {
                    self.set_notice("Clipboard has no image or URL", true);
                }
            }
            Err(err) => self.set_notice(err, true),
        }
    }

    pub(crate) fn handle_drop(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        ctx.input_mut(|input| input.raw.dropped_files.clear());
        let screen_pos = ctx
            .input(|input| input.pointer.hover_pos())
            .unwrap_or(egui::pos2(120.0, 120.0));
        let graph_pos = self.editor.to_global.inverse() * screen_pos;
        let target = self.editor.node_at_screen(screen_pos);

        let mut files = Vec::new();
        for file in dropped {
            let path = file.path();
            if !path.as_os_str().is_empty() {
                files.push(path.to_path_buf());
            } else if let Ok(bytes) = file.bytes()
                && !bytes.is_empty()
            {
                let asset_id = assets::save_to_input("dropped-image.png", &bytes)
                    .ok()
                    .and_then(|path| self.editor.assets.insert_path(path).ok())
                    .or_else(|| {
                        self.editor
                            .assets
                            .insert_bytes("dropped-image.png", bytes.clone(), None)
                            .ok()
                    });
                if let Some(asset_id) = asset_id {
                    if files.is_empty() {
                        if let Some(target) = target {
                            self.replace_image_asset(target, &asset_id);
                        } else {
                            self.add_image_asset(ctx, &asset_id, graph_pos);
                        }
                    } else {
                        self.add_image_asset(ctx, &asset_id, graph_pos);
                    }
                } else {
                    self.set_notice("Could not store dropped image", true);
                }
            }
        }

        if !files.is_empty() {
            if let Some(target) = target {
                if let Some((first, rest)) = files.split_first() {
                    match self.editor.assets.insert_path(first) {
                        Ok(asset_id) => self.replace_image_asset(target, &asset_id),
                        Err(err) => self.set_notice(err, true),
                    }
                    self.add_images_from_paths(ctx, rest.to_vec(), graph_pos);
                }
            } else {
                self.add_images_from_paths(ctx, files, graph_pos);
            }
        }

        self.images_need_load = true;
    }

    fn show_asset_dialogs(&mut self, ctx: &egui::Context) {
        if self.url_open {
            let mut open = self.url_open;
            egui::Window::new("Add image from URL")
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.url_text)
                            .desired_width(360.0)
                            .hint_text("https://…"),
                    );
                    if ui.add(Button::new("Add").small()).clicked()
                        && !self.url_text.trim().is_empty()
                    {
                        let url = self.url_text.trim().to_owned();
                        let target = self.selected_image_node();
                        let pos = egui::pos2(100.0, 100.0);
                        assets::fetch_url(ctx.clone(), self.url_tx.clone(), url, target, pos);
                        self.url_text.clear();
                        self.url_open = false;
                    }
                });
            self.url_open = open;
        }

        if self.input_folder_open {
            let mut open = self.input_folder_open;
            egui::Window::new("Input folder")
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Click an image to insert it.");
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for path in assets::list_input_images() {
                                let name = path
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                if ui.add(Button::new(name).small().full_width()).clicked() {
                                    let pos = egui::pos2(100.0, 100.0);
                                    match self.editor.assets.insert_path(&path) {
                                        Ok(asset_id) => self.add_image_asset(ctx, &asset_id, pos),
                                        Err(err) => self.set_notice(err, true),
                                    }
                                }
                            }
                        });
                });
            self.input_folder_open = open;
        }
    }

    fn selected_image_node(&self) -> Option<egui_snarl::NodeId> {
        let selected = self.editor.selected_nodes();
        if selected.len() == 1 {
            let node = selected[0];
            if self.snarl.get_node_info(node).is_some() && self.snarl[node].image_node().is_some() {
                return Some(node);
            }
        }
        None
    }

    /// Start a fresh, unsaved graph.
    fn clear(&mut self) {
        self.snarl = Snarl::default();
        self.editor.clear_history();
        crate::graph::ensure_output_node(&mut self.snarl);
        self.image_canvas.reset_graph();
        self.editor.reset_view();
        self.session_path = None;
    }

    fn set_notice(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
            until: Instant::now() + Duration::from_secs(6),
        });
    }

    fn inference_config(&self) -> InferenceConfig {
        InferenceConfig {
            cache_models: self.settings.cache_models,
            model_dir: self.settings.model_dir(),
            cache_dir: self.settings.cache_dir(),
        }
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        let next = self.settings_draft.clone();
        if let Err(err) = next.ensure_workspace() {
            self.set_notice(err, true);
            return;
        }
        if let Err(err) = next.save() {
            self.set_notice(err, true);
            return;
        }

        self.settings = next;
        self.cache = OutputCache::new(self.settings.cache_dir());
        self.autosave_path = self.settings.autosave_path();
        self.settings_open = false;
        let config = self.inference_config();
        if self.worker.send(WorkerRequest::Configure(config)).is_err() {
            self.set_notice(
                "Settings saved, but the inference worker is not running",
                true,
            );
        } else {
            self.set_notice("Settings saved", false);
        }
        ctx.request_repaint();
    }

    fn status_text(&self, busy: usize, active_job: Option<&str>) -> (String, Color32) {
        let (mut text, color) = if let Some(notice) = &self.notice {
            (
                notice.text.clone(),
                if notice.error {
                    pal::DANGER
                } else {
                    pal::SOCKET
                },
            )
        } else if busy > 0 {
            (
                format!("{} running ({busy})", active_job.unwrap_or("Worker job")),
                pal::SOCKET,
            )
        } else if self.autosave_due.is_some() {
            ("Autosaving…".to_owned(), pal::ACCENT)
        } else {
            let name = self
                .session_path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_owned());
            (format!("{name} · autosaved"), pal::SUCCESS)
        };
        if text.chars().count() > 46 {
            text = format!("{}…", text.chars().take(45).collect::<String>());
        }
        (text, color)
    }

    /// Apply menu/title-bar actions. Returns `true` when the app should quit.
    fn apply_actions(
        &mut self,
        ctx: &egui::Context,
        actions: &Actions,
        maximized: bool,
        fullscreen: bool,
    ) -> bool {
        if actions.open {
            self.open_session_dialog(ctx);
        }
        if actions.add_images
            && let Some(paths) = crate::nodes::image::pick_images()
        {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            let pos = egui::pos2(80.0, 80.0);
            self.add_images_from_paths(ctx, paths, pos);
        }
        if actions.add_url {
            self.url_open = true;
            self.url_text.clear();
        }
        if actions.insert_input {
            self.input_folder_open = true;
        }
        if actions.paste_clipboard {
            self.paste_from_clipboard(ctx);
        }
        if actions.save_as {
            self.save_session_dialog(true);
        } else if actions.save {
            self.save_session_dialog(false);
        }
        if actions.new_graph {
            self.clear();
            self.set_notice("New graph", false);
        }
        if actions.duplicate_selected {
            self.editor.duplicate_selected(&mut self.snarl);
        }
        if actions.delete_selected {
            self.editor.delete_selected(&mut self.snarl);
        }
        if actions.undo {
            self.editor.undo(&mut self.snarl);
        }
        if actions.redo {
            self.editor.redo(&mut self.snarl);
        }
        if actions.settings {
            self.settings_draft = self.settings.clone();
            self.settings_open = true;
        }
        if actions.toggle_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        if actions.minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if actions.toggle_maximize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        if actions.about {
            self.set_notice("Right-click the canvas to add a node · Ctrl+S saves", false);
        }
        if actions.quit {
            self.autosave_now();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return true;
        }
        false
    }
    fn show_sidebar_contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // --- Image canvas: sync with viewer node layers ---
        // Sync canvas size so both previews use the same logical size.
        for (_, node) in self.snarl.nodes_ids_mut() {
            if let crate::model::Node::Output(viewer) = node {
                let size = self.image_canvas.canvas_size();
                if viewer.canvas_size != size {
                    viewer.canvas_size = size;
                }
            }
            if node.kind().is_layer_kind() {
                let size = self.image_canvas.canvas_size();
                if node.canvas_size() != size {
                    node.set_canvas_size(size);
                }
            }
        }

        // Only rebuild/sync layers when they changed (or a
        // decode finished). Signature is allocation-free.
        let selected_layers = self.editor.selected_nodes().first().copied().filter(|id| {
            self.snarl.get_node_info(*id).is_some() && self.snarl[*id].kind().is_layer_kind()
        });
        let layer_iter_sig = if let Some(gid) = selected_layers {
            // only this node's layers
            self.snarl[gid]
                .layers()
                .into_iter()
                .flatten()
                .map(|l| {
                    (
                        l.source,
                        l.source_path.as_deref(),
                        l.texture.as_ref().map(|t| t.id()),
                    )
                })
                .collect::<Vec<_>>()
        } else {
            self.snarl
                .node_ids()
                .filter_map(|(_, node)| match node {
                    crate::model::Node::Output(o) => Some(o),
                    _ => None,
                })
                .flat_map(|o| o.layers.iter())
                .map(|l| {
                    (
                        l.source,
                        l.source_path.as_deref(),
                        l.texture.as_ref().map(|t| t.id()),
                    )
                })
                .collect::<Vec<_>>()
        };
        let sig = ImageCanvas::layers_signature(layer_iter_sig.into_iter());
        if self.image_canvas.begin_sync(&ctx, sig) {
            // Collect layers from the edited node (bottom-to-top).
            let layers: Vec<(
                egui_snarl::NodeId,
                Kind,
                String,
                Option<egui::TextureHandle>,
                Option<PathBuf>,
                (egui::Vec2, egui::Vec2, f32),
                bool,
                Option<(u32, u32)>,
                Option<image::RgbaImage>,
            )> = if let Some(gid) = selected_layers {
                self.snarl[gid]
                    .layers()
                    .map(|ls| {
                        ls.iter()
                            .map(|l| {
                                (
                                    l.source,
                                    l.kind,
                                    l.source_name.clone(),
                                    l.texture.clone(),
                                    l.source_path.clone(),
                                    (l.center, l.size, l.angle),
                                    layer_uses_preview(&self.snarl, l.source),
                                    crate::graph::effective_dimensions(
                                        &self.snarl,
                                        l.source,
                                        &self.editor.assets,
                                    ),
                                    self.snarl.get_node(l.source).and_then(|n| n.cpu()).cloned(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                self.snarl
                    .node_ids()
                    .filter_map(|(_, node)| match node {
                        crate::model::Node::Output(o) => Some(o),
                        _ => None,
                    })
                    .flat_map(|o| o.layers.iter())
                    .map(|l| {
                        (
                            l.source,
                            l.kind,
                            l.source_name.clone(),
                            l.texture.clone(),
                            l.source_path.clone(),
                            (l.center, l.size, l.angle),
                            layer_uses_preview(&self.snarl, l.source),
                            crate::graph::effective_dimensions(
                                &self.snarl,
                                l.source,
                                &self.editor.assets,
                            ),
                            self.snarl.get_node(l.source).and_then(|n| n.cpu()).cloned(),
                        )
                    })
                    .collect()
            };
            self.image_canvas.sync_layers(&ctx, &layers);
        }

        if self.image_canvas.has_uncommitted_transform() {
            self.commit_canvas_layers();
        }

        // Write transforms/order back to the viewer layers once,
        // after the canvas has been idle for 3s (not every frame).
        if self.image_canvas.take_stable_commit(0.3) {
            self.commit_canvas_layers();
            // A canvas transform is session state even when no node changed.
            // `autosave_tick` only re-checks the graph after an edit, and this
            // commit happens with no input activity, so schedule the save here.
            self.autosave_due
                .get_or_insert_with(|| Instant::now() + Duration::from_millis(700));
        } else if self.image_canvas.has_uncommitted_transform() {
            // The idle commit needs a frame after the timeout. Request one so
            // it (and the autosave it schedules) is not delayed until the next
            // mouse move.
            ctx.request_repaint_after(Duration::from_millis(350));
        }

        let blend_preview = if let Some(gid) = selected_layers {
            if self.snarl[gid].is_blend() {
                self.snarl[gid].texture().cloned()
            } else {
                None
            }
        } else {
            None
        };
        self.image_canvas.set_blend_preview(blend_preview);

        // Render the image canvas widget in the sidebar.
        self.image_canvas.show(ui, &self.settings);
    }

    /// Copy the canvas item transforms/order into the edited Group or Output
    /// node. Also called before saving so a session never captures a stale
    /// transform that is still waiting for the idle timer.
    fn commit_canvas_layers(&mut self) {
        let canvas_transforms: std::collections::HashMap<
            egui_snarl::NodeId,
            (egui::Vec2, egui::Vec2, f32),
        > = self
            .image_canvas
            .layers()
            .map(|l| (l.id, (l.center, l.size, l.angle)))
            .collect();
        let canvas_layer_order: Vec<egui_snarl::NodeId> =
            self.image_canvas.layers().map(|l| l.id).collect();

        let edited_id = self.editor.selected_nodes().first().copied().filter(|id| {
            self.snarl.get_node_info(*id).is_some() && self.snarl[*id].kind().is_layer_kind()
        });
        // fallback: single output node
        let target_id = edited_id.or_else(|| {
            self.snarl
                .node_ids()
                .find(|(_, n)| n.viewer_node().is_some())
                .map(|(id, _)| id)
        });

        if let Some(tid) = target_id {
            let before = self.snarl[tid].clone();
            let old_state = before.layers().map(|layers| {
                layers
                    .iter()
                    .map(|l| (l.source, l.center, l.size, l.angle))
                    .collect::<Vec<_>>()
            });
            if let Some(layers) = self.snarl[tid].layers_mut() {
                for layer in layers.iter_mut() {
                    if let Some((center, size, angle)) = canvas_transforms.get(&layer.source) {
                        layer.center = *center;
                        layer.size = *size;
                        layer.angle = *angle;
                    }
                }
                layers.sort_by_key(|l| {
                    canvas_layer_order
                        .iter()
                        .position(|id| *id == l.source)
                        .unwrap_or(usize::MAX)
                });
            }
            let new_state = self.snarl[tid].layers().map(|layers| {
                layers
                    .iter()
                    .map(|l| (l.source, l.center, l.size, l.angle))
                    .collect::<Vec<_>>()
            });
            if old_state != new_state {
                self.editor.record_node_change(tid, before, &self.snarl);
            }
        }
    }
}

impl App for CompNodeApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(target_os = "windows")]
        {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn GetAsyncKeyState(v_key: i32) -> i16;
            }

            if raw_input.focused {
                let ctrl_down = unsafe { GetAsyncKeyState(0x11) } & 0x8000u16 as i16 != 0;
                let v_down = unsafe { GetAsyncKeyState(0x56) } & 0x8000u16 as i16 != 0;
                let ctrl_v_down = ctrl_down && v_down;
                if ctrl_v_down && !self.ctrl_v_was_down {
                    self.paste_pending = true;
                }
                self.ctrl_v_was_down = ctrl_v_down;
            } else {
                self.ctrl_v_was_down = false;
            }
        }
        let _ = raw_input;
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.image_canvas.gpaste_app().poll_messages(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let has_edit_activity = ctx.input(|input| {
            input.pointer.any_pressed()
                || input.pointer.any_released()
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
        });

        self.poll_jobs(&ctx);
        self.poll_urls(&ctx);
        let has_unloaded_images = self
            .snarl
            .node_ids()
            .filter(|(_, node)| node.image_node().is_some())
            .any(|(id, _)| {
                self.snarl[id]
                    .image_node()
                    .map(|image| image.preview.texture.is_none() && image.preview.error.is_none())
                    .unwrap_or(false)
            });
        if has_edit_activity || self.autosave_due.is_some() || has_unloaded_images {
            self.editor.trim_image_memory(&mut self.snarl);
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            self.autosave_now();
        }

        if self
            .notice
            .as_ref()
            .is_some_and(|notice| Instant::now() >= notice.until)
        {
            self.notice = None;
        } else if self.notice.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        let busy_nodes: Vec<String> = self
            .snarl
            .node_ids()
            .filter(|(_, node)| node.job().is_some())
            .map(|(_, node)| crate::catalog::spec(node.kind()).title.to_owned())
            .collect();
        let busy = busy_nodes.len();
        let selected_count = self.editor.selected_nodes().len();
        let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
        let fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
        // Snarl nodes are focusable widgets, so a focused node must not suppress
        // Delete/Backspace. Only real text entry should own those keys.
        let text_edit_focused = ctx.text_edit_focused();

        let (status_text, status_dot) =
            self.status_text(busy, busy_nodes.first().map(String::as_str));
        let (mut actions, _) = title_bar::show(
            ui,
            &status_text,
            status_dot,
            selected_count,
            maximized,
            fullscreen,
        );

        ui.input(|input| {
            if input.modifiers.command {
                if input.key_pressed(egui::Key::O) {
                    actions.open = true;
                }
                if input.key_pressed(egui::Key::N) {
                    actions.new_graph = true;
                }
                if input.key_pressed(egui::Key::S) {
                    if input.modifiers.shift {
                        actions.save_as = true;
                    } else {
                        actions.save = true;
                    }
                }
                if !text_edit_focused && input.key_pressed(egui::Key::Z) {
                    if input.modifiers.shift {
                        actions.redo = true;
                    } else {
                        actions.undo = true;
                    }
                }
                if !text_edit_focused && input.key_pressed(egui::Key::Y) {
                    actions.redo = true;
                }
                if input.key_pressed(egui::Key::Comma) {
                    actions.settings = true;
                }
                if input.modifiers.command && input.key_pressed(egui::Key::V) {
                    actions.paste_clipboard = true;
                }
            }
            if input.key_pressed(egui::Key::F11) {
                actions.toggle_fullscreen = true;
            }
            let delete_pressed =
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace);
            if !input.modifiers.command
                && !text_edit_focused
                && delete_pressed
                && selected_count > 0
            {
                actions.delete_selected = true;
            }
            if !input.modifiers.command
                && !text_edit_focused
                && input.modifiers.shift
                && input.key_pressed(egui::Key::D)
                && selected_count > 0
            {
                actions.duplicate_selected = true;
            }
        });

        if self.paste_pending {
            actions.paste_clipboard = true;
            self.paste_pending = false;
        }

        if self.apply_actions(&ctx, &actions, maximized, fullscreen) {
            return;
        }

        self.handle_paste(&ctx);
        if ctx.input(|input| !input.raw.dropped_files.is_empty()) {
            self.handle_drop(&ctx);
        }

        // Load image nodes and mirror upstream outputs into viewers. Neither
        // step triggers model inference. Skip this during idle pan/zoom to avoid
        // rebuilding every frame.
        let images: Vec<NodeId> = self
            .snarl
            .node_ids()
            .filter(|(_, node)| node.image_node().is_some())
            .map(|(id, _)| id)
            .collect();
        let should_refresh_images =
            has_edit_activity || self.autosave_due.is_some() || self.images_need_load;
        if should_refresh_images {
            for id in &images {
                ensure_image_loaded(&mut self.snarl, *id, &ctx, &mut self.editor.assets);
            }
            self.images_need_load = false;
        }

        let canvas_size = egui::vec2(self.settings.canvas_width, self.settings.canvas_height);
        self.image_canvas
            .set_canvas_size(canvas_size.x, canvas_size.y);
        for (_, node) in self.snarl.nodes_ids_mut() {
            if let crate::model::Node::Output(viewer) = node {
                if viewer.canvas_size != canvas_size {
                    viewer.canvas_size = canvas_size;
                }
            }
            if node.kind().is_layer_kind() {
                if node.canvas_size() != canvas_size {
                    node.set_canvas_size(canvas_size);
                }
            }
        }

        // Evaluate filter nodes (cached by signature) before the viewer is
        // refreshed so it can pick up the new preview textures.
        let filters_changed = crate::eval::ensure_all(&mut self.snarl, &ctx);
        self.poll_filter_renders(&ctx);

        let viewers: Vec<NodeId> = self
            .snarl
            .node_ids()
            .filter(|(_, node)| node.viewer_node().is_some() || node.kind().is_layer_kind())
            .map(|(id, _)| id)
            .collect();
        if should_refresh_images
            || has_edit_activity
            || self.autosave_due.is_some()
            || filters_changed
        {
            for id in &viewers {
                refresh_viewer(&mut self.snarl, *id, &ctx, &mut self.editor.assets);
            }
        }
        let screen_w = ctx.content_rect().width();

        egui::Panel::right("root_sidebar")
            .min_size(300.0)
            .max_size(screen_w * 0.8)
            .frame(sidebar_frame())
            .show(ui, |ui| {
                self.show_sidebar_contents(ui, &ctx);
            });

        let central = egui::CentralPanel::default().show(ui, |ui| {
            let view_id = self.editor.view_id();
            self.editor.begin_cut_drag(ui.ctx(), ui.max_rect());
            SnarlWidget::new().id(view_id).style(self.style).show(
                &mut self.snarl,
                &mut self.editor,
                ui,
            )
        });
        let snarl_response = central.inner;
        let graph_background_clicked =
            self.editor
                .finish_frame(&ctx, &mut self.snarl, &snarl_response);
        if graph_background_clicked {
            // Match the graph's node deselection: clicking empty graph space
            // also dismisses the sidebar layer selection and its canvas gizmos.
            self.image_canvas.clear_selection();
        }

        // Build the graph menu ourselves instead of using Snarl's
        // `show_graph_menu`: the stock context menu uses
        // `PopupCloseBehavior::CloseOnClick`, so clicking the filter input
        // immediately dismissed it. `CloseOnClickOutside` keeps the menu open
        // for text entry while still closing it on outside clicks and Escape.
        let menu = egui::Popup::context_menu(&snarl_response)
            .id(egui::Id::new("graph_node_menu"))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                let graph_pos = self.editor.to_global.inverse() * ui.cursor().min;
                let before = self.editor.graph_snapshot(&self.snarl);
                let old_count = self.snarl.node_ids().count();
                graph_menu::show(
                    ui,
                    &mut self.snarl,
                    &mut self.node_filter,
                    &mut self.recent_node_kinds,
                    graph_pos,
                );
                if self.snarl.node_ids().count() != old_count {
                    self.editor.record_graph_change(before, &self.snarl);
                }
            });
        if menu.is_none() {
            self.node_filter.clear();
        }

        self.show_asset_dialogs(&ctx);

        self.autosave_tick(&ctx);

        if self.settings_open {
            match settings_window::show(&ctx, &mut self.settings_open, &mut self.settings_draft) {
                SettingsAction::Apply => self.apply_settings(&ctx),
                SettingsAction::Cancel => {
                    self.settings_open = false;
                    self.settings_draft = self.settings.clone();
                }
                SettingsAction::None => {}
            }
        }

        // Keep inference status and notices fresh while the worker is busy.
        if busy > 0 || self.notice.is_some() {
            ctx.request_repaint();
        }
    }
}

use crossbeam_channel::unbounded;
use gpaste::protocol;
use std::{net::TcpListener, thread};
const ADDRESS: &str = "127.0.0.1:38471";

pub fn run() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_maximized(true)
            .with_min_inner_size([640.0, 420.0])
            .with_title("CompNode")
            .with_decorations(false),
        ..Default::default()
    };
    let (to_gui_tx, to_gui_rx) = unbounded::<protocol::Message>();
    let (to_host_tx, to_host_rx) = unbounded::<protocol::Message>();
    eframe::run_native(
        "CompNode",
        native_options,
        Box::new(|cx| {
            start_bridge_server(to_gui_tx, to_host_rx, cx.egui_ctx.clone());
            Ok(Box::new(CompNodeApp::new(cx, to_host_tx, to_gui_rx)))
        }),
    )
}

/// True when a layer's node has no full-resolution output file of its own, so
/// its low-res preview texture is the only correct content (plain filters).
fn layer_uses_preview(snarl: &Snarl<Node>, source: egui_snarl::NodeId) -> bool {
    // Layers normally point at the real source, but older sessions may still
    // store a Route id; resolve it before deciding.
    let source = crate::graph::resolve_route(snarl, source);
    let Some(node) = snarl.get_node(source) else {
        return false;
    };
    matches!(
        node,
        crate::model::Node::Filter(_) | crate::model::Node::Texts(_)
    ) && node.output_path().is_none()
}

/// Node ids that can reach an Output node through input wires, plus the layers
/// of a selected Group/Blend node (the sidebar shows those instead of the
/// Output). Full-resolution renders are only scheduled for these, so orphan
/// filters never allocate large images.
fn filters_reaching_output(snarl: &Snarl<Node>, selected: &[NodeId]) -> HashSet<NodeId> {
    let mut needed = HashSet::new();
    let mut stack: Vec<NodeId> = snarl
        .node_ids()
        .filter(|(_, node)| node.viewer_node().is_some())
        .map(|(id, _)| id)
        .collect();
    stack.extend(
        selected
            .iter()
            .copied()
            .filter(|id| snarl.get_node_info(*id).is_some() && snarl[*id].kind().is_layer_kind()),
    );
    while let Some(id) = stack.pop() {
        if !needed.insert(id) {
            continue;
        }
        let Some(node) = snarl.get_node(id) else {
            continue;
        };
        let sockets = crate::catalog::spec(node.kind()).sockets.len();
        for input in 0..sockets {
            let remotes = snarl
                .in_pin(egui_snarl::InPinId { node: id, input })
                .remotes
                .clone();
            for remote in remotes {
                stack.push(remote.node);
            }
        }
    }
    needed
}
fn start_bridge_server(
    incoming_tx: crossbeam_channel::Sender<protocol::Message>,
    outgoing_rx: crossbeam_channel::Receiver<protocol::Message>,
    ctx: eframe::egui::Context,
) {
    thread::spawn(move || {
        let listener = loop {
            match TcpListener::bind(ADDRESS) {
                Ok(listener) => break listener,
                Err(error) => {
                    eprintln!("Cannot bind {ADDRESS}: {error}; retrying…");
                    thread::sleep(Duration::from_secs(2));
                }
            }
        };
        eprintln!("Bridge listening on {ADDRESS}");

        loop {
            let (stream, peer) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("TCP accept failed: {error}");
                    continue;
                }
            };
            eprintln!("Native host connected: {peer}");

            let mut writer = match stream.try_clone() {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Could not clone TCP stream: {error}");
                    continue;
                }
            };
            let read_stream = match stream.try_clone() {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Could not clone TCP stream: {error}");
                    continue;
                }
            };
            let _ = stream.set_nodelay(true); // keep abort messages low-latency
            let read_tx = incoming_tx.clone();
            let read_ctx = ctx.clone();
            let reader_thread = thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let mut reader = BufReader::new(read_stream);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => match serde_json::from_str::<protocol::Message>(line.trim()) {
                            Ok(message) => {
                                if read_tx.send(message).is_err() {
                                    break;
                                }
                                read_ctx.request_repaint();
                            }
                            Err(error) => eprintln!("Invalid message from host: {error}"),
                        },
                        Err(error) => {
                            eprintln!("TCP read failed: {error}");
                            break;
                        }
                    }
                }
            });

            let _ = incoming_tx.send(protocol::Message::BridgeStatus {
                connected: true,
                detail: "native host".into(),
            });
            ctx.request_repaint();

            loop {
                match outgoing_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(message) => {
                        use std::io::Write;
                        let mut bytes = match serde_json::to_vec(&message) {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                eprintln!("Could not serialize message: {error}");
                                continue;
                            }
                        };
                        bytes.push(b'\n');
                        if let Err(error) = writer.write_all(&bytes) {
                            eprintln!("TCP write failed: {error}");
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if reader_thread.is_finished() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            }
            let _ = incoming_tx.send(protocol::Message::BridgeStatus {
                connected: false,
                detail: "native host disconnected".into(),
            });
            ctx.request_repaint();
        }
    });
}
