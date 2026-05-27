struct PackedBitsView<'a> {
    longs: &'a [i64],
    bit_width: usize,
}

/// returns the bit width required to index into a slice of length n > 0
pub fn bits_required(n: usize) -> usize {
    debug_assert_ne!(n, 0);
    (usize::BITS - (n - 1).leading_zeros()) as usize
}

pub fn unpack_from_longs(longs: &[i64], bit_width: usize, idx: usize) -> u16 {
    debug_assert!(bit_width > 0);
    debug_assert!(bit_width <= 12);

    let nums_per_long = 64 / bit_width;
    let long_idx = idx / nums_per_long;

    let u = longs[long_idx];

    let i = idx % nums_per_long;

    ((u >> (i * bit_width)) & ((1 << bit_width) - 1)) as u16
}

// would be cool to have a const param for the width, but then i need to do extra
// work to do dynamic dispatch, since this will mostly be used with bit width that is only known at runtime
impl<'a> PackedBitsView<'a> {
    fn new(longs: &'a [i64], bit_width: usize) -> PackedBitsView<'a> {
        if !(1..=12).contains(&bit_width) {
            panic!("bit_width must be in 1..=12")
        }

        Self { longs, bit_width }
    }
    fn get(&self, idx: usize) -> u16 {
        // let num_u64s = self.bytes.len() / 8;
        let nums_per_long = 64 / self.bit_width;

        let long_idx = idx / nums_per_long;

        let u = self.longs[long_idx];

        let i = idx % nums_per_long;

        ((u >> (i * self.bit_width)) & ((1 << self.bit_width) - 1)) as u16
    }

    fn len(&self) -> usize {
        let nums_per_u64 = 64 / self.bit_width;
        self.longs.len() * nums_per_u64
    }
}

fn pack_into_i64s<const BIT_WIDTH: usize>(nums: &[u64]) -> Vec<i64> {
    const {
        assert!(BIT_WIDTH > 0);
        assert!(BIT_WIDTH <= 12);
    }

    let nums_per_u64 = 64 / BIT_WIDTH;

    debug_assert!(
        nums.iter().all(|x| *x < (1 << BIT_WIDTH)),
        "some n in nums does not fit into BIT_WIDTH many bits"
    );

    nums.chunks(nums_per_u64)
        .map(|c| {
            c.iter()
                .rev()
                .fold(0i64, |acc, v| acc << BIT_WIDTH | *v as i64)
        })
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn simple_test() {
        let u = vec![-1i64]; // all 1s

        let p = PackedBitsView::new(&u, 8);

        assert_eq!(p.len(), 8);

        let expected: Vec<u16> = vec![255, 255, 255, 255, 255, 255, 255, 255];

        let unpacked: Vec<u16> = (0..p.len()).map(|i| p.get(i)).collect();

        assert_eq!(expected, unpacked);
    }

    #[test]
    fn roundtrip_test() {
        let v: Vec<u64> = vec![1, 2, 3, 4, 5, 6, 7];

        let packed = pack_into_i64s::<8>(&v);

        assert_eq!(packed.len(), 1);

        let p = vec![packed[0]];

        let unpacked_view = PackedBitsView::new(&packed, 8);

        assert_eq!(unpacked_view.len(), 8);

        let unpacked_nums_1: Vec<u64> = (0..7).map(|i| unpacked_view.get(i) as u64).collect();
        let unpacked_nums_2: Vec<u64> = (0..7)
            .map(|i| unpack_from_longs(p.as_slice(), 8, i) as u64)
            .collect();

        assert_eq!(v, unpacked_nums_1);
        assert_eq!(v, unpacked_nums_2);
    }

    #[test]
    fn test_bits_required() {
        assert_eq!(3, bits_required(7));
        assert_eq!(3, bits_required(8));
        assert_eq!(4, bits_required(9));
    }
}
