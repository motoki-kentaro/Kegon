//! Settings values that tolerate hand edits: numbers that may be invalid, and keys this build does not know.
//!
//! A malformed value in `settings.toml` must never reset the other settings, and saving must never
//! rewrite what the user wrote. These types therefore keep the original TOML value verbatim and only
//! decide at use time whether it is usable.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// A numeric setting as it appears in `settings.toml`.
///
/// Anything that is not a TOML integer or float is kept as [`NumericSetting::Other`] and written back
/// unchanged. A number is kept as written even when it is not finite or outside the allowed range;
/// [`NumericSetting::effective`] decides whether it can be used.
#[derive(Debug, Clone)]
pub enum NumericSetting {
    Number(f64),
    Other(toml::Value),
}

impl NumericSetting {
    /// A value the user just committed.
    pub fn from_number(value: f64) -> Self {
        Self::Number(value)
    }

    /// The usable value: a finite number inside `range`, otherwise `None` (the setting counts as unset).
    pub fn effective(&self, range: &RangeInclusive<f64>) -> Option<f64> {
        match self {
            Self::Number(value) if value.is_finite() && range.contains(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as written in the file, for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Self::Number(value) => value.to_string(),
            Self::Other(value) => value.to_string(),
        }
    }
}

impl PartialEq for NumericSetting {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::Other(a), Self::Other(b)) => a.to_string() == b.to_string(),
            _ => false,
        }
    }
}

impl Eq for NumericSetting {}

impl Serialize for NumericSetting {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Number(value) => serializer.serialize_f64(*value),
            Self::Other(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for NumericSetting {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match toml::Value::deserialize(deserializer)? {
            toml::Value::Float(value) => Self::Number(value),
            toml::Value::Integer(value) => Self::Number(value as f64),
            other => Self::Other(other),
        })
    }
}

/// Keys of a table that this build has no field for. Kept so that saving does not drop them.
#[derive(Debug, Clone, Default)]
pub struct ExtraKeys(pub toml::Table);

impl PartialEq for ExtraKeys {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_string() == other.0.to_string()
    }
}

impl Eq for ExtraKeys {}

impl Serialize for ExtraKeys {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExtraKeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        toml::Table::deserialize(deserializer).map(Self)
    }
}
