use nbt::{
    ConstBytes, DeserializationError, DeserializePayload, DeserializeReuse, List,
    arrays::LongArrayBytes, read_i32_len,
};

use crate::{bits_required, cautious_unsafe, unpack_from_u64_bytes_dispatch};

const MINECRAFT_AIR: &[u8] = b"minecraft:air";

#[cfg(not(feature = "cautious_unsafe"))]
unsafe impl<'d> DeserializeReuse for Chunk<'d> {
    type Borrow<'b> = Chunk<'b>;
}

#[derive(DeserializePayload)]
pub struct Chunk<'a> {
    #[nbt(rename = "Status")]
    pub status: StatusMinecraftFull,
    pub sections: List<Section<'a>>,
    #[nbt(rename = "Heightmaps")]
    pub heightmaps: Heightmaps,

    #[nbt(skip)]
    sections_lookup: Vec<Option<usize>>,

    #[nbt(skip)]
    pub y_limit: u16,
}

impl<'a> Chunk<'a> {
    pub fn block_at(&'a self, x: u16, y: u16, z: u16) -> &'a [u8] {
        debug_assert!(x < 16);
        debug_assert!(z < 16);
        debug_assert!(y < self.y_limit);

        match self.section_for_y(y) {
            Some(s) => {
                let block_idx = (y % 16) * 256 + 16 * z + x;
                // NOTE: observe that block_idx < 4095 by the above asserts
                s.block_states.get_block(block_idx)
            }
            None => MINECRAFT_AIR,
        }
    }

    pub fn validate_and_init(&mut self) -> anyhow::Result<()> {
        let (min_section_idx, max_section_idx) = self
            .sections
            .iter()
            .map(|s| (s.y, s.y))
            .reduce(|(a_1, b_1), (a_2, b_2)| (a_1.min(a_2), b_1.max(b_2)))
            .ok_or(anyhow::anyhow!("no sections"))?;

        self.y_limit = 16 * ((max_section_idx - min_section_idx) as u16 + 1);

        for s in self.sections.iter_mut() {
            let l = s.block_states.palette.len();
            s.block_states.bit_width = bits_required(l).max(4);
        }

        self.sections_lookup.resize(self.sections.len(), None);

        for i in 0..self.sections.len() {
            let s = &self.sections[i];
            let idx = (s.y - min_section_idx) as usize;
            if idx >= self.sections_lookup.len() {
                // can only happen if there are gaps in the section indices (y field)
                // e.g. consider the case of only 2 sections:
                // s_1 with s_1.y == 0 and s_2 with s_2.y == 3
                // for s_2, we get idx = 3 - 0 = 3 > 2 = self.sections_lookup.len()
                // (this could theoretically be dealt with, but well-formed chunk data always
                // contains all section indices, even if the section is empty)
                anyhow::bail!("unexpected sections layout")
            };

            self.sections_lookup[idx] = Some(i);
        }

        Ok(())
    }

    fn section_for_y(&self, y: u16) -> Option<&Section<'a>> {
        let idx = (y as usize) / 16;
        self.sections_lookup[idx].map(|i| &self.sections[i])
    }
}

impl Default for Chunk<'_> {
    fn default() -> Self {
        Self {
            status: StatusMinecraftFull,
            sections: List::with_capacity(24),
            sections_lookup: Vec::with_capacity(24),
            heightmaps: Heightmaps::default(),
            y_limit: 0u16,
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
    #[inline(always)]
    pub fn world_surface_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_u9_from_u64s(&self.world_surface.0, idx as usize)
    }
    #[inline(always)]
    pub fn ocean_floor_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_u9_from_u64s(&self.ocean_floor.0, idx as usize)
    }
}

#[inline(always)]
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

#[inline(always)]
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
    /// must have idx < 4096
    pub fn get_block(&self, idx: u16) -> &'a [u8] {
        if cautious_unsafe!() {
            assert!(idx < 4096)
        } else {
            debug_assert!(idx < 4096);
        }

        match self.palette.len() {
            0 => MINECRAFT_AIR,
            1 => self.palette[0].name.inner_lifetimed(),
            _ => match self.data {
                Some(ref d) => {
                    let pal_idx =
                        unpack_from_u64_bytes_dispatch(d, self.bit_width, idx as usize) as usize;
                    if pal_idx > self.palette.len() {
                        MINECRAFT_AIR
                    } else {
                        self.palette[pal_idx].name.inner_lifetimed()
                    }
                }
                None => MINECRAFT_AIR,
            },
        }
    }
}

#[derive(DeserializePayload)]
pub struct BlockStates<'a> {
    pub palette: List<Block<'a>>,

    #[nbt(optional)]
    data: Option<LongArrayBytes<'a>>,

    #[nbt(skip)]
    bit_width: usize,
}

impl Default for BlockStates<'_> {
    fn default() -> Self {
        Self {
            palette: List::with_capacity(7),
            data: None,
            bit_width: 0,
        }
    }
}

#[derive(DeserializePayload, Default)]
pub struct Block<'a> {
    #[nbt(rename = "Name")]
    pub name: nbt::RawString<'a>,
}
