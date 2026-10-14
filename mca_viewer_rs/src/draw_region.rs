use std::ops::DerefMut;

use crate::block_map::BlockMap;
use crate::chunk::Chunk;
use crate::color_ops::{calc_water_color, modulate_color, soft_depth_darken_color};
use crate::palette::PaletteCtx;
use crate::region::Region;
use mca_tools::loader;
use nbt::DeserializeExt;

#[derive(Debug, thiserror::Error)]
pub enum DrawError {
    #[error("no chunk drawn")]
    NoChunkOk,

    #[error("mca error: {:?}", .0)]
    McaError(#[from] mca_tools::loader::McaError),

    #[error("IO error: {:?}", .0)]
    IoError(#[from] std::io::Error),
}

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

// NOTE:
// extract chunk accessor trait?

pub fn draw_region<P: PaletteCtx>(
    region: &Region,
    img: &mut image::RgbaImage,
    loader: &mut loader::McaLoader,
    block_map: &mut BlockMap,
    depth_darken: bool,
    with_block_map: bool,
    ctx: &P,
) -> Result<(), DrawError> {
    let f = std::fs::File::open(&region.path)?;
    loader.load_mca(f)?;
    img.fill(0);

    if with_block_map {
        block_map.clear()
    }

    let block_map = block_map.deref_mut();

    debug_assert_eq!(img.width(), 512);
    debug_assert_eq!(img.height(), 512);

    // let mut height_cache = HeightCache::default();
    // NOTE: put too much stuff on the stack, crashing in debug build
    let mut height_cache = Box::<HeightCache>::default();
    let mut any_chunk_ok = false;

    let water_id = ctx.get_or_intern_block_id(b"minecraft:water");
    let water_col = ctx.get_color(water_id);

    #[cfg(not(feature = "cautious_unsafe"))]
    let mut chunk = Chunk::default();

    // for overworld regions, this will be the actual sea level
    // otherwise, it is a relatively arbitrary value i picked to make the
    // darkening of blocks below "sea level" work
    // (only meaningful for the end dimension, nether can only be rendered as a slice,
    // because this view will always just show the bedrock roof)
    let mut sea_level = 0;
    let mut sea_level_found = false;

    for chunk_z in 0..32 {
        for chunk_x in (0..32).rev() {
            let chunk_idx = 32 * chunk_z + chunk_x;

            #[cfg(not(feature = "cautious_unsafe"))]
            {
                chunk = chunk.reclaim();
            }

            let cd = match loader.get_chunk_data(chunk_idx) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok(cd) => cd,
            };

            #[cfg(not(feature = "cautious_unsafe"))]
            {
                if chunk.try_deserialize_into(cd).is_err() {
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

            if chunk.validate_and_init(ctx).is_err() {
                height_cache.invalidate(chunk_x);
                continue;
            };

            if !sea_level_found {
                // there is no direct way to distinguish between dimensions in the chunk data.
                // using the y_pos (minimal section index) as a proxy
                //
                // could do this detection "globally" (i.e. for the entire collection of regions)
                // and pass it in (either by folder structure or inspecting one chunk)
                if chunk.y_pos == 0 {
                    // end and nether
                    sea_level = 63;
                } else {
                    // overworld, y_pos would be -4
                    // (127 instead of 63 because i keep y nonnegative)
                    sea_level = 127;
                }
                sea_level_found = true;
            }

            for i in 0..256 {
                let block_x = i % 16;
                let block_z = i / 16;

                let mut modulus = 0;

                let h = chunk.heightmaps.world_surface_at(block_x, block_z);
                let block = chunk.block_at(block_x, h.saturating_sub(1), block_z, ctx);

                let mut col: [u8; 4] = block.color;

                if with_block_map {
                    let block_in_region_idx =
                        16 * chunk_x + block_x as usize + 512 * (16 * chunk_z + block_z as usize);
                    block_map[block_in_region_idx] = block.id;
                }

                if block.id != water_id {
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

                    col = modulate_color(col, modulus);
                    if depth_darken && h < sea_level {
                        col = soft_depth_darken_color(col, sea_level - h)
                    }
                } else {
                    let h_ocean_floor = chunk.heightmaps.ocean_floor_at(block_x, block_z);
                    let block_below =
                        chunk.block_at(block_x, h_ocean_floor.saturating_sub(1), block_z, ctx);

                    let depth = h.saturating_sub(h_ocean_floor) + 1;

                    let block_below_col = block_below.color;
                    col = calc_water_color(water_col, block_below_col, depth)
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
                if block_x == 0 && block_z == 15 {
                    height_cache.next_sw_corner[chunk_x] = h
                }
            }
            height_cache.prev_left_contour = height_cache.next_left_contour;
            any_chunk_ok = true
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    if !any_chunk_ok {
        return Err(DrawError::NoChunkOk);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn draw_slice<P: PaletteCtx>(
    region: &Region,
    img: &mut image::RgbaImage,
    loader: &mut loader::McaLoader,
    block_map: &mut BlockMap,
    mut slice_height: u16,
    depth_darken: bool,
    with_block_map: bool,
    ctx: &P,
) -> Result<(), DrawError> {
    let f = std::fs::File::open(&region.path)?;
    loader.load_mca(f)?;
    img.fill(0);
    if with_block_map {
        block_map.clear();
    }

    if img.height() != 512 || img.width() != 512 {
        panic!("unexpected img size");
    }

    let water_id = ctx.get_or_intern_block_id(b"minecraft:water");
    let water_col = ctx.get_color(water_id);

    #[cfg(not(feature = "cautious_unsafe"))]
    let mut chunk = Chunk::default();

    let mut height_cache = HeightCache::default();
    let mut any_chunk_ok = false;
    for chunk_z in 0..32 {
        for chunk_x in (0..32).rev() {
            let chunk_idx = 32 * chunk_z + chunk_x;

            #[cfg(not(feature = "cautious_unsafe"))]
            {
                chunk = chunk.reclaim();
            }

            let cd = match loader.get_chunk_data(chunk_idx) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok(cd) => cd,
            };

            #[cfg(not(feature = "cautious_unsafe"))]
            {
                if chunk.try_deserialize_into(cd).is_err() {
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

            if chunk.validate_and_init(ctx).is_err() {
                height_cache.invalidate(chunk_x);
                continue;
            };

            slice_height = slice_height.min(chunk.y_limit);

            let mut synth_world_surface = [slice_height; 256];
            let mut synth_ocean_floor = [slice_height; 256];

            let limit = 32;
            for j in 0..256 {
                let block_x = (j % 16) as u16;
                let block_z = (j / 16) as u16;

                let h_ = chunk
                    .non_airy_at_or_below(
                        block_x,
                        slice_height.saturating_sub(1),
                        block_z,
                        limit,
                        ctx,
                    )
                    .map_or(slice_height.saturating_sub(limit), |(h, _)| h);

                synth_world_surface[j] = h_ + 1;

                let h_ = chunk
                    .non_watery_at_or_below(
                        block_x, h_, // slice_height.saturating_sub(1),
                        block_z, limit, ctx,
                    )
                    .map_or(slice_height.saturating_sub(limit), |(h, _)| h);

                synth_ocean_floor[j] = h_ + 1;
            }

            for i in 0..256 {
                let block_x = (i % 16) as u16;
                let block_z = (i / 16) as u16;

                let mut modulus = 0;

                let h = synth_world_surface[i];
                let block = chunk.block_at(block_x, h.saturating_sub(1), block_z, ctx);
                let id = block.id;

                let block_in_region_idx =
                    16 * chunk_x + block_x as usize + 512 * (16 * chunk_z + block_z as usize);
                block_map[block_in_region_idx] = id;

                let mut col = block.color;

                if block.id != water_id {
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

                    col = modulate_color(col, modulus);
                } else {
                    let h_ocean_floor = synth_ocean_floor[i];
                    let block_below =
                        chunk.block_at(block_x, h_ocean_floor.saturating_sub(1), block_z, ctx);

                    let depth = h.saturating_sub(h_ocean_floor) + 1;

                    col = calc_water_color(water_col, block_below.color, depth);
                }
                if depth_darken && h < slice_height {
                    col = soft_depth_darken_color(col, slice_height.saturating_sub(h))
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
                if block_x == 0 && block_z == 15 {
                    height_cache.next_sw_corner[chunk_x] = h
                }
            }
            height_cache.prev_left_contour = height_cache.next_left_contour;
            any_chunk_ok = true
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    if !any_chunk_ok {
        return Err(DrawError::NoChunkOk);
    }

    Ok(())
}
