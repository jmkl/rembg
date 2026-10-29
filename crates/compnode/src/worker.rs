//! Long-lived inference worker.
//!
//! The worker can keep models resident between runs or load and release them
//! for each job, depending on the app settings. Results (including PNG
//! encoding) are written to the result cache on this thread as well.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use egui_snarl::NodeId;
use rembg_core::{
    BirefNet, FaceDetector, Gfpgan, MODEL_BIREFNET_FILE, MODEL_GFPGAN_FILE, MODEL_RMBG_FILE,
    MODEL_SCRFD_FILE, RemoveBg,
};

#[cfg(test)]
use crate::cache::MemoryOutputCache;
use crate::cache::{
    CacheError, CacheKey, OutputCache, OutputCacheInterface, cache_key, cache_key_layers,
    cache_key_refs,
};
use crate::cancellation::CancellationToken;
use crate::model::{FilterKind, JobOutput, Kind, RunTiming, ViewerLayer};
use crate::params::Params;

/// Common boundary for image-to-image inference backends. The worker owns
/// backend lifetimes; implementations only need to predict from RGBA pixels.
trait InferenceBackend {
    fn predict(&self, input: &image::RgbaImage) -> Result<image::RgbaImage, InferenceError>;
}

type InferenceError = String;
const MODEL_KEEP_ALIVE: Duration = Duration::from_secs(10 * 60);

impl InferenceBackend for BirefNet {
    fn predict(&self, input: &image::RgbaImage) -> Result<image::RgbaImage, InferenceError> {
        let image = image::DynamicImage::ImageRgba8(input.clone());
        self.process_image(&image).map_err(|err| format!("{err:#}"))
    }
}

impl InferenceBackend for RemoveBg {
    fn predict(&self, input: &image::RgbaImage) -> Result<image::RgbaImage, InferenceError> {
        let image = image::DynamicImage::ImageRgba8(input.clone());
        self.process_image(&image).map_err(|err| format!("{err:#}"))
    }
}

enum Job {
    Inference(InferenceJob),
    Render(RenderJob),
}

pub(crate) enum InferenceJob {
    RemoveBg {
        token: u64,
        node: NodeId,
        input: image::RgbaImage,
        source_name: String,
    },
    /// Remove background using the new RemoveBg library (RMBG-1.4 logic)
    RemoveBgV2 {
        token: u64,
        node: NodeId,
        input: image::RgbaImage,
        source_name: String,
    },
    FaceRestore {
        token: u64,
        node: NodeId,
        input: image::RgbaImage,
        strength: f32,
        only_center: bool,
        source_name: String,
    },
}

/// One full-resolution input to a filter render. Paths and encoded assets are
/// loaded on the worker thread so the UI never blocks on a large decode.
pub(crate) enum FilterInput {
    Path(PathBuf),
    Encoded(std::sync::Arc<Vec<u8>>),
    Pixels(image::RgbaImage),
}

impl FilterInput {
    fn load(self) -> Result<image::RgbaImage, String> {
        match self {
            Self::Path(path) => image::open(&path)
                .map(|image| image.to_rgba8())
                .map_err(|err| format!("Could not load {}: {err}", path.display())),
            Self::Encoded(bytes) => image::load_from_memory(bytes.as_slice())
                .map(|image| image.to_rgba8())
                .map_err(|err| format!("Could not decode filter input: {err}")),
            Self::Pixels(image) => Ok(image),
        }
    }
}

/// One layer of a full-resolution Group render: an on-disk source image plus
/// its canvas transform.
#[derive(Clone, Debug)]
pub(crate) struct CompositeLayer {
    pub source: NodeId,
    pub kind: Kind,
    pub path: PathBuf,
    pub center: egui::Vec2,
    pub size: egui::Vec2,
    pub angle: f32,
}

/// Settings the worker needs to load and cache models.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InferenceConfig {
    pub cache_models: bool,
    pub model_dir: PathBuf,
    pub cache_dir: PathBuf, // <- new
}

