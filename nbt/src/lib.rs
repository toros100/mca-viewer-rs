#![allow(unused)]

use std::{default, fmt::Debug, io::Write, marker::PhantomData, ops::Deref};
pub mod arrays;
mod de_std_impls;
mod error;
mod helper;
mod serialize;
pub use helper::*;
pub use serialize::*;
mod deserialize;
pub use deserialize::*;
mod unsafe_de_reuse;
pub use unsafe_de_reuse::*;

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
    const fn fixed_size(&self) -> Option<usize> {
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
    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
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

pub fn from_bytes<'d, T: DeserializePayload<'d> + Default>(
    data: &'d [u8],
) -> DeserializationResult<(&'d str, T, usize)> {
    let t = T::default();

    if data.len() < 4 {
        // the shortest possible fully formed tag is a byte tag with:
        // 1 byte Tag::Byte, 2 bytes name length (min. 0), 1 byte payload
        // thus 4 total
        return Err(DeserializationError::EOF);
    };

    let tag = Tag::try_from(data[0])?;

    if tag == Tag::End {
        return Err(DeserializationError::UnexpectedTag(tag as u8));
    };

    let mut off = 1;
    let (name, k) = deserialize_nbt_str(&data[off..])?;
    off += k;

    let mut payload = T::default();

    off += payload.deserialize_payload(&data[off..])?;

    Ok((name, payload, off))
}

/// any type implementing this trait will implement DeserializePayload via a blanket implementation,
/// which will expect exactly the specified tag TAG and payload bytes BYTES. if the found payload
/// bytes are different from BYTES, then DeserializationError::Custom containing ERR_MSG will
/// be returned.
///
/// example:
/// ```
/// #[derive(Default)]
/// pub struct StatusMinecraftFull;
///
/// impl nbt::ConstBytes for StatusMinecraftFull {
///     const TAG: nbt::Tag = nbt::Tag::String;
///     const BYTES: &'static [u8] = b"\x00\x0eminecraft:full";
///     const ERR_MSG: Option<&'static str> = Some("status not minecraft:full");
/// }
/// ```
pub trait ConstBytes {
    const TAG: Tag;
    const BYTES: &'static [u8];
    const ERR_MSG: Option<&'static str> = None;
}

impl<'d, C: ConstBytes + Default> DeserializePayload<'d> for C {
    const TAG: Tag = C::TAG;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        if data.len() < C::BYTES.len() {
            return Err(DeserializationError::EOF);
        };

        if C::BYTES == &data[0..C::BYTES.len()] {
            Ok(C::BYTES.len())
        } else {
            Err(DeserializationError::Custom(
                C::ERR_MSG.unwrap_or("ConstBytes mismatch").into(),
            ))
        }
    }
}

/// raw nbt string bytes, not checked for utf8
/// (which is surprisingly expensive with the std implementation)
#[derive(Default, Debug)]
pub struct RawString<'a>(&'a [u8]);

impl<'a> RawString<'a> {
    // getting lifetime issues with the deref
    pub fn inner_lifetimed(&self) -> &'a [u8] {
        self.0
    }
}

impl<'a> Deref for RawString<'a> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

unsafe impl<'d> DeserializeReuse for RawString<'d> {
    type Borrow<'b> = RawString<'b>;
}

impl<'d> DeserializePayload<'d> for RawString<'d> {
    const TAG: Tag = Tag::String;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        if data.len() < 2 {
            return Err(DeserializationError::EOF);
        }
        let l = u16::from_be_bytes(data[0..2].try_into().unwrap()) as usize;

        if data.len() < l + 2 {
            return Err(DeserializationError::EOF);
        }

        self.0 = &data[2..l + 2];

        Ok(l + 2)
    }
}
