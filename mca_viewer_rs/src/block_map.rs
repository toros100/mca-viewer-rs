use crate::block_id::BlockId;
use crate::palette::PaletteCtx;
use crate::{cautious_unsafe, cfg_unreachable};
use mca_tools::bitpack::bits_required;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

pub struct BlockMap(Box<[BlockId; 512 * 512]>);

impl BlockMap {
    pub fn clear(&mut self) {
        self.fill(BlockId::INVALID);
    }
}

impl Default for BlockMap {
    fn default() -> Self {
        Self(Box::<[BlockId; 512 * 512]>::try_from(vec![BlockId::INVALID; 512 * 512]).unwrap())
    }
}

impl Deref for BlockMap {
    type Target = [BlockId; 512 * 512];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for BlockMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// storage of every (top) block in a region
/// bit-packed due to RAM prices
pub struct PackedBlockMap<P> {
    palette: Arc<P>,
    local_palette: Vec<BlockId>,
    bit_width: u64,
    u64_div_bit_width: usize,
    mask: u64,
    packed: Vec<u64>,
}

impl<P: PaletteCtx> PackedBlockMap<P> {
    pub fn byte_size(&self) -> usize {
        let struct_size = std::mem::size_of::<Self>();

        let packed_vec_size = self.packed.len() * std::mem::size_of::<u64>(); // u64s

        // BlockId: repr transparent u16
        let palette_vec_size = self.local_palette.len() * std::mem::size_of::<BlockId>();

        // not counting actual size of P since its shared
        struct_size + packed_vec_size + palette_vec_size
    }

    pub fn palette_len(&self) -> usize {
        self.local_palette.len()
    }

    pub fn new(cs: &[BlockId; 512 * 512], origin: Arc<P>) -> Self {
        let mapping = BlockIdMapping::new(cs);

        debug_assert!(!mapping.pal.is_empty());

        if mapping.pal.len() == 1 {
            Self {
                palette: origin,
                local_palette: mapping.into_palette(),
                bit_width: 0,
                u64_div_bit_width: 0,
                packed: Vec::new(),
                mask: 0,
            }
        } else {
            let bit_width = bits_required(mapping.pal.len()) as u64;
            let packed = pack_dispatch(&mapping, bit_width as usize);

            let u64_div_bit_width = 64 / (bit_width as usize);
            let mask = (1u64 << bit_width) - 1;

            Self {
                palette: origin,
                local_palette: mapping.into_palette(),
                bit_width,
                u64_div_bit_width,
                packed,
                mask,
            }
        }
    }
    pub fn get(&self, x: usize, z: usize) -> (BlockId, Option<&[u8]>) {
        // NOTE: maybe i should just reuse the existing unpacking code here

        debug_assert!(x <= 511 && z <= 511);
        debug_assert!(!self.local_palette.is_empty());

        if self.local_palette.len() == 1 {
            let id = self.local_palette[0];
            return (id, self.palette.get_block_name(id));
        };
        let idx = x + z * 512;

        let u64_idx = idx / self.u64_div_bit_width;
        let j = (idx % self.u64_div_bit_width) as u64;

        debug_assert!(u64_idx < self.packed.len());
        let pal_idx = ((self.packed[u64_idx] >> (j * self.bit_width)) & self.mask) as usize;

        debug_assert!(pal_idx < self.local_palette.len());
        let id = self.local_palette[pal_idx];
        (id, self.palette.get_block_name(id))
    }
}

/// helper type for mapping block ids to palette indices
/// (without using a hashmap)
#[derive(Debug)]
struct BlockIdMapping<'a> {
    blocks: &'a [BlockId; 512 * 512],
    pal: Vec<BlockId>,
    m: Vec<u16>,
    rg: BlockIdRange,
}

#[derive(Debug)]
struct BlockIdRange(std::range::RangeInclusive<BlockId>);

impl BlockIdRange {
    fn new(blocks: &[BlockId; 512 * 512]) -> Self {
        let (start, last) = blocks.iter().fold((blocks[0], blocks[0]), |(mi, ma), x| {
            (mi.min(*x), ma.max(*x))
        });

        Self(std::range::RangeInclusive { start, last })
    }
    fn to_index(&self, id: BlockId) -> usize {
        debug_assert!(self.0.contains(&id));
        usize::from(id) - usize::from(self.0.start)
    }

    fn len(&self) -> usize {
        debug_assert!(self.0.start <= self.0.last);
        usize::from(self.0.last) - usize::from(self.0.start) + 1
    }
}

impl<'a> BlockIdMapping<'a> {
    fn new(blocks: &'a [BlockId; 512 * 512]) -> Self {
        let rg = BlockIdRange::new(blocks);

        let mut hits = vec![false; rg.len()];
        let mut pal = vec![];

        for i in blocks {
            let idx = rg.to_index(*i);

            if !hits[idx] {
                hits[idx] = true;
                pal.push(*i)
            }
        }

        let mut m = vec![0u16; rg.len()];

        for (i, &s) in pal.iter().enumerate() {
            let idx = rg.to_index(s);
            m[idx] = i as u16
        }

        Self { pal, m, rg, blocks }
    }

    fn blocks(&self) -> &[BlockId; 512 * 512] {
        self.blocks
    }

    fn get_index(&self, id: BlockId) -> usize {
        let idx = self.rg.to_index(id);
        if cautious_unsafe!() {
            self.m[idx] as usize
        } else {
            // SAFETY:
            // should only be called with the BlockId values it was constructed with, then the index
            // is in bounds by construction
            unsafe { *self.m.get_unchecked(idx) as usize }
        }
    }

