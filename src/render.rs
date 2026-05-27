use tracing::debug;

use crate::{Chunk, get_palette, loader};

struct HeightCache {
    prev_lower_countours: [[u16; 16]; 32],
    prev_left_contour: [u16; 16],
    next_left_contour: [u16; 16],
    prev_sw_corner: [u16; 32],
    next_sw_corner: [u16; 32],
}

const INVALID_HEIGHT: u16 = u16::MAX;

impl Default for HeightCache {
    fn default() -> Self {
        Self {
            prev_lower_countours: [[INVALID_HEIGHT; 16]; 32],
            prev_left_contour: [INVALID_HEIGHT; 16],
            next_left_contour: [INVALID_HEIGHT; 16],
            prev_sw_corner: [INVALID_HEIGHT; 32],
            next_sw_corner: [INVALID_HEIGHT; 32],
        }
    }
}

impl HeightCache {
    fn invalidate(&mut self, chunk_x: usize) {
        for v in self.prev_lower_countours[chunk_x].iter_mut() {
            *v = INVALID_HEIGHT
        }
        self.next_sw_corner[chunk_x] = INVALID_HEIGHT
    }
}

fn height_diff(h: u16, other: u16) -> i8 {
    if h == INVALID_HEIGHT || other == INVALID_HEIGHT {
        0
    } else {
        match h.cmp(&other) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

pub fn render_mca<T: std::io::Read>(mca_bytes_read: T) -> anyhow::Result<image::RgbaImage> {
    let mut loader = loader::McaLoader::new();

    loader.load_mca(mca_bytes_read)?;

    let pal = get_palette();

    let mut img = image::RgbaImage::new(512, 512);

    let mut height_cache = HeightCache::default();

    for chunk_z in 0..32 {
        for chunk_x in (0..32).rev() {
            let chunk_idx = 32 * chunk_z + chunk_x;

            let cd = match loader.get_chunk_data(chunk_idx) {
                Err(e) => {
                    debug!("failed to load chunk {}: {}", chunk_idx, e);
                    continue;
                }
                Ok(cd) => cd,
            };

            let chunk: Chunk = match fastnbt::from_bytes(cd) {
                Err(e) => {
                    debug!("failed to deserialize chunk data: {}", e);
                    continue;
                }
                Ok(ch) => ch,
            };

            for i in 0..256 {
                let block_x = i % 16;
                let block_z = i / 16;

                let mut modulus = 0;

                let h = chunk.heightmaps.world_surface_at(block_x, block_z);
                let block = chunk.block_at(block_x, h - 1, block_z);
                let mut col = *pal.get(block).unwrap_or(&[0, 0, 255, 255]);

                if !block.eq("minecraft:water") {
                    // TODO: simplify jesus christ
                    //
                    let n_neighbour_height = if block_z > 0 {
                        chunk.heightmaps.world_surface_at(block_x, block_z - 1)
                    } else if chunk_z > 0 {
                        height_cache.prev_lower_countours[chunk_x][block_x as usize]
                    } else {
                        INVALID_HEIGHT
                    };

                    let e_neighbour_height = if block_x < 15 {
                        chunk.heightmaps.world_surface_at(block_x + 1, block_z)
                    } else if chunk_x < 31 {
                        height_cache.prev_left_contour[block_z as usize]
                    } else {
                        INVALID_HEIGHT
                    };

                    let ne_neighbour_height = if block_z > 0 && block_x < 15 {
                        chunk.heightmaps.world_surface_at(block_x + 1, block_z - 1)
                    } else if block_z == 0 && block_x == 15 && chunk_z > 0 && chunk_x < 31 {
                        height_cache.prev_sw_corner[chunk_x + 1]
                    } else if block_z == 0 && chunk_z > 0 && block_x < 15 {
                        height_cache.prev_lower_countours[chunk_x][block_x as usize + 1]
                    } else if block_x == 15 && block_z > 0 {
                        height_cache.prev_left_contour[block_z as usize - 1]
                    } else {
                        INVALID_HEIGHT
                    };

                    modulus += height_diff(h, n_neighbour_height);
                    modulus += height_diff(h, e_neighbour_height);
                    modulus += height_diff(h, ne_neighbour_height);

                    // let val = (h as u8).saturating_sub(128).saturating_mul(2);
                    // let col = &[val; 4];

                    col = modulate_col(col, modulus);
                    if h < 127 {
                        col = soft_depth_darken_col(col, 127 - h)
                    }
                } else {
                    let h_ocean_floor = chunk.heightmaps.ocean_floor_at(block_x, block_z);
                    let block_below = chunk.block_at(block_x, h_ocean_floor - 1, block_z);

                    let depth = h.saturating_sub(h_ocean_floor) + 1;

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
            height_cache.prev_left_contour = height_cache.next_left_contour
        }
        height_cache.prev_sw_corner = height_cache.next_sw_corner
    }

    Ok(img)
}

const SCALE_133: u32 = 341;
const SCALE_121: u32 = 310;
const SCALE_110: u32 = 282;
const SCALE_90: u32 = 230;
const SCALE_81: u32 = 207;
const SCALE_73: u32 = 187;

fn alpha_water_dark(depth: u16) -> u8 {
    match depth {
        0..2 => 255,
        2..4 => 204,
        4..7 => 153,
        7..13 => 102,
        13..24 => 51,
        _ => 0,
    }
}

fn alpha_water_transparency(depth: u16) -> u8 {
    match depth {
        0 => 140,
        1 => 109,
        2 => 73,
        3 => 73,
        4 => 10,
        _ => 0,
    }
}

fn modulate_u8(a: u8, m: i8) -> u8 {
    match m {
        3 => (((a as u32) * SCALE_133 + 128) >> 8).min(255) as u8,
        2 => (((a as u32) * SCALE_121 + 128) >> 8).min(255) as u8,
        1 => (((a as u32) * SCALE_110 + 128) >> 8).min(255) as u8,
        0 => a,
        -1 => (((a as u32) * SCALE_90 + 128) >> 8).min(255) as u8,
        -2 => (((a as u32) * SCALE_81 + 128) >> 8).min(255) as u8,
        -3 => (((a as u32) * SCALE_73 + 128) >> 8).min(255) as u8,
        _ => unreachable!(),
    }
}

fn modulate_col(col: [u8; 4], m: i8) -> [u8; 4] {
    if !(-3..=3).contains(&m) {
        panic!("m out of range (expected -3..=3)")
    }

    let r = modulate_u8(col[0], m);
    let g = modulate_u8(col[1], m);
    let b = modulate_u8(col[2], m);

    [r, g, b, col[3]]
}

fn alpha_blend(c: u8, d: u8, alpha: u8) -> u8 {
    let c = (c as u32) * (alpha as u32);
    let d = (d as u32) * (255 - alpha as u32);
    let sum = c + d + 128;
    ((sum + (sum >> 8)) >> 8) as u8
}

fn alpha_blend_cols(c_1: [u8; 4], c_2: [u8; 4], alpha: u8) -> [u8; 4] {
    let r = alpha_blend(c_1[0], c_2[0], alpha);
    let g = alpha_blend(c_1[1], c_2[1], alpha);
    let b = alpha_blend(c_1[2], c_2[2], alpha);
    [r, g, b, 255]
}

fn calc_water_color(block_below: &str, depth: u16) -> [u8; 4] {
    let water_col = get_palette()
        .get("minecraft:water")
        .expect("should have water col");
    let block_col = get_palette().get(block_below).unwrap_or(water_col);

    let water_col_dark = alpha_blend_cols(*water_col, [0, 0, 0, 255], 153);

    let darkened_water_col = alpha_blend_cols(*water_col, water_col_dark, alpha_water_dark(depth));

    alpha_blend_cols(
        *block_col,
        darkened_water_col,
        alpha_water_transparency(depth),
    )
}

fn soft_depth_darken_col(c: [u8; 4], depth: u16) -> [u8; 4] {
    let alpha = match depth {
        0..3 => return c,
        3..8 => 255,
        8..16 => 180,
        16..24 => 130,
        24..28 => 60,
        _ => 30,
    };

    alpha_blend_cols(c, [0, 0, 0, 255], alpha)
}
