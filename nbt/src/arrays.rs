use crate::{DeserializationError, DeserializationResult, DeserializePayload, Tag, read_i32_len};

#[inline]
fn deserialize_array_payload<'d, const ELEM_BYTE_SIZE: usize>(
    dst: &mut &'d [u8],
    data: &'d [u8],
) -> DeserializationResult<usize> {
    const {
        assert!(ELEM_BYTE_SIZE == 1 || ELEM_BYTE_SIZE == 4 || ELEM_BYTE_SIZE == 8);
    };

    let byte_size = read_i32_len(data)? * ELEM_BYTE_SIZE;

    if data.len() < 4 + byte_size {
        return Err(DeserializationError::EOF);
    };
    *dst = &data[4..4 + byte_size];
    Ok(4 + byte_size)
}

#[derive(Debug, Default, Clone)]
pub struct ByteArrayBytes<'a>(&'a [u8]);

impl<'d> DeserializePayload<'d> for ByteArrayBytes<'d> {
    const TAG: Tag = Tag::ByteArray;

    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        deserialize_array_payload::<1>(&mut self.0, data)
    }
}

impl<'d> std::ops::Deref for ByteArrayBytes<'d> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

#[derive(Debug, Default, Clone)]
pub struct IntArrayBytes<'a>(&'a [u8]);

impl<'d> DeserializePayload<'d> for IntArrayBytes<'d> {
    const TAG: Tag = Tag::IntArray;

    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        deserialize_array_payload::<4>(&mut self.0, data)
    }
}

impl<'d> std::ops::Deref for IntArrayBytes<'d> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

#[derive(Debug, Default, Clone)]
pub struct LongArrayBytes<'a>(&'a [u8]);

impl<'d> DeserializePayload<'d> for LongArrayBytes<'d> {
    const TAG: Tag = Tag::LongArray;

    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        deserialize_array_payload::<8>(&mut self.0, data)
    }
}

impl<'d> std::ops::Deref for LongArrayBytes<'d> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}
