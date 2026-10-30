use std::ops::Index;

use nbt::{
    BumpCtx, ConstBytes, DeserializationError, DeserializePayload, arrays::LongArrayBytes, bumpalo,
    read_i32_len,
};

use crate::{block_id::BlockId, palette::PaletteCtx};
use mca_tools::bitpack::{PackedIndices, PackedIndicesError};

#[derive(Default)]
pub struct ChunkSections<'a> {
    sections: &'a mut [Section<'a>],
    air_info: BlockInfo,
    pub y_limit: u16,
}

impl<'d, C> DeserializePayload<'d, C> for ChunkSections<'d>
where
    C: nbt::BumpCtx<'d>,
{
    const TAG: nbt::Tag = nbt::Tag::List;
    fn deserialize_payload(
        &mut self,
        data: &'d [u8],
        ctx: &mut C,
    ) -> nbt::DeserializationResult<usize> {
        self.sections.deserialize_payload(data, ctx)
    }
}

impl<'a> ChunkSections<'a> {
    pub fn validate_and_init<P: PaletteCtx>(&mut self, p: &P) -> Result<(), DataError> {
        let (min_section_idx, max_section_idx) = self
            .sections
            .iter()
            .map(|s| (s.y, s.y))
            .reduce(|(a_1, b_1), (a_2, b_2)| (a_1.min(a_2), b_1.max(b_2)))
            .ok_or(DataError::NoSections)?;

        self.y_limit = 16 * ((max_section_idx - min_section_idx) as u16 + 1);

        for s in self.sections.iter_mut() {
            match s.block_states.palette.len() {
                0 => return Err(DataError::EmptyPalette),
                1 => s.block_states.indices = SectionIndices::Zero,
                n => {
                    let packed = PackedIndices::new(
                        s.block_states.data.as_ref().map_or(&[], |b| b.as_slice()),
                        n,
                    )?;
                    s.block_states.indices = SectionIndices::Packed(packed);
                }
            }

            s.block_states.cached_palette_ok = false;
        }

        if !self.sections.is_sorted_by_key(|s| s.y) {
            self.sections.sort_by_key(|s| s.y);
        }

        for [s1, s2] in self.sections.array_windows::<2>() {
            if s1.y + 1 != s2.y {
                return Err(DataError::UnexpectedSectionsLayout);
            }
        }

        let air_id = p.get_or_intern_block_id(b"minecraft:air");
        let air_col = p.get_color(air_id);

        self.air_info = BlockInfo {
            id: BlockId::INVALID,
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
        bump: &'a bumpalo::Bump,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(z < 16);

        let mut section_idx = (y as usize) / 16;
        let mut local_y = y % 16;

        loop {
            let s = &mut self.sections.get_mut(section_idx)?;
            let c = s
                .block_states
                .non_airy_at_or_below(x, local_y, z, limit, p, bump);
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
        bump: &'a bumpalo::Bump,
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
                .non_watery_at_or_below(x, local_y, z, limit, p, bump);
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
    pub fn block_at<P: PaletteCtx>(
        &mut self,
        x: u16,
        y: u16,
        z: u16,
        p: &P,
        bump: &'a bumpalo::Bump,
    ) -> BlockInfo {
        debug_assert!(x < 16);
        debug_assert!(z < 16);

        match self.sections.get_mut((y as usize) / 16) {
            Some(s) => {
                let block_idx = (y % 16) * 256 + 16 * z + x;
                // NOTE: observe that block_idx <= 4095 by the above asserts
                s.block_states
                    .get_block(block_idx, p, bump)
                    .unwrap_or(self.air_info)
            }
            None => self.air_info,
        }
    }
}

#[derive(Default, DeserializePayload)]
pub struct Chunk<'a> {
    #[nbt(rename = "Status")]
    pub status: StatusMinecraftFull,

    pub sections: ChunkSections<'a>,
    #[nbt(rename = "yPos")]
    pub y_pos: i32,

    #[nbt(rename = "Heightmaps")]
    pub heightmaps: Heightmaps,
}

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("no sections found")]
    NoSections,
    #[error("unexpected sections layout")]
    UnexpectedSectionsLayout,
    #[error("empty palette")]
    EmptyPalette,
    #[error("packed indices error: {:?}", .0)]
    PackedIndices(#[from] PackedIndicesError),
}

/// chunk type for slice mode, which does not need heightmaps
/// (and the exit_early appears to skip slightly more here, probably not relevant)
#[derive(Default, DeserializePayload)]
#[nbt(exit_early)]
pub struct SliceChunk<'a> {
    #[nbt(rename = "Status")]
    pub status: StatusMinecraftFull,

    pub sections: ChunkSections<'a>,

    #[nbt(rename = "yPos")]
    pub y_pos: i32,
}

#[inline]
pub fn unpack_u9_from_u64s(longs: &[u64; 37], idx: usize) -> u16 {
    const BIT_WIDTH: usize = 9;
    const LAST_9_MASK: u64 = (1 << BIT_WIDTH) - 1;
    const NUMS_PER_LONG: usize = 64 / BIT_WIDTH;

    let long_idx = idx / NUMS_PER_LONG;
    let u = longs[long_idx];
    let i = idx % NUMS_PER_LONG;
    ((u >> (i * BIT_WIDTH)) & LAST_9_MASK) as u16
}

