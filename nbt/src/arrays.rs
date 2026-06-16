use crate::{
    DeserializationError, DeserializationResult, DeserializePayload, DeserializeReuse, Tag,
    read_i32_len,
};

#[derive(Debug, Default, Clone)]
pub struct LongArrayBytes<'a> {
    bytes: &'a [u8],
}

impl<'d> DeserializePayload<'d> for LongArrayBytes<'d> {
    const TAG: Tag = Tag::LongArray;

    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let l = read_i32_len(data)?;

        let byte_size = l * 8;

        if data.len() < byte_size + 4 {
            return Err(DeserializationError::EOF);
        };
        self.bytes = &data[4..4 + byte_size];
        Ok(4 + byte_size)
    }
}

impl<'d> std::ops::Deref for LongArrayBytes<'d> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.bytes
    }
}

#[derive(Debug, Default, Clone)]
pub struct IntArrayBytes<'a> {
    bytes: &'a [u8],
}

impl<'d> DeserializePayload<'d> for IntArrayBytes<'d> {
    const TAG: Tag = Tag::IntArray;

    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let l = read_i32_len(data)?;

        let byte_size = l * 4;

        if data.len() < byte_size + 4 {
            return Err(DeserializationError::EOF);
        };
        self.bytes = &data[4..4 + byte_size];
        Ok(4 + byte_size)
    }
}

impl<'d> std::ops::Deref for IntArrayBytes<'d> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.bytes
    }
}

#[derive(Debug, Default, Clone)]
pub struct ByteArrayBytes<'a> {
    bytes: &'a [u8],
}

impl<'d> DeserializePayload<'d> for ByteArrayBytes<'d> {
    const TAG: Tag = Tag::ByteArray;

    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let byte_size = read_i32_len(data)?;

        if data.len() < byte_size + 4 {
            return Err(DeserializationError::EOF);
        };
        self.bytes = &data[4..4 + byte_size];
        Ok(4 + byte_size)
    }
}

impl<'d> std::ops::Deref for ByteArrayBytes<'d> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.bytes
    }
}
