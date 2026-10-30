#![cfg(feature = "bumpalo")]

use crate::{DeserializationError, DeserializeExt, DeserializePayload, Tag, read_i32_len};
pub use bumpalo;

impl<'b> BumpCtx<'b> for &'b bumpalo::Bump {
    fn bump(&self) -> &'b bumpalo::Bump {
        self
    }
}

pub trait BumpCtx<'b> {
    fn bump(&self) -> &'b bumpalo::Bump;
}

/// allocates T in the given bumpalo::Bump via Default and tries to deserialize into the resulting &mut T.
pub fn try_from_bytes_in<'d, T>(
    data: &'d [u8],
    mut bump: &'d bumpalo::Bump,
) -> Result<(&'d mut T, &'d str, usize), DeserializationError>
where
    T: DeserializePayload<'d, &'d bumpalo::Bump> + Default,
{
    let t = bump.alloc_with(T::default);

    let (root_name, n) = t.try_deserialize_into(data, &mut bump)?;
    Ok((t, root_name, n))
}

impl<'d, C, T> DeserializePayload<'d, C> for &'d mut [T]
where
    C: BumpCtx<'d>,
    T: DeserializePayload<'d, C> + Default,
{
    const TAG: crate::Tag = Tag::List;
    fn deserialize_payload(
        &mut self,
        data: &'d [u8],
        ctx: &mut C,
    ) -> crate::DeserializationResult<usize> {
        if data.len() < 5 {
            return Err(DeserializationError::EOF);
        };

        let t = Tag::try_from(data[0])?;
        let mut off = 1;

        if t != T::TAG {
            return Err(DeserializationError::UnexpectedTag {
                expected: T::TAG,
                found: t,
            });
        }

        let len = read_i32_len(&data[off..])?;
        off += 4;

        *self =
            ctx.bump()
                .alloc_slice_try_fill_with(len, |_| -> Result<T, DeserializationError> {
                    let mut dst = T::default();
                    off += dst.deserialize_payload(&data[off..], ctx)?;
                    Ok(dst)
                })?;

        Ok(off)
    }
}

impl<'d, C, T> DeserializePayload<'d, C> for &'d [T]
where
    C: BumpCtx<'d>,
    T: DeserializePayload<'d, C> + Default,
{
    const TAG: crate::Tag = Tag::List;
    fn deserialize_payload(
        &mut self,
        data: &'d [u8],
        ctx: &mut C,
    ) -> crate::DeserializationResult<usize> {
        let mut dst: &mut [T] = &mut [];
        let n = dst.deserialize_payload(data, ctx)?;
        *self = dst;
        Ok(n)
    }
}
