use std::slice::Chunks;

/// returns the bit width required to index into a slice of length n > 0
pub const fn bits_required(n: usize) -> usize {
    debug_assert!(n != 0);
    (usize::BITS - (n - 1).leading_zeros()) as usize
}

pub struct PackedIndices<'a> {
    raw: &'a [u8],
    bit_width: usize,
    palette_len: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PackedIndicesError {
    #[error("bad palette length")]
    BadPaletteLength,

    #[error("data length does not match palette length")]
    BadDataLength,
}

impl<'a> PackedIndices<'a> {
    /// palette_len should be in 2..=4096
    pub fn new(raw: &'a [u8], palette_len: usize) -> Result<Self, PackedIndicesError> {
        if palette_len < 2 {
            return Err(PackedIndicesError::BadPaletteLength);
        }
        let bit_width = bits_required(palette_len);

        if bit_width > 12 {
            // bit_width > 12 iff palette_len > 4096
            Err(PackedIndicesError::BadPaletteLength)
        } else {
            let bit_width = bit_width.clamp(4, 12);
            let expected_bytes = 4096usize.div_ceil(64 / bit_width) * 8;

            if raw.len() != expected_bytes {
                Err(PackedIndicesError::BadDataLength)
            } else {
                Ok(Self {
                    raw,
                    bit_width,
                    palette_len,
                })
            }
        }
    }

    /// unpacks the i-th index, for i < 4096.
    ///
    /// if get returns the Some(j), it is guaranteed that j < palette_len
    ///
    /// i >= 4096 will panic in debug. in release, i is truncated and will produce an unspecified
    /// (but safe and panic-free) j < palette_len.
    ///
    /// returns None if and only if the unpacked index is >= palette_len, which can only happen with
    /// malformed chunk data
    pub fn get(&self, i: usize) -> Option<usize> {
        debug_assert!(i < 4096);

        // truncating to 12 bits to ensure i < 4096
        let i = i & 0xFFF;

        // SAFETY:
        // have self.bit_width in 4..=12 by construction
        //
        // because i < 4096, we have
        //
        // (i + 1).div_ceil(64 / bit_width) * 8
        // <= 4096.div_ceil(64 / bit_width) * 8
        // == self.raw.len()
        let j = unsafe { unpack_from_u64_bytes_dispatch(self.raw, self.bit_width, i) };
        if j < self.palette_len { Some(j) } else { None }
    }
}

// unpacks the i-th index from data
//
// # Safety
//
// the caller must ensure that
//
// 1. bit_width is in 4..=12
// 2. data.len() >= (i + 1).div_ceil(64 / bit_width) * 8
#[inline]
unsafe fn unpack_from_u64_bytes_dispatch(data: &[u8], bit_width: usize, i: usize) -> usize {
    debug_assert!((4..=12).contains(&bit_width));
    debug_assert!(data.len() >= (i + 1).div_ceil(64 / bit_width) * 8);

    unsafe {
        match bit_width {
            4 => unpack_from_u64_bytes_width_4(data, i),
            5 => unpack_from_u64_bytes_width_generic::<5>(data, i),
            6 => unpack_from_u64_bytes_width_generic::<6>(data, i),
            7 => unpack_from_u64_bytes_width_generic::<7>(data, i),
            8 => unpack_from_u64_bytes_width_8(data, i),
            9 => unpack_from_u64_bytes_width_generic::<9>(data, i),
            10 => unpack_from_u64_bytes_width_generic::<10>(data, i),
            11 => unpack_from_u64_bytes_width_generic::<11>(data, i),
            12 => unpack_from_u64_bytes_width_generic::<12>(data, i),
            _ => std::hint::unreachable_unchecked(),
        }
    }
}

#[inline]
unsafe fn unpack_from_u64_bytes_width_4(bs: &[u8], idx: usize) -> usize {
    let k = (idx / 16) * 8 + 7 - (idx % 16) / 2;
    debug_assert!(k < bs.len());

    let shift = (idx & 1) * 4;

    let val = *unsafe { bs.get_unchecked(k) };

    ((val >> shift) & 0x0f) as usize
}

