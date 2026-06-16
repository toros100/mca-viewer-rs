mod app;
mod bitpack;
mod block_id;
mod block_map;
pub mod chunk;
mod colors;
mod loader;
mod palette;
mod render;
mod tile_renderer;

use std::path::{Path, PathBuf};

pub use app::App;
pub use bitpack::*;
pub use loader::*;
pub use palette::*;
pub use render::*;
pub use tile_renderer::*;

#[derive(Clone)]
pub struct Region {
    pub x: i32,
    pub z: i32,
    pub path: PathBuf,
}

impl Region {
    pub fn new(x: i32, z: i32, path: PathBuf) -> Self {
        Self { x, z, path }
    }
}

fn validate_mca_file(pb: PathBuf) -> anyhow::Result<Region> {
    // NOTE: i realized it's very costly to check pb.is_file() here
    // if it's not, that will be caught later

    let s = pb
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("no file name"))?
        .to_str()
        .ok_or(anyhow::anyhow!("name is not valid unicode"))?;

    let (x, z) = parse_mca_file_name(s)?;

    Ok(Region::new(x, z, pb))
}

fn parse_mca_file_name(s: &str) -> anyhow::Result<(i32, i32)> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
        anyhow::bail!("unexpected file name, expected r.x.z.mca")
    };
    let region_x: i32 = parts[1].parse()?;
    let region_z: i32 = parts[2].parse()?;
    Ok((region_x, region_z))
}

pub fn parse_regions_in_dir(path: &Path) -> anyhow::Result<Vec<Region>> {
    Ok(std::fs::read_dir(path)?
        .filter_map(|entry| validate_mca_file(entry.ok()?.path()).ok())
        .collect())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mca_file_name() {
        assert_eq!((-3, 7), parse_mca_file_name("r.-3.7.mca").unwrap());

        assert!(parse_mca_file_name("").is_err());
        assert!(parse_mca_file_name("-3.7.mca").is_err());
        assert!(parse_mca_file_name("r.-3.4.7.mca").is_err());
        assert!(parse_mca_file_name("r.1,2.mca").is_err());
    }
}
