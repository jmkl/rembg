//! SAM 2 / SAM 2.1 (Segment Anything Model 2) — ONNX Runtime inference.
//!
//! Architecture: two-stage split model.
//!   1. **Encoder** (`*.encoder.onnx`): image → feature embeddings (run once per image).
//!   2. **Decoder** (`*.decoder.onnx`): embeddings + prompts → segmentation mask.
//!
//! Supported prompt types:
//!   - [`Prompt::Point`]  — foreground / background point click.
//!   - [`Prompt::Box`]    — bounding box (encoded as two corner points with labels 2/3).
//!
//! Pre-exported ONNX models: <https://huggingface.co/vietanhdev/segment-anything-2.1-onnx-models>
//!
//! # Quick start
//! ```no_run
//! use rembg_core::sam2::{Sam2, Prompt, PointLabel};
//!
//! let sam = Sam2::load()?;
//! let img = image::open("photo.jpg")?;
//!
//! // Click the centre of the image as a foreground point.
//! let cx = img.width() as f32 / 2.0;
//! let cy = img.height() as f32 / 2.0;
//! let prompts = vec![Prompt::Point { x: cx, y: cy, label: PointLabel::Foreground }];
//!
//! let cutout = sam.process_image(&img, &prompts)?;
//! cutout.save("output.png")?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::{path::Path, sync::Mutex};

use anyhow::{Result, anyhow, bail};
use image::{DynamicImage, GrayImage, RgbaImage, imageops::FilterType};
use ort::{session::Session, value::Tensor};

use crate::config;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// SAM 2 encoder input resolution (width = height).
pub const SIZE_SAM2: u32 = 1024;

/// SAM 2 low-resolution mask size produced by the decoder (used for iterative
/// refinement). Not a user-facing resolution; the decoder upsamples internally.
pub const SIZE_SAM2_MASK_LOW: u32 = 256;

/// Per-channel pixel mean used by SAM 2 (identical to SAM 1).
/// Applied as `(pixel_value − mean) / std` on the [0, 255] scale.
pub const MEAN_SAM2: [f32; 3] = [123.675, 116.28, 103.53];

