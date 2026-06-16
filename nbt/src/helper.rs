use crate::{DeserializationError, DeserializationResult, SerializationError, Tag};

// named as such because lengths are encoded in nbt as big endian 32 bit signed integers
// the len arg is still usize for convenience
pub fn write_i32_len(len: usize, buf: &mut Vec<u8>) -> Result<(), SerializationError> {
    if len > i32::MAX as usize {
        Err(SerializationError::SeqTooLong)
    } else {
        let bs = (len as i32).to_be_bytes();
        buf.extend_from_slice(&bs);
        Ok(())
    }
}

pub fn serialize_nbt_str(s: &str, buf: &mut Vec<u8>) -> Result<(), SerializationError> {
    if s.len() > u16::MAX as usize {
        return Err(SerializationError::StringTooLong);
    }
    buf.extend_from_slice(&(s.len() as u16).to_be_bytes());
    buf.extend_from_slice(s.as_bytes());
    Ok(())
}

/// unchecked w.r.t. utf8
pub fn deserialize_nbt_str_bytes(data: &[u8]) -> DeserializationResult<(&[u8], usize)> {
    if data.len() < 2 {
        return Err(DeserializationError::EOF);
    };

    let l = u16::from_be_bytes(data[0..2].try_into().unwrap()) as usize;

    if data.len() < 2 + l {
        return Err(DeserializationError::EOF);
    };
    let d = &data[2..2 + l];
    Ok((d, 2 + l))
}

pub fn deserialize_nbt_str(data: &[u8]) -> DeserializationResult<(&str, usize)> {
    if data.len() < 2 {
        return Err(DeserializationError::EOF);
    };

    let l = u16::from_be_bytes(data[0..2].try_into().unwrap()) as usize;

    if data.len() < 2 + l {
        return Err(DeserializationError::EOF);
    };
    let s = std::str::from_utf8(&data[2..2 + l])?;
    Ok((s, 2 + l))
}

fn skip_fixed_size_seq(data: &[u8], byte_size: usize) -> DeserializationResult<usize> {
    if data.len() < 4 {
        Err(DeserializationError::EOF)
    } else {
        let l = read_i32_len(data)?;

        let off = 4 + l * byte_size;

        if data.len() < off {
            return Err(DeserializationError::EOF);
        }
        Ok(off)
    }
}

/// NOTE: don't forget to advance the offset by 4 after using this
pub fn read_i32_len(data: &[u8]) -> DeserializationResult<usize> {
    if data.len() < 4 {
        return Err(DeserializationError::EOF);
    }

    let l = i32::from_be_bytes(data[0..4].try_into().unwrap());

    if l < 0 {
        return Err(DeserializationError::InvalidI32Length);
    }

    Ok(l as usize)
}

fn skip_list_payload(data: &[u8]) -> DeserializationResult<usize> {
    if data.is_empty() {
        return Err(DeserializationError::EOF);
    }
    let elem_tag: Tag = data[0].try_into()?;
    let mut off = 1;
    if elem_tag == Tag::End {
        let l = read_i32_len(&data[off..])?;
        if l != 0 {
            return Err(DeserializationError::InvalidList);
        };
        Ok(off + 4)
    } else if let Some(s) = elem_tag.fixed_size() {
        off += skip_fixed_size_seq(&data[off..], s)?;
        Ok(off)
    } else {
        let l = read_i32_len(&data[off..])?;
        off += 4;
        for _ in 0..l {
            off += skip_payload(&data[off..], elem_tag)?;
        }
        Ok(off)
    }
}

fn skip_compound_payload(data: &[u8]) -> DeserializationResult<usize> {
    let mut off = 0;
    loop {
        if data.len() < off {
            return Err(DeserializationError::EOF);
        }

        let t: Tag = data[off].try_into()?;
        off += 1;

        if t == Tag::End {
            // compound tag closed
            break;
        }

        // skip inner tags name
        off += skip_nbt_str(&data[off..])?;

        // skip inner tags payload
        off += skip_payload(&data[off..], t)?;
    }
    Ok(off)
}

pub fn skip_payload(data: &[u8], tag: Tag) -> DeserializationResult<usize> {
    match tag {
        Tag::Byte => skip_k(data, 1),
        Tag::Short => skip_k(data, 2),
        Tag::Int => skip_k(data, 4),
        Tag::Long => skip_k(data, 8),
        Tag::Float => skip_k(data, 4),
        Tag::Double => skip_k(data, 8),
        Tag::List => skip_list_payload(data),
        Tag::ByteArray => skip_fixed_size_seq(data, 1),
        Tag::IntArray => skip_fixed_size_seq(data, 4),
        Tag::LongArray => skip_fixed_size_seq(data, 8),
        Tag::String => skip_nbt_str(data),
        Tag::Compound => skip_compound_payload(data),
        t => panic!("skip_payload with tag == Tag::End"),
    }
}

fn skip_k(data: &[u8], k: usize) -> DeserializationResult<usize> {
    if data.len() < k {
        Err(DeserializationError::EOF)
    } else {
        Ok(k)
    }
}

fn skip_nbt_str(data: &[u8]) -> DeserializationResult<usize> {
    if data.len() < 2 {
        return Err(DeserializationError::EOF);
    }
    let l = u16::from_be_bytes(data[0..2].try_into().unwrap()) as usize;
    if data.len() < 2 + l {
        return Err(DeserializationError::EOF);
    }

    Ok(2 + l)
}
