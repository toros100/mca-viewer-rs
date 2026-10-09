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
        let slice_height = value.height.clamp(0, u16::MAX as i32) as u16;
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
