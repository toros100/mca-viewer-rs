use crate::block_id::{BlockId, BlockIdVariants, DynId, SeqId, StaticId};
use crate::{bits_required, cfg_unreachable};

/// storage of every (top) block in a region
/// bit-packed due to RAM prices
pub struct PackedBlockMap {
    palette: Vec<BlockId>,
    bit_width: u64,
    u64_div_bit_width: usize,
    mask: u64,
    packed: Vec<u64>,
}

/// helper type for mapping block ids to palette indices
/// (without using a hashmap)
struct BlockIdMapping {
    pal: Vec<BlockId>,
    m: Vec<u16>,
    s_bounds: Bounds<StaticId>,
    d_bounds: Bounds<DynId>,
}

impl BlockIdMapping {
    fn new(cs: &[BlockId; 512 * 512]) -> Self {
        let (s_bounds, d_bounds) = get_bounds(cs);

        let mut s_hits = vec![false; s_bounds.size];
        let mut d_hits = vec![false; d_bounds.size];
        let mut have_invalid = false;

        let mut s_res = Vec::new();
        let mut d_res = Vec::new();

        for i in cs {
            match i.into_specialized() {
                BlockIdVariants::Static(s) => {
                    let idx = s_bounds.to_index(s);
                    if !s_hits[idx] {
                        s_hits[idx] = true;
                        s_res.push(s);
                    }
                }
                BlockIdVariants::Dyn(d) => {
                    let idx = d_bounds.to_index(d);
                    if !d_hits[idx] {
                        d_hits[idx] = true;
                        d_res.push(d);
                    }
                }
                BlockIdVariants::Invalid => have_invalid = true,
            }
        }

        let cap = if have_invalid {
            1 + s_bounds.size + d_bounds.size
        } else {
            s_bounds.size + d_bounds.size
        };

        let mut m = vec![0u16; cap];

        for (i, &s) in s_res.iter().enumerate() {
            let idx = s_bounds.to_index(s);
            m[idx] = i as u16
        }
        for (i, &d) in d_res.iter().enumerate() {
            let idx = d_bounds.to_index(d) + s_bounds.size;
            m[idx] = (i + s_res.len()) as u16
        }

        if have_invalid {
            m[s_bounds.size + d_bounds.size] = (d_res.len() + s_res.len()) as u16
        }

        let p_cap = if have_invalid {
            s_res.len() + d_res.len() + 1
        } else {
            s_res.len() + d_res.len()
        };

        let mut pal: Vec<BlockId> = Vec::with_capacity(p_cap);

        pal.extend(s_res.into_iter().map(Into::<BlockId>::into));
        pal.extend(d_res.into_iter().map(Into::<BlockId>::into));
        if have_invalid {
            pal.push(BlockId::INVALID);
        }
        Self {
            pal,
            m,
            s_bounds,
            d_bounds,
        }
    }
    fn get_index(&self, b: BlockId) -> usize {
        match b.into_specialized() {
            BlockIdVariants::Static(s) => self.m[self.s_bounds.to_index(s)] as usize,
            BlockIdVariants::Dyn(d) => {
                let idx = self.d_bounds.to_index(d) + self.s_bounds.size;
                self.m[idx] as usize
            }
            BlockIdVariants::Invalid => self.pal.len() - 1,
        }
    }

    fn into_palette(self) -> Vec<BlockId> {
        self.pal
    }
}

/// helper type to represent an inclusive range t_0..=t_1 of T values
struct Bounds<T> {
    min: T,
    max: T,
    size: usize,
}
// NOTE: can i maybe just use the actual inclusive range type?
// this is really not doing that much

impl<T: Into<usize> + SeqId + Copy> Bounds<T> {
    fn new(min: T, max: T) -> Self {
        let min_u = min.into();
        let max_u = max.into();

        // NOTE: min_u > max_u indicates that no value was found, see fn get_bounds
        let size = if min_u > max_u { 0 } else { max_u - min_u + 1 };

        Self { min, max, size }
    }

    fn to_index(&self, v: T) -> usize {
        debug_assert!((self.min..=self.max).contains(&v));
        v.into() - self.min.into()
    }
}