    fn into_palette(self) -> Vec<BlockId> {
        self.pal
    }
}

/// returns the number of u64s required to hold num_elements many numbers of the specified bit width
/// (packed numbers not crossing u64 boundaries)
/// assumes 0 < bit_width <= 64
const fn u64s_required(num_elements: usize, bit_width: usize) -> usize {
    (num_elements).div_ceil(64 / bit_width)
}

#[inline]
fn pack_width_8(m: &BlockIdMapping) -> Vec<u64> {
    let mut packed = Vec::with_capacity(512 * 512 / 8);
    packed.extend(
        // SAFETY: 8 divides 512*512
        unsafe { m.blocks().as_chunks_unchecked::<8>() }
            .iter()
            .map(|a| std::array::from_fn(|i| m.get_index(a[7 - i]) as u8))
            .map(u64::from_be_bytes),
    );
    packed
}

#[inline]
fn pack_dispatch(m: &BlockIdMapping, bit_width: usize) -> Vec<u64> {
    debug_assert!(
        (1..=12).contains(&bit_width),
        "unexpected bit width: {bit_width}"
    );
    // NOTE: is it worth it to special case 4?

    macro_rules! pack {
        ($bit_width:literal, $id_mapping:expr) => {
            pack_width_generic::<$bit_width, { 64 / $bit_width }>($id_mapping)
        };
    }

    match bit_width {
        1 => pack!(1, m),
        2 => pack!(2, m),
        3 => pack!(3, m),
        4 => pack!(4, m),
        5 => pack!(5, m),
        6 => pack!(6, m),
        7 => pack!(7, m),
        8 => pack_width_8(m),
        9 => pack!(9, m),
        10 => pack!(10, m),
        11 => pack!(11, m),
        12 => pack!(12, m),
        _ => cfg_unreachable!(),
        // NOTE: this allows for up to 4096 different blocks in a region
        // there are only around 1000 different blocks in the actual game
    }
}

#[inline]
fn pack_width_generic<const BIT_WIDTH: usize, const NUMS_PER_U64: usize>(
    m: &BlockIdMapping,
) -> Vec<u64> {
    const {
        // sanity check
        assert!(64 / BIT_WIDTH == NUMS_PER_U64);
        // (i can't just calculate NUMS_PER_U64 here, because i need to use it as a const param)
        assert!(0 < BIT_WIDTH && BIT_WIDTH <= 12);
    }

    let mut packed = Vec::<u64>::with_capacity(u64s_required(512 * 512, BIT_WIDTH));

    let f = |b: &BlockId| m.get_index(*b) as u64;

    let pack_exact = |bs: &[BlockId; NUMS_PER_U64]| {
        pack_numbers_exact::<BIT_WIDTH, NUMS_PER_U64, BlockId>(bs, f)
    };

    // compiler already does the following optimization in the release build
    // so this is pretty much just optimizing the debug build

    if (512usize * 512usize).is_multiple_of(NUMS_PER_U64) {
        // SAFETY: immediate
        let chunks = unsafe { m.blocks().as_chunks_unchecked::<NUMS_PER_U64>() };
        packed.extend(chunks.iter().map(pack_exact));
    } else {
        let (chunks, rem) = m.blocks().as_chunks::<NUMS_PER_U64>();

        packed.extend(chunks.iter().map(pack_exact));

        // NOTE: !rem.is_empty() by condition (important for correctness)
        packed.push(pack_numbers::<BIT_WIDTH, BlockId>(rem, f));
    }
    packed
}

#[inline]
fn pack_numbers_exact<const BIT_WIDTH: usize, const N: usize, T>(
    nums: &[T; N],
    f: impl Fn(&T) -> u64,
) -> u64 {
    const {
        assert!(64 / BIT_WIDTH == N);
    }
    nums.iter()
        .rev()
        .fold(0u64, |acc, x| (acc << BIT_WIDTH) | f(x))
}

#[inline]
fn pack_numbers<const BIT_WIDTH: usize, T>(nums: &[T], f: impl Fn(&T) -> u64) -> u64 {
    debug_assert!(nums.len() <= 64 / BIT_WIDTH);
    nums.iter()
        .rev()
        .fold(0u64, |acc, x| (acc << BIT_WIDTH) | f(x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::DummyPalette;
    use rand::{SeedableRng, seq::IndexedRandom, seq::IteratorRandom};

    fn get_data(target_bit_width: usize) -> BlockMap {
        assert!((1..=12).contains(&target_bit_width), "bad test");

        let num_distinct_block_ids = 2usize.pow(target_bit_width as u32);

        assert!(bits_required(num_distinct_block_ids) == target_bit_width);

        const SEED: u64 = 1234;
        let mut r = rand::rngs::StdRng::seed_from_u64(SEED);

        let blocks_u16 = (0u16..=u16::MAX).sample(&mut r, num_distinct_block_ids);
        let mut blocks = BlockMap::default();

        for b in blocks.iter_mut() {
            let block = *blocks_u16.choose(&mut r).unwrap();
            *b = block.into()
        }

        blocks
    }

    #[test]
    fn roundtrip() {
        for target_bit_width in 1..=12 {
            let block_map = get_data(target_bit_width);
            let packed = PackedBlockMap::new(&block_map, Arc::new(DummyPalette));

            for (i, id_from_array) in block_map.iter().copied().enumerate() {
                let x = i % 512;
                let z = i / 512;
                let id_from_packed = packed.get(x, z).0;

                assert_eq!(id_from_array, id_from_packed,);
            }
        }
    }
}