pub(crate) enum RenderJob {
    /// Full-resolution Group/Blend composite rendered from on-disk source images.
    Composite {
        token: u64,
        node: NodeId,
        layers: Vec<CompositeLayer>,
        canvas: (u32, u32),
        mode: String,
        opacity: f32,
        source_name: String,
    },
    Filter {
        token: u64,
        node: NodeId,
        kind: FilterKind,
        params: Params,
        inputs: Vec<Vec<FilterInput>>,
        canvas: (u32, u32),
        source_name: String,
    },
    Texts {
        token: u64,
        node: NodeId,
        params: Params,
        canvas: (u32, u32),
        source_name: String,
    },
}

pub(crate) struct InferenceRequest {
    pub(crate) job: InferenceJob,
    pub(crate) cancellation: CancellationToken,
}

pub(crate) struct RenderRequest {
    pub(crate) job: RenderJob,
    pub(crate) cancellation: CancellationToken,
}

pub(crate) enum WorkerRequest {
    Inference(InferenceRequest),
    Render(RenderRequest),
    Configure(InferenceConfig),
    #[cfg(test)]
    SimulateLoad {
        started: Sender<()>,
        release: Receiver<()>,
    },
}

pub(crate) struct JobDone {
    pub token: u64,
    pub node: NodeId,
    pub result: Result<JobOutput, String>,
}

pub(crate) struct InferenceResponse {
    pub done: JobDone,
}

pub(crate) struct RenderResponse {
    pub done: JobDone,
}

pub(crate) enum WorkerResponse {
    Inference(InferenceResponse),
    Render(RenderResponse),
}

impl WorkerResponse {
    pub(crate) fn into_job_done(self) -> JobDone {
        match self {
            Self::Inference(response) => response.done,
            Self::Render(response) => response.done,
        }
    }
}

/// Render a Group/Blend node at full resolution from on-disk source images and
/// persist the result in the output cache. Repeated renders with identical
/// inputs hit the disk cache instead of recompositing.
fn composite_full(
    cache: &dyn OutputCacheInterface,
    layers: &[CompositeLayer],
    canvas: (u32, u32),
    mode: &str,
    opacity: f32,
    source_name: &str,
) -> Result<JobOutput, String> {
    let started = Instant::now();
    if layers.is_empty() {
        return Err("No layers connected".to_owned());
    }
    if canvas.0 == 0 || canvas.1 == 0 {
        return Err("Invalid canvas size".to_owned());
    }

    // Serialize transforms for the cache key.
    let mut params = Vec::new();
    for layer in layers {
        params.extend_from_slice(&layer.center.x.to_le_bytes());
        params.extend_from_slice(&layer.center.y.to_le_bytes());
        params.extend_from_slice(&layer.size.x.to_le_bytes());
        params.extend_from_slice(&layer.size.y.to_le_bytes());
        params.extend_from_slice(&layer.angle.to_le_bytes());
    }
    params.extend_from_slice(&canvas.0.to_le_bytes());
    params.extend_from_slice(&canvas.1.to_le_bytes());
    params.extend_from_slice(mode.as_bytes());
    params.extend_from_slice(&opacity.to_le_bytes());

    let mut images = Vec::with_capacity(layers.len());
    for layer in layers {
        let img = image::open(&layer.path)
            .map_err(|err| format!("Could not load {}: {err}", layer.path.display()))?
            .to_rgba8();
        images.push(img);
    }

    let key = cache_key_layers("compnode-composite-v1", &images, &params);

    // Cache hit: reuse the stored composite.
    let cached_path = cache.path_for(&key);
    if let Some(image) = cache.get(&CacheKey(key.clone())) {
        return Ok(JobOutput {
            image,
            path: Some(cached_path),
            timing: RunTiming {
                total: started.elapsed(),
                load: Duration::ZERO,
                infer: Duration::ZERO,
                cache_models: false,
            },
        });
    }

    let pairs: Vec<(ViewerLayer, image::RgbaImage)> = layers
        .iter()
        .zip(images.into_iter())
        .map(|(layer, img)| {
            (
                ViewerLayer {
                    source: layer.source,
                    kind: layer.kind,
                    texture: None,
                    source_name: String::new(),
                    source_path: Some(layer.path.clone()),
                    center: layer.center,
                    size: layer.size,
                    angle: layer.angle,
                },
                img,
            )
        })
        .collect();
    let composited =
        crate::ops::composite::composite_layers_with(&pairs, canvas, None, mode, opacity)?;

    let path = match cache.store_named(&key, source_name, "composite", &composited) {
        Ok(path) => Some(path),
        Err(err) => {
            eprintln!("compnode: {err}");
            None
        }
    };
    Ok(JobOutput {
        image: composited,
        path,
        timing: RunTiming {
            total: started.elapsed(),
            load: Duration::ZERO,
            infer: Duration::ZERO,
            cache_models: false,
        },
    })
}

