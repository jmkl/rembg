//! Pure CPU image operations behind the generic filter nodes.
//!
//! Every function takes pixels in and returns pixels out, with no egui or graph
//! dependencies. Evaluation and caching live in [`crate::eval`].

mod adjust;
pub(crate) mod composite;
mod generate;
mod rawfilter;
mod rf;
use image::RgbaImage;

use crate::model::FilterKind;
use crate::params::Params;

pub(crate) use adjust::curve_lut;

pub(crate) const EVAL_MAX_EDGE: u32 = 256;

/// Evaluate one filter node.
///
/// `inputs` is indexed by socket; each socket holds the images connected to it
/// in wire order (only multi-input sockets can have more than one).
pub(crate) fn evaluate(
    kind: FilterKind,
    params: &Params,
    inputs: &[Vec<RgbaImage>],
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    let first = |socket: usize| -> Result<&RgbaImage, String> {
        inputs
            .get(socket)
            .and_then(|list| list.first())
            .ok_or_else(|| "No input connected".to_owned())
    };

    match kind {
        FilterKind::Route => Ok(first(0)?.clone()),
        FilterKind::Group => composite::stack(inputs.first().map(Vec::as_slice).unwrap_or(&[])),
        FilterKind::Blend => composite::blend(
            inputs.first().and_then(|list| list.first()),
            inputs.get(1).and_then(|list| list.first()),
            params,
            canvas,
        ),
        FilterKind::Mask => Ok(composite::apply_mask(
            first(0)?,
            inputs.get(1).and_then(|list| list.first()),
            params,
        )),
        FilterKind::Hsv => Ok(adjust::hsv(first(0)?, params)),
        FilterKind::Curves => Ok(adjust::curves(first(0)?, params)),
        FilterKind::Exposure => Ok(adjust::exposure(first(0)?, params)),
        FilterKind::ColorBalance => Ok(adjust::color_balance(first(0)?, params)),
        FilterKind::GradientMap => Ok(adjust::gradient_map(first(0)?, params)),
        FilterKind::Invert => Ok(adjust::invert(first(0)?, params)),
        FilterKind::Gradient => Ok(generate::gradient(params)),
        FilterKind::Noise => Ok(generate::noise(params, canvas)),
        FilterKind::RawFilter => Ok(rawfilter::raw_filter(first(0)?, params)),
    }
}
