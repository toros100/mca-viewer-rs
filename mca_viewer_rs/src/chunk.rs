use std::ops::Index;

use nbt::{
    ConstBytes, DeserializationError, DeserializePayload, arrays::LongArrayBytes, read_i32_len,
};

use crate::{block_id::BlockId, cautious_unsafe, palette::PaletteCtx};
use mca_tools::bitpack::{bits_required, unpack_from_u64_bytes_dispatch};

#[derive(DeserializePayload)]
pub struct Chunk<'a> {
    #[nbt(rename = "Status")]
    pub status: StatusMinecraftFull,

    pub sections: Vec<Section<'a>>,
    #[nbt(rename = "yPos")]
    pub y_pos: i32,

    #[nbt(rename = "Heightmaps")]
    pub heightmaps: Heightmaps,

    #[nbt(skip)]
    air_info: BlockInfo,

    #[nbt(skip)]
    pub y_limit: u16,
}

impl<'a> Chunk<'a> {
    /// consumes a Chunk<'a> (typically with borrowed data), clears any references with lifetime 'a
    /// and returns Chunk<'static> using unsafe
    pub fn reclaim(mut self) -> Chunk<'static> {
        self.sanitize();
        // # Safety
        // sanitize gets rid of any 'a-lifetimed references in self (currently only in Vec and
        // Option, which are cleared resp. set to None).
        //
        // i believe this is actually sound, what could possibly go wrong?
        //
        unsafe { std::mem::transmute::<Chunk<'a>, Chunk<'static>>(self) }
    }

    pub fn sanitize(&mut self) {
        // NOTE: deliberately NOT clearing self.sections, so we can keep the delicious heap
        // allocations inside the invidual sections
        self.y_limit = 0;
        for s in self.sections.iter_mut() {
            s.block_states.sanitize();
        }
    }

    pub fn validate_and_init<P: PaletteCtx>(&mut self, p: &P) -> anyhow::Result<()> {
        let (min_section_idx, max_section_idx) = self
            .sections
            .iter()
            .map(|s| (s.y, s.y))
            .reduce(|(a_1, b_1), (a_2, b_2)| (a_1.min(a_2), b_1.max(b_2)))
            .ok_or(anyhow::anyhow!("no sections"))?;

        self.y_limit = 16 * ((max_section_idx - min_section_idx) as u16 + 1);

        for s in self.sections.iter_mut() {
            let l = s.block_states.palette.len();
            s.block_states.bit_width = bits_required(l).clamp(4, 12);

            s.block_states.cached_palette.clear();
            s.block_states.cached_palette_ok = false;
        }

        if !self.sections.is_sorted_by_key(|s| s.y) {
            self.sections.sort_by_key(|s| s.y);
        }

        for [s1, s2] in self.sections.array_windows::<2>() {
            if s1.y + 1 != s2.y {
                anyhow::bail!("unexpected sections layout")
            }
        }

        let air_id = p.get_or_intern_block_id(b"minecraft:air");
        let air_col = p.get_color(air_id);

        self.air_info = BlockInfo {
            id: air_id,
            color: air_col,
            is_airy: true,
            is_watery: true,
        };

        Ok(())
    }

    pub fn non_airy_at_or_below<P: PaletteCtx>(
        &mut self,
        x: u16,
        y: u16,
        z: u16,
        mut limit: u16,
        p: &P,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(z < 16);
        debug_assert!(y < self.y_limit);

        let mut section_idx = (y as usize) / 16;
        let mut local_y = y % 16;

        loop {
            let s = self.sections.get_mut(section_idx)?;
            let c = s.block_states.non_airy_at_or_below(x, local_y, z, limit, p);
            if c.is_some() {
                return c.map(|(yy, b)| (yy + (section_idx as u16) * 16, b));
            } else {
                if limit <= local_y + 1 || section_idx == 0 {
                    return None;
                } else {
                    limit -= local_y + 1;
                    section_idx -= 1;
                    local_y = 15;
                }
            }
        }
    }
    pub fn non_watery_at_or_below<P: PaletteCtx>(
        &mut self,
        x: u16,
        y: u16,
        z: u16,
        mut limit: u16,
        p: &P,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(z < 16);
        debug_assert!(y < self.y_limit);

        let mut section_idx = (y as usize) / 16;
        let mut local_y = y % 16;

        loop {
            let s = self.sections.get_mut(section_idx)?;
            let c = s
                .block_states
                .non_watery_at_or_below(x, local_y, z, limit, p);
            if c.is_some() {
                return c.map(|(yy, b)| (yy + (section_idx as u16) * 16, b));
            } else {
                if limit <= local_y + 1 || section_idx == 0 {
                    return None;
                } else {
                    limit -= local_y + 1;
                    section_idx -= 1;
                    local_y = 15;
                }
            }
        }
    }

    #[inline]
    pub fn block_at<P: PaletteCtx>(&mut self, x: u16, y: u16, z: u16, p: &P) -> BlockInfo {
        debug_assert!(x < 16);
        debug_assert!(z < 16);
        debug_assert!(y < self.y_limit);

        match self.section_for_y(y) {
            Some(s) => {
                let block_idx = (y % 16) * 256 + 16 * z + x;
                // NOTE: observe that block_idx <= 4095 by the above asserts
                s.block_states
                    .get_block(block_idx, p)
                    .unwrap_or(self.air_info)
            }
            None => self.air_info,
        }
    }

    fn section_for_y(&mut self, y: u16) -> Option<&mut Section<'a>> {
        self.sections.get_mut((y as usize) / 16)
    }
}

