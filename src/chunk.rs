use serde::Deserialize;

use crate::{bits_required, unpack_from_longs};

// TODO: a lot to optimize here
//
// have to benchmark deserializing long array as actual i64 and then unpacking bits vs keeping them
// as bytes and unpacking from bytes. mildly annoyed that fastnbt apparently does not allow me to
// get the bytes of a long array, and their zero copy view type apparently does not allow indexing?
// might look into writing a custom deserializer, if i even keep using fastnbt at all.
//
// i also want to look into reusing the Chunk struct (in the sense of deserializing into the same
// chunk struct). unless i am missing something, this is probably not possible with serde/fastnbt?
// the allocations are not large (e.g. Vec<&'a str> of size of around 1-20 or so), but it's still an
// allocation and there are a lot of them, so i would expect some measurable benefit from reuse?
// i want to keep the zero copy techniques, so to reuse the memory on for example Vec<&'a str>, i
// would have to use unsafe to erase/transmute the lifetime.
//

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Chunk<'a> {
    #[serde(rename = "Status")]
    pub status: &'a str,

    #[serde(borrow)]
    pub sections: Vec<Section<'a>>,

    #[serde(rename = "yPos")]
    pub y_pos: i32,

    #[serde(rename = "Heightmaps")]
    pub heightmaps: Heightmaps,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Heightmaps {
    #[serde(rename = "WORLD_SURFACE")]
    world_surface: fastnbt::LongArray,

    #[serde(rename = "OCEAN_FLOOR")]
    ocean_floor: fastnbt::LongArray,
}

impl Heightmaps {
    pub fn world_surface_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_from_longs(&self.world_surface, 9, idx as usize)
    }
    pub fn ocean_floor_at(&self, x: u16, z: u16) -> u16 {
        let idx = z * 16 + x;
        unpack_from_longs(&self.ocean_floor, 9, idx as usize)
    }
}

const MINECRAFT_AIR: &str = "minecraft:air";

impl<'a> Chunk<'a> {
    pub fn block_at(&self, x: u16, y: u16, z: u16) -> &'a str {
        match self.section_for_y(y) {
            Some(s) => match s.block_states.palette.len() {
                0 => MINECRAFT_AIR,
                1 => s.block_states.palette[0],
                n => match &s.block_states.data {
                    None => MINECRAFT_AIR,
                    Some(data) => {
                        let bit_width = bits_required(n).max(4);
                        let block_idx = ((y % 16) * 256 + 16 * z + x) as usize;
                        let idx = unpack_from_longs(data, bit_width, block_idx) as usize;
                        if idx >= n {
                            MINECRAFT_AIR
                        } else {
                            s.block_states.palette[idx]
                        }
                    }
                },
            },
            None => MINECRAFT_AIR,
        }
    }

    pub fn section_for_y(&self, y: u16) -> Option<&Section<'a>> {
        let section_idx = (y / 16) as i8 + self.y_pos as i8;
        self.sections
            .iter()
            .find(|&s| s.y == section_idx)
            .map(|v| v as _)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Section<'a> {
    #[serde(rename = "Y")]
    y: i8,

    #[serde(borrow)]
    block_states: BlockState<'a>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct BlockState<'a> {
    #[serde(borrow)]
    #[serde(deserialize_with = "flat_deserialize_palette")]
    palette: Vec<&'a str>,
    data: Option<fastnbt::LongArray>,
}

fn flat_deserialize_palette<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<&'de str>, D::Error> {
    // this is just bad lol
    // tiny ergonomics benefit, complete waste of an allocation
    let blocks = Vec::<Block>::deserialize(d)?;
    Ok(blocks.iter().map(|b| b.name).collect())
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Block<'a> {
    #[serde(rename = "Name", borrow)]
    name: &'a str,
}

#[cfg(test)]
mod tests {

    use crate::{McaLoader, chunk::Chunk};
    use tracing::{debug, info};

    #[test]
    fn load_chunk() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("debug"))
            .init();
        let f = std::fs::File::open("/home/tobias/GolandProjects/go-mca/worlds/flu3/r.-1.-2.mca")
            .unwrap();

        let mut loader = McaLoader::new();

        loader.load_mca(f).unwrap();

        let chunk_data = loader.get_chunk_data(0).unwrap();

        let c: Chunk = fastnbt::from_bytes(chunk_data).unwrap();

        for s in &c.sections {
            debug!(
                "section {} has palette length {}",
                s.y,
                s.block_states.palette.len()
            );
        }

        let h = c.heightmaps.world_surface_at(0, 0);
        debug!("world surface at x={}, z={} is {}", 0, 0, h);

        let block = c.block_at(0, h - 1, 0);

        debug!("block at x={}, y={}, z={} is {}", 0, h - 1, 0, block);
        let block_above = c.block_at(0, h, 0);

        debug!("above that is {}", block_above);

        info!("tracing info")
    }
}
