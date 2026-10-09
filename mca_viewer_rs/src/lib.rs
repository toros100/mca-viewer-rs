pub mod block_id;
pub mod block_map;
pub mod color_ops;

pub mod draw_region;
pub mod palette;
pub mod region;
pub mod renderer;
pub mod worker;

static_assertions::assert_type_eq_all!(slint::wgpu_30::wgpu::Device, vello::wgpu::Device);

#[macro_export]
macro_rules! cfg_unreachable {
    () => {
        if cfg!(feature = "cautious_unsafe") {
            ::std::unreachable!()
        } else {
            unsafe { ::std::hint::unreachable_unchecked() }
        }
    };
}

#[macro_export]
/// shorthand for cfg!(feature = "cautious_unsafe")
macro_rules! cautious_unsafe {
    () => {
        cfg!(feature = "cautious_unsafe")
    };
}
