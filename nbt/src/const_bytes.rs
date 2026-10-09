use crate::{
    DeserializationError, DeserializationResult, DeserializePayload, SerializationError,
    SerializationResult, SerializePayload, Tag,
};

/// any type implementing this trait will implement DeserializePayload via a blanket implementation,
/// which will expect exactly the specified tag TAG and payload bytes BYTES. if the found payload
/// bytes are different from BYTES, then DeserializationError::Custom containing ERR_MSG will
/// be returned. analgously, a blanket implementation of SerializePayload will serialize exactly the
/// specified tag and payload bytes.
///
/// example:
/// ```
/// pub struct StatusMinecraftFull;
///
/// impl nbt::ConstBytes for StatusMinecraftFull {
///     const TAG: nbt::Tag = nbt::Tag::String;
///     const BYTES: &'static [u8] = b"\x00\x0eminecraft:full";
///     // note that BYTES is a full string payload, including the u16 big endian length prefix
///     const ERR_MSG: Option<&'static str> = Some("status not minecraft:full");
/// }
/// ```
pub trait ConstBytes {
    const TAG: Tag;
    const BYTES: &'static [u8];
    const ERR_MSG: Option<&'static str> = None;
}

impl<'d, C: ConstBytes> DeserializePayload<'d> for C {
    // TODO: const asserts for reasonability of the payload bytes?
    // for some values of Tag, the len is fully determined (e.g. Tag::Int always 4 bytes)
    // for Tag::String the len is always at least 2 (length prefix), and if it is exactly 2, then it
    // must be b"\x00\x00" etc
    // a lot could be done here i guess, but it can't be fully validated

    const TAG: Tag = C::TAG;

    #[inline]
    fn deserialize_payload(&mut self, data: &'d [u8]) -> DeserializationResult<usize> {
        const _: () = {};

        if data.len() < C::BYTES.len() {
            return Err(DeserializationError::EOF);
        };
        if C::BYTES == &data[0..C::BYTES.len()] {
            Ok(C::BYTES.len())
        } else {
            Err(DeserializationError::Custom(
                C::ERR_MSG.unwrap_or("ConstBytes mismatch").into(),
            ))
        }
    }
}

impl<C: ConstBytes> SerializePayload for C {
    const TAG: Tag = C::TAG;
    #[inline]
    fn serialize_payload(&self, buf: &mut Vec<u8>) -> SerializationResult<()> {
        buf.extend(C::BYTES);
        Ok(())
    }
}