#[inline(always)]
fn get_bounds(cs: &[BlockId; 512 * 512]) -> (Bounds<StaticId>, Bounds<DynId>) {
    debug_assert!(StaticId::MIN < StaticId::MAX);
    debug_assert!(DynId::MIN < DynId::MAX);

    let mut stat_min = StaticId::MAX;
    let mut stat_max = StaticId::MIN;
    let mut dyn_min = DynId::MAX;
    let mut dyn_max = DynId::MIN;
    for v in cs {
        match v.into_specialized() {
            BlockIdVariants::Dyn(d) => {
                dyn_min = dyn_min.min(d);
                dyn_max = dyn_max.max(d);
            }
            BlockIdVariants::Static(s) => {
                stat_min = stat_min.min(s);
                stat_max = stat_max.max(s)
            }
            _ => {}
        }
    }

    (
        Bounds::new(stat_min, stat_max),
        Bounds::new(dyn_min, dyn_max),
    )

    // NOTE: the above version appears to be faster than the following one?
    // should check out the asm to make sure

    // let (s_min, s_max, d_min, d_max) = cs.iter().fold(
    //     (StaticId::MAX, StaticId::MIN, DynId::MAX, DynId::MIN),
    //     |(s_mi, s_ma, d_mi, d_ma), v| match v.into_specialized() {
    //         crate::BlockIdVariants::Dyn(d) => (s_mi, s_ma, d_mi.min(d), d_ma.max(d)),
    //         crate::BlockIdVariants::Static(s) => (s_mi.min(s), s_ma.max(s), d_mi, d_ma),
    //         crate::BlockIdVariants::Invalid => (s_mi, s_ma, d_mi, d_ma),
    //     },
    // );
    //
    // (Bounds::new(s_min, s_max), Bounds::new(d_min, d_max))
}

#[inline(always)]
fn pack_width_8(m: &BlockIdMapping, cs: &[BlockId; 512 * 512]) -> Vec<u64> {
    let mut packed = Vec::with_capacity(512 * 512 / 8);
    packed.extend(
        // SAFETY: 8 divides 512*512
        unsafe { cs.as_chunks_unchecked::<8>() }
            .iter()
            .map(|a| std::array::from_fn(|i| m.get_index(a[i]) as u8))
            .map(u64::from_be_bytes),
    );
    packed
}

#[inline(always)]
fn pack_dispatch(m: &BlockIdMapping, cs: &[BlockId; 512 * 512], bit_width: usize) -> Vec<u64> {
    debug_assert!((1..=12).contains(&bit_width));
    // NOTE: is it worth it to special case 4?
    match bit_width {
        1 => pack_width_generic::<1, 64>(m, cs),
        2 => pack_width_generic::<2, 32>(m, cs),
        3 => pack_width_generic::<3, 21>(m, cs),
        4 => pack_width_generic::<4, 16>(m, cs),
        5 => pack_width_generic::<5, 12>(m, cs),
        6 => pack_width_generic::<6, 10>(m, cs),
        7 => pack_width_generic::<7, 9>(m, cs),
        8 => pack_width_8(m, cs),
        9 => pack_width_generic::<9, 7>(m, cs),
        10 => pack_width_generic::<10, 6>(m, cs),
        11 => pack_width_generic::<11, 5>(m, cs),
        12 => pack_width_generic::<12, 5>(m, cs),
        _ => cfg_unreachable!(),
        // NOTE: this allows for up to 4096 different blocks in a region
        // there are only around 1000 different blocks in the actual game
    }
}

