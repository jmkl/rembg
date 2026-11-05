//! Socket values and socket types, with the conversion/compatibility rules.
use compnode_core::Color;

/// Opaque id of an image owned by the host (a texture, a thumbnail, ...).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ImageHandle(pub u64);

/// A value stored in (or flowing through) a socket.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vec3([f32; 3]),
    /// Linear-light, straight alpha.
    Color(Color),
    String(String),
    /// Index into the socket's option list.
    Enum(usize),
    /// `None` means "no image yet".
    Image(Option<ImageHandle>),
}

/// The type of a socket. Drives pin/wire color and connection rules.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SocketType {
    Float,
    Int,
    Bool,
    Vec3,
    Color,
    String,
    Enum,
    Image,
    /// Wildcard used by reroute nodes: connects to and from anything.
    Any,
}

impl SocketType {
    pub const ALL: [SocketType; 9] = [
        SocketType::Float,
        SocketType::Int,
        SocketType::Bool,
        SocketType::Vec3,
        SocketType::Color,
        SocketType::String,
        SocketType::Enum,
        SocketType::Image,
        SocketType::Any,
    ];

    /// Stable index into per-type tables (themes, ...).
    pub fn index(self) -> usize {
        match self {
            SocketType::Float => 0,
            SocketType::Int => 1,
            SocketType::Bool => 2,
            SocketType::Vec3 => 3,
            SocketType::Color => 4,
            SocketType::String => 5,
            SocketType::Enum => 6,
            SocketType::Image => 7,
            SocketType::Any => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SocketType::Float => "float",
            SocketType::Int => "int",
            SocketType::Bool => "bool",
            SocketType::Vec3 => "vec3",
            SocketType::Color => "color",
            SocketType::String => "string",
            SocketType::Enum => "enum",
            SocketType::Image => "image",
            SocketType::Any => "any",
        }
    }

    pub fn from_name(s: &str) -> Option<SocketType> {
        SocketType::ALL.into_iter().find(|t| t.name() == s)
    }

    /// May an output of type `from` feed an input of type `to`?
    pub fn can_connect(from: SocketType, to: SocketType) -> bool {
        use SocketType::*;
        if from == to || from == Any || to == Any {
            return true;
        }
        matches!(
            (from, to),
            (Int, Float)
                | (Float, Int)
                | (Bool, Int)
                | (Bool, Float)
                | (Float, Bool)
                | (Int, Bool)
                | (Float, Color)
                | (Color, Float)
                | (Vec3, Color)
                | (Color, Vec3)
                | (Enum, Int)
        )
    }

    pub fn default_value(self) -> Value {
        match self {
            SocketType::Float | SocketType::Any => Value::Float(0.0),
            SocketType::Int => Value::Int(0),
            SocketType::Bool => Value::Bool(false),
            SocketType::Vec3 => Value::Vec3([0.0; 3]),
            SocketType::Color => Value::Color(Color::BLACK),
            SocketType::String => Value::String(String::new()),
            SocketType::Enum => Value::Enum(0),
            SocketType::Image => Value::Image(None),
        }
    }
}

impl Value {
    pub fn socket_type(&self) -> SocketType {
        match self {
            Value::Float(_) => SocketType::Float,
            Value::Int(_) => SocketType::Int,
            Value::Bool(_) => SocketType::Bool,
            Value::Vec3(_) => SocketType::Vec3,
            Value::Color(_) => SocketType::Color,
            Value::String(_) => SocketType::String,
            Value::Enum(_) => SocketType::Enum,
            Value::Image(_) => SocketType::Image,
        }
    }

    /// Convert to another socket type following [`SocketType::can_connect`].
    /// Converting to [`SocketType::Any`] keeps the value as it is.
    pub fn convert(&self, to: SocketType) -> Option<Value> {
        if to == SocketType::Any || self.socket_type() == to {
            return Some(self.clone());
        }
        let v = match (self, to) {
            (Value::Int(i), SocketType::Float) => Value::Float(*i as f32),
            (Value::Float(f), SocketType::Int) => Value::Int(f.round() as i32),
            (Value::Bool(b), SocketType::Int) => Value::Int(i32::from(*b)),
            (Value::Bool(b), SocketType::Float) => Value::Float(if *b { 1.0 } else { 0.0 }),
            (Value::Float(f), SocketType::Bool) => Value::Bool(*f != 0.0),
            (Value::Int(i), SocketType::Bool) => Value::Bool(*i != 0),
            (Value::Float(f), SocketType::Color) => Value::Color(Color::rgb(*f, *f, *f)),
            (Value::Color(c), SocketType::Float) => {
                Value::Float(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b)
            }
            (Value::Vec3(v), SocketType::Color) => Value::Color(Color::rgb(v[0], v[1], v[2])),
            (Value::Color(c), SocketType::Vec3) => Value::Vec3([c.r, c.g, c.b]),
            (Value::Enum(e), SocketType::Int) => Value::Int(*e as i32),
            _ => return None,
        };
        Some(v)
    }

    /// Numeric view (Float, Int, Bool).
    pub fn as_f32(&self) -> Option<f32> {
        match self.convert(SocketType::Float)? {
            Value::Float(f) => Some(f),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_agrees_with_compatibility_table() {
        for from in SocketType::ALL {
            if from == SocketType::Any {
                continue;
            }
            for to in SocketType::ALL {
                let v = from.default_value();
                assert_eq!(
                    v.convert(to).is_some(),
                    SocketType::can_connect(from, to),
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn numeric_conversions() {
        assert_eq!(
            Value::Int(3).convert(SocketType::Float),
            Some(Value::Float(3.0))
        );
        assert_eq!(
            Value::Float(2.6).convert(SocketType::Int),
            Some(Value::Int(3))
        );
        assert_eq!(Value::Bool(true).as_f32(), Some(1.0));
        assert_eq!(Value::String("x".into()).convert(SocketType::Float), None);
        let gray = Value::Float(0.5).convert(SocketType::Color).unwrap();
        assert_eq!(gray, Value::Color(Color::rgb(0.5, 0.5, 0.5)));
        let l = Value::Color(Color::WHITE).as_f32().unwrap();
        assert!((l - 1.0).abs() < 1e-5);
    }

    #[test]
    fn any_connects_everywhere_and_names_roundtrip() {
        for t in SocketType::ALL {
            assert!(SocketType::can_connect(SocketType::Any, t));
            assert!(SocketType::can_connect(t, SocketType::Any));
            assert_eq!(SocketType::from_name(t.name()), Some(t));
            assert_eq!(SocketType::ALL[t.index()], t);
        }
        assert!(!SocketType::can_connect(
            SocketType::Image,
            SocketType::Float
        ));
    }
}
