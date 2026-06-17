use nbt::DeserializeExt;
use std::ops::{Deref, DerefMut};

use crate::{
    Region, block_id::BlockId, block_map::PackedBlockMap, chunk::Chunk,
    colors::soft_depth_darken_col, get_interner, get_palette, loader,
};

use crate::colors::{calc_water_color, modulate_col};

const WATER: &[u8] = b"minecraft:water";

pub struct HeightCache {
    prev_lower_countours: [[u16; 16]; 32],
    prev_left_contour: [u16; 16],
    next_left_contour: [u16; 16],
    prev_sw_corner: [u16; 32],
    next_sw_corner: [u16; 32],
}

impl Default for HeightCache {
    fn default() -> Self {
        Self {
            prev_lower_countours: [[Self::INVALID_HEIGHT; 16]; 32],
            prev_left_contour: [Self::INVALID_HEIGHT; 16],
            next_left_contour: [Self::INVALID_HEIGHT; 16],
            prev_sw_corner: [Self::INVALID_HEIGHT; 32],
            next_sw_corner: [Self::INVALID_HEIGHT; 32],
        }
    }
}

impl HeightCache {
    const INVALID_HEIGHT: u16 = u16::MAX;
    fn invalidate(&mut self, chunk_x: usize) {
        for v in self.prev_lower_countours[chunk_x].iter_mut() {
            *v = Self::INVALID_HEIGHT
        }
        self.next_sw_corner[chunk_x] = Self::INVALID_HEIGHT
    }
}

