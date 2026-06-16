use std::ops::DerefMut;

use crate::arrays::*;
use crate::*;

/// # actually unsafe do not use (see end)
///
/// this is me experimenting with reusing memory for deserialization
///
/// basic idea:
///
/// 1. let T be a type that can be deserialized from NBT bytes. in the following, we assume that T
///    has a lifetime parameter (that is supposed to pertain to borrowed data from the input bytes)
/// 2. we hide T in Reusable<T>, so we can reuse the memory of T for multiple deserializations,
///    which normally would not be possible due to the lifetime. currently, the only way to obtain a
///    Reusable<T> is if T : Default (which implicitly makes the inner T have a static lifetime, i guess?)
/// 3. try_deserialize_into on Reusable<T> with input bytes &'d [u8] uses unsafe to transmute the
///    inner T into having lifetime 'd, then deserializes into that
/// 4. we only get to see the inner T via a RefGuard, which holds an inaccessible mut ref to the
///    Reusable<T> to prevent it from being mutated while RefGuard is held (in particular by another call to
///    try_deserialize_into). That mutable ref has lifetime 'd as well, ensuring that the ref to the
///    input bytes must stay alive until we drop the RefGuard.
/// 5. RefGuard derefs into T with transmuted lifetime 'd
///
///
/// the biggest safety consideration when implementing DeserializeReuse is that the associated type
/// Borrow<'b> needs to be essentially the same as the implementing type T, with 'b substituted
/// for the lifetime parameter of T. it needs to be safe to transmute T into T::Borrow<'x> for
/// arbitrary 'x. (cf. Reusable and RefGuard to see exactly which transmutes are performed)
///
/// (TODO: more notes, also relies on the implementation of DeserializePayload never giving access
/// to stale data, particulary important for optional fields)
///
/// -----------
///
/// WARN: while writing this, i just realized that this approach is kind of inherently unsound.
/// i thought it would be fine to have stale references in the supposedly static T, as long as i
/// don't touch them. but apparently the mere existence of dangling references is undefined
/// behavior https://doc.rust-lang.org/reference/behavior-considered-undefined.html#r-undefined.dangling.def
///
/// pain
///
/// then i tried to come up with some sort of cleanup strategy, but the only place where that could
/// really live is a Drop impl on the RefGuard. in my mind it would have efficiently "sanitized" any
/// fields with borrowed data (e.g. replacing &'d str with ""), but since we can't have nice things,
/// we can't rely on Drop to actually run? (cf. the docstring for std::mem::forget)
///
/// will still commit this for posterity
///
pub unsafe trait DeserializeReuse {
    type Borrow<'b>: DeserializePayload<'b> + Default;
}

#[derive(Default, Debug)]
pub struct Reusable<T> {
    inner: T,
}

impl<T: DeserializePayload<'static> + DeserializeReuse> Reusable<T> {
    pub fn try_deserialize_into<'d>(
        &'d mut self,
        data: &'d [u8],
    ) -> DeserializationResult<(&'d str, RefGuard<'d, T>, usize)> {
        if data.is_empty() {
            return Err(DeserializationError::EOF);
        };

        let t = Tag::try_from(data[0])?;

        if t == Tag::End {
            return Err(DeserializationError::UnexpectedTag(0));
        }

        let mut off = 1;

        let (root_name, k) = deserialize_nbt_str(&data[off..])?;
        off += k;

        // SAFETY: should be safe by the contract of DeserializeReuse
        // it's also very important that inner_lifetimed is not leaked
        unsafe {
            let inner_lifetimed: &'d mut T::Borrow<'d> = std::mem::transmute(&mut self.inner);
            off += inner_lifetimed.deserialize_payload(&data[off..])?;
        }

        Ok((
            root_name,
            RefGuard {
                origin: self,
                _phantom: PhantomData,
            },
            off,
        ))
    }
}

pub struct RefGuard<'d, T: DeserializePayload<'static>> {
    origin: &'d mut Reusable<T>,
    _phantom: PhantomData<&'d ()>,
}

impl<'d, T: DeserializePayload<'static> + DeserializeReuse> std::ops::Deref for RefGuard<'d, T> {
    type Target = T::Borrow<'d>;
    fn deref(&self) -> &Self::Target {
        unsafe { std::mem::transmute(&self.origin.inner) }
    }
}

impl<'d, T: DeserializePayload<'static> + DeserializeReuse> std::ops::DerefMut for RefGuard<'d, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::mem::transmute(&mut self.origin.inner) }
    }
}

/// option wrapper to reuse memory across deserializations
/// if we first deserialize Some(_), then None, then Some(_) again,
/// the allocation for the first Some(_) was wasted
/// (this applies to heap allocated T)
#[derive(Default, Debug)]
pub struct OptionalValue<T> {
    is_some: bool,
    value: T,
}

impl<T> OptionalValue<T> {
    pub fn as_option(&self) -> Option<&T> {
        if self.is_some {
            Some(&self.value)
        } else {
            None
        }
    }
    pub fn as_option_mut(&mut self) -> Option<&mut T> {
        if self.is_some {
            Some(&mut self.value)
        } else {
            None
        }
    }
    pub fn into_option(self) -> Option<T> {
        if self.is_some { Some(self.value) } else { None }
    }
}

impl<'d, T: DeserializePayload<'d>> DeserializePayload<'d> for OptionalValue<T> {
    const TAG: Tag = T::TAG;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        let k = self.value.deserialize_payload(data)?;
        self.is_some = true;
        Ok(k)
    }
}

impl<T> Optional for OptionalValue<T> {
    fn handle_absent(&mut self) {
        self.is_some = false
    }
}

/// wrapper around Vec for memory reuse, analogous to OptionalValue
/// even though Vec::clear() does retain the allocated capacity, it would still drop the elements, which
/// wastes allocations if the element type is heap-allocated
#[derive(Debug, Default, Clone)]
pub struct List<T> {
    inner: Vec<T>,
    logical_len: usize,
}

impl<T> List<T> {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Vec::with_capacity(cap),
            logical_len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.logical_len
    }

    pub fn is_empty(&self) -> bool {
        self.logical_len == 0
    }
}

impl<T> Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.inner[0..self.logical_len]
    }
}
impl<T> DerefMut for List<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner[0..self.logical_len]
    }
}

impl<'d, T: DeserializePayload<'d> + Default> DeserializePayload<'d> for List<T> {
    const TAG: Tag = Tag::List;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        if data.is_empty() {
            return Err(DeserializationError::EOF);
        }

        let elem_tag = Tag::try_from(data[0])?;

        let mut off = 1;

        let l = read_i32_len(&data[off..])?;
        off += 4;
        self.logical_len = l;

        if l > self.inner.len() {
            self.inner.resize_with(l, T::default);
        };

        for i in 0..l {
            off += self.inner[i].deserialize_payload(&data[off..])?;
        }
        Ok(off)
    }
}
