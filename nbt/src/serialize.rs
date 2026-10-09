use crate::{
    SerializationError, SerializationResult, Tag, arrays::LongArrayBytes, serialize_nbt_str,
    write_be_i32_len,
};

use std::result::Result;

pub trait SerializePayload {
    const TAG: Tag;
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()>;
}

pub fn to_bytes<P: SerializePayload>(
    name: &str,
    payload: &P,
) -> Result<Vec<u8>, SerializationError> {
    let mut buf = Vec::<u8>::new();
    buf.push(P::TAG as u8);
    serialize_nbt_str(name, &mut buf)?;
    payload.serialize_payload(&mut buf)?;
    Ok(buf)
}

impl SerializePayload for &str {
    const TAG: Tag = Tag::String;
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
        if self.len() > u16::MAX as usize {
            return Err(SerializationError::StringTooLong);
        }
        buf.extend_from_slice(&(self.len() as u16).to_be_bytes());
        buf.extend_from_slice(self.as_bytes());
        Ok(())
    }
}
impl SerializePayload for String {
    const TAG: Tag = Tag::String;
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
        self.as_str().serialize_payload(buf)
    }
}

impl<T: SerializePayload> SerializePayload for Vec<T> {
    const TAG: Tag = Tag::List;
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
        buf.push(T::TAG as u8);
        write_be_i32_len(self.len(), buf)?;

        if let Some(s) = T::TAG.fixed_size() {
            buf.reserve(self.len() * s);
        }

        for x in self {
            x.serialize_payload(buf)?;
        }
        Ok(())
    }
}

impl<T: SerializePayload> SerializePayload for Option<T> {
    const TAG: Tag = T::TAG;
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
        // WARN: odd
        self.as_ref()
            .ok_or(SerializationError::Custom("no value in Option".into()))?
            .serialize_payload(buf)
    }
}

macro_rules! impl_serialize_numeric {
    ($(($t:ty, $tag:expr)),*) => {
        $(
            impl SerializePayload for $t {
                const TAG: Tag = $tag;
                fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
                    buf.extend_from_slice(&self.to_be_bytes());
                    Ok(())
                }
            }
        )*
    }
}

impl_serialize_numeric!(
    (i8, Tag::Byte),
    (i16, Tag::Short),
    (i32, Tag::Int),
    (i64, Tag::Long),
    (f32, Tag::Float),
    (f64, Tag::Double)
);
