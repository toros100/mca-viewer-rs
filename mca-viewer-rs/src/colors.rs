use crate::{cfg_unreachable, get_palette, get_water_col};

#[inline(always)]
fn alpha_water_dark_branchless(depth: u16) -> u8 {
    // totally necessary optimization lol
    const VALS: [u8; 25] = [
        255, 255, 204, 204, 153, 153, 153, 102, 102, 102, 102, 102, 102, 51, 51, 51, 51, 51, 51,
        51, 51, 51, 51, 51, 0,
    ];
    VALS[depth.min(24) as usize]
}

#[allow(unused)]
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

#[inline(always)]
fn alpha_water_transparency(depth: u16) -> u8 {
    match depth {
        0 => 140,
        1 => 109,
        2 => 73,
        3 => 43,
        4 => 10,
        _ => 0,
    }
}

#[inline(always)]
/// m must be in -3..=3
pub(crate) fn modulate_col(col: [u8; 4], m: i8) -> [u8; 4] {
    // roughly brightens/darkens a color
    // SCALE_133 is roughly 133%
    // im sure this does not make any sense in color theory
    // i just did it in a way that looks decent to me

    const SCALE_133: u32 = 341;
    const SCALE_121: u32 = 310;
    const SCALE_110: u32 = 282;
    const SCALE_90: u32 = 230;
    const SCALE_81: u32 = 207;
    const SCALE_73: u32 = 187;

    #[inline(always)]
    fn apply_one<const C: u32>(a: u8) -> u8 {
        // for the three constants that fit into u8, the min(255) is unnecessary
        // luckily, the compiler figures it out: https://godbolt.org/z/jYPGEfbcv
        (((a as u32) * C + 128) >> 8).min(255) as u8
    }

    #[inline(always)]
    fn apply_rgb<const C: u32>(col: [u8; 4]) -> [u8; 4] {
        [
            apply_one::<C>(col[0]),
            apply_one::<C>(col[1]),
            apply_one::<C>(col[2]),
            col[3],
        ]
    }

    match m {
        3 => apply_rgb::<SCALE_133>(col),
        2 => apply_rgb::<SCALE_121>(col),
        1 => apply_rgb::<SCALE_110>(col),
        0 => col,
        -1 => apply_rgb::<SCALE_90>(col),
        -2 => apply_rgb::<SCALE_81>(col),
        -3 => apply_rgb::<SCALE_73>(col),
        _ => cfg_unreachable!(),
    }
}

#[inline(always)]
fn alpha_blend(c: u8, d: u8, alpha: u8) -> u8 {
    let c = (c as u32) * (alpha as u32);
    let d = (d as u32) * (255 - alpha as u32);
    let sum = c + d + 128;
    ((sum + (sum >> 8)) >> 8) as u8
}

#[inline(always)]
fn alpha_blend_cols(c_1: [u8; 4], c_2: [u8; 4], alpha: u8) -> [u8; 4] {
    let r = alpha_blend(c_1[0], c_2[0], alpha);
    let g = alpha_blend(c_1[1], c_2[1], alpha);
    let b = alpha_blend(c_1[2], c_2[2], alpha);
    [r, g, b, 255]
}

#[inline(always)]
pub(crate) fn calc_water_color(block_below: &[u8], depth: u16) -> [u8; 4] {
    let water_col = get_water_col();

    let block_col = get_palette().get_static_color_by_bytes(block_below);

    let water_col_dark = alpha_blend_cols(water_col, [0, 0, 0, 255], 153);

    let darkened_water_col = alpha_blend_cols(
        water_col,
        water_col_dark,
        alpha_water_dark_branchless(depth),
    );

    alpha_blend_cols(
        block_col,
        darkened_water_col,
        alpha_water_transparency(depth),
    )
}

#[inline(always)]
pub(crate) fn soft_depth_darken_col(c: [u8; 4], depth: u16) -> [u8; 4] {
    let alpha = alpha_soft_depth_darken_branchless(depth);
    alpha_blend_cols(c, [0, 0, 0, 255], alpha)
}

#[inline(always)]
fn alpha_soft_depth_darken_branchless(depth: u16) -> u8 {
    const VALS: [u8; 29] = [
        255, 255, 255, 245, 245, 245, 245, 245, 180, 180, 180, 180, 180, 180, 180, 180, 130, 130,
        130, 130, 130, 130, 130, 130, 60, 60, 60, 60, 30,
    ];
    VALS[depth.min(28) as usize]
}

#[allow(unused)]
fn alpha_soft_depth_darken(depth: u16) -> u8 {
    match depth {
        0..3 => 255,
        3..8 => 245,
        8..16 => 180,
        16..24 => 130,
        24..28 => 60,
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eq() {
        for depth in 0u16..30 {
            assert_eq!(alpha_water_dark(depth), alpha_water_dark_branchless(depth));
            assert_eq!(
                alpha_soft_depth_darken(depth),
                alpha_soft_depth_darken_branchless(depth)
            )
        }
    }
}