// it appears to be optimal to only pre-unpack the world surface heightmap but not the ocean floor
// one. makes sense given the access pattern: world surface generally gets accessed multiple times
// per position (local height difference computation), ocean floor at most once
#[derive(DeserializePayload, Default)]
pub struct Heightmaps {
    #[nbt(rename = "WORLD_SURFACE")]
    world_surface: UnpackedHeightmap,
    #[nbt(rename = "OCEAN_FLOOR")]
    ocean_floor: Heightmap,
}

impl Heightmaps {
    #[inline]
    pub fn world_surface_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        self.world_surface.0[idx as usize]
    }
    #[inline]
    pub fn ocean_floor_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_u9_from_u64s(&self.ocean_floor.0, idx as usize)
    }
}

pub struct Heightmap([u64; 37]);

impl Default for Heightmap {
    fn default() -> Self {
        Self([0u64; 37])
    }
}

impl<C> DeserializePayload<'_, C> for Heightmap {
    const TAG: nbt::Tag = nbt::Tag::LongArray;
    fn deserialize_payload(&mut self, data: &[u8], _: &mut C) -> nbt::DeserializationResult<usize> {
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

pub struct UnpackedHeightmap([u16; 256]);

impl Default for UnpackedHeightmap {
    fn default() -> Self {
        Self([0u16; 256])
    }
}

impl UnpackedHeightmap {
    fn unpack_from(&mut self, data: &[u8; 37 * 8]) {
        const BIT_WIDTH: usize = 9;
        const LAST_9_MASK: u64 = (1 << BIT_WIDTH) - 1;
        const NUMS_PER_LONG: usize = 64 / BIT_WIDTH;

        fn unpack(bs: [u8; 8]) -> [u16; NUMS_PER_LONG] {
            let u = u64::from_be_bytes(bs);
            std::array::from_fn(|i| ((u >> (i * BIT_WIDTH)) & LAST_9_MASK) as u16)
        }

        let (cs, _) = data.as_chunks::<8>();

        let (ds, rem) = self.0.as_chunks_mut::<7>();

        // comptime known:
        // ds.len() == 36
        // rem.len() == 4

        for (d, c) in ds.iter_mut().zip(cs.iter().copied()) {
            *d = unpack(c)
        }

        for (d, v) in rem.iter_mut().zip(unpack(cs[36])) {
            *d = v
        }
    }
}

impl<C> DeserializePayload<'_, C> for UnpackedHeightmap {
    const TAG: nbt::Tag = nbt::Tag::LongArray;
    fn deserialize_payload(
        &mut self,
        data: &'_ [u8],
        _: &mut C,
    ) -> nbt::DeserializationResult<usize> {
        let l = read_i32_len(data)?;
        if l != 37 {
            return Err(DeserializationError::Custom(
                "unexected array length".into(),
            ));
        }
        if data.len() < 4 + 37 * 8 {
            return Err(DeserializationError::EOF);
        }

        self.unpack_from(&data[4..4 + 37 * 8].try_into().unwrap());

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
    pub fn cache_palette<P: PaletteCtx>(&mut self, p: &P, bump: &'a bumpalo::Bump) {
        self.all_watery = true;
        self.all_airy = true;

        let sl: &mut [BlockInfo] = bump.alloc_slice_fill_default(self.palette.len());

        for (i, bs) in sl.iter_mut().zip(self.palette.iter()) {
            i.id = p.get_or_intern_block_id(bs);
            i.is_watery = is_watery(bs);
            i.is_airy = is_airy(bs);
            i.color = p.get_color(i.id);

            if !i.is_airy {
                self.all_airy = false;
            }
            if !i.is_watery {
                self.all_watery = false;
            }
        }
        self.cached_palette = sl;
        self.cached_palette_ok = true;
        debug_assert_eq!(self.cached_palette.len(), self.palette.len());
    }

    pub fn non_airy_at_or_below<P: PaletteCtx>(
        &mut self,
        x: u16,
        y: u16,
        z: u16,
        limit: u16,
        p: &P,
        bump: &'a bumpalo::Bump,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(y < 16);
        debug_assert!(z < 16);

        if !self.cached_palette_ok {
            self.cache_palette(p, bump);
        }

        if self.all_airy {
            return None;
        }

        for yb in (0..=y).rev().take(limit as usize) {
            let block_idx = yb * 256 + 16 * z + x;
            if let Some(b) = self.get_block(block_idx, p, bump)
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
        bump: &'a bumpalo::Bump,
    ) -> Option<(u16, BlockInfo)> {
        debug_assert!(x < 16);
        debug_assert!(y < 16);
        debug_assert!(z < 16);

        if !self.cached_palette_ok {
            self.cache_palette(p, bump);
        }

        if self.all_watery {
            return None;
        }

        for yb in (0..=y).rev().take(limit as usize) {
            let block_idx = yb * 256 + 16 * z + x;
            if let Some(b) = self.get_block(block_idx, p, bump)
                && !b.is_watery
            {
                return Some((yb, b));
            }
        }

        None
    }

    /// must have idx < 4096
    #[inline]
    pub fn get_block<P: PaletteCtx>(
        &mut self,
        idx: u16,
        p: &P,
        bump: &'a bumpalo::Bump,
    ) -> Option<BlockInfo> {
        debug_assert!(idx < 4096);

        if !self.cached_palette_ok {
            self.cache_palette(p, bump);
        }

        // if self.indices is SectionIndices::Packed(p), then p was made for self.palette.len()
        // so the lengths need to match
        debug_assert_eq!(self.palette.len(), self.cached_palette.len());

        Some(match &self.indices {
            // could store self.cached_palette[0] directly in the Zero variant, but i don't really
            // like that
            SectionIndices::Zero => self.cached_palette[0],
            SectionIndices::Packed(p) => self.cached_palette[p.get(idx as usize)?],
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct BlockInfo {
    pub id: BlockId,
    pub color: [u8; 4],
    pub is_airy: bool,
    pub is_watery: bool,
}

pub enum SectionIndices<'a> {
    Zero,
    Packed(PackedIndices<'a>),
}

#[derive(DeserializePayload)]
pub struct BlockStates<'a> {
    pub palette: Palette<'a>,

    #[nbt(skip)]
    all_airy: bool,

    #[nbt(skip)]
    all_watery: bool,

    #[nbt(skip)]
    pub cached_palette: &'a [BlockInfo],

    #[nbt(skip)]
    cached_palette_ok: bool,

    #[nbt(optional)]
    data: Option<LongArrayBytes<'a>>,

    #[nbt(skip)]
    indices: SectionIndices<'a>,
}

/// used for the palette of blocks per section
/// only stores block names/identifiers, block states are discarded
/// TODO: look into for which game/data versions this structure works
#[derive(Debug, Default)]
pub struct Palette<'a> {
    inner: &'a [&'a [u8]],
}

impl Palette<'_> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.inner.iter().copied()
    }
}

impl<'a> Index<usize> for Palette<'a> {
    type Output = &'a [u8];
    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl<'d, C> nbt::DeserializePayload<'d, C> for Palette<'d>
where
    C: BumpCtx<'d>,
{
    const TAG: nbt::Tag = nbt::Tag::List;
    fn deserialize_payload(
        &mut self,
        data: &'d [u8],
        ctx: &mut C,
    ) -> nbt::DeserializationResult<usize> {
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
                self.inner = ctx.bump().alloc_slice_try_fill_with(
                    l,
                    |_| -> Result<&[u8], DeserializationError> {
                        let (bs, n) = nbt::deserialize_nbt_str_bytes(&data[off..])?;
                        off += n;
                        Ok(bs)
                    },
                )?;
            }
            nbt::Tag::Compound => {
                // this is painful, i wish my rust implementation had the same API as my zig implementation

                self.inner = ctx.bump().alloc_slice_try_fill_with(
                    l,
                    |_| -> Result<&[u8], DeserializationError> {
                        let mut block_name = None;
                        loop {
                            if off >= data.len() {
                                return Err(DeserializationError::EOF);
                            }

                            let t = nbt::Tag::try_from(data[off])?;
                            off += 1;

                            if t == nbt::Tag::End {
                                if let Some(block_name) = block_name {
                                    return Ok(block_name);
                                } else {
                                    return Err(DeserializationError::Custom(
                                        "failed to find block name in palette".into(),
                                    ));
                                }
                            }

                            let (name, n) = nbt::deserialize_nbt_str_bytes(&data[off..])?;
                            off += n;

                            if t == nbt::Tag::String && matches!(name, b"" | b"id" | b"Name") {
                                if block_name.is_some() {
                                    return Err(DeserializationError::Custom(
                                        "duplicate block name in palette".into(),
                                    ));
                                }
                                let (bn, n) = nbt::deserialize_nbt_str_bytes(&data[off..])?;
                                off += n;
                                block_name = Some(bn);
                            } else {
                                off += nbt::skip_payload(&data[off..], t)?;
                            }
                        }
                    },
                )?;
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

impl Default for BlockStates<'_> {
    fn default() -> Self {
        Self {
            palette: Palette::default(),
            cached_palette: &[],
            data: None,
            cached_palette_ok: false,
            indices: SectionIndices::Zero,
            all_airy: true,
            all_watery: true,
        }
    }
}

fn is_airy(block: &[u8]) -> bool {
    matches!(block, b"minecraft:air" | b"minecraft:cave_air",)
}

fn is_watery(block: &[u8]) -> bool {
    // NOTE: skipping air here because we don't want to draw air
    // not filtering the water plants makes the water darkening look bad (bright spots)

    matches!(
        block,
        b"minecraft:air"
            | b"minecraft:cave_air"
            | b"minecraft:water"
            | b"minecraft:bubble_column"
            | b"minecraft:tall_seagrass"
            | b"minecraft:seagrass"
            | b"minecraft:kelp_plant"
            | b"minecraft:kelp"
    )
}
