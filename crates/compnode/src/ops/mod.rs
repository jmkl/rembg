//! Pure CPU image operations behind the generic filter nodes.
//!
//! Every function takes pixels in and returns pixels out, with no egui or graph
//! dependencies. Evaluation and caching live in [`crate::eval`].

pub(crate) mod color;
pub(crate) mod composite;
pub(crate) mod effects;
pub(crate) mod generators;
pub(crate) mod math;
pub(crate) mod rf;

// Keep established internal paths stable while callers migrate to domains.
pub(crate) use color::adjust;
pub(crate) use generators::generate;
pub(crate) use generators::text as texts;
use image::RgbaImage;

use crate::catalog;
use crate::model::{FilterKind, Kind};
use crate::params::Params;

pub(crate) use color::adjust::curve_lut;

pub(crate) fn primary_input(inputs: &[Vec<RgbaImage>]) -> Result<&RgbaImage, String> {
    inputs
        .first()
        .and_then(|list| list.first())
        .ok_or_else(|| "No input connected".to_owned())
}

/// Evaluate one filter node via its specification's `eval` function.
pub(crate) fn evaluate(
    kind: FilterKind,
    params: &Params,
    inputs: &[Vec<RgbaImage>],
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    let spec = catalog::spec(Kind::Filter(kind));
    let eval_fn = spec
        .eval
        .ok_or_else(|| format!("Filter {:?} has no eval function", kind))?;
    eval_fn(inputs, params, canvas)
}
