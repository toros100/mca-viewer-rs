use std::slice::Chunks;

use crate::{cautious_unsafe, cfg_unreachable};

/// returns the bit width required to index into a slice of length n > 0
pub const fn bits_required(n: usize) -> usize {
    debug_assert!(n != 0);
    (usize::BITS - (n - 1).leading_zeros()) as usize
}

#[inline(always)]
pub fn unpack_from_u64_bytes_dispatch(bs: &[u8], bit_width: usize, idx: usize) -> u16 {
    debug_assert!((4..=12).contains(&bit_width));
    match bit_width {
        4 => unpack_from_u64_bytes_width_4(bs, idx),
        5 => unpack_from_u64_bytes_width_generic::<5>(bs, idx),
        6 => unpack_from_u64_bytes_width_generic::<6>(bs, idx),
        7 => unpack_from_u64_bytes_width_generic::<7>(bs, idx),
        8 => unpack_from_u64_bytes_width_8(bs, idx),
        9 => unpack_from_u64_bytes_width_generic::<9>(bs, idx),
        10 => unpack_from_u64_bytes_width_generic::<10>(bs, idx),
        11 => unpack_from_u64_bytes_width_generic::<11>(bs, idx),
        12 => unpack_from_u64_bytes_width_generic::<12>(bs, idx),
        _ => cfg_unreachable!(),
    }
}

#[inline(always)]
pub fn unpack_from_u64_bytes_width_4(bs: &[u8], idx: usize) -> u16 {
    let k = (idx / 16) * 8 + 7 - (idx % 16) / 2;

    let shift = (idx & 1) * 4;
    // NOTE: "let shift = if idx.is_multiple_of(2) { 0 } else { 4 }"
    // produces identical assembly on godbolt, but only with -O
    // (so this still makes the debug build faster at least lol)

    debug_assert!(k < bs.len());

    let val = if cautious_unsafe!() {
        bs[k]
    } else {
        // SAFETY: do not hold it wrong
        *unsafe { bs.get_unchecked(k) }
    };

    ((val >> shift) & 0x0f) as u16
}

#[inline(always)]
pub fn unpack_from_u64_bytes_width_8(bs: &[u8], idx: usize) -> u16 {
    let j = (idx / 8) * 8 + 7 - idx % 8;

    debug_assert!(j < bs.len());

    if cautious_unsafe!() {
        bs[j] as u16
    } else {
        // SAFETY: do not hold it wrong
        unsafe { *bs.get_unchecked(j) as u16 }
    }
}

#[inline(always)]
pub fn unpack_from_u64_bytes_width_generic<const BIT_WIDTH: usize>(bs: &[u8], idx: usize) -> u16 {
    const {
        assert!(4 <= BIT_WIDTH && BIT_WIDTH <= 12);
    }

    let long_idx = idx / (64 / BIT_WIDTH);

    let j = long_idx * 8;

    debug_assert!(j + 7 < bs.len());

    let u = if cautious_unsafe!() {
        u64::from_be_bytes(bs[j..j + 8].try_into().unwrap())
    } else {
        // SAFETY: do not hold it wrong
        unsafe { u64::from_be_bytes(*(bs.as_ptr().add(j) as *const [u8; 8])) }
    };

    let inner_idx = idx % (64 / BIT_WIDTH);

    ((u >> (inner_idx * BIT_WIDTH)) & ((1 << BIT_WIDTH) - 1)) as u16
}

/// boring an unoptimized packing iterator, currently only used for testing
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
        assert!((1..=12).contains(&bit_width));

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

#[inline(always)]
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

        assert!(
            (0..nums.len())
                .all(|i| { nums[i] == unpack_from_u64_bytes_dispatch(&bs, bit_width, i) })
        );
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