/// Evaluate any non-Group filter at full resolution and persist the result in
/// the output cache. The cache key hashes the resolved inputs, parameters and
/// canvas size, so an unchanged filter reuses its previous full-resolution file.
fn filter_full(
    cache: &dyn OutputCacheInterface,
    kind: FilterKind,
    params: &Params,
    inputs: Vec<Vec<FilterInput>>,
    canvas: (u32, u32),
    source_name: &str,
) -> Result<JobOutput, String> {
    let started = Instant::now();
    let mut resolved: Vec<Vec<image::RgbaImage>> = Vec::with_capacity(inputs.len());
    for socket in inputs {
        let mut images = Vec::with_capacity(socket.len());
        for input in socket {
            images.push(input.load()?);
        }
        resolved.push(images);
    }

    let mut params_bytes = serde_json::to_vec(params).unwrap_or_default();
    params_bytes.extend_from_slice(&canvas.0.to_le_bytes());
    params_bytes.extend_from_slice(&canvas.1.to_le_bytes());
    let domain = format!("compnode-filter-v1:{}", kind.id());
    let key = cache_key_refs(&domain, resolved.iter().flatten(), &params_bytes);

    // Cache hit: reuse the stored full-resolution render.
    let cached_path = cache.path_for(&key);
    if let Some(image) = cache.get(&CacheKey(key.clone())) {
        return Ok(JobOutput {
            image,
            path: Some(cached_path),
            timing: RunTiming {
                total: started.elapsed(),
                load: Duration::ZERO,
                infer: Duration::ZERO,
                cache_models: false,
            },
        });
    }

    let image = crate::ops::evaluate(kind, params, &resolved, canvas)?;
    let suffix = format!("filter-{}", kind.id());
    let path = match cache.store_named(&key, source_name, &suffix, &image) {
        Ok(path) => Some(path),
        Err(err) => {
            eprintln!("compnode: {err}");
            None
        }
    };
    Ok(JobOutput {
        image,
        path,
        timing: RunTiming {
            total: started.elapsed(),
            load: Duration::ZERO,
            infer: Duration::ZERO,
            cache_models: false,
        },
    })
}

fn texts_full(
    cache: &dyn OutputCacheInterface,
    params: &Params,
    canvas: (u32, u32),
    source_name: &str,
) -> Result<JobOutput, String> {
    let started = Instant::now();
    let mut params_bytes = serde_json::to_vec(params).unwrap_or_default();
    params_bytes.extend_from_slice(&canvas.0.to_le_bytes());
    params_bytes.extend_from_slice(&canvas.1.to_le_bytes());
    let domain = "compnode-filter-v1:texts";
    let key = cache_key_refs(
        domain,
        std::iter::empty::<&image::RgbaImage>(),
        &params_bytes,
    );

    let cached_path = cache.path_for(&key);
    if let Some(image) = cache.get(&CacheKey(key.clone())) {
        return Ok(JobOutput {
            image,
            path: Some(cached_path),
            timing: RunTiming {
                total: started.elapsed(),
                load: Duration::ZERO,
                infer: Duration::ZERO,
                cache_models: false,
            },
        });
    }

    let image = crate::ops::texts::texts(params, canvas)?;
    let path = match cache.store_named(&key, source_name, "texts", &image) {
        Ok(path) => Some(path),
        Err(err) => {
            eprintln!("compnode: {err}");
            None
        }
    };
    Ok(JobOutput {
        image,
        path,
        timing: RunTiming {
            total: started.elapsed(),
            load: Duration::ZERO,
            infer: Duration::ZERO,
            cache_models: false,
        },
    })
}

