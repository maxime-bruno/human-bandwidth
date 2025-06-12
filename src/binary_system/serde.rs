//! Serde support for humanized bandwidth in
//! [binary prefix format](https://en.wikipedia.org/wiki/Binary_prefix).
//!
//! # Example
//! ```
//! use serde::{Serialize, Deserialize};
//! use bandwidth::Bandwidth;
//!
//! #[derive(Serialize, Deserialize)]
//! struct Foo {
//!     #[serde(with = "human_bandwidth::binary_system::serde")]
//!     bandwidth: Bandwidth,
//! }
//! ```
//!
//! Or use the `Serde` wrapper type:
//!
//! ```
//! use serde::{Serialize, Deserialize};
//! use human_bandwidth::binary_system::serde::Serde;
//! use bandwidth::Bandwidth;
//!
//! #[derive(Serialize, Deserialize)]
//! struct Foo {
//!     bandwidth: Vec<Serde<Bandwidth>>,
//! }
//! ```

use bandwidth::Bandwidth;
use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::ops::{Deref, DerefMut};

/// Deserializes a `Bandwidth` in human-readable format.
///
/// This function can be used with `serde_derive`'s `with` and
/// `deserialize_with` annotations.
pub fn deserialize<'a, T, D>(d: D) -> Result<T, D::Error>
where
    Serde<T>: Deserialize<'a>,
    D: Deserializer<'a>,
{
    Serde::deserialize(d).map(Serde::into_inner)
}

/// Serializes a `Bandwidth` in human-readable format.
///
/// This function can be used with `serde_derive`'s `with` and
/// `serialize_with` annotations.
pub fn serialize<T, S>(d: &T, s: S) -> Result<S::Ok, S::Error>
where
    for<'a> Serde<&'a T>: Serialize,
    S: Serializer,
{
    Serde::from(d).serialize(s)
}

/// A wrapper type which implements `Serialize` and `Deserialize` for
/// types involving `Bandwidth`.
#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub struct Serde<T>(T);

impl<T> fmt::Debug for Serde<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.0.fmt(formatter)
    }
}

impl<T> Deref for Serde<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Serde<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Serde<T> {
    /// Consumes the `De`, returning the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for Serde<T> {
    fn from(val: T) -> Serde<T> {
        Serde(val)
    }
}

impl<'de> Deserialize<'de> for Serde<Bandwidth> {
    fn deserialize<D>(d: D) -> Result<Serde<Bandwidth>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;

        impl de::Visitor<'_> for V {
            type Value = Bandwidth;

            fn expecting(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
                fmt.write_str("a bandwidth")
            }

            fn visit_str<E>(self, v: &str) -> Result<Bandwidth, E>
            where
                E: de::Error,
            {
                super::parse_binary_bandwidth(v)
                    .map_err(|_| E::invalid_value(de::Unexpected::Str(v), &self))
            }
        }

        d.deserialize_str(V).map(Serde)
    }
}

impl<'de> Deserialize<'de> for Serde<Option<Bandwidth>> {
    fn deserialize<D>(d: D) -> Result<Serde<Option<Bandwidth>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<Serde<Bandwidth>>::deserialize(d)? {
            Some(Serde(dur)) => Ok(Serde(Some(dur))),
            None => Ok(Serde(None)),
        }
    }
}

impl ser::Serialize for Serde<&Bandwidth> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        super::format_binary_bandwidth(*self.0)
            .to_string()
            .serialize(serializer)
    }
}

impl ser::Serialize for Serde<Bandwidth> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        super::format_binary_bandwidth(self.0)
            .to_string()
            .serialize(serializer)
    }
}

impl ser::Serialize for Serde<&Option<Bandwidth>> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        match *self.0 {
            Some(dur) => serializer.serialize_some(&Serde(dur)),
            None => serializer.serialize_none(),
        }
    }
}

impl ser::Serialize for Serde<Option<Bandwidth>> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        Serde(&self.0).serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with() {
        #[derive(Serialize, Deserialize)]
        struct Foo {
            #[serde(with = "super")]
            bandwidth: Bandwidth,
        }

        let json = r#"{"bandwidth": "1KiBps"}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(foo.bandwidth, Bandwidth::from_bps(8 * 1024));
        let reverse = serde_json::to_string(&foo).unwrap();
        assert_eq!(reverse, r#"{"bandwidth":"1KiB/s"}"#);
    }

    #[test]
    fn with_option() {
        #[derive(Serialize, Deserialize)]
        struct Foo {
            #[serde(with = "super", default)]
            bandwidth: Option<Bandwidth>,
        }

        let json = r#"{"bandwidth": "15MiBps"}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(
            foo.bandwidth,
            Some(Bandwidth::from_bps(15 * 1024 * 1024 * 8))
        );
        let reverse = serde_json::to_string(&foo).unwrap();
        assert_eq!(reverse, r#"{"bandwidth":"15MiB/s"}"#);

        let json = r#"{"bandwidth": null}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(foo.bandwidth, None);
        let reverse = serde_json::to_string(&foo).unwrap();
        assert_eq!(reverse, r#"{"bandwidth":null}"#);

        let json = r#"{}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(foo.bandwidth, None);
    }
}
