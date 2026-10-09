use std::range::RangeInclusive;

fn validate_mca_file(pb: std::path::PathBuf) -> anyhow::Result<Region> {
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
    let mut it = s.split('.');

    let (Some("r"), Some(x), Some(z), Some("mca"), None) =
        (it.next(), it.next(), it.next(), it.next(), it.next())
    else {
        anyhow::bail!("unexpected file name, expected r.x.z.mca")
    };

    Ok((x.parse()?, z.parse()?))
}

pub fn parse_regions_in_dir(
    path: &std::path::Path,
) -> anyhow::Result<impl Iterator<Item = Region>> {
    Ok(std::fs::read_dir(path)?.filter_map(|entry| validate_mca_file(entry.ok()?.path()).ok()))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionPosition {
    pub x: i32,
    pub z: i32,
}

impl RegionPosition {
    pub fn rect(self) -> vello::kurbo::Rect {
        vello::kurbo::Rect::from_origin_size(
            (self.x as f64 * 512., self.z as f64 * 512.),
            (512., 512.),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionRange {
    full: bool,
    x: RangeInclusive<i32>,
    z: RangeInclusive<i32>,
}

impl RegionRange {
    pub const FULL: Self = Self {
        full: true,
        ..Self::single(0, 0)
    };

    pub const fn new(x_min: i32, x_max: i32, z_min: i32, z_max: i32) -> Self {
        Self {
            full: false,
            x: RangeInclusive {
                start: x_min,
                last: x_max,
            },
            z: RangeInclusive {
                start: z_min,
                last: z_max,
            },
        }
    }

    pub const fn single(x: i32, z: i32) -> Self {
        Self {
            full: false,
            x: RangeInclusive { start: x, last: x },
            z: RangeInclusive { start: z, last: z },
        }
    }

    pub fn grow(self, x: i32, z: i32) -> Self {
        let x = RangeInclusive {
            start: self.x.start - x,
            last: self.x.last + x,
        };
        let z = RangeInclusive {
            start: self.z.start - z,
            last: self.z.last + z,
        };
        Self {
            full: self.full,
            x,
            z,
        }
    }

    pub fn contains(self, pos: RegionPosition) -> bool {
        self.full || (self.x.contains(&pos.x) && self.z.contains(&pos.z))
    }

    pub fn center(self) -> Option<RegionPosition> {
        if self.full {
            return Some(RegionPosition { x: 0, z: 0 });
        }
        if self.is_empty() {
            None
        } else {
            let x = self.x.start + (self.x.last - self.x.start) / 2;
            let z = self.z.start + (self.z.last - self.z.start) / 2;
            Some(RegionPosition::new(x, z))
        }
    }

    pub fn iter_positions(self) -> impl Iterator<Item = RegionPosition> {
        self.z
            .iter()
            .flat_map(move |z| self.x.iter().map(move |x| RegionPosition { x, z }))
    }

    fn is_empty(&self) -> bool {
        !self.full && (self.x.is_empty() || self.z.is_empty())
    }
}

impl RegionPosition {
    pub fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub fn box_dist(self, other: Self) -> u32 {
        self.x.abs_diff(other.x).max(self.z.abs_diff(other.z))
    }
}

#[derive(Debug, Clone)]
pub struct Region {
    pub x: i32,
    pub z: i32,
    pub path: std::path::PathBuf,
}

impl Region {
    pub fn new(x: i32, z: i32, path: std::path::PathBuf) -> Self {
        Self { x, z, path }
    }
    pub fn pos(&self) -> RegionPosition {
        RegionPosition {
            x: self.x,
            z: self.z,
        }
    }
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