enum WorkerCache {
    Disk(OutputCache),
    #[cfg(test)]
    Memory(MemoryOutputCache),
}

impl OutputCacheInterface for WorkerCache {
    fn get(&self, key: &CacheKey) -> Option<image::RgbaImage> {
        match self {
            Self::Disk(cache) => cache.get(key),
            #[cfg(test)]
            Self::Memory(cache) => cache.get(key),
        }
    }

    fn put(&self, key: &CacheKey, image: &image::RgbaImage) -> Result<PathBuf, CacheError> {
        match self {
            Self::Disk(cache) => cache.put(key, image),
            #[cfg(test)]
            Self::Memory(cache) => cache.put(key, image),
        }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        match self {
            Self::Disk(cache) => cache.path_for(key),
            #[cfg(test)]
            Self::Memory(cache) => cache.path_for(key),
        }
    }

    fn store_named(
        &self,
        key: &str,
        source_name: &str,
        suffix: &str,
        image: &image::RgbaImage,
    ) -> Result<PathBuf, CacheError> {
        match self {
            Self::Disk(cache) => {
                OutputCacheInterface::store_named(cache, key, source_name, suffix, image)
            }
            #[cfg(test)]
            Self::Memory(cache) => {
                OutputCacheInterface::store_named(cache, key, source_name, suffix, image)
            }
        }
    }
}

pub(crate) fn spawn_worker(
    ctx: egui::Context,
    cache: OutputCache,
    config: InferenceConfig,
) -> (Sender<WorkerRequest>, Receiver<WorkerResponse>) {
    spawn_worker_with_cache(ctx, WorkerCache::Disk(cache), config)
}

