mod app;
mod bitpack;
mod chunk;
mod loader;
mod palette;
mod render;

use std::{
    ops::RangeBounds,
    path::{Path, PathBuf},
};

pub use app::App;
pub use bitpack::*;
pub use chunk::*;
pub use loader::*;
pub use palette::*;
pub use render::*;

pub struct Region {
    x: i32,
    z: i32,
    path: PathBuf,
}

impl Region {
    pub fn new(x: i32, z: i32, path: PathBuf) -> Self {
        Self { x, z, path }
    }
}

fn validate_mca_file(pb: PathBuf) -> anyhow::Result<Region> {
    if !pb.is_file() {
        anyhow::bail!("not a file")
    };

    let s = pb
        .file_name()
        .unwrap() // unwrap safe because pb.is_file()
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

pub fn parse_regions_in_dir(
    path: &Path,
    x_range: Option<impl RangeBounds<i32>>,
    z_range: Option<impl RangeBounds<i32>>,
) -> anyhow::Result<Vec<Region>> {
    Ok(std::fs::read_dir(path)?
        .filter_map(|entry| validate_mca_file(entry.ok()?.path()).ok())
        .filter(|region: &Region| {
            x_range.as_ref().is_none_or(|x_rg| x_rg.contains(&region.x))
                && z_range.as_ref().is_none_or(|z_rg| z_rg.contains(&region.z))
        })
        .collect())
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