impl Default for Chunk<'_> {
    fn default() -> Self {
        Self {
            status: StatusMinecraftFull,
            sections: Vec::with_capacity(24),
            heightmaps: Heightmaps::default(),
            y_limit: 0u16,
            air_info: Default::default(),
            y_pos: 0,
        }
    }
}

#[derive(DeserializePayload, Default)]
pub struct Heightmaps {
    #[nbt(rename = "WORLD_SURFACE")]
    world_surface: Heightmap,
    #[nbt(rename = "OCEAN_FLOOR")]
    ocean_floor: Heightmap,
}

impl Heightmaps {
    #[inline]
    pub fn world_surface_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_u9_from_u64s(&self.world_surface.0, idx as usize)
    }
    #[inline]
    pub fn ocean_floor_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_u9_from_u64s(&self.ocean_floor.0, idx as usize)
    }
}

#[inline]
pub fn unpack_u9_from_u64s(longs: &[u64], idx: usize) -> u16 {
    const BIT_WIDTH: usize = 9;
    const LAST_9_MASK: u64 = (1 << BIT_WIDTH) - 1;
    const NUMS_PER_LONG: usize = 64 / BIT_WIDTH;

    let long_idx = idx / NUMS_PER_LONG;
    let u = longs[long_idx];
    let i = idx % NUMS_PER_LONG;
    ((u >> (i * BIT_WIDTH)) & LAST_9_MASK) as u16
}

pub struct Heightmap([u64; 37]);
impl Default for Heightmap {
    fn default() -> Self {
        Self([0u64; 37])
    }
}

impl DeserializePayload<'_> for Heightmap {
    const TAG: nbt::Tag = nbt::Tag::LongArray;
    fn deserialize_payload(&mut self, data: &[u8]) -> nbt::DeserializationResult<usize> {
        let l = read_i32_len(data)?;
        if l != 37 {
            return Err(DeserializationError::Custom(
                "unexected array length".into(),
            ));
        }
        if data.len() < 4 + 37 * 8 {
            return Err(DeserializationError::EOF);
        }

        if let Ok(us) = bytemuck::try_cast_slice::<u8, u64>(&data[4..4 + 37 * 8]) {
            self.0 = std::array::from_fn(|i| u64::from_be(us[i]))
        } else {
            u64x37_from_be_bytes(&mut self.0, &data[4..4 + 37 * 8].try_into().unwrap());
        }
        Ok(4 + 37 * 8)
    }
}

#[inline]
fn u64x37_from_be_bytes(us: &mut [u64; 37], bs: &[u8; 37 * 8]) {
    // SAFETY: 37*8 is a multiple of 8
    unsafe {
        for (u, b) in us.iter_mut().zip(bs.as_chunks_unchecked::<8>()) {
            *u = u64::from_be_bytes(*b)
        }
    }
}

