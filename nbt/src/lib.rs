#![allow(unused)]
extern crate self as nbt;

use std::{default, fmt::Debug, io::Write, marker::PhantomData, ops::Deref};
pub mod arrays;
mod error;
mod helper;
mod serialize;
pub use helper::*;
pub use serialize::*;
mod deserialize;
pub use deserialize::*;
mod const_bytes;
pub use const_bytes::*;
mod raw_str;
pub use raw_str::*;

pub use error::*;

pub use nbt_derive::DeserializePayload;
pub use nbt_derive::SerializePayload;

#[derive(PartialEq, Debug, Clone, Copy)]
#[repr(u8)]
pub enum Tag {
    End = 0,
    Byte = 1,
    Short = 2,
    Int = 3,
    Long = 4,
    Float = 5,
    Double = 6,
    ByteArray = 7,
    String = 8,
    List = 9,
    Compound = 10,
    IntArray = 11,
    LongArray = 12,
}

impl Tag {
    #[inline]
    const fn fixed_size(self) -> Option<usize> {
        match self {
            Tag::Byte => Some(1),
            Tag::Short => Some(2),
            Tag::Int => Some(4),
            Tag::Long => Some(8),
            Tag::Float => Some(4),
            Tag::Double => Some(8),
            _ => None,
        }
    }
}

impl TryFrom<u8> for Tag {
    type Error = crate::DeserializationError;
    #[inline]
    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        // TODO: investigate whether i can do some unhinged optimization here
        match value {
            0 => Ok(Tag::End),
            1 => Ok(Tag::Byte),
            2 => Ok(Tag::Short),
            3 => Ok(Tag::Int),
            4 => Ok(Tag::Long),
            5 => Ok(Tag::Float),
            6 => Ok(Tag::Double),
            7 => Ok(Tag::ByteArray),
            8 => Ok(Tag::String),
            9 => Ok(Tag::List),
            10 => Ok(Tag::Compound),
            11 => Ok(Tag::IntArray),
            12 => Ok(Tag::LongArray),
            v => Err(DeserializationError::UnexpectedTag(v)),
        }
    }
}

