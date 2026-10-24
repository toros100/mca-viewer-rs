use super::slint_gen;

use mca_viewer_rs::worker;

impl From<slint_gen::RenderMode> for worker::RenderMode {
    fn from(value: slint_gen::RenderMode) -> Self {
        match value {
            slint_gen::RenderMode::Slice => worker::RenderMode::Slice,
            slint_gen::RenderMode::Normal => worker::RenderMode::Normal,
        }
    }
}

impl From<worker::RenderMode> for slint_gen::RenderMode {
    fn from(value: worker::RenderMode) -> Self {
        match value {
            worker::RenderMode::Slice => slint_gen::RenderMode::Slice,
            worker::RenderMode::Normal => slint_gen::RenderMode::Normal,
        }
    }
}

impl From<slint_gen::RenderOptions> for worker::RenderOptions {
    fn from(value: slint_gen::RenderOptions) -> Self {
        let slice_height = value.height.clamp(0, i16::MAX as i32) as i16;
        Self {
            depth_darken: value.depth_darken,
            block_map: value.block_map,
            mode: value.mode.into(),
            slice_height,
        }
    }
}

impl From<worker::RenderOptions> for slint_gen::RenderOptions {
    fn from(value: worker::RenderOptions) -> Self {
        Self {
            depth_darken: value.depth_darken,
            block_map: value.block_map,
            mode: value.mode.into(),
            height: value.slice_height as i32,
        }
    }
}

/// not a fan, thanks orphan rule (i should probably just newtype [u8; 4])
pub trait ColorConv<T> {
    fn conv_into(self) -> T;
    #[expect(unused)]
    fn conv_from(c: T) -> Self;
}

impl ColorConv<slint::Color> for [u8; 4] {
    fn conv_into(self) -> slint::Color {
        debug_assert_eq!(self[3], 255);
        slint::Color::from_rgb_u8(self[0], self[1], self[2])
    }

    fn conv_from(c: slint::Color) -> Self {
        debug_assert_eq!(c.alpha(), 255);
        [c.red(), c.green(), c.blue(), 255]
    }
}

impl ColorConv<[u8; 4]> for slint::Color {
    fn conv_into(self) -> [u8; 4] {
        debug_assert_eq!(self.alpha(), 255);
        [self.red(), self.green(), self.blue(), 255]
    }
    fn conv_from(c: [u8; 4]) -> Self {
        debug_assert_eq!(c[3], 255);
        slint::Color::from_rgb_u8(c[0], c[1], c[2])
    }
}
