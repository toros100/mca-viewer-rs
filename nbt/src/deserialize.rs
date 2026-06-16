use std::{default, marker::PhantomData};

use crate::{
    DeserializationError, DeserializationResult, DeserializeReuse, OptionalValue, Tag,
    deserialize_nbt_str, read_i32_len,
};

// this only exists because of my unsafe experiments
pub trait Optional {
    fn handle_absent(&mut self);
}

impl<T> Optional for Option<T> {
    fn handle_absent(&mut self) {
        *self = None
    }
}

unsafe impl<T: DeserializeReuse> DeserializeReuse for Option<T> {
    type Borrow<'b> = Option<T::Borrow<'b>>;
}

impl<'d, T: DeserializePayload<'d> + Default> DeserializePayload<'d> for Option<T> {
    const TAG: Tag = T::TAG;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        match self {
            None => self.insert(T::default()).deserialize_payload(data),
            Some(v) => v.deserialize_payload(data),
        }
    }
}

pub trait DeserializePayload<'d>: Default {
    const TAG: Tag;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize>;
}
