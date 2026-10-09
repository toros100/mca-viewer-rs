use crate::{
    DeserializationResult, DeserializePayload, SerializationResult, SerializePayload, Tag,
    deserialize_nbt_str_bytes, serialize_nbt_str, serialize_nbt_str_bytes,
};

/// raw nbt string bytes, not checked for utf8
/// (which is surprisingly expensive with the std implementation)
#[derive(Debug, Default)]
pub struct RawString<'a>(&'a [u8]);

impl<'a> RawString<'a> {
    // getting lifetime issues with the deref
    #[inline]
    pub fn inner_lifetimed(&self) -> &'a [u8] {
        self.0
    }
}

impl<'a> std::ops::Deref for RawString<'a> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'d> DeserializePayload<'d> for RawString<'d> {
    const TAG: Tag = Tag::String;
    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let (b, off) = deserialize_nbt_str_bytes(data)?;
        self.0 = b;
        Ok(off)
    }
}

impl<'d> SerializePayload for RawString<'d> {
    const TAG: Tag = Tag::String;
    #[inline]
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
        serialize_nbt_str_bytes(self.0, buf)
    }
}