#[derive(Default)]
pub struct StatusMinecraftFull;

impl ConstBytes for StatusMinecraftFull {
    const TAG: nbt::Tag = nbt::Tag::String;
    const BYTES: &'static [u8] = b"\x00\x0eminecraft:full";
    const ERR_MSG: Option<&'static str> = Some("status not minecraft:full");
}

#[derive(DeserializePayload, Default)]
pub struct Section<'a> {
    #[nbt(rename = "Y")]
    y: i8,
    pub block_states: BlockStates<'a>,
}

impl<'a> BlockStates<'a> {
    pub fn cache_palette<P: PaletteCtx>(&mut self, p: &P) {
        self.cached_palette.clear();
        self.cached_palette.reserve(self.palette.len());

        self.all_watery = true;
        self.all_airy = true;

        for bs in self.palette.iter() {
            let id = p.get_or_intern_block_id(bs);
            let is_watery = is_watery(bs);
            let is_airy = is_airy(bs);
            let color = p.get_color(id);

            if !is_airy {
                self.all_airy = false;
            }
            if !is_watery {
                self.all_watery = false;
            }

            self.cached_palette.push(BlockInfo {
                id,
                color,
                is_airy,
                is_watery,
            })
        }

        self.cached_palette_ok = true;
    }

    pub fn non_airy_at_or_below<P: PaletteCtx>(
        &mut self,
        x: u16,
        y: u16,
        z: u16,
        limit: u16,
        p: &P,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(y < 16);
        debug_assert!(z < 16);

        if !self.cached_palette_ok {
            self.cache_palette(p);
        }

        if self.all_airy {
            return None;
        }

        for yb in (0..=y).rev().take(limit as usize) {
            let block_idx = yb * 256 + 16 * z + x;
            if let Some(b) = self.get_block(block_idx, p)
                && !b.is_airy
            {
                return Some((yb, b));
            }
        }

        None
    }
    pub fn non_watery_at_or_below<P: PaletteCtx>(
        &mut self,
        x: u16,
        y: u16,
        z: u16,
        limit: u16,
        p: &P,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(y < 16);
        debug_assert!(z < 16);

        if !self.cached_palette_ok {
            self.cache_palette(p);
        }

        if self.all_watery {
            return None;
        }

        for yb in (0..=y).rev().take(limit as usize) {
            let block_idx = yb * 256 + 16 * z + x;
            if let Some(b) = self.get_block(block_idx, p)
                && !b.is_watery
            {
                return Some((yb, b));
            }
        }

        None
    }

    /// must have idx < 4096
    #[inline]
    pub fn get_block<P: PaletteCtx>(&mut self, idx: u16, p: &P) -> Option<BlockInfo> {
        if cautious_unsafe!() {
            assert!(idx < 4096)
        } else {
            debug_assert!(idx < 4096);
        }

        if !self.cached_palette_ok {
            self.cache_palette(p);
        }

        match self.palette.len() {
            0 => None,
            1 => Some(self.cached_palette[0]),
            _ => match self.data {
                Some(ref d) => {
                    let pal_idx =
                        unpack_from_u64_bytes_dispatch(d, self.bit_width, idx as usize) as usize;
                    if pal_idx > self.palette.len() {
                        None
                    } else {
                        Some(self.cached_palette[pal_idx])
                    }
                }
                None => None,
            },
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct BlockInfo {
    pub id: BlockId,
    pub color: [u8; 4],
    pub is_airy: bool,
    pub is_watery: bool,
}

#[derive(DeserializePayload)]
pub struct BlockStates<'a> {
    pub palette: Palette<'a>,

    #[nbt(skip)]
    all_airy: bool,

    #[nbt(skip)]
    all_watery: bool,

    #[nbt(skip)]
    pub cached_palette: Vec<BlockInfo>,

    #[nbt(skip)]
    cached_palette_ok: bool,

    #[nbt(optional)]
    data: Option<LongArrayBytes<'a>>,

    #[nbt(skip)]
    bit_width: usize,
}

/// used for the palette of blocks per section
/// only stores block names/identifiers, block states are discarded
/// TODO: look into for which game/data versions this structure works
#[derive(Debug, Default)]
pub struct Palette<'a> {
    inner: Vec<&'a [u8]>,
}

impl Palette<'_> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.inner.iter().copied()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }
}

