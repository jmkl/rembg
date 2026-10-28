//! YOLO (Ultralytics YOLO11 / YOLOv8) object detector via ONNX Runtime.
//!
//! Input: [1, 3, 640, 640] normalized to [0, 1]. Supports float16 and float32 models.
//! Output: [1, 84, 8400] (for 80 COCO classes) where class 0 is `person`.
//!
//! Typical usage:
//! ```no_run
//! use rembg_core::yolo::Yolo;
//!
//! let yolo = Yolo::load()?;
//! let img = image::open("photo.jpg")?;
//! let persons = yolo.detect_persons(&img, 0.4, 0.45)?;
//! for p in &persons {
//!     println!("Person detected at {:?} with score {:.2}", p.bbox, p.score);
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use half::f16;
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use ort::{session::Session, value::Tensor};

use crate::config::{self, MODEL_YOLO, SIZE_YOLO, VRAM_LIMIT_YOLO_MB};

/// Class index for `person` in the standard 80-class COCO dataset.
pub const COCO_PERSON_CLASS_ID: usize = 0;

#[derive(Clone, Debug)]
pub struct Detection {
    /// Bounding box in source-image pixels: `[x1, y1, x2, y2]`.
    pub bbox: [f32; 4],
    /// Confidence score (0.0 .. 1.0).
    pub score: f32,
    /// Detected class index (e.g. 0 for person in COCO).
    pub class_id: usize,
}

pub struct Yolo {
    session: Mutex<Session>,
    is_fp16: bool,
}

impl Yolo {
    pub fn load() -> Result<Self> {
        Self::load_from(MODEL_YOLO)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let session = config::load_session(path, VRAM_LIMIT_YOLO_MB)?;
        let is_fp16 = session
            .inputs()
            .first()
            .map(|i| format!("{:?}", i.dtype()).contains("Float16"))
            .unwrap_or(false);

        Ok(Self {
            session: Mutex::new(session),
            is_fp16,
        })
    }

    /// Detect only persons (COCO class 0). Sorted by score descending.
    pub fn detect_persons(
        &self,
        img: &DynamicImage,
        score_thr: f32,
        nms_thr: f32,
    ) -> Result<Vec<Detection>> {
        let all = self.detect(img, score_thr, nms_thr)?;
        Ok(all.into_iter().filter(|d| d.class_id == COCO_PERSON_CLASS_ID).collect())
    }

    /// Detect all objects. Sorted by score descending.
    pub fn detect(
        &self,
        img: &DynamicImage,
        score_thr: f32,
        nms_thr: f32,
    ) -> Result<Vec<Detection>> {
        let (orig_w, orig_h) = img.dimensions();
        if orig_w == 0 || orig_h == 0 {
            return Ok(vec![]);
        }

        let target_size = SIZE_YOLO; // 640
        let scale = (target_size as f32 / orig_w as f32).min(target_size as f32 / orig_h as f32);
        let nw = ((orig_w as f32 * scale).round() as u32).clamp(1, target_size);
        let nh = ((orig_h as f32 * scale).round() as u32).clamp(1, target_size);

        // Center padding (standard letterbox)
        let pad_x = (target_size - nw) as f32 / 2.0;
        let pad_y = (target_size - nh) as f32 / 2.0;

        let resized = img.resize_exact(nw, nh, FilterType::Triangle).to_rgb8();

        let n = (target_size * target_size) as usize;
        let mut f16_data = vec![f16::from_f32(114.0 / 255.0); 3 * n]; // 114 gray fill
        let x_offset = pad_x.round() as u32;
        let y_offset = pad_y.round() as u32;

        for (x, y, p) in resized.enumerate_pixels() {
            let cx = x + x_offset;
            let cy = y + y_offset;
            if cx < target_size && cy < target_size {
                let idx = (cy * target_size + cx) as usize;
                f16_data[idx] = f16::from_f32(p[0] as f32 / 255.0);
                f16_data[n + idx] = f16::from_f32(p[1] as f32 / 255.0);
                f16_data[2 * n + idx] = f16::from_f32(p[2] as f32 / 255.0);
            }
        }

        let mut session = self.session.lock().map_err(config::lock_err)?;

        let outputs = if self.is_fp16 {
            let tensor_f16 = Tensor::from_array((
                [1usize, 3, target_size as usize, target_size as usize],
                f16_data,
            ))?;
            session.run(ort::inputs![tensor_f16])?
        } else {
            let f32_data: Vec<f32> = f16_data.iter().map(|v| v.to_f32()).collect();
            let tensor_f32 = Tensor::from_array((
                [1usize, 3, target_size as usize, target_size as usize],
                f32_data,
            ))?;
            session.run(ort::inputs![tensor_f32])?
        };

        let detections = parse_outputs(&outputs, scale, pad_x, pad_y, orig_w, orig_h, score_thr)?;

        Ok(nms(detections, nms_thr))
    }
}

