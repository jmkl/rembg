//! Untyped parameter bag used by the generic filter nodes.
//!
//! The frontend stores node parameters as a plain object (`Record<string, any>`),
//! and the same approach keeps twelve very different node types from needing
//! twelve near-identical Rust structs. Values are read through the typed
//! helpers below, so a missing or malformed value falls back to the field's
//! default instead of panicking.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub(crate) type Params = Map<String, Value>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

pub(crate) fn f32(params: &Params, key: &str, default: f32) -> f32 {
    params
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(default)
}

pub(crate) fn u32(params: &Params, key: &str, default: u32) -> u32 {
    params
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value.max(0.0) as u32)
        .unwrap_or(default)
}

pub(crate) fn string<'a>(params: &'a Params, key: &str, default: &'a str) -> &'a str {
    params.get(key).and_then(Value::as_str).unwrap_or(default)
}

pub(crate) fn bool(params: &Params, key: &str, default: bool) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(crate) fn set_f32(params: &mut Params, key: &str, value: f32) {
    params.insert(key.to_owned(), json!(value));
}

pub(crate) fn set_bool(params: &mut Params, key: &str, value: bool) {
    params.insert(key.to_owned(), json!(value));
}

pub(crate) fn set_u32(params: &mut Params, key: &str, value: u32) {
    params.insert(key.to_owned(), json!(value));
}

pub(crate) fn set_string(params: &mut Params, key: &str, value: impl Into<String>) {
    params.insert(key.to_owned(), json!(value.into()));
}

/// `#rrggbb` → RGB. Invalid values fall back to black, matching the frontend.
pub(crate) fn hex_rgb(value: &str) -> [u8; 3] {
    let trimmed = value.trim().trim_start_matches('#');
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return [0, 0, 0];
    }
    let n = u32::from_str_radix(trimmed, 16).unwrap_or(0);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

pub(crate) fn rgb_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

pub(crate) fn color(params: &Params, key: &str, default: &str) -> [u8; 3] {
    hex_rgb(string(params, key, default))
}

/// Read one channel of the Curves node. Falls back to the identity curve.
pub(crate) fn curve(params: &Params, channel: &str) -> Vec<CurvePoint> {
    let points = params
        .get("curves")
        .and_then(Value::as_object)
        .and_then(|curves| curves.get(channel))
        .and_then(Value::as_array)
        .map(|points| {
            points
                .iter()
                .filter_map(|point| serde_json::from_value::<CurvePoint>(point.clone()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if points.len() < 2 {
        identity_curve()
    } else {
        points
    }
}

pub(crate) fn set_curve(params: &mut Params, channel: &str, points: &[CurvePoint]) {
    let Some(curves) = params
        .entry("curves")
        .or_insert_with(|| Value::Object(Params::new()))
        .as_object_mut()
    else {
        return;
    };
    curves.insert(
        channel.to_owned(),
        serde_json::to_value(points).unwrap_or(Value::Null),
    );
}

pub(crate) fn identity_curve() -> Vec<CurvePoint> {
    vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }]
}

/// JSON value for the default identity curve, used when a node is created.
pub(crate) fn identity_curve_value() -> Value {
    serde_json::to_value(identity_curve()).unwrap_or(Value::Null)
}
