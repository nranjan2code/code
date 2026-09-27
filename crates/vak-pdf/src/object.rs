//! The PDF object model (ISO 32000-2 §7.3), owned and bounded.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    String(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(u32, u16),
}

/// A dictionary in file order. Lookups take the first entry for a key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dict(pub Vec<(Vec<u8>, Object)>);

/// A stream's dictionary and its still-encoded bytes, as a range of the
/// file: a stream never lives inside an object stream, so its data is
/// always in the file itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Range<usize>,
}

pub(crate) static NULL: Object = Object::Null;

impl Dict {
    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn has(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &Object)> {
        self.0.iter().map(|(name, value)| (name.as_slice(), value))
    }

    /// The name stored under `key` when it is a direct name.
    pub fn name(&self, key: &[u8]) -> Option<&[u8]> {
        self.get(key).and_then(Object::as_name)
    }
}

impl Object {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Object::Int(value) => Some(*value),
            Object::Real(value) if value.is_finite() && value.fract() == 0.0 => Some(*value as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Int(value) => Some(*value as f64),
            Object::Real(value) if value.is_finite() => Some(*value),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(name) => Some(name),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Object::String(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(items) => Some(items),
            _ => None,
        }
    }

    /// A dictionary, or a stream's dictionary.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(dict) => Some(dict),
            Object::Stream(stream) => Some(&stream.dict),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(stream) => Some(stream),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<(u32, u16)> {
        match self {
            Object::Ref(number, generation) => Some((*number, *generation)),
            _ => None,
        }
    }
}