fn parse_outputs(
    outputs: &ort::session::SessionOutputs,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    orig_w: u32,
    orig_h: u32,
    score_thr: f32,
) -> Result<Vec<Detection>> {
    let first = outputs.iter().next().ok_or_else(|| anyhow!("YOLO model produced no outputs"))?;

    // Try extracting as f16, otherwise as f32
    if let Ok((shape, data)) = first.1.try_extract_tensor::<f16>() {
        let f32_vec: Vec<f32> = data.iter().map(|v| v.to_f32()).collect();
        parse_tensor_data(&shape, &f32_vec, scale, pad_x, pad_y, orig_w, orig_h, score_thr)
    } else if let Ok((shape, data)) = first.1.try_extract_tensor::<f32>() {
        parse_tensor_data(&shape, data, scale, pad_x, pad_y, orig_w, orig_h, score_thr)
    } else {
        bail!("YOLO output tensor is neither f16 nor f32");
    }
}

fn parse_tensor_data(
    shape: &[i64],
    data: &[f32],
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    orig_w: u32,
    orig_h: u32,
    score_thr: f32,
) -> Result<Vec<Detection>> {
    // YOLO output shape is typically [1, 84, 8400] or [1, 8400, 84]
    if shape.len() != 3 {
        bail!("Unexpected YOLO output shape: {shape:?}");
    }

    let is_transposed = shape[1] == 8400;
    let (num_channels, num_anchors) = if is_transposed {
        (shape[2] as usize, shape[1] as usize)
    } else {
        (shape[1] as usize, shape[2] as usize)
    };

    if num_channels < 5 {
        bail!("YOLO channels ({num_channels}) is less than 5 (cx, cy, w, h + class_scores)");
    }
    let num_classes = num_channels - 4;

    let mut detections = Vec::new();

    for i in 0..num_anchors {
        let get_val = |ch: usize| -> f32 {
            if is_transposed {
                data[i * num_channels + ch]
            } else {
                data[ch * num_anchors + i]
            }
        };

        // Find best class score among 4..num_channels
        let mut max_score = f32::NEG_INFINITY;
        let mut best_class = 0;
        for c in 0..num_classes {
            let s = get_val(4 + c);
            if s > max_score {
                max_score = s;
                best_class = c;
            }
        }

        if max_score < score_thr {
            continue;
        }

        let cx = get_val(0);
        let cy = get_val(1);
        let bw = get_val(2);
        let bh = get_val(3);

        let x1 = (cx - bw / 2.0 - pad_x) / scale;
        let y1 = (cy - bh / 2.0 - pad_y) / scale;
        let x2 = (cx + bw / 2.0 - pad_x) / scale;
        let y2 = (cy + bh / 2.0 - pad_y) / scale;

        let bbox = [
            x1.clamp(0.0, orig_w as f32),
            y1.clamp(0.0, orig_h as f32),
            x2.clamp(0.0, orig_w as f32),
            y2.clamp(0.0, orig_h as f32),
        ];

        detections.push(Detection {
            bbox,
            score: max_score,
            class_id: best_class,
        });
    }

    Ok(detections)
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let union = area(a) + area(b) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

fn nms(mut detections: Vec<Detection>, thr: f32) -> Vec<Detection> {
    detections.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Detection> = Vec::new();
    for d in detections {
        if keep.iter().all(|k| iou(&k.bbox, &d.bbox) <= thr) {
            keep.push(d);
        }
    }
    keep
}
