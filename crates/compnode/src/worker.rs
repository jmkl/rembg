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
    BirefNet, FaceDetector, Gfpgan, MODEL_BIREFNET_FILE, MODEL_GFPGAN_FILE, MODEL_SCRFD_FILE,
};

use crate::cache::{OutputCache, cache_key, cache_key_layers};
use crate::model::{JobOutput, RunTiming, ViewerLayer};

pub(crate) enum Job {
    RemoveBg {
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
    /// Full-resolution Group composite rendered from on-disk source images.
    Composite {
        token: u64,
        node: NodeId,
        layers: Vec<CompositeLayer>,
        canvas: (u32, u32),
        source_name: String,
    },
}

/// One layer of a full-resolution Group render: an on-disk source image plus
/// its canvas transform.
#[derive(Clone, Debug)]
pub(crate) struct CompositeLayer {
    pub source: NodeId,
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

pub(crate) enum WorkerRequest {
    Job(Job),
    Configure(InferenceConfig),
}

pub(crate) struct JobDone {
    pub token: u64,
    pub node: NodeId,
    pub result: Result<JobOutput, String>,
}

/// Render a Group node at full resolution from on-disk source images and
/// persist the result in the output cache. Repeated renders with identical
/// inputs hit the disk cache instead of recompositing.
fn composite_full(
    cache: &OutputCache,
    layers: &[CompositeLayer],
    canvas: (u32, u32),
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

    let mut images = Vec::with_capacity(layers.len());
    for layer in layers {
        let img = image::open(&layer.path)
            .map_err(|err| format!("Could not load {}: {err}", layer.path.display()))?
            .to_rgba8();
        images.push(img);
    }

    let key = cache_key_layers("compnode-group-v1", &images, &params);

    // Cache hit: reuse the stored composite.
    let cached_path = cache.path_for(&key);
    if cached_path.is_file() {
        if let Ok(img) = image::open(&cached_path).map(|img| img.to_rgba8()) {
            return Ok(JobOutput {
                image: img,
                path: Some(cached_path),
                timing: RunTiming {
                    total: started.elapsed(),
                    load: Duration::ZERO,
                    infer: Duration::ZERO,
                    cache_models: false,
                },
            });
        }
    }

    let pairs: Vec<(ViewerLayer, image::RgbaImage)> = layers
        .iter()
        .zip(images.into_iter())
        .map(|(layer, img)| {
            (
                ViewerLayer {
                    source: layer.source,
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
    let composited = crate::ops::composite::composite_layers_at(&pairs, canvas, None)?;

    let path = match cache.store_named(&key, source_name, "group", &composited) {
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

pub(crate) fn spawn_worker(
    ctx: egui::Context,
    mut cache: OutputCache,
    config: InferenceConfig,
) -> (Sender<WorkerRequest>, Receiver<JobDone>) {
    let (request_tx, request_rx) = mpsc::channel::<WorkerRequest>();
    let (done_tx, done_rx) = mpsc::channel::<JobDone>();

    thread::Builder::new()
        .name("compnode-inference".to_owned())
        .spawn(move || {
            let mut models = Models::new(config);

            while let Ok(request) = request_rx.recv() {
                let (token, node, result) = match request {
                    WorkerRequest::Configure(config) => {
                        if config.cache_dir != models.config.cache_dir {
                            cache = OutputCache::new(config.cache_dir.clone());
                        }
                        models.configure(config);
                        continue;
                    }
                    WorkerRequest::Job(Job::RemoveBg {
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
                            let path = match cache.store_named(
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
                    WorkerRequest::Job(Job::FaceRestore {
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
                                    let path = match cache.store_named(
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
                    WorkerRequest::Job(Job::Composite {
                        token,
                        node,
                        layers,
                        canvas,
                        source_name,
                    }) => {
                        let result = composite_full(&cache, &layers, canvas, &source_name);
                        (token, node, result)
                    }
                };

                if done_tx
                    .send(JobDone {
                        token,
                        node,
                        result,
                    })
                    .is_err()
                {
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
    birefnet: Option<BirefNet>,
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
            birefnet: None,
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
        self.birefnet = None;
        self.gfpgan = None;
        self.detector = None;
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
                self.birefnet = Some(load_birefnet(&path)?);
                load = started.elapsed();
            }
            let started = Instant::now();
            let image = process_birefnet(self.birefnet.as_ref().expect("just loaded"), input)?;
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
            let image = process_birefnet(&model, input)?;
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

fn process_birefnet(
    model: &BirefNet,
    input: &image::RgbaImage,
) -> Result<image::RgbaImage, String> {
    let img = image::DynamicImage::ImageRgba8(input.clone());
    model.process_image(&img).map_err(|e| format!("{e:#}"))
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
