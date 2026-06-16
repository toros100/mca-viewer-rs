use crate::{
    DeserializationError, DeserializationResult, DeserializePayload, DeserializeReuse, Tag,
    deserialize_nbt_str, read_i32_len,
};

unsafe impl DeserializeReuse for &str {
    type Borrow<'b> = &'b str;
}

unsafe impl DeserializeReuse for String {
    type Borrow<'b> = String;
}

impl<'d> DeserializePayload<'d> for &'d str {
    const TAG: Tag = Tag::String;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let (s, off) = deserialize_nbt_str(data)?;
        *self = s;
        Ok(off)
    }
}

impl<'d> DeserializePayload<'d> for String {
    const TAG: Tag = Tag::String;
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
            return Err(DeserializationError::UnexpectedTag(t as u8));
        }

        let len = read_i32_len(&data[off..])?;
        off += 4;

        self.resize_with(len, T::default);

        for v in self.iter_mut() {
            off += v.deserialize_payload(&data[off..])?;
        }
        Ok(off)
    }
}

macro_rules! impl_traits_numeric {
    ($(($t:ty, $tag:expr)),*) => {
        $(
            unsafe impl DeserializeReuse for $t {
                type Borrow<'b> = $t;
        }
            impl<'d> DeserializePayload<'d> for $t {
                const TAG: Tag = $tag;
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