fn spawn_worker_with_cache(
    ctx: egui::Context,
    mut cache: WorkerCache,
    config: InferenceConfig,
) -> (Sender<WorkerRequest>, Receiver<WorkerResponse>) {
    let (request_tx, request_rx) = mpsc::channel::<WorkerRequest>();
    let (done_tx, done_rx) = mpsc::channel::<WorkerResponse>();

    thread::Builder::new()
        .name("compnode-inference".to_owned())
        .spawn(move || {
            let mut models = Models::new(config.clone());

            loop {
                let request = match models.keep_alive_remaining() {
                    Some(remaining) => match request_rx.recv_timeout(remaining) {
                        Ok(request) => request,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            models.unload();
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    },
                    None => match request_rx.recv() {
                        Ok(request) => request,
                        Err(_) => break,
                    },
                };
                #[cfg(test)]
                let request = match request {
                    WorkerRequest::SimulateLoad { started, release } => {
                        let _ = started.send(());
                        let _ = release.recv();
                        continue;
                    }
                    request => request,
                };
                let (job, cancellation, inference) = match request {
                    WorkerRequest::Configure(config) => {
                        if config.cache_dir != models.config.cache_dir {
                            cache = WorkerCache::Disk(OutputCache::new(config.cache_dir.clone()));
                        }
                        models.configure(config);
                        continue;
                    }
                    WorkerRequest::Inference(request) => {
                        (Job::Inference(request.job), request.cancellation, true)
                    }
                    WorkerRequest::Render(request) => {
                        (Job::Render(request.job), request.cancellation, false)
                    }
                    #[cfg(test)]
                    WorkerRequest::SimulateLoad { .. } => unreachable!("handled above"),
                };
                if cancellation.is_cancelled() {
                    continue;
                }
                let (token, node, result) = match job {
                    Job::Inference(InferenceJob::RemoveBg {
                        token,
                        node,
                        input,
                        source_name,
                    }) => {
                        let domain = models.birefnet_cache_domain();
                        let cache_models = models.config.cache_models;
                        let started = Instant::now();
                        let result = models.remove_bg(&input).map(|run| {
                            let key = cache_key(&domain, &input, &[]);
                            let path = match OutputCacheInterface::store_named(
                                &cache,
                                &key,
                                &source_name,
                                "remove-bg",
                                &run.image,
                            ) {
                                Ok(path) => Some(path),
                                Err(err) => {
                                    eprintln!("compnode: {err}");
                                    None
                                }
                            };
                            JobOutput {
                                image: run.image,
                                path,
                                timing: RunTiming {
                                    total: started.elapsed(),
                                    load: run.load,
                                    infer: run.infer,
                                    cache_models,
                                },
                            }
                        });
                        (token, node, result)
                    }
                    Job::Inference(InferenceJob::RemoveBgV2 {
                        token,
                        node,
                        input,
                        source_name,
                    }) => {
                        let domain = format!(
                            "rmbg-v1:{}",
                            model_tag(&config.model_dir.join(MODEL_RMBG_FILE))
                        );
                        let cache_models = models.config.cache_models;
                        let started = Instant::now();
                        let result = remove_bg_v2(&mut models, &input).map(|run| {
                            let key = cache_key(&domain, &input, &[]);
                            let path = match OutputCacheInterface::store_named(
                                &cache,
                                &key,
                                &source_name,
                                "remove-bg-v2",
                                &run.image,
                            ) {
                                Ok(path) => Some(path),
                                Err(err) => {
                                    eprintln!("compnode: {err}");
                                    None
                                }
                            };
                            JobOutput {
                                image: run.image,
                                path,
                                timing: RunTiming {
                                    total: started.elapsed(),
                                    load: run.load,
                                    infer: run.infer,
                                    cache_models,
                                },
                            }
                        });
                        (token, node, result)
                    }
                    Job::Inference(InferenceJob::FaceRestore {
                        token,
                        node,
                        input,
                        strength,
                        only_center,
                        source_name,
                    }) => {
                        let domain = models.face_cache_domain();
                        let cache_models = models.config.cache_models;
                        let mut params = Vec::with_capacity(5);
                        params.extend_from_slice(&strength.to_le_bytes());
                        params.push(u8::from(only_center));
                        let started = Instant::now();
                        let result =
                            models
                                .restore_faces(&input, strength, only_center)
                                .map(|run| {
                                    let key = cache_key(&domain, &input, &params);
                                    let path = match OutputCacheInterface::store_named(
                                        &cache,
                                        &key,
                                        &source_name,
                                        "face-restore",
                                        &run.image,
                                    ) {
                                        Ok(path) => Some(path),
                                        Err(err) => {
                                            eprintln!("compnode: {err}");
                                            None
                                        }
                                    };
                                    JobOutput {
                                        image: run.image,
                                        path,
                                        timing: RunTiming {
                                            total: started.elapsed(),
                                            load: run.load,
                                            infer: run.infer,
                                            cache_models,
                                        },
                                    }
                                });
                        (token, node, result)
                    }
                    Job::Render(RenderJob::Composite {
                        token,
                        node,
                        layers,
                        canvas,
                        mode,
                        opacity,
                        source_name,
                    }) => {
                        let result =
                            composite_full(&cache, &layers, canvas, &mode, opacity, &source_name);
                        (token, node, result)
                    }
                    Job::Render(RenderJob::Filter {
                        token,
                        node,
                        kind,
                        params,
                        inputs,
                        canvas,
                        source_name,
                    }) => {
                        let result =
                            filter_full(&cache, kind, &params, inputs, canvas, &source_name);
                        (token, node, result)
                    }
                    Job::Render(RenderJob::Texts {
                        token,
                        node,
                        params,
                        canvas,
                        source_name,
                    }) => {
                        let result = texts_full(&cache, &params, canvas, &source_name);
                        (token, node, result)
                    }
                };

                let done = JobDone {
                    token,
                    node,
                    result,
                };
                if inference {
                    models.touch();
                }
                let response = if inference {
                    WorkerResponse::Inference(InferenceResponse { done })
                } else {
                    WorkerResponse::Render(RenderResponse { done })
                };
                if done_tx.send(response).is_err() {
                    break;
                }
                // Wake the UI so the finished job is picked up immediately.
                ctx.request_repaint();
            }
        })
        .expect("failed to spawn the inference thread");

    (request_tx, done_rx)
}