/// Per-channel pixel standard deviation used by SAM 2.
pub const STD_SAM2: [f32; 3] = [58.395, 57.12, 57.375];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A foreground/background label attached to a [`Prompt::Point`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointLabel {
    /// The point lies on the object of interest (include this region).
    Foreground = 1,
    /// The point lies on the background (exclude this region).
    Background = 0,
}

/// Prompt used to guide the SAM 2 mask decoder.
#[derive(Debug, Clone)]
pub enum Prompt {
    /// A single clicked point in **original-image pixel coordinates**.
    Point {
        x: f32,
        y: f32,
        label: PointLabel,
    },
    /// An axis-aligned bounding box in **original-image pixel coordinates**.
    /// Encoded internally as two corner points with decoder labels 2 (top-left)
    /// and 3 (bottom-right), as expected by the SAM 2 ONNX decoder.
    Box {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
}

/// Image embeddings produced by the SAM 2 encoder.
///
/// Holds the three feature tensors together with the original image dimensions
/// required to rescale decoder output masks back to image space. Pass this to
/// [`Sam2::decode`] together with a set of [`Prompt`]s to obtain a mask.
///
/// Storing the embedding allows you to run multiple decoder passes (different
/// prompts) against the same image without re-encoding.
pub struct Sam2Embedding {
    // Tensor data stored as flat `Vec<f32>` together with explicit shapes so we
    // can re-create `Tensor` objects for each decoder call without re-running
    // the (expensive) encoder.
    pub(crate) image_embed: Vec<f32>,
    pub(crate) image_embed_shape: Vec<usize>,

    pub(crate) high_res_feats_0: Vec<f32>,
    pub(crate) high_res_feats_0_shape: Vec<usize>,

    pub(crate) high_res_feats_1: Vec<f32>,
    pub(crate) high_res_feats_1_shape: Vec<usize>,

    /// Original image width (before any resizing), in pixels.
    pub orig_width: u32,
    /// Original image height (before any resizing), in pixels.
    pub orig_height: u32,
}

impl Sam2Embedding {
    /// Returns the shapes of the three encoder output tensors, in order:
    /// `(image_embed_shape, high_res_feats_0_shape, high_res_feats_1_shape)`.
    pub fn shapes(&self) -> (&[usize], &[usize], &[usize]) {
        (
            &self.image_embed_shape,
            &self.high_res_feats_0_shape,
            &self.high_res_feats_1_shape,
        )
    }
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

/// SAM 2 segmentation model backed by two ONNX sessions (encoder + decoder).
///
/// Both sessions are wrapped in a [`Mutex`] so a single `Sam2` instance can be
/// shared across threads; the GPU call is the only critical section.
pub struct Sam2 {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
}

impl Sam2 {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Load the default SAM 2 models from the standard `models/` directory.
    ///
    /// Expects:
    /// - `models/sam2.encoder.onnx`
    /// - `models/sam2.decoder.onnx`
    pub fn load() -> Result<Self> {
        Self::load_from(
            config::MODEL_SAM2_ENCODER,
            config::MODEL_SAM2_DECODER,
        )
    }

    /// Load SAM 2 models from explicit file paths.
    ///
    /// # Arguments
    /// - `encoder_path` — path to the `*.encoder.onnx` file.
    /// - `decoder_path` — path to the `*.decoder.onnx` file.
    pub fn load_from(
        encoder_path: impl AsRef<Path>,
        decoder_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let encoder = config::load_session(encoder_path, config::VRAM_LIMIT_SAM2_MB)?;
        let decoder = config::load_session(decoder_path, config::VRAM_LIMIT_SAM2_MB)?;
        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
        })
    }

    // -----------------------------------------------------------------------
    // High-level API
    // -----------------------------------------------------------------------

    /// Convenience wrapper: open an image file, segment it using `prompts`,
    /// and return an RGBA image with the mask applied to the alpha channel.
    ///
    /// Equivalent to calling [`Sam2::encode`] then [`Sam2::decode`] then
    /// compositing the resulting mask.
    pub fn process(&self, path: &Path, prompts: &[Prompt]) -> Result<RgbaImage> {
        let img = image::open(path)?;
        self.process_image(&img, prompts)
    }

    /// Encode `img`, decode with `prompts`, and return an RGBA cutout.
    ///
    /// The highest-scoring mask candidate is selected automatically.
    pub fn process_image(&self, img: &DynamicImage, prompts: &[Prompt]) -> Result<RgbaImage> {
        let embedding = self.encode(img)?;
        let mask = self.decode(&embedding, prompts)?;

        // Apply the mask to the alpha channel of the original image.
        let mut result = img.to_rgba8();
        for (p, m) in result.pixels_mut().zip(mask.pixels()) {
            p[3] = m[0];
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Two-stage API
    // -----------------------------------------------------------------------

    /// **Stage 1** — Run the image encoder and return the resulting embeddings.
    ///
    /// Embeddings can be reused across multiple [`Sam2::decode`] calls, which is
    /// useful for interactive segmentation where the user refines prompts
    /// without changing the image.
    pub fn encode(&self, img: &DynamicImage) -> Result<Sam2Embedding> {
        let (orig_width, orig_height) = (img.width(), img.height());

        // Resize to encoder input resolution and convert to RGB.
        let resized = img
            .resize_exact(SIZE_SAM2, SIZE_SAM2, FilterType::Lanczos3)
            .to_rgb8();

        // Normalise: (pixel − mean) / std, layout CHW, batch = 1.
        let n = (SIZE_SAM2 * SIZE_SAM2) as usize;
        let mut data = vec![0.0f32; 3 * n];
        for (i, px) in resized.pixels().enumerate() {
            data[i] = (px[0] as f32 - MEAN_SAM2[0]) / STD_SAM2[0];
            data[n + i] = (px[1] as f32 - MEAN_SAM2[1]) / STD_SAM2[1];
            data[2 * n + i] = (px[2] as f32 - MEAN_SAM2[2]) / STD_SAM2[2];
        }
        let input = Tensor::from_array((
            [1usize, 3, SIZE_SAM2 as usize, SIZE_SAM2 as usize],
            data,
        ))?;

        let mut encoder = self.encoder.lock().map_err(config::lock_err)?;
        let outputs = encoder.run(ort::inputs![input])?;

        // The encoder yields 3 feature tensors; the exact output *order* varies
        // between export tools so we match by shape rather than by index:
        //   image_embed      — [1, 256,  64,  64]  (primary feature map)
        //   high_res_feats_0 — [1,  32, 256, 256]  (fine-grained, stride 4)
        //   high_res_feats_1 — [1,  64, 128, 128]  (mid-level,    stride 8)
        if outputs.len() < 3 {
            bail!(
                "SAM2 encoder: expected ≥ 3 outputs, got {}. \
                 Make sure you are using a community-exported split model \
                 (encoder + decoder pair), not the monolithic checkpoint.",
                outputs.len()
            );
        }

        // Collect all three tensors, then sort by shape.
        let mut tensors: Vec<(Vec<usize>, Vec<f32>)> = (0..outputs.len().min(3))
            .map(|i| extract_tensor(&outputs[i]))
            .collect::<Result<_>>()?;

        // Sort descending by channel count (index 1) so assignment is deterministic:
        //   [0] → C=256 → image_embed
        //   [1] → C=64  → high_res_feats_1
        //   [2] → C=32  → high_res_feats_0
        tensors.sort_by(|a, b| {
            let ca = a.0.get(1).copied().unwrap_or(0);
            let cb = b.0.get(1).copied().unwrap_or(0);
            cb.cmp(&ca)
        });

        let (image_embed_shape, image_embed) = tensors.remove(0);       // C=256
        let (high_res_feats_1_shape, high_res_feats_1) = tensors.remove(0); // C=64
        let (high_res_feats_0_shape, high_res_feats_0) = tensors.remove(0); // C=32

        Ok(Sam2Embedding {
            image_embed,
            image_embed_shape,
            high_res_feats_0,
            high_res_feats_0_shape,
            high_res_feats_1,
            high_res_feats_1_shape,
            orig_width,
            orig_height,
        })
    }

    /// **Stage 2** — Run the mask decoder for the given embedding and prompts.
    ///
    /// Returns a grayscale mask (same dimensions as the original image) where
    /// 255 = foreground and 0 = background. The highest-IoU candidate among the
    /// three mask proposals is selected automatically.
    ///
    /// # Errors
    /// Returns an error if `prompts` is empty — SAM 2 requires at least one
    /// point or box to produce a meaningful mask.
    pub fn decode(&self, embedding: &Sam2Embedding, prompts: &[Prompt]) -> Result<GrayImage> {
        if prompts.is_empty() {
            bail!("SAM2 decoder: at least one prompt (point or box) is required");
        }

        // Build flat point_coords [1, N, 2] and point_labels [1, N] tensors.
        // Box prompts are expanded to two corner points with labels 2 and 3.
        let scale_x = SIZE_SAM2 as f32 / embedding.orig_width as f32;
        let scale_y = SIZE_SAM2 as f32 / embedding.orig_height as f32;

        let mut coords: Vec<f32> = Vec::new();
        let mut labels: Vec<f32> = Vec::new();

        for prompt in prompts {
            match *prompt {
                Prompt::Point { x, y, label } => {
                    coords.push(x * scale_x);
                    coords.push(y * scale_y);
                    labels.push(label as i32 as f32);
                }
                Prompt::Box { x1, y1, x2, y2 } => {
                    // Top-left corner → label 2
                    coords.push(x1 * scale_x);
                    coords.push(y1 * scale_y);
                    labels.push(2.0);
                    // Bottom-right corner → label 3
                    coords.push(x2 * scale_x);
                    coords.push(y2 * scale_y);
                    labels.push(3.0);
                }
            }
        }

        let num_points = labels.len();

        let point_coords =
            Tensor::from_array(([1usize, num_points, 2], coords))?;
        let point_labels =
            Tensor::from_array(([1usize, num_points], labels))?;

        // No prior mask → zero-fill mask_input and has_mask_input.
        let mask_size = SIZE_SAM2_MASK_LOW as usize;
        let mask_input =
            Tensor::from_array(([1usize, 1, mask_size, mask_size], vec![0.0f32; mask_size * mask_size]))?;
        let has_mask_input = Tensor::from_array(([1usize], vec![0.0f32]))?;

        // Reconstruct encoder feature tensors.
        let image_embed = Tensor::from_array((
            embedding.image_embed_shape.clone(),
            embedding.image_embed.clone(),
        ))?;
        let high_res_feats_0 = Tensor::from_array((
            embedding.high_res_feats_0_shape.clone(),
            embedding.high_res_feats_0.clone(),
        ))?;
        let high_res_feats_1 = Tensor::from_array((
            embedding.high_res_feats_1_shape.clone(),
            embedding.high_res_feats_1.clone(),
        ))?;

        let mut decoder = self.decoder.lock().map_err(config::lock_err)?;
        let outputs = decoder.run(ort::inputs![
            image_embed,
            high_res_feats_0,
            high_res_feats_1,
            point_coords,
            point_labels,
            mask_input,
            has_mask_input
        ])?;

        // Decoder outputs:
        //   [0] masks            — [1, num_candidates, H, W]  (logits)
        //   [1] iou_predictions  — [1, num_candidates]
        //   [2] low_res_masks    — [1, num_candidates, 256, 256]
        if outputs.len() < 2 {
            bail!("SAM2 decoder: expected ≥ 2 outputs, got {}", outputs.len());
        }

        let (mask_shape, mask_logits) = extract_tensor(&outputs[0])?;
        let (_iou_shape, iou_scores) = extract_tensor(&outputs[1])?;

        // mask_shape: [1, num_candidates, H, W]
        if mask_shape.len() != 4 {
            bail!("SAM2 decoder: unexpected mask tensor rank {}", mask_shape.len());
        }
        let num_candidates = mask_shape[1];
        let mask_h = mask_shape[2];
        let mask_w = mask_shape[3];
        let plane = mask_h * mask_w;

        // Pick the candidate with the highest predicted IoU score.
        let best = (0..num_candidates)
            .max_by(|&a, &b| {
                iou_scores
                    .get(a)
                    .unwrap_or(&f32::NEG_INFINITY)
                    .total_cmp(iou_scores.get(b).unwrap_or(&f32::NEG_INFINITY))
            })
            .unwrap_or(0);

        let slice = &mask_logits[best * plane..(best + 1) * plane];

        // Convert logits → binary mask via sigmoid, then scale to u8.
        let raw: Vec<u8> = slice
            .iter()
            .map(|&v| {
                let p = 1.0 / (1.0 + (-v).exp());
                (p.clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect();

        let mask_small =
            GrayImage::from_raw(mask_w as u32, mask_h as u32, raw)
                .ok_or_else(|| anyhow!("SAM2: failed to construct mask image from decoder output"))?;

        // Upscale mask back to original image dimensions.
        let mask = image::imageops::resize(
            &mask_small,
            embedding.orig_width,
            embedding.orig_height,
            FilterType::Lanczos3,
        );

        Ok(mask)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a tensor from an ORT output value, returning `(shape, flat_data)`.
fn extract_tensor(
    value: &ort::value::Value,
) -> Result<(Vec<usize>, Vec<f32>)> {
    let (shape, data) = value.try_extract_tensor::<f32>()?;
    let shape: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
    Ok((shape, data.to_vec()))
}
