use std::{default, marker::PhantomData};

use arrayvec::ArrayVec;

use crate::{
    DeserializationError, DeserializationResult, Optional, RawString, Tag, deserialize_nbt_str,
    deserialize_nbt_str_bytes, read_i32_len,
};

pub trait DeserializePayload<'d> {
    const TAG: Tag;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize>;
}

impl<'d, T: DeserializePayload<'d> + Default> DeserializePayload<'d> for Option<T> {
    const TAG: Tag = T::TAG;
    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        self.get_or_insert_default().deserialize_payload(data)
    }
}

impl<'d> DeserializePayload<'d> for &'d str {
    const TAG: Tag = Tag::String;
    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let (s, off) = deserialize_nbt_str(data)?;
        *self = s;
        Ok(off)
    }
}

impl<'d> DeserializePayload<'d> for String {
    const TAG: Tag = Tag::String;
    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let (s, off) = deserialize_nbt_str(data)?;
        *self = String::from(s);
        Ok(off)
    }
}

impl<'d, T: DeserializePayload<'d> + Default> DeserializePayload<'d> for Vec<T> {
    const TAG: Tag = Tag::List;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
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

        self.clear();
        self.reserve(len);

        for _ in 0..len {
            off += self
                .push_mut(T::default())
                .deserialize_payload(&data[off..])?;
        }
        Ok(off)
    }
}

impl<'d, const N: usize, T: DeserializePayload<'d> + Default> DeserializePayload<'d>
    for ArrayVec<T, N>
{
    const TAG: Tag = Tag::List;

    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
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
        if len > N {
            return Err(DeserializationError::Custom(
                "list length exceeded ArrayVec capacity".into(),
            ));
        }
        off += 4;

        self.clear();

        for _ in 0..len {
            self.push(T::default());
            off +=
                unsafe { self.last_mut().unwrap_unchecked() }.deserialize_payload(&data[off..])?;
        }

        Ok(off)
    }
}

macro_rules! impl_traits_numeric {
    ($(($t:ty, $tag:expr)),*) => {
        $(
            impl<'d> DeserializePayload<'d> for $t {
                const TAG: Tag = $tag;
                #[inline]
                fn deserialize_payload(&mut self, data: &'d[u8]) -> DeserializationResult<usize> {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    if data.len() < SIZE {
                        return Err(DeserializationError::EOF);
                    }
                    *self = <$t>::from_be_bytes(data[..SIZE].try_into().unwrap());
                    Ok(SIZE)
                }
            }
        )*
    }
}

// WARN: only doing it like this to make the association between type and tag work as a declarative
// macro, there is no meaningful choice here (e.g. (i8, Tag::Double) would produce nonsense)
impl_traits_numeric!(
    (i8, Tag::Byte),
    (i16, Tag::Short),
    (i32, Tag::Int),
    (i64, Tag::Long),
    (f32, Tag::Float),
    (f64, Tag::Double)
);