struct Models {
    config: InferenceConfig,
    last_used: Option<Instant>,
    birefnet: Option<Box<dyn InferenceBackend>>,
    remove_bg_v2: Option<Box<dyn InferenceBackend>>,
    gfpgan: Option<Gfpgan>,
    detector: Option<FaceDetector>,
}

/// Output plus the two timings users compare when toggling the model cache.
struct TimedImage {
    image: image::RgbaImage,
    /// Time spent actually loading models for this run.
    load: Duration,
    /// Time spent running inference.
    infer: Duration,
}

impl Models {
    fn new(config: InferenceConfig) -> Self {
        Self {
            config,
            last_used: None,
            birefnet: None,
            remove_bg_v2: None,
            gfpgan: None,
            detector: None,
        }
    }

    fn configure(&mut self, config: InferenceConfig) {
        // A different model directory invalidates every resident session.
        // Turning the cache off also releases them immediately.
        if config.model_dir != self.config.model_dir || !config.cache_models {
            self.unload();
        }
        self.config = config;
    }

    fn unload(&mut self) {
        self.last_used = None;
        self.birefnet = None;
        self.remove_bg_v2 = None;
        self.gfpgan = None;
        self.detector = None;
    }

    fn touch(&mut self) {
        if self.config.cache_models
            && (self.birefnet.is_some()
                || self.remove_bg_v2.is_some()
                || self.gfpgan.is_some()
                || self.detector.is_some())
        {
            self.last_used = Some(Instant::now());
        }
    }

    fn keep_alive_remaining(&self) -> Option<Duration> {
        let has_models = self.birefnet.is_some()
            || self.remove_bg_v2.is_some()
            || self.gfpgan.is_some()
            || self.detector.is_some();
        if !self.config.cache_models || !has_models {
            return None;
        }
        Some(MODEL_KEEP_ALIVE.saturating_sub(self.last_used.unwrap_or_else(Instant::now).elapsed()))
    }

    fn birefnet_path(&self) -> PathBuf {
        self.config.model_dir.join(MODEL_BIREFNET_FILE)
    }

    fn gfpgan_path(&self) -> PathBuf {
        self.config.model_dir.join(MODEL_GFPGAN_FILE)
    }

    fn detector_path(&self) -> PathBuf {
        self.config.model_dir.join(MODEL_SCRFD_FILE)
    }

    fn birefnet_cache_domain(&self) -> String {
        format!("birefnet-v1:{}", model_tag(&self.birefnet_path()))
    }

    fn face_cache_domain(&self) -> String {
        format!(
            "gfpgan14-scrfd-v1:{}:{}",
            model_tag(&self.gfpgan_path()),
            model_tag(&self.detector_path())
        )
    }

    fn remove_bg(&mut self, input: &image::RgbaImage) -> Result<TimedImage, String> {
        let path = self.birefnet_path();
        let mut load = Duration::ZERO;

        if self.config.cache_models {
            if self.birefnet.is_none() {
                let started = Instant::now();
                self.birefnet = Some(Box::new(load_birefnet(&path)?));
                load = started.elapsed();
            }
            let started = Instant::now();
            let image = self
                .birefnet
                .as_ref()
                .expect("just loaded")
                .predict(input)?;
            Ok(TimedImage {
                image,
                load,
                infer: started.elapsed(),
            })
        } else {
            let started = Instant::now();
            let model = load_birefnet(&path)?;
            load = started.elapsed();

            let started = Instant::now();
            let image = model.predict(input)?;
            Ok(TimedImage {
                image,
                load,
                infer: started.elapsed(),
            })
        }
    }

