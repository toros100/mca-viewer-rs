use crate::{
    List, SerializationError, Tag, arrays::LongArrayBytes, serialize_nbt_str, write_i32_len,
};

use std::result::Result;

pub trait SerializePayload {
    const TAG: Tag;
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), SerializationError>;
}

pub fn serialize_nbt<P: SerializePayload>(
    name: &str,
    payload: &P,
) -> Result<Vec<u8>, SerializationError> {
    let mut buf = Vec::<u8>::new();
    buf.push(P::TAG as u8);
    serialize_nbt_str(name, &mut buf)?;
    payload.serialize_into(&mut buf)?;
    Ok(buf)
}

impl SerializePayload for &str {
    const TAG: Tag = Tag::String;
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), SerializationError> {
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
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), SerializationError> {
        self.as_str().serialize_into(buf)
    }
}

impl<T: SerializePayload> SerializePayload for Vec<T> {
    const TAG: Tag = Tag::List;
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), SerializationError> {
        buf.push(T::TAG as u8);
        write_i32_len(self.len(), buf)?;
        for x in self.as_slice().iter() {
            x.serialize_into(buf)?;
        }
        Ok(())
    }
}

macro_rules! impl_serialize_numeric {
    ($(($t:ty, $tag:expr)),*) => {
        $(
            impl SerializePayload for $t {
                const TAG: Tag = $tag;
                fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), SerializationError> {
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
