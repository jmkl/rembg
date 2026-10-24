//! SCRFD (insightface det_10g.onnx) face detector -> 5 landmarks for GFPGAN.
//! ort = "=2.0.0-rc.12"
//!
//! Model: det_10g.onnx from insightface's buffalo_l pack, input 1x3x640x640,
//! 9 outputs (scores / boxes / keypoints for strides 8, 16, 32, 2 anchors each).
//! Outputs are matched by shape rather than position, so export order
//! differences don't matter.

use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use image::{
    RgbImage,
    imageops::{self, FilterType},
};
use ort::{session::Session, value::Tensor};

// Adjust paths/names to your project. Define MODEL_SCRFD / VRAM_LIMIT_SCRFD_MB yourself.
use crate::{MODEL_SCRFD, VRAM_LIMIT_SCRFD_MB, gfpgan::Landmarks, load_session};

const DET_SIZE: u32 = 640;
const STRIDES: [usize; 3] = [8, 16, 32];
const NUM_ANCHORS: usize = 2;

#[derive(Clone, Debug)]
pub struct Face {
    /// x1, y1, x2, y2 in source-image pixels
    pub bbox: [f32; 4],
    pub score: f32,
    pub landmarks: Landmarks,
}

pub struct FaceDetector {
    session: Mutex<Session>,
}

impl FaceDetector {
    pub fn load() -> Result<Self> {
        let session = load_session(MODEL_SCRFD, VRAM_LIMIT_SCRFD_MB)?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Detect faces, best score first. Typical: `detect(&img, 0.5, 0.4)`.
    pub fn detect(&self, img: &RgbImage, score_thr: f32, nms_thr: f32) -> Result<Vec<Face>> {
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Ok(vec![]);
        }

        // Letterbox: keep aspect, top-left on a DET_SIZE canvas.
        let scale = (DET_SIZE as f32 / w as f32).min(DET_SIZE as f32 / h as f32);
        let nw = ((w as f32 * scale).round() as u32).clamp(1, DET_SIZE);
        let nh = ((h as f32 * scale).round() as u32).clamp(1, DET_SIZE);
        let resized = imageops::resize(img, nw, nh, FilterType::Triangle);

        // RGB, (v - 127.5) / 128. Padding is a black pixel, normalised like the rest.
        let n = (DET_SIZE * DET_SIZE) as usize;
        let mut data = vec![-127.5f32 / 128.0; 3 * n];
        for (x, y, p) in resized.enumerate_pixels() {
            let i = (y * DET_SIZE + x) as usize;
            for c in 0..3 {
                data[c * n + i] = (p[c] as f32 - 127.5) / 128.0;
            }
        }
        let input = Tensor::from_array(([1usize, 3, DET_SIZE as usize, DET_SIZE as usize], data))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("SCRFD session mutex poisoned"))?;
        let outputs = session.run(ort::inputs![input])?;

        // tbl[kind][stride_idx]; kind 0 = scores, 1 = boxes, 2 = keypoints.
        let mut tbl: [[Option<&[f32]>; 3]; 3] = [[None; 3]; 3];
        for i in 0..outputs.len() {
            let (shape, d) = outputs[i].try_extract_tensor::<f32>()?;
            let last = match shape.last() {
                Some(&l) if l > 0 => l as usize,
                _ => continue,
            };
            let kind = match last {
                1 => 0,
                4 => 1,
                10 => 2,
                _ => continue,
            };
            let rows = d.len() / last;
            let found = STRIDES.iter().position(|&s| {
                let fm = DET_SIZE as usize / s;
                fm * fm * NUM_ANCHORS == rows
            });
            if let Some(si) = found {
                tbl[kind][si] = Some(d);
            }
        }

        let mut faces: Vec<Face> = Vec::new();
        for (si, &stride) in STRIDES.iter().enumerate() {
            let (Some(scores), Some(boxes), Some(kps)) = (tbl[0][si], tbl[1][si], tbl[2][si])
            else {
                bail!(
                    "SCRFD output for stride {stride} missing; is this det_10g.onnx with keypoints?"
                );
            };
            let fm = DET_SIZE as usize / stride;
            let st = stride as f32;

            for i in 0..fm * fm * NUM_ANCHORS {
                let score = scores[i];
                if score < score_thr {
                    continue;
                }
                let cell = i / NUM_ANCHORS; // each grid cell has 2 consecutive anchors
                let cx = ((cell % fm) * stride) as f32;
                let cy = ((cell / fm) * stride) as f32;

                let b = &boxes[i * 4..i * 4 + 4]; // distances l, t, r, b in stride units
                let bbox = [
                    (cx - b[0] * st) / scale,
                    (cy - b[1] * st) / scale,
                    (cx + b[2] * st) / scale,
                    (cy + b[3] * st) / scale,
                ];

                let k = &kps[i * 10..i * 10 + 10];
                let mut landmarks = [[0f32; 2]; 5];
                for j in 0..5 {
                    landmarks[j] = [
                        (cx + k[2 * j] * st) / scale,
                        (cy + k[2 * j + 1] * st) / scale,
                    ];
                }
                faces.push(Face {
                    bbox,
                    score,
                    landmarks,
                });
            }
        }
        drop(outputs);
        drop(session);

        Ok(nms(faces, nms_thr))
    }
}

/// facexlib's `only_center_face`: the face whose box centre is closest to the image centre.
pub fn pick_center(faces: Vec<Face>, w: u32, h: u32) -> Vec<Face> {
    let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
    let dist = |f: &Face| {
        let cx = (f.bbox[0] + f.bbox[2]) / 2.0 - mx;
        let cy = (f.bbox[1] + f.bbox[3]) / 2.0 - my;
        cx * cx + cy * cy
    };
    faces
        .into_iter()
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .into_iter()
        .collect()
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let union = area(a) + area(b) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

fn nms(mut faces: Vec<Face>, thr: f32) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Face> = Vec::new();
    for f in faces {
        if keep.iter().all(|k| iou(&k.bbox, &f.bbox) <= thr) {
            keep.push(f);
        }
    }
    keep
}
