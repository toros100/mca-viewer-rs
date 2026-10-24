use crate::block_id::BlockId;
use crate::worker::ChunkAccessor;
use std::ops::DerefMut;

use crate::block_map::BlockMap;
use crate::chunk::{BlockInfo, Chunk, SliceChunk};
use crate::color_ops::{blend_over, darken_water, mul_rgb, soft_depth_darken, water_opacity};
use crate::palette::PaletteCtx;
use nbt::DeserializeExt;

#[derive(Debug, thiserror::Error)]
pub enum DrawError {
    #[error("no chunk drawn")]
    NoChunkOk,

    #[error("mca error: {:?}", .0)]
    McaError(#[from] mca_tools::loader::McaError),

    #[error("IO error: {:?}", .0)]
    IoError(#[from] std::io::Error),

    #[error("palette invalidated")]
    PaletteInvalidated,
}

pub struct HeightCache {
    prev_lower_contours: [[u16; 16]; 32],
    prev_left_contour: [u16; 16],
    next_left_contour: [u16; 16],
    prev_sw_corner: [u16; 32],
    next_sw_corner: [u16; 32],
}

impl Default for HeightCache {
    fn default() -> Self {
        Self {
            prev_lower_contours: [[Self::INVALID_HEIGHT; 16]; 32],
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
        for v in self.prev_lower_contours[chunk_x].iter_mut() {
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

pub struct HeightShadeValues(Box<[u8; 512 * 512]>);

impl HeightShadeValues {
    const ADD: u8 = 187;

    // roughly brightens/darkens a color
    // SCALE_133 is roughly 133% etc

    // storing value - 187 to let it fit into u8 instead of u16, adding 187 at usage site

    const SCALE_133: u8 = 154; // 341 - 187
    const SCALE_121: u8 = 123; // 310 - 187
    const SCALE_110: u8 = 94; // 282 - 187
    const SCALE_100: u8 = 68; // 255 - 187 (no-op)
    const SCALE_90: u8 = 43; // 230 - 187
    const SCALE_81: u8 = 20; // 207 - 187
    const SCALE_73: u8 = 0; // 187 - 187

    fn clear(&mut self) {
        self.0.fill(Self::SCALE_100)
    }

    /// must have i < 512*512
    #[inline]
    fn set(&mut self, i: usize, m: i8) {
        debug_assert!((-3..=3).contains(&m));

        let v: u8 = match m {
            3 => Self::SCALE_133,
            2 => Self::SCALE_121,
            1 => Self::SCALE_110,
            -1 => Self::SCALE_90,
            -2 => Self::SCALE_81,
            -3 => Self::SCALE_73,

            // no-op value if for whatever reason a bad m slips through
            // probably better than unreachable_unchecked
            _ => Self::SCALE_100,
        };

        self.0[i] = v;
    }
}

impl Default for HeightShadeValues {
    fn default() -> Self {
        Self(vec![Self::SCALE_100; 512 * 512].try_into().unwrap())
    }
}

#[expect(clippy::too_many_arguments)]
pub fn draw_region<P: PaletteCtx>(
    main_layer: &mut [u8; 512 * 512 * 4],
    water_layer: &mut [u8; 512 * 512 * 4],
    height_shade: &mut HeightShadeValues,
    darken_data: &mut [u8; 512 * 512],
    loader: &mut impl ChunkAccessor,
    block_map: &mut BlockMap,
    depth_darken: bool,
    with_block_map: bool,
    ctx: &P,
) -> Result<(), DrawError> {
    main_layer.fill(0);
    water_layer.fill(0);

    let main_layer = unsafe { main_layer.as_chunks_unchecked_mut::<4>() }
        .as_mut_array::<{ 512 * 512 }>()
        .unwrap();

    let water_layer = unsafe { water_layer.as_chunks_unchecked_mut::<4>() }
        .as_mut_array::<{ 512 * 512 }>()
        .unwrap();

    if with_block_map {
        block_map.clear()
    }

    height_shade.clear();
    darken_data.fill(255);

    let block_map = block_map.deref_mut();

    let mut height_cache = HeightCache::default();
    let mut any_chunk_ok = false;

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

            if chunk.sections.validate_and_init(ctx).is_err() {
                height_cache.invalidate(chunk_x);
                continue;
            };

            if !sea_level_found {
                // there is no direct way to distinguish between dimensions in the chunk data.
                // using the y_pos (minimal section index) as a proxy
                //
                // TODO: pass in "sea level" per dimension
                if chunk.y_pos == 0 {
                    // end and nether
                    sea_level = 62;
                } else {
                    // overworld, y_pos would be -4
                    // (126 instead of 62 because i keep y nonnegative)
                    sea_level = 126;
                }
                sea_level_found = true;
            }

            draw_chunk(
                chunk_x,
                chunk_z,
                main_layer,
                water_layer,
                height_shade,
                darken_data,
                ChunkWithPalette {
                    chunk: &mut chunk,
                    palette: ctx,
                },
                block_map,
                &mut height_cache,
                depth_darken,
                sea_level,
                with_block_map,
            );
            any_chunk_ok = true;
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    if !any_chunk_ok {
        return Err(DrawError::NoChunkOk);
    }

    compose_image(
        main_layer.as_mut_array().unwrap(),
        water_layer.as_array().unwrap(),
        height_shade,
        darken_data,
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn draw_slice<P: PaletteCtx>(
    main_layer: &mut [u8; 512 * 512 * 4],
    water_layer: &mut [u8; 512 * 512 * 4],
    height_shade: &mut HeightShadeValues,
    darken_data: &mut [u8; 512 * 512],
    loader: &mut impl ChunkAccessor,
    block_map: &mut BlockMap,
    slice_height: i16,
    depth_darken: bool,
    with_block_map: bool,
    ctx: &P,
) -> Result<(), DrawError> {
    main_layer.fill(0);
    water_layer.fill(0);

    let main_layer = unsafe { main_layer.as_chunks_unchecked_mut::<4>() }
        .as_mut_array()
        .unwrap();

    let water_layer = unsafe { water_layer.as_chunks_unchecked_mut::<4>() }
        .as_mut_array()
        .unwrap();

    if with_block_map {
        block_map.clear();
    }

    height_shade.clear();
    darken_data.fill(255);

    #[cfg(not(feature = "cautious_unsafe"))]
    let mut chunk = crate::chunk::SliceChunk::default();

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
            let mut chunk = match nbt::from_bytes::<SliceChunk>(cd) {
                Err(_) => {
                    height_cache.invalidate(chunk_x);
                    continue;
                }
                Ok((_, c, _)) => c,
            };

            if chunk.sections.validate_and_init(ctx).is_err() {
                height_cache.invalidate(chunk_x);
                continue;
            };

            let slice_height = {
                (slice_height as i32)
                    .saturating_sub(chunk.y_pos.saturating_mul(16))
                    .clamp(0, chunk.sections.y_limit as i32) as u16
            };

            let mut synth_world_surface = [slice_height; 256];
            let mut synth_ocean_floor = [slice_height; 256];

            let limit = 128;
            for j in 0..256 {
                let block_x = (j % 16) as u16;
                let block_z = (j / 16) as u16;

                let h_non_airy = {
                    let res = chunk.sections.non_airy_at_or_below(
                        block_x,
                        slice_height,
                        block_z,
                        limit,
                        ctx,
                    );
                    #[cfg(debug_assertions)]
                    {
                        if let Some((h, b)) = res {
                            let c = chunk.sections.block_at(block_x, h, block_z, ctx);
                            assert_eq!(b.id, c.id);
                            assert!(!c.is_airy);
                        }
                    }
                    res.map_or(slice_height.saturating_sub(limit), |(h, _)| h)
                };

                // + 1 to match how regular heightmaps work
                synth_world_surface[j] = h_non_airy + 1;

                let h_non_watery = {
                    let res = chunk
                        .sections
                        .non_watery_at_or_below(block_x, h_non_airy, block_z, limit, ctx);

                    #[cfg(debug_assertions)]
                    {
                        if let Some((h, b)) = res {
                            let c = chunk.sections.block_at(block_x, h, block_z, ctx);
                            assert_eq!(b.id, c.id);
                            assert!(!c.is_watery);
                        }
                    }

                    res.map_or(h_non_airy.saturating_sub(limit), |(h, _)| h)
                };

                synth_ocean_floor[j] = h_non_watery + 1;
            }

            draw_chunk(
                chunk_x,
                chunk_z,
                main_layer,
                water_layer,
                height_shade,
                darken_data,
                SliceChunkWithPalette {
                    chunk: &mut chunk,
                    palette: ctx,
                    world_surface: &synth_world_surface,
                    ocean_floor: &synth_ocean_floor,
                },
                block_map,
                &mut height_cache,
                depth_darken,
                slice_height,
                with_block_map,
            );

            any_chunk_ok = true;
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    if !any_chunk_ok {
        return Err(DrawError::NoChunkOk);
    }

    compose_image(main_layer, water_layer, height_shade, darken_data);

    Ok(())
}

trait ChunkData {
    fn world_surface_at(&self, x: u16, z: u16) -> u16;
    fn ocean_floor_at(&self, x: u16, z: u16) -> u16;
    fn block_at(&mut self, x: u16, y: u16, z: u16) -> BlockInfo;
}

struct ChunkWithPalette<'d, 'a, P> {
    chunk: &'a mut Chunk<'d>,
    palette: &'a P,
}

impl<'d, 'a, P> ChunkData for ChunkWithPalette<'d, 'a, P>
where
    P: PaletteCtx,
{
    fn block_at(&mut self, x: u16, y: u16, z: u16) -> BlockInfo {
        self.chunk.sections.block_at(x, y, z, self.palette)
    }

    fn ocean_floor_at(&self, x: u16, z: u16) -> u16 {
        self.chunk.heightmaps.ocean_floor_at(x, z)
    }

    fn world_surface_at(&self, x: u16, z: u16) -> u16 {
        self.chunk.heightmaps.world_surface_at(x, z)
    }
}

struct SliceChunkWithPalette<'d, 'a, P> {
    chunk: &'a mut SliceChunk<'d>,
    palette: &'a P,
    world_surface: &'a [u16; 256],
    ocean_floor: &'a [u16; 256],
}

impl<'d, 'a, P> ChunkData for SliceChunkWithPalette<'d, 'a, P>
where
    P: PaletteCtx,
{
    fn block_at(&mut self, x: u16, y: u16, z: u16) -> BlockInfo {
        self.chunk.sections.block_at(x, y, z, self.palette)
    }

    fn world_surface_at(&self, x: u16, z: u16) -> u16 {
        self.world_surface[x as usize + 16 * z as usize]
    }

    fn ocean_floor_at(&self, x: u16, z: u16) -> u16 {
        self.ocean_floor[x as usize + 16 * z as usize]
    }
}

#[expect(clippy::too_many_arguments)]
fn draw_chunk<CD: ChunkData>(
    chunk_x: usize,
    chunk_z: usize,
    main_layer: &mut [[u8; 4]; 512 * 512],
    water_layer: &mut [[u8; 4]; 512 * 512],
    height_shade: &mut HeightShadeValues,
    darken_data: &mut [u8; 512 * 512],
    mut chunk_data: CD,
    block_map: &mut [BlockId; 512 * 512],
    height_cache: &mut HeightCache,
    depth_darken: bool,
    darken_below: u16,
    with_block_map: bool,
) {
    for i in 0..256 {
        let block_x = i % 16;
        let block_z = i / 16;

        let px_x = 16 * chunk_x as u32 + block_x as u32;
        let px_y = 16 * chunk_z as u32 + block_z as u32;
        let px_idx = (px_x + 512 * px_y) as usize;

        let mut modulus = 0;

        let h = chunk_data.world_surface_at(block_x, block_z);
        let block = chunk_data.block_at(block_x, h.saturating_sub(1), block_z);

        if with_block_map {
            let block_in_region_idx =
                16 * chunk_x + block_x as usize + 512 * (16 * chunk_z + block_z as usize);
            block_map[block_in_region_idx] = block.id;
        }

        if !block.is_watery {
            let n_neighbour_height = if block_z > 0 {
                chunk_data.world_surface_at(block_x, block_z - 1)
            } else if chunk_z > 0 {
                height_cache.prev_lower_contours[chunk_x][block_x as usize]
            } else {
                HeightCache::INVALID_HEIGHT
            };

            let e_neighbour_height = if block_x < 15 {
                chunk_data.world_surface_at(block_x + 1, block_z)
            } else if chunk_x < 31 {
                height_cache.prev_left_contour[block_z as usize]
            } else {
                HeightCache::INVALID_HEIGHT
            };

            let ne_neighbour_height = if block_z > 0 && block_x < 15 {
                chunk_data.world_surface_at(block_x + 1, block_z - 1)
            } else if block_z == 0 && block_x == 15 && chunk_z > 0 && chunk_x < 31 {
                height_cache.prev_sw_corner[chunk_x + 1]
            } else if block_z == 0 && chunk_z > 0 && block_x < 15 {
                height_cache.prev_lower_contours[chunk_x][block_x as usize + 1]
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

            main_layer[px_idx] = block.color;
            // main_layer.put_pixel(px_x, px_y, image::Rgba::from(block.color));
            height_shade.set(px_idx, modulus);
        } else {
            let h_ocean_floor = chunk_data.ocean_floor_at(block_x, block_z);
            let block_below =
                chunk_data.block_at(block_x, h_ocean_floor.saturating_sub(1), block_z);

            let depth = h.saturating_sub(h_ocean_floor) + 1;

            main_layer[px_idx] = block_below.color;

            let a = water_opacity(depth);
            let mut wc = block.color;
            wc[3] = a;

            let ad = darken_water(depth);

            darken_data[px_x as usize + px_y as usize * 512] = ad;

            water_layer[px_idx] = wc;
        }
        if depth_darken && h <= darken_below {
            let i = px_x as usize + px_y as usize * 512;
            darken_data[i] = darken_data[i].min(soft_depth_darken(darken_below - h));
        }

        if block_z == 15 {
            height_cache.prev_lower_contours[chunk_x][block_x as usize] = h
        };
        if block_x == 0 {
            height_cache.next_left_contour[block_z as usize] = h
        }
        if block_x == 0 && block_z == 15 {
            height_cache.next_sw_corner[chunk_x] = h
        }
    }
    height_cache.prev_left_contour = height_cache.next_left_contour;
}

pub fn compose_image(
    main_layer: &mut [[u8; 4]; 512 * 512],
    water_layer: &[[u8; 4]; 512 * 512],
    height_shade_data: &HeightShadeValues,
    darken_data: &[u8; 512 * 512],
) {
    // idea:
    // first pass collects data, does all the branchy work second pass (this fn) just
    // applies data uniformly. even though we do a whole second pass and keep more
    // scratch memory, this ends up being way faster, because the color arithmetic now
    // vectorizes very nicely

    // main_layer: non-water blocks, including blocks below water
    // water_layer: only water blocks, with the depth-based opacity already baked in

    // height_shade_data: values based on height difference with neighbours, is applied to a color
    // multiplicatively to brighten/darken

    // darken_data: values applied to a color multiplicatively to darken the color (i.e. water based
    // on depth, or based on depth w.r.t slice height or sea level)

    let height_shade_data: &[u8; 512 * 512] = &height_shade_data.0;

    // no remainder, length is clearly multiple of 4
    // let (pixels, _) = main_layer.as_chunks_mut::<4>();
    // let (water_pixels, _) = water_layer.as_chunks::<4>();

    for (((p, m), wp), d) in main_layer
        .iter_mut()
        .zip(height_shade_data)
        .zip(water_layer)
        .zip(darken_data)
    {
        let m = (*m as u32) + const { HeightShadeValues::ADD as u32 };
        let d = *d as u32;

        // pre-combining height shade and depth-based multipliers
        let c = (m * d + 128) >> 8;

        *p = blend_over(*p, *wp);
        *p = mul_rgb(*p, c)
    }
}