    fn restore_faces(
        &mut self,
        input: &image::RgbaImage,
        strength: f32,
        only_center: bool,
    ) -> Result<TimedImage, String> {
        let gfpgan_path = self.gfpgan_path();
        let detector_path = self.detector_path();
        let mut load = Duration::ZERO;

        if self.config.cache_models {
            if self.gfpgan.is_none() {
                let started = Instant::now();
                self.gfpgan = Some(load_gfpgan(&gfpgan_path)?);
                load += started.elapsed();
            }
            if self.detector.is_none() {
                let started = Instant::now();
                self.detector = Some(load_detector(&detector_path)?);
                load += started.elapsed();
            }
            let started = Instant::now();
            let image = restore_faces(
                self.gfpgan.as_ref().expect("just loaded"),
                self.detector.as_ref().expect("just loaded"),
                input,
                strength,
                only_center,
            )?;
            Ok(TimedImage {
                image,
                load,
                infer: started.elapsed(),
            })
        } else {
            let started = Instant::now();
            let gfpgan = load_gfpgan(&gfpgan_path)?;
            let detector = load_detector(&detector_path)?;
            load = started.elapsed();

            let started = Instant::now();
            let image = restore_faces(&gfpgan, &detector, input, strength, only_center)?;
            Ok(TimedImage {
                image,
                load,
                infer: started.elapsed(),
            })
        }
    }
}

fn load_birefnet(path: &Path) -> Result<BirefNet, String> {
    BirefNet::load_from(path).map_err(|e| format!("load {}: {e:#}", path.display()))
}

fn load_gfpgan(path: &Path) -> Result<Gfpgan, String> {
    Gfpgan::load_from(path).map_err(|e| format!("load {}: {e:#}", path.display()))
}

fn load_detector(path: &Path) -> Result<FaceDetector, String> {
    FaceDetector::load_from(path).map_err(|e| format!("load {}: {e:#}", path.display()))
}

/// Include the model location and size in the cache domain so changing models
/// cannot return results produced by a different checkpoint.
fn model_tag(path: &Path) -> String {
    let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
    format!("{}:{size}", path.display())
}

fn remove_bg_v2(models: &mut Models, input: &image::RgbaImage) -> Result<TimedImage, String> {
    let path = models.config.model_dir.join(MODEL_RMBG_FILE);
    let mut load = Duration::ZERO;

    if models.config.cache_models {
        if models.remove_bg_v2.is_none() {
            let started = Instant::now();
            models.remove_bg_v2 = Some(Box::new(load_rmbg(&path)?));
            load = started.elapsed();
        }
        let started = Instant::now();
        let image = models
            .remove_bg_v2
            .as_ref()
            .expect("just loaded")
            .predict(input)?;
        Ok(TimedImage {
            image,
            load,
            infer: started.elapsed(),
        })
    } else {
        let started = Instant::now();
        let model = load_rmbg(&path)?;
        load = started.elapsed();

        let started = Instant::now();
        let image = model.predict(input)?;
        Ok(TimedImage {
            image,
            load,
            infer: started.elapsed(),
        })
    }
}

fn load_rmbg(path: &Path) -> Result<RemoveBg, String> {
    RemoveBg::load_from(path).map_err(|e| format!("load {}: {e:#}", path.display()))
}

