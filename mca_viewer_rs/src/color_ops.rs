/// takes a function and produces a new function based on a const lookup table
///
/// $fn: identifier referring to fn(u16) -> u8, must be a const fn
/// $new_fn: fresh identifier for the new function produced by the macro, will be a const fn
/// $lut_size: size of the lookup table, must have 0 < $lut_size <= u16::MAX + 1
///
/// properties:
/// 1. $new_fn(x) == $fn(x) for x < $lut_size
/// 2. $new_fn(x) == $fn($lut_size - 1) for x >= $lut_size - 1
macro_rules! make_lut {
    ($fn:ident, $(#[$attr:meta])* $vis:vis $new_fn:ident, $lut_size:expr $(,)?) => {
        const _: fn(u16) -> u8 = $fn;
        const _: () = {
            let _: u8 = $fn(0u16); // asserting that $fn is const
            assert!($lut_size > 0usize);
            assert!($lut_size <= u16::MAX as usize + 1);
        };

        #[doc = concat!("generated from [`", stringify!($fn), "`] by [`make_lut`]")]
        $(#[$attr])*
        $vis const fn $new_fn(v: u16) -> u8 {
            const N: usize = $lut_size;
            // should pretty much match what the compiler already does for a match with <= 9 arms
            if N <= 9 {
                const B : usize = if N < 8 { N } else { 8 };
                const PACKED_VALS: u64 = {
                    let mut res = 0u64;
                    let mut i = 0;
                    while i < B {
                        res |= ($fn(i as u16) as u64) << (i * 8);
                        i += 1;
                    }
                    res
                };
                const LAST : u8 = $fn((N - 1) as u16);
                let vu = v as usize;
                if vu < B {
                    (PACKED_VALS >> (vu * 8)) as u8
                } else {
                    LAST
                }
            } else {
                const VALS: [u8; N] = {
                    let mut vs = [0u8; N];
                    let mut i = 0usize;
                    while i < N {
                        vs[i] = $fn(i as u16);
                        i += 1;
                    }
                    vs
                };
                let vu = v as usize;
                VALS[if vu < N { vu } else { N - 1 }]
            }

        }
        const _ : () = {
            const N : usize = $lut_size;
            let mut i = 0usize;
            while i < N {
                assert!($fn(i as u16) == $new_fn(i as u16));
                i += 1;
            }
        };
    };
}

#[expect(unused)]
const fn powi(f: f64, n: usize) -> f64 {
    // helper because f64::powi is not const
    let mut res = 1.0;
    let mut i = 0;
    while i < n {
        res *= f;
        i += 1;
    }
    res
}

const fn darken_water_proto(depth: u16) -> u8 {
    match depth {
        0..2 => 255,
        2..4 => 235,
        4..7 => 215,
        7..13 => 195,
        13..24 => 175,
        24..48 => 155,
        _ => 135,
    }
}

make_lut!(
    darken_water_proto,
    #[inline] pub darken_water,
    49,
);

#[inline]
pub const fn water_opacity_proto(depth: u16) -> u8 {
    let d = depth.saturating_mul(depth);
    let d8 = if d > 255 { 255 } else { d as u8 };
    155u8.saturating_add(d8.saturating_mul(10))
}

make_lut!(
    water_opacity_proto,
    #[inline] pub water_opacity,
    9,
);

// pretty sure a lut would be worse for this function (needs a lot of values, and the arithmetic
// is already simple enough)
#[inline]
pub const fn soft_depth_darken(depth: u16) -> u8 {
    255u16.saturating_sub((1 + (depth / 4)) * 20) as u8
}

/// blends c_2 over c_1 by the alpha value of c_2.
/// the alpha of the returned color is the alpha of c_1.
#[inline]
pub fn blend_over(c_1: [u8; 4], c_2: [u8; 4]) -> [u8; 4] {
    #[inline(always)]
    fn blend_channel(c: u8, d: u8, alpha: u8) -> u8 {
        let c = (c as u32) * (255 - alpha as u32);
        let d = (d as u32) * (alpha as u32);
        let sum = d + c + 128;
        ((sum + (sum >> 8)) >> 8) as u8
    }

    [
        blend_channel(c_1[0], c_2[0], c_2[3]),
        blend_channel(c_1[1], c_2[1], c_2[3]),
        blend_channel(c_1[2], c_2[2], c_2[3]),
        c_1[3],
    ]
}

#[inline]
pub fn mul_rgb(col: [u8; 4], c: u32) -> [u8; 4] {
    #[inline(always)]
    fn mul_channel(a: u8, c: u32) -> u8 {
        (((a as u32) * c + 128) >> 8).min(255) as u8
    }
    [
        mul_channel(col[0], c),
        mul_channel(col[1], c),
        mul_channel(col[2], c),
        col[3],
    ]
}