#[inline(always)]
fn pack_width_generic<const BIT_WIDTH: usize, const NUMS_PER_U64: usize>(
    m: &BlockIdMapping,
    cs: &[BlockId; 512 * 512],
) -> Vec<u64> {
    const fn vec_cap(bit_width: usize) -> usize {
        (512usize * 512usize).div_ceil(64 / bit_width)
    }

    const {
        // sanity check
        assert!(64 / BIT_WIDTH == NUMS_PER_U64);
        // (i can't just calculate NUMS_PER_U64 here, because i need to use it as a const param)
    }

    let mut packed = Vec::<u64>::with_capacity(vec_cap(BIT_WIDTH));

    // compiler already does the following optimization in the release build
    // so this is pretty much just optimizing the debug build

    if (512usize * 512usize).is_multiple_of(NUMS_PER_U64) {
        // SAFETY: immediate
        let chunks = unsafe { cs.as_chunks_unchecked::<NUMS_PER_U64>() };

        packed.extend(chunks.iter().map(|c| {
            c.iter()
                .rev()
                .fold(0u64, |acc, &v| (acc << BIT_WIDTH) | (m.get_index(v)) as u64)
        }));
    } else {
        let (chunks, rem) = cs.as_chunks::<NUMS_PER_U64>();

        packed.extend(chunks.iter().map(|c| {
            c.iter()
                .rev()
                .fold(0u64, |acc, &v| (acc << BIT_WIDTH) | (m.get_index(v)) as u64)
        }));

        // NOTE: !rem.is_empty() by condition (important for correctness)
        packed.push(
            rem.iter()
                .rev()
                .fold(0u64, |acc, &v| (acc << BIT_WIDTH) | (m.get_index(v)) as u64),
        );
    }
    packed
}

impl PackedBlockMap {
    pub fn new(cs: &[BlockId; 512 * 512]) -> Self {
        let mapping = BlockIdMapping::new(cs);

        debug_assert!(!mapping.pal.is_empty());

        if mapping.pal.len() == 1 {
            Self {
                palette: mapping.into_palette(),
                bit_width: 0,
                u64_div_bit_width: 0,
                packed: Vec::new(),
                mask: 0,
            }
        } else {
            let bit_width = bits_required(mapping.pal.len()) as u64;
            let packed = pack_dispatch(&mapping, cs, bit_width as usize);

            let u64_div_bit_width = 64 / (bit_width as usize);
            let mask = (1u64 << bit_width) - 1;

            Self {
                palette: mapping.into_palette(),
                bit_width,
                u64_div_bit_width,
                packed,
                mask,
            }
        }
    }
    pub fn get(&self, x: usize, z: usize) -> BlockId {
        // NOTE: maybe i should just reuse the existing unpacking code here

        debug_assert!(x <= 511 && z <= 511);
        debug_assert!(!self.palette.is_empty());

        if self.palette.len() == 1 {
            return self.palette[0];
        };
        let idx = x + z * 512;

        let u64_idx = idx / self.u64_div_bit_width;
        let j = (idx % self.u64_div_bit_width) as u64;

        debug_assert!(u64_idx < self.packed.len());
        let pal_idx = ((self.packed[u64_idx] >> (j * self.bit_width)) & self.mask) as usize;

        debug_assert!(pal_idx < self.palette.len());
        self.palette[pal_idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, seq::IndexedRandom, seq::IteratorRandom};

    fn get_data<const NUM_DISTINCT_BLOCK_IDS: usize>() -> [BlockId; 512 * 512] {
        const {
            assert!(
                NUM_DISTINCT_BLOCK_IDS != 0,
                "logically impossible to have no block id"
            );
        }

        const SEED: u64 = 1234;
        let mut r = rand::rngs::StdRng::seed_from_u64(SEED);

        let blocks_u16 = (0u16..=u16::MAX).sample(&mut r, NUM_DISTINCT_BLOCK_IDS);

        let mut blocks = [BlockId::INVALID; 512 * 512];

        for b in blocks.iter_mut() {
            let block = *blocks_u16.choose(&mut r).unwrap();
            *b = block.into()
        }

        blocks
    }

    fn roundtrip_helper<const NUM_DISTINCT_BLOCK_IDS: usize>() {
        let block_map = get_data::<NUM_DISTINCT_BLOCK_IDS>();
        let packed = PackedBlockMap::new(&block_map);

        for (i, id_from_array) in block_map.iter().copied().enumerate() {
            let x = i % 512;
            let z = i / 512;
            let id_from_packed = packed.get(x, z);

            assert_eq!(id_from_array, id_from_packed);
        }
    }

    #[test]
    fn roundtrip() {
        roundtrip_helper::<1>();
        roundtrip_helper::<2>();
        roundtrip_helper::<10>();
        roundtrip_helper::<1000>();
    }
}