fn restore_faces(
    gfpgan: &Gfpgan,
    detector: &FaceDetector,
    input: &image::RgbaImage,
    strength: f32,
    only_center: bool,
) -> Result<image::RgbaImage, String> {
    let (w, h) = input.dimensions();
    let mut rgb = image::RgbImage::new(w, h);
    for (x, y, p) in input.enumerate_pixels() {
        rgb.put_pixel(x, y, image::Rgb([p[0], p[1], p[2]]));
    }

    let faces = detector
        .detect(&rgb, 0.5, 0.4)
        .map_err(|e| format!("{e:#}"))?;
    let faces = if only_center {
        rembg_core::pick_center(faces, w, h)
    } else {
        faces
    };
    let landmarks: Vec<rembg_core::Landmarks> = faces.iter().map(|f| f.landmarks).collect();
    let restored = gfpgan
        .restore_faces(&rgb, &landmarks, strength)
        .map_err(|e| format!("{e:#}"))?;

    let mut out = image::RgbaImage::new(w, h);
    for (x, y, p) in restored.enumerate_pixels() {
        let a = input.get_pixel(x, y)[3];
        out.put_pixel(x, y, image::Rgba([p[0], p[1], p[2], a]));
    }
    Ok(out)
}

#[cfg(test)]
mod inference_backend_tests {
    use super::{
        InferenceBackend, InferenceConfig, InferenceError, RenderJob, RenderRequest, WorkerCache,
        WorkerRequest, spawn_worker_with_cache,
    };
    use crate::cache::MemoryOutputCache;
    use crate::cancellation::CancellationToken;
    use std::sync::mpsc;
    use std::time::Duration;

    struct MockBackend;

    impl InferenceBackend for MockBackend {
        fn predict(&self, input: &image::RgbaImage) -> Result<image::RgbaImage, InferenceError> {
            let mut output = input.clone();
            for pixel in output.pixels_mut() {
                pixel[0] = pixel[0].saturating_add(1);
            }
            Ok(output)
        }
    }

    #[test]
    fn mock_backend_uses_the_production_prediction_contract() {
        let input = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 40]));
        let output = MockBackend.predict(&input).unwrap();

        assert_eq!(output.get_pixel(0, 0), &image::Rgba([11, 20, 30, 40]));
    }

    #[test]
    fn worker_drops_pre_cancelled_render_jobs_without_a_completion() {
        let root = std::env::temp_dir().join(format!(
            "compnode-cancel-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = InferenceConfig {
            cache_models: false,
            model_dir: root.join("models"),
            cache_dir: root.join("cache"),
        };
        let (request_tx, done_rx) = spawn_worker_with_cache(
            egui::Context::default(),
            WorkerCache::Memory(MemoryOutputCache::default()),
            config,
        );
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        request_tx
            .send(WorkerRequest::Render(RenderRequest {
                job: RenderJob::Texts {
                    token: 1,
                    node: egui_snarl::NodeId(0),
                    params: Default::default(),
                    canvas: (1, 1),
                    source_name: "cancelled".to_owned(),
                },
                cancellation,
            }))
            .unwrap();
        drop(request_tx);

        assert!(matches!(
            done_rx.recv_timeout(std::time::Duration::from_secs(2)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn ui_frame_runs_while_worker_is_under_simulated_load() {
        let config = InferenceConfig {
            cache_models: false,
            model_dir: "models".into(),
            cache_dir: "cache".into(),
        };
        let ctx = egui::Context::default();
        let (request_tx, done_rx) = spawn_worker_with_cache(
            ctx.clone(),
            WorkerCache::Memory(MemoryOutputCache::default()),
            config,
        );
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        request_tx
            .send(WorkerRequest::SimulateLoad {
                started: started_tx,
                release: release_rx,
            })
            .unwrap();
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not enter simulated load");

        ctx.begin_pass(egui::RawInput::default());
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        release_tx.send(()).unwrap();
        drop(request_tx);
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn cached_models_expire_after_the_keep_alive_window() {
        let mut config = InferenceConfig {
            cache_models: true,
            model_dir: "models".into(),
            cache_dir: "cache".into(),
        };
        let mut models = super::Models::new(config.clone());
        models.birefnet = Some(Box::new(MockBackend));
        models.last_used = Some(std::time::Instant::now() - super::MODEL_KEEP_ALIVE);

        assert_eq!(
            models.keep_alive_remaining(),
            Some(std::time::Duration::ZERO)
        );

        config.cache_models = false;
        models.configure(config);
        assert!(models.keep_alive_remaining().is_none());
    }
}
