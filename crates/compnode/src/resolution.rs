//! Shared preview resolution policies used by graph evaluation and preview
//! caches. Full-resolution worker renders do not use these limits, and Texts
//! previews remain at document-canvas resolution for accurate canvas-space
//! alpha bounds.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)] // Draft and High are exposed for preview-quality controls.
pub(crate) enum PreviewQuality {
    Draft,
    Normal,
    High,
}

impl PreviewQuality {
    pub(crate) const fn max_edge(self) -> u32 {
        match self {
            Self::Draft => 128,
            Self::Normal => NORMAL_PREVIEW_MAX_EDGE,
            Self::High => 512,
        }
    }

    pub(crate) fn dimensions(self, dimensions: (u32, u32)) -> (u32, u32) {
        let (width, height) = dimensions;
        let longest = width.max(height);
        let edge = self.max_edge();
        if longest <= edge {
            return (width, height);
        }
        let scale = edge as f64 / longest as f64;
        (
            ((width as f64 * scale).round() as u32).max(1),
            ((height as f64 * scale).round() as u32).max(1),
        )
    }
}

pub(crate) const NORMAL_PREVIEW_MAX_EDGE: u32 = 256;