fn height_diff(h: u16, other: u16) -> i8 {
    if h == HeightCache::INVALID_HEIGHT || other == HeightCache::INVALID_HEIGHT {
        0
    } else {
        match h.cmp(&other) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

pub struct ColorLookup([BlockId; 512 * 512]);

impl ColorLookup {
    fn clear(&mut self) {
        self.fill(BlockId::INVALID);
    }
}

impl Default for ColorLookup {
    fn default() -> Self {
        Self([BlockId::INVALID; 512 * 512])
    }
}

impl Deref for ColorLookup {
    type Target = [BlockId; 512 * 512];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ColorLookup {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub fn render_region(
    region: &Region,
    img: &mut image::RgbaImage,
    loader: &mut loader::McaLoader,
    col_lookup: &mut ColorLookup,
    depth_darken: bool,
    with_block_map: bool,
) -> anyhow::Result<Option<PackedBlockMap>> {
    let f = std::fs::File::open(&region.path)?;
    loader.load_mca(f)?;
    img.fill(0);

    if with_block_map {
        col_lookup.clear()
    }

    let pal = get_palette();
    let int = get_interner();

    debug_assert_eq!(img.width(), 512);
    debug_assert_eq!(img.height(), 512);

    let mut height_cache = HeightCache::default();
    let mut any_chunk_ok = false;

    #[cfg(not(feature = "cautious_unsafe"))]
    let mut chunk = Chunk::default();

    for chunk_z in 0..32 {
        for chunk_x in (0..32).rev() {
            let chunk_idx = 32 * chunk_z + chunk_x;

            chunk = chunk.reclaim();

            let cd = match loader.get_chunk_data(chunk_idx) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok(cd) => cd,
            };

            #[cfg(not(feature = "cautious_unsafe"))]
            {
                let (c, res) = chunk.try_deserialize_into(cd);
                chunk = c;
                if res.is_err() {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
            }

            #[cfg(feature = "cautious_unsafe")]
            let mut chunk = match nbt::from_bytes::<Chunk>(cd) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok((_, c, _)) => c,
            };

            if chunk.validate_and_init().is_err() {
                height_cache.invalidate(chunk_x);
                continue;
            };

            for i in 0..256 {
                let block_x = i % 16;
                let block_z = i / 16;

                let mut modulus = 0;

                let h = chunk.heightmaps.world_surface_at(block_x, block_z);
                let block = chunk.block_at(block_x, h.saturating_sub(1), block_z);

                let mut col: [u8; 4];
                if with_block_map {
                    let id = int.get_or_intern(block);
                    let block_in_region_idx =
                        16 * chunk_x + block_x as usize + 512 * (16 * chunk_z + block_z as usize);
                    col_lookup[block_in_region_idx] = id;
                    col = pal.get_static_color_by_id(id)
                } else {
                    // lock-free fast path
                    // i don't think there is a measurable difference though
                    // (there would be, if there were a lot of unmapped blocks)
                    col = pal.get_static_color_by_bytes(block);
                }

                if !block.eq(b"minecraft:water") {
                    // TODO: simplify jesus christ
                    //
                    let n_neighbour_height = if block_z > 0 {
                        chunk.heightmaps.world_surface_at(block_x, block_z - 1)
                    } else if chunk_z > 0 {
                        height_cache.prev_lower_countours[chunk_x][block_x as usize]
                    } else {
                        HeightCache::INVALID_HEIGHT
                    };

                    let e_neighbour_height = if block_x < 15 {
                        chunk.heightmaps.world_surface_at(block_x + 1, block_z)
                    } else if chunk_x < 31 {
                        height_cache.prev_left_contour[block_z as usize]
                    } else {
                        HeightCache::INVALID_HEIGHT
                    };

                    let ne_neighbour_height = if block_z > 0 && block_x < 15 {
                        chunk.heightmaps.world_surface_at(block_x + 1, block_z - 1)
                    } else if block_z == 0 && block_x == 15 && chunk_z > 0 && chunk_x < 31 {
                        height_cache.prev_sw_corner[chunk_x + 1]
                    } else if block_z == 0 && chunk_z > 0 && block_x < 15 {
                        height_cache.prev_lower_countours[chunk_x][block_x as usize + 1]
                    } else if block_x == 15 && block_z > 0 && chunk_x < 31 {
                        height_cache.prev_left_contour[block_z as usize - 1]
                    } else {
                        HeightCache::INVALID_HEIGHT
                    };

                    modulus += height_diff(h, n_neighbour_height);
                    modulus += height_diff(h, e_neighbour_height);
                    modulus += height_diff(h, ne_neighbour_height);

                    // NOTE: to get the height shading to be perfect (without any artifacts around
                    // region borders), we would need to take into account the height of some blocks
                    // on the contours of neighbouring regions (e.g. lower contour of the region
                    // "above", analogous to what is done at the chunk-level here)
                    //
                    // (but it would be annoying to not be able to draw the regions completely
                    // independently anymore.)

                    col = modulate_col(col, modulus);
                    if depth_darken && h < 127 {
                        col = soft_depth_darken_col(col, 127 - h)
                    }
                } else {
                    let h_ocean_floor = chunk.heightmaps.ocean_floor_at(block_x, block_z);
                    let block_below =
                        chunk.block_at(block_x, h_ocean_floor.saturating_sub(1), block_z);

                    let depth = h.saturating_sub(h_ocean_floor) + 1;

                    // NOTE: not interning the names of blocks below water
                    // (would not be "inspectable" in any case)
                    col = calc_water_color(block_below, depth)
                }

                img.put_pixel(
                    16 * chunk_x as u32 + block_x as u32,
                    16 * chunk_z as u32 + block_z as u32,
                    image::Rgba::from(col),
                );
                if block_z == 15 {
                    height_cache.prev_lower_countours[chunk_x][block_x as usize] = h
                };
                if block_x == 0 {
                    height_cache.next_left_contour[block_z as usize] = h
                }
                if block_z == 0 && block_z == 15 {
                    height_cache.next_sw_corner[chunk_x] = h
                }
            }
            height_cache.prev_left_contour = height_cache.next_left_contour;
            any_chunk_ok = true
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    if !any_chunk_ok {
        anyhow::bail!("no chunk ok")
    }

    if with_block_map {
        let pack = PackedBlockMap::new(col_lookup);
        Ok(Some(pack))
    } else {
        Ok(None)
    }
}

pub fn render_slice(
    region: &Region,
    img: &mut image::RgbaImage,
    loader: &mut loader::McaLoader,
    color_lookup: &mut ColorLookup,
    mut slice_height: u16,
    depth_darken: bool,
    with_block_map: bool,
) -> anyhow::Result<Option<PackedBlockMap>> {
    let f = std::fs::File::open(&region.path)?;
    loader.load_mca(f)?;
    img.fill(0);
    if with_block_map {
        color_lookup.clear();
    }

    let pal = get_palette();
    let int = get_interner();

    if img.height() != 512 || img.width() != 512 {
        panic!("unexpected img size");
    }

    #[cfg(not(feature = "cautious_unsafe"))]
    let mut chunk = Chunk::default();

    let mut height_cache = HeightCache::default();
    let mut any_chunk_ok = false;
    for chunk_z in 0..32 {
        for chunk_x in (0..32).rev() {
            let chunk_idx = 32 * chunk_z + chunk_x;

            chunk = chunk.reclaim();
            let cd = match loader.get_chunk_data(chunk_idx) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok(cd) => cd,
            };

            #[cfg(not(feature = "cautious_unsafe"))]
            {
                let (c, res) = chunk.try_deserialize_into(cd);
                chunk = c;
                if res.is_err() {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
            }

            #[cfg(feature = "cautious_unsafe")]
            let mut chunk = match nbt::from_bytes::<Chunk>(cd) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok((_, c, _)) => c,
            };

            if chunk.validate_and_init().is_err() {
                height_cache.invalidate(chunk_x);
                continue;
            };

            slice_height = slice_height.min(chunk.y_limit);

            let mut synth_world_surface = [slice_height; 256];
            let mut synth_ocean_floor = [slice_height; 256];

            for j in 0..256 {
                let block_x = (j % 16) as u16;
                let block_z = (j / 16) as u16;

                let block_name = chunk.block_at(block_x, slice_height.saturating_sub(1), block_z);

                if is_airy(block_name) {
                    for k in 2..100 {
                        let h = slice_height.saturating_sub(k);
                        let block_below = chunk.block_at(block_x, h, block_z);
                        synth_world_surface[j] = h + 1;
                        if !is_airy(block_below) {
                            break;
                        }
                    }
                }

                if is_watery(block_name) {
                    for k in 2..100 {
                        let h = slice_height.saturating_sub(k);
                        let block_below = chunk.block_at(block_x, h, block_z);
                        synth_ocean_floor[j] = h + 1;
                        if !is_watery(block_below) {
                            break;
                        }
                    }
                }
            }
            for i in 0..256 {
                let block_x = (i % 16) as u16;
                let block_z = (i / 16) as u16;

                let mut modulus = 0;

                let h = synth_world_surface[i];
                let block = chunk.block_at(block_x, h.saturating_sub(1), block_z);

                let id = int.get_or_intern(block);
                let block_in_region_idx =
                    16 * chunk_x + block_x as usize + 512 * (16 * chunk_z + block_z as usize);
                color_lookup[block_in_region_idx] = id;

                let mut col = pal.get(id);

                if !block.eq(WATER) {
                    // TODO: simplify jesus christ
                    //
                    let n_neighbour_height = if block_z > 0 {
                        synth_world_surface[i - 16]
                    } else if chunk_z > 0 {
                        height_cache.prev_lower_countours[chunk_x][block_x as usize]
                    } else {
                        HeightCache::INVALID_HEIGHT
                    };

                    let e_neighbour_height = if block_x < 15 {
                        synth_world_surface[i + 1]
                    } else if chunk_x < 31 {
                        height_cache.prev_left_contour[block_z as usize]
                    } else {
                        HeightCache::INVALID_HEIGHT
                    };

                    let ne_neighbour_height = if block_z > 0 && block_x < 15 {
                        synth_world_surface[i - 16 + 1]
                    } else if block_z == 0 && block_x == 15 && chunk_z > 0 && chunk_x < 31 {
                        height_cache.prev_sw_corner[chunk_x + 1]
                    } else if block_z == 0 && chunk_z > 0 && block_x < 15 {
                        height_cache.prev_lower_countours[chunk_x][block_x as usize + 1]
                    } else if block_x == 15 && block_z > 0 && chunk_x < 31 {
                        height_cache.prev_left_contour[block_z as usize - 1]
                    } else {
                        HeightCache::INVALID_HEIGHT
                    };

                    modulus += height_diff(h, n_neighbour_height);
                    modulus += height_diff(h, e_neighbour_height);
                    modulus += height_diff(h, ne_neighbour_height);

                    // NOTE: to get the height shading to be perfect (without any artifacts around
                    // region borders), we would need to take into account the height of some blocks
                    // on the contours of neighbouring regions (e.g. lower contour of the region
                    // "above", analogous to what is done at the chunk-level here)
                    //
                    // (but it would be annoying to not be able to draw the regions completely
                    // independently anymore.)

                    col = modulate_col(col, modulus);
                } else {
                    let h_ocean_floor = synth_ocean_floor[i];
                    let block_below =
                        chunk.block_at(block_x, h_ocean_floor.saturating_sub(1), block_z);

                    let depth = h.saturating_sub(h_ocean_floor) + 1;
                    col = calc_water_color(block_below, depth)
                }
                if depth_darken && h < slice_height {
                    col = soft_depth_darken_col(col, slice_height.saturating_sub(h))
                }
                img.put_pixel(
                    16 * chunk_x as u32 + block_x as u32,
                    16 * chunk_z as u32 + block_z as u32,
                    image::Rgba::from(col),
                );
                if block_z == 15 {
                    height_cache.prev_lower_countours[chunk_x][block_x as usize] = h
                };
                if block_x == 0 {
                    height_cache.next_left_contour[block_z as usize] = h
                }
                if block_z == 0 && block_z == 15 {
                    height_cache.next_sw_corner[chunk_x] = h
                }
            }
            height_cache.prev_left_contour = height_cache.next_left_contour;
            any_chunk_ok = true
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    if !any_chunk_ok {
        anyhow::bail!("no chunk ok")
    }

    if with_block_map {
        let pack = PackedBlockMap::new(color_lookup);
        Ok(Some(pack))
    } else {
        Ok(None)
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