#[inline]
unsafe fn unpack_from_u64_bytes_width_8(bs: &[u8], idx: usize) -> usize {
    let j = (idx / 8) * 8 + 7 - idx % 8;
    debug_assert!(j < bs.len());

    unsafe { *bs.get_unchecked(j) as usize }
}

#[inline]
unsafe fn unpack_from_u64_bytes_width_generic<const BIT_WIDTH: usize>(
    bs: &[u8],
    idx: usize,
) -> usize {
    const {
        assert!(4 <= BIT_WIDTH && BIT_WIDTH <= 12);
    }

    let j = (idx / (64 / BIT_WIDTH)) * 8;

    debug_assert!(j + 7 < bs.len());

    let u = unsafe { u64::from_be_bytes(*(bs.as_ptr().add(j) as *const [u8; 8])) };

    let inner_idx = idx % (64 / BIT_WIDTH);

    ((u >> (inner_idx * BIT_WIDTH)) & ((1 << BIT_WIDTH) - 1)) as usize
}

/// boring and unoptimized packing iterator, currently only used for testing
#[allow(unused)]
struct PackedIter<'a> {
    nums: &'a [u16],
    off: usize,
    bit_width: usize,
    chunks: Chunks<'a, u16>,
}

#[allow(unused)]
impl<'a> PackedIter<'a> {
    fn new(nums: &'a [u16], bit_width: usize) -> Self {
        // using u16 input values for convenience (compat with existing code)

        // these are the only bit widths i am using right now
        assert!((4..=12).contains(&bit_width));

        assert!(
            nums.iter().all(|x| *x < (1 << bit_width)),
            "some n in nums does not fit into bit_width many bits"
        );

        let nums_per_u64 = 64 / bit_width;

        let chunks = nums.chunks(nums_per_u64);
        Self {
            nums,
            off: 0,
            bit_width,
            chunks,
        }
    }
}

impl<'a> Iterator for PackedIter<'a> {
    type Item = u64;
    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(|c| {
            c.iter()
                .rev()
                .fold(0u64, |acc, &v| (acc << self.bit_width) | v as u64)
        })
    }
}

impl ExactSizeIterator for PackedIter<'_> {
    fn len(&self) -> usize {
        self.chunks.len()
    }
}

#[inline]
pub fn unpack_from_long_bytes(bs: &[u8], bit_width: usize, idx: usize) -> u16 {
    debug_assert!(bit_width > 0);
    debug_assert!(bit_width <= 12);

    let nums_per_u64 = 64 / bit_width;
    let long_idx = idx / nums_per_u64;

    let i = long_idx * 8;

    let u = i64::from_be_bytes(bs[i..i + 8].try_into().unwrap());

    let i = idx % nums_per_u64;

    ((u >> (i * bit_width)) & ((1 << bit_width) - 1)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_helper(bit_width: usize, input_len: usize) {
        assert!((4..=12).contains(&bit_width));

        let nums = (0..input_len)
            .map(|i| (i as u16) % (1 << bit_width))
            .collect::<Vec<u16>>();

        let u64s = PackedIter::new(&nums, bit_width).collect::<Vec<u64>>();

        // sanity checking assumptions
        assert_eq!(u64s.len(), input_len.div_ceil(64 / bit_width));

        let bs = PackedIter::new(&nums, bit_width)
            .flat_map(|v| v.to_be_bytes())
            .collect::<Vec<u8>>();

        assert!((0..nums.len()).all(|i| {
            nums[i] as usize == unsafe { unpack_from_u64_bytes_dispatch(&bs, bit_width, i) }
        }));
    }

    #[test]
    fn roundtrip() {
        for bit_width in 4..=12 {
            // empty input
            roundtrip_helper(bit_width, 0);

            // input that does not "fill" the u64s completely (1019 is not a multiple of 64/bit_width)
            roundtrip_helper(bit_width, 1019);

            // that perfectly fills the u64s
            roundtrip_helper(bit_width, (64 / bit_width) * 10);
        }
    }

    #[test]
    fn test_bits_required() {
        // kinda obvious lol
        assert_eq!(0, bits_required(1));
        assert_eq!(1, bits_required(2));
        assert_eq!(3, bits_required(7));
        assert_eq!(3, bits_required(8));
        assert_eq!(4, bits_required(9));
    }
}
