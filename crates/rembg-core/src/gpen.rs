//! GPEN-BFR-512 face restoration on `ort = "=2.0.0-rc.12"`.
//!
//! Uses the same FFHQ-512 alignment and feathered paste-back as [`crate::gfpgan`]:
//!   5 landmarks -> similarity transform onto the FFHQ-512 template
//!   -> warp 512x512 -> GPEN -> inverse warp -> feathered paste-back.
//!
//! Model: GPEN-BFR-512.onnx, input 1x3x512x512, output 1x3x512x512,
//! both float32 RGB in [-1, 1].

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use image::RgbImage;
use ort::session::Session;

use crate::config::{MODEL_GPEN, SIZE_GPEN, VRAM_LIMIT_GPEN_MB, WARMUP, load_session, warmup};
use crate::gfpgan::{Landmarks, restore_faces_with, run_aligned};

pub struct Gpen {
    // Only the GPU call holds the lock; CPU pre/post-processing runs unlocked.
    session: Mutex<Session>,
}

impl Gpen {
    pub fn load() -> Result<Self> {
        Self::load_from(MODEL_GPEN)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let mut session = load_session(path, VRAM_LIMIT_GPEN_MB)?;
        if WARMUP {
            warmup(&mut session, SIZE_GPEN)?;
        }
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Full pipeline: restore every face given by `faces` and paste it back.
    /// `strength` in 0..=1 blends the restored face over the original.
    pub fn restore_faces(
        &self,
        img: &RgbImage,
        faces: &[Landmarks],
        strength: f32,
    ) -> Result<RgbImage> {
        restore_faces_with(img, faces, strength, SIZE_GPEN, |face| {
            self.restore_aligned(face)
        })
    }

    /// Run GPEN on a face that is ALREADY FFHQ-aligned (size x size).
    /// Input and output are RGB.
    pub fn restore_aligned(&self, face: &RgbImage) -> Result<RgbImage> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("GPEN session mutex poisoned"))?;
        run_aligned(&mut session, face, SIZE_GPEN, "GPEN")
    }
}