impl<'a> Index<usize> for Palette<'a> {
    type Output = &'a [u8];
    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl<'d> nbt::DeserializePayload<'d> for Palette<'d> {
    const TAG: nbt::Tag = nbt::Tag::List;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> nbt::DeserializationResult<usize> {
        if data.len() < 5 {
            return Err(DeserializationError::EOF);
        };

        let t = nbt::Tag::try_from(data[0])?;
        let mut off = 1;

        let l = nbt::read_i32_len(&data[off..])?;
        off += 4;

        // older versions seem to always have a list of compounds
        // (where the name field is called "Name")
        //
        // newer versions do some very weird stuff:
        // 1. if all blocks are in their default block state, the palette is just a list of strings
        //       (just the block names)
        //
        // 2. otherwise, it is a list of compounds, where the name field is called "id",
        //       and blocks in their default block states are stored as a compound tag with
        //       a single child tag (string tag) with name "" and NOT name "id"

        // this is a slightly hacky way to try to handle any kind of palette shape (that i currently
        // know about)
        //
        // the proper thing to do would be to detect the data version, figure out the exact schema
        // for each version and use a specialized parser per version
        //
        // will have to deal with further changes to the format when i handle biomes, F
        match t {
            nbt::Tag::String => {
                self.inner.clear();
                self.inner.reserve(l);

                for _ in 0..l {
                    let (bs, n) = nbt::deserialize_nbt_str_bytes(&data[off..])?;
                    off += n;
                    self.inner.push(bs);
                }
            }
            nbt::Tag::Compound => {
                self.inner.clear();
                self.inner.reserve(l);

                // this is painful, i wish my rust implementation had the same API as my zig implementation

                'list_payloads: for _ in 0..l {
                    let mut found_block_name = false;
                    loop {
                        if off >= data.len() {
                            return Err(DeserializationError::EOF);
                        }

                        let t = nbt::Tag::try_from(data[off])?;
                        off += 1;

                        if t == nbt::Tag::End {
                            if !found_block_name {
                                return Err(DeserializationError::Custom(
                                    "failed to find block name in palette".into(),
                                ));
                            } else {
                                continue 'list_payloads;
                            }
                        }

                        let (name, n) = nbt::deserialize_nbt_str_bytes(&data[off..])?;
                        off += n;

                        if t == nbt::Tag::String && matches!(name, b"" | b"id" | b"Name") {
                            if found_block_name {
                                return Err(DeserializationError::Custom(
                                    "duplicate block name in palette".into(),
                                ));
                            }
                            let (block_name, n) = nbt::deserialize_nbt_str_bytes(&data[off..])?;
                            off += n;
                            self.inner.push(block_name);
                            found_block_name = true;
                        } else {
                            off += nbt::skip_payload(&data[off..], t)?;
                        }
                    }
                }
            }
            _ => {
                return Err(nbt::DeserializationError::Custom(
                    "bad palette list (neither string nor compound as element tag)".into(),
                ));
            }
        }
        Ok(off)
    }
}

impl BlockStates<'_> {
    fn sanitize(&mut self) {
        self.palette.clear();
        self.data = None;
        self.bit_width = 0;
    }
}

impl Default for BlockStates<'_> {
    fn default() -> Self {
        Self {
            palette: Palette::with_capacity(10),
            cached_palette: Vec::with_capacity(10),
            data: None,
            bit_width: 0,
            cached_palette_ok: false,
            all_airy: true,
            all_watery: true,
        }
    }
}

fn is_airy(block: &[u8]) -> bool {
    block == b"minecraft:air" || block == b"minecraft:cave_air"
}

fn is_watery(block: &[u8]) -> bool {
    // NOTE: skipping air here because we don't want to draw air
    block == b"minecraft:air"
        || block == b"minecraft:cave_air"
        || block == b"minecraft:water"
        || block == b"minecraft:bubble_column"
}
