use std::path::Path;

use anyhow::{Result, anyhow, bail};
use image::RgbImage;
use ort::{
    ep::{self, ExecutionProviderDispatch},
    session::Session,
    value::Tensor,
};
// Mutex not used here

pub const WARMUP: bool = false;

pub const MODEL_GFPGAN_FILE: &str = "GFPGANv1.4.onnx";
pub const MODEL_BIREFNET_FILE: &str = "birefnet.onnx";
pub const MODEL_RMBG_FILE: &str = "rmbg-1.4.onnx";
pub const MODEL_SCRFD_FILE: &str = "det_10g.onnx";
pub const MODEL_SAM2_ENCODER_FILE: &str = "sam2.1_hiera_small.encoder.onnx";
pub const MODEL_SAM2_DECODER_FILE: &str = "sam2.1_hiera_small.decoder.onnx";
pub const MODEL_YOLO_FILE: &str = "yolo11n.onnx";

pub const MODEL_GFPGAN: &str = "models/GFPGANv1.4.onnx";

/// Resolve a model root directory, defaulting to "models" when None/empty.
pub fn model_root(root: Option<&str>) -> String {
    root.map(|s| s.trim_end_matches(['/', '\\']).to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "models".to_string())
}

/// Join a model root directory with a model file name.
pub fn model_path(root: Option<&str>, file: &str) -> String {
    format!("{}/{}", model_root(root), file)
}

/// Alias for `model_root`, matching the requested name.
pub fn load_model_path_root(root: Option<&str>) -> String {
    model_root(root)
}
pub const MODEL_BIREFNET: &str = "models/birefnet.onnx";
pub const MODEL_RMBG: &str = "models/rmbg-1.4.onnx";
pub const MODEL_SCRFD: &str = "models/det_10g.onnx";
pub const MODEL_SAM2_ENCODER: &str = "models/sam2.1_hiera_small.encoder.onnx";
pub const MODEL_SAM2_DECODER: &str = "models/sam2.1_hiera_small.decoder.onnx";
pub const MODEL_YOLO: &str = "models/yolo11n.onnx";

pub const SIZE_GFPGAN: u32 = 512;
pub const SIZE_BIREFNET: u32 = 1024;
pub const SIZE_YOLO: u32 = 640;
pub const MAX_WIDTH: u32 = 1920;
pub const SIGMOID: bool = true;

pub const MEAN_IMAGENET: [f32; 3] = [0.485, 0.456, 0.406];
pub const STD_IMAGENET: [f32; 3] = [0.229, 0.224, 0.225];

pub const VRAM_LIMIT_GFPGAN_MB: usize = 0;
pub const VRAM_LIMIT_BIREFNET_MB: usize = 0;
pub const VRAM_LIMIT_SCRFD_MB: usize = 0;
pub const VRAM_LIMIT_SAM2_MB: usize = 0;
pub const VRAM_LIMIT_YOLO_MB: usize = 0;

pub fn cuda_ep(vram_limit_mb: usize) -> ExecutionProviderDispatch {
    let mut cuda = ep::CUDA::default()
        .with_arena_extend_strategy(ep::ArenaExtendStrategy::SameAsRequested)
        .with_conv_algorithm_search(ep::cuda::ConvAlgorithmSearch::Heuristic);
    if vram_limit_mb > 0 {
        cuda = cuda.with_memory_limit(vram_limit_mb * 1024 * 1024);
    }
    cuda.build().error_on_failure()
}

pub fn load_session(model: impl AsRef<Path>, vram_limit_mb: usize) -> Result<Session> {
    let model = model.as_ref();
    let session = Session::builder()?
        .with_memory_pattern(false)
        .map_err(|e| anyhow!("Yikes {e}"))?
        .with_execution_providers([cuda_ep(vram_limit_mb)])
        .map_err(|e| anyhow!("{}: {e:?}", model.display()))?
        .commit_from_file(model)?;
    Ok(session)
}

pub fn warmup(session: &mut Session, size: u32) -> Result<()> {
    let n = (size * size) as usize;
    let input = Tensor::from_array((
        [1usize, 3, size as usize, size as usize],
        vec![0.0f32; 3 * n],
    ))?;
    session.run(ort::inputs![input])?;
    Ok(())
}

pub fn to_chw(img: &RgbImage, mean: [f32; 3], std: [f32; 3]) -> Vec<f32> {
    let n = (img.width() * img.height()) as usize;
    let scale = [
        1.0 / (255.0 * std[0]),
        1.0 / (255.0 * std[1]),
        1.0 / (255.0 * std[2]),
    ];
    let bias = [-mean[0] / std[0], -mean[1] / std[1], -mean[2] / std[2]];

    let mut data = vec![0.0f32; 3 * n];
    let (r, rest) = data.split_at_mut(n);
    let (g, b) = rest.split_at_mut(n);
    for (i, px) in img.as_raw().as_chunks::<3>().0.iter().enumerate() {
        r[i] = px[0] as f32 * scale[0] + bias[0];
        g[i] = px[1] as f32 * scale[1] + bias[1];
        b[i] = px[2] as f32 * scale[2] + bias[2];
    }
    data
}

pub fn mask_from_logits(out: &[f32]) -> Result<image::GrayImage> {
    let n = (SIZE_BIREFNET * SIZE_BIREFNET) as usize;
    if out.len() < n {
        bail!("BiRefNet output tensor is too small");
    }
    let out = &out[..n];

    let mut raw = vec![0u8; n];
    for (dst, &v) in raw.iter_mut().zip(out) {
        let p = 1.0 / (1.0 + (-v).exp());
        *dst = (p.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    image::GrayImage::from_raw(SIZE_BIREFNET, SIZE_BIREFNET, raw)
        .ok_or_else(|| anyhow!("failed to build mask image"))
}

pub fn lock_err<T>(_: std::sync::PoisonError<T>) -> anyhow::Error {
    anyhow!("session mutex poisoned (another thread panicked)")
}

// Mutex already imported above