pub trait DeserializeExt<'d>: DeserializePayload<'d> + Sized {
    fn try_deserialize_into(
        mut self,
        data: &'d [u8],
    ) -> (Self, Result<(&'d str, usize), DeserializationError>) {
        // would maybe be nicer if the Self was "inside" the result variants (the way you can get the sent
        // value back on a TrySendError on channels)
        //
        // but here, you want to get the value back in any case, so idk what the most ergonomic API
        // would be. this seems annoying (can't use ?), but pretty much always having to match Ok and
        // Err to retrieve the Self also seems annoying? maybe getting the Self out of the tuples .0
        // unconditionally, and then matching on .1 is fine?
        let r = try_deserialize_into_inner(&mut self, data);
        (self, r)
    }
}

impl<'d, T: DeserializePayload<'d>> DeserializeExt<'d> for T {}

fn try_deserialize_into_inner<'d, T: DeserializePayload<'d>>(
    t: &mut T,
    data: &'d [u8],
) -> DeserializationResult<(&'d str, usize)> {
    if data.len() < 4 {
        // the shortest possible fully formed tag is a byte tag with:
        // 1 byte Tag::Byte, 2 bytes name length (min. 0), 1 byte payload
        // thus 4 total
        return Err(DeserializationError::EOF);
    };

    let tag = Tag::try_from(data[0])?;

    if tag != T::TAG {
        return Err(DeserializationError::UnexpectedTag(tag as u8));
    };

    let mut off = 1;
    let (name, k) = deserialize_nbt_str(&data[off..])?;
    off += k;

    off += t.deserialize_payload(&data[off..])?;

    Ok((name, off))
}

pub fn from_bytes<'d, T: DeserializePayload<'d> + Default>(
    data: &'d [u8],
) -> DeserializationResult<(&'d str, T, usize)> {
    if data.len() < 4 {
        // the shortest possible fully formed tag is a byte tag with:
        // 1 byte Tag::Byte, 2 bytes name length (min. 0), 1 byte payload
        // thus 4 total
        return Err(DeserializationError::EOF);
    };

    let tag = Tag::try_from(data[0])?;

    if tag != T::TAG {
        return Err(DeserializationError::UnexpectedTag(tag as u8));
    };

    let mut off = 1;
    let (name, k) = deserialize_nbt_str(&data[off..])?;
    off += k;

    let mut payload = T::default();

    off += payload.deserialize_payload(&data[off..])?;

    Ok((name, payload, off))
}

/// required trait for optional fields (with attribute #[nbt(optional)]). failure to implement
/// this trait on a field that is marked with #[nbt(optional)] will be a compile error.
pub trait Optional {
    /// will be called to set the value on an optional field, if no value was deserialized.
    fn handle_absent(&mut self);
    /// will be called before during. the entire field (type tag, name and payload) will
    /// only be serialized if is_present returns true.
    fn is_present(&self) -> bool;
}

impl<T> Optional for Option<T> {
    #[inline]
    fn handle_absent(&mut self) {
        *self = None
    }
    fn is_present(&self) -> bool {
        self.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_helper<T>(root_name: &str, t: T)
    where
        T: SerializePayload + for<'d> DeserializePayload<'d> + Default + PartialEq + Debug,
    {
        let serialized = nbt::to_bytes(root_name, &t).expect("should serialize without error");

        let (root_name_de, t_de, bytes_consumed) =
            nbt::from_bytes::<T>(&serialized).expect("should deserialize without error");

        assert_eq!(root_name, root_name_de);
        assert_eq!(serialized.len(), bytes_consumed);
        assert_eq!(t, t_de);
    }

    #[test]
    fn simple() {
        #[derive(SerializePayload, DeserializePayload, Default, PartialEq, Debug)]
        struct Data {
            byte: i8,
            short: i16,
            int: i32,
            long: i64,
            float: f32,
            double: f64,
            string: String,
        }

        let data = Data {
            byte: 0,
            short: 1,
            int: 3,
            long: 4,
            float: 4.0,
            double: 5.0,
            string: "6".into(),
        };
        roundtrip_helper("root", data);
    }

    #[test]
    fn nested() {
        #[derive(SerializePayload, DeserializePayload, Default, PartialEq, Debug)]
        struct Outer {
            inner: Inner,
        }
        #[derive(SerializePayload, DeserializePayload, Default, PartialEq, Debug)]
        struct Inner {
            a: i32,
        }

        let outer = Outer {
            inner: Inner { a: 17 },
        };

        roundtrip_helper("", outer);
    }

    #[test]
    fn optional() {
        #[derive(SerializePayload, DeserializePayload, Default, PartialEq, Debug, Clone)]
        struct Data {
            #[nbt(optional)]
            some_field: Option<i32>,
            other_field: String,
        }

        let mut data = Data {
            some_field: Some(17),
            other_field: "abc".into(),
        };
        roundtrip_helper("root", data.clone());

        data.some_field = None;
        roundtrip_helper("root", data.clone());

        let bs = nbt::to_bytes("root", &data).unwrap();

        // same as Data, except some_field is not actually marked as optional
        #[derive(DeserializePayload, Default, PartialEq, Debug, Clone)]
        struct Data2 {
            some_field: Option<i32>,
            other_field: String,
        }

        let r = nbt::from_bytes::<Data2>(bs.as_slice());
        std::assert_matches!(r, Err(nbt::DeserializationError::FieldNotFound(_)));

        // yes this is a bit confusing, not a huge fan
        // but i don't think there really is a clean way of detecting particular types in a proc
        // macro? so you are kind of forced to use an extra attribute to mark optional fields, but
        // then you can have a field that uses Option but is not really optional semantically during
        // deserialization
        //
        // the whole serialization side of this is a bit of an afterthought, i just wanted fast
        // deserialization
    }

    #[test]
    fn rename() {
        #[derive(DeserializePayload, SerializePayload, Debug, Default, PartialEq)]
        struct Meme {
            #[nbt(rename = "cool")]
            very: i32,
        }

        let m = Meme { very: 0 };

        let d = nbt::to_bytes("", &m).unwrap();

        let expected = b"\x0A\x00\x00\x03\x00\x04cool";
        assert!(d.len() > expected.len());

        assert_eq!(&d[..expected.len()], expected);

        roundtrip_helper("", m);
    }
}
