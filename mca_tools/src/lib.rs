pub mod bitpack;
pub mod chunk;
pub mod loader;

/// regular `unreachable!()` if  the cautious_unsafe feature is enabled, otherwise `unreachable_unchecked()`
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
