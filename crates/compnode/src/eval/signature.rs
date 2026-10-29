//! Pure hashing for graph evaluation cache keys.

use std::hash::{Hash, Hasher};

use crate::model::Kind;
use crate::params::Params;

use super::PreviewQuality;

#[derive(Clone, Copy)]
pub(super) struct LayerSignature {
    pub source_id: usize,
    pub source_signature: u64,
    pub center_x: u32,
    pub center_y: u32,
    pub size_x: u32,
    pub size_y: u32,
    pub angle: u32,
}

/// Combine all cache-relevant values in the same order used by the original
/// evaluator. Keeping this pure makes signature behavior directly testable.
pub(super) fn compute(
    kind: Kind,
    params: &Params,
    canvas: (u32, u32),
    quality: PreviewQuality,
    layers: &[LayerSignature],
    inputs: &[u64],
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    if let Ok(serialized) = serde_json::to_string(params) {
        serialized.hash(&mut hasher);
    }
    canvas.hash(&mut hasher);
    quality.hash(&mut hasher);
    for layer in layers {
        layer.source_id.hash(&mut hasher);
        layer.source_signature.hash(&mut hasher);
        layer.center_x.hash(&mut hasher);
        layer.center_y.hash(&mut hasher);
        layer.size_x.hash(&mut hasher);
        layer.size_y.hash(&mut hasher);
        layer.angle.hash(&mut hasher);
    }
    for input in inputs {
        input.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn signature(canvas: (u32, u32), inputs: &[u64]) -> u64 {
        let params: Params = [("amount".to_owned(), json!(25.0))].into_iter().collect();
        compute(
            Kind::Filter(crate::model::FilterKind::Invert),
            &params,
            canvas,
            PreviewQuality::Normal,
            &[],
            inputs,
        )
    }

    #[test]
    fn signature_is_deterministic_and_tracks_canvas_and_inputs() {
        assert_eq!(
            signature((640, 480), &[7, 11]),
            signature((640, 480), &[7, 11])
        );
        assert_ne!(
            signature((640, 480), &[7, 11]),
            signature((641, 480), &[7, 11])
        );
        assert_ne!(
            signature((640, 480), &[7, 11]),
            signature((640, 480), &[7, 12])
        );
    }

    #[test]
    fn signature_tracks_params_and_layer_transforms() {
        let params = Params::new();
        let mut changed_params = Params::new();
        changed_params.insert("amount".into(), json!(25.0));
        let layer = LayerSignature {
            source_id: 2,
            source_signature: 17,
            center_x: 10.0_f32.to_bits(),
            center_y: 20.0_f32.to_bits(),
            size_x: 30.0_f32.to_bits(),
            size_y: 40.0_f32.to_bits(),
            angle: 0.0_f32.to_bits(),
        };
        let base = compute(
            Kind::Group,
            &params,
            (640, 480),
            PreviewQuality::Normal,
            &[layer],
            &[],
        );
        let moved = compute(
            Kind::Group,
            &params,
            (640, 480),
            PreviewQuality::Normal,
            &[LayerSignature {
                center_x: 11.0_f32.to_bits(),
                ..layer
            }],
            &[],
        );
        let changed_params = compute(
            Kind::Group,
            &changed_params,
            (640, 480),
            PreviewQuality::Normal,
            &[layer],
            &[],
        );
        assert_ne!(base, moved);
        assert_ne!(base, changed_params);
    }
}
