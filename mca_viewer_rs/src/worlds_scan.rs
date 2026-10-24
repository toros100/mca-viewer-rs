use mca_tools::level_dat::DatReader;
use std::fs::ReadDir;
use std::ops::Deref;
use std::path::PathBuf;

#[derive(nbt::DeserializePayload, Default)]
pub struct LevelDatShell {
    #[nbt(rename = "Data")]
    data: LevelDat,
}

#[derive(Debug, Default, Clone, nbt::DeserializePayload)]
pub struct LevelDat {
    #[nbt(rename = "LevelName")]
    pub level_name: String,

    #[nbt(rename = "LastPlayed")]
    pub last_played: UnixMillisLong,

    #[nbt(rename = "Version")]
    pub version: Version,
}

#[derive(Debug, Default, Clone, nbt::DeserializePayload)]
pub struct Version {
    #[nbt(rename = "Id")]
    pub id: i32,

    #[nbt(rename = "Name")]
    pub name: String,
}

#[derive(Debug, Default, Clone)]
pub struct UnixMillisLong(jiff::Timestamp);

impl Deref for UnixMillisLong {
    type Target = jiff::Timestamp;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'d> nbt::DeserializePayload<'d> for UnixMillisLong {
    const TAG: nbt::Tag = nbt::Tag::Long;
    fn deserialize_payload(&mut self, data: &'d [u8]) -> nbt::DeserializationResult<usize> {
        if data.len() < 8 {
            return Err(nbt::DeserializationError::EOF);
        }
        let val = i64::from_be_bytes(data[..8].try_into().unwrap());

        match jiff::Timestamp::from_millisecond(val) {
            Ok(t) => self.0 = t,
            Err(_) => {
                return Err(nbt::DeserializationError::Custom(
                    "timestamp out of range".into(),
                ));
            }
        }
        Ok(8)
    }
}

/// scans for minecraft worlds, starting at the given path and descending into directories with the
/// specified depth limit (where Option::<usize>::from(depth_limit) == None means no limit)
///
/// worlds are identified by the presence of a level.dat file, which is read to determine the
/// expected directory structure
///
/// once a level.dat file is discovered in a directory, subdirectories are NOT scanned for
/// further level.dat files (i am not aware of that occuring with game-created directories)
///
/// expected usage would be to pass the .minecraft directory path with a limit of around 3 to find
/// all worlds
///
/// TODO: custom dimensions?
pub fn scan_for_worlds(path: PathBuf, depth_limit: impl Into<Option<usize>>) -> Vec<WorldInfo> {
    let mut ldr = DatReader::default();
    let lds = WorldDirScanner::new(path, depth_limit);

    lds.filter_map(|root| {
        let f = std::fs::File::open(root.join("level.dat")).ok()?;
        let level_dat = ldr.read::<LevelDatShell>(f).ok()?.data;

        // directory structure changed with data version 4774
        let (o, n, e) = if level_dat.version.id < 4774 {
            let ow_path = root.join("region");
            let nether_path = root.join("DIM-1").join("region");
            let end_path = root.join("DIM1").join("region");
            (ow_path, nether_path, end_path)
        } else {
            let base = root.join("dimensions").join("minecraft");
            let ow_path = base.join("overworld").join("region");
            let nether_path = base.join("the_nether").join("region");
            let end_path = base.join("the_end").join("region");
            (ow_path, nether_path, end_path)
        };
        Some(WorldInfo {
            level_dat,
            root,
            overworld_regions: as_regions_dir(o),
            nether_regions: as_regions_dir(n),
            end_regions: as_regions_dir(e),
        })
    })
    .collect()
}

/// represents one minecraft world
///
/// `root` is the path of the directory containing level.dat, not of level.dat
/// itself
#[derive(Debug, Clone)]
pub struct WorldInfo {
    pub level_dat: LevelDat,
    pub root: PathBuf,

    // None: corresponding directory was not found (or otherwise not able to be read with
    // read_dir)
    pub overworld_regions: Option<RegionsDir>,
    pub nether_regions: Option<RegionsDir>,
    pub end_regions: Option<RegionsDir>,
}

/// represents a directory of region files associated with one dimension
#[derive(Debug, Clone)]
pub struct RegionsDir {
    pub path: PathBuf,

    // count (of .mca files in the directory) is just used for UI
    pub count: usize,
}

fn as_regions_dir(path: PathBuf) -> Option<RegionsDir> {
    let count = std::fs::read_dir(&path)
        .ok()?
        .filter_map(|d| {
            d.ok()
                .filter(|d| d.file_name().as_encoded_bytes().ends_with(b".mca"))
        })
        .count();

    Some(RegionsDir { path, count })
}

enum DirOrWorldRoot {
    Dir(ReadDir),
    WorldRoot(PathBuf),
}

struct WorldDirScanner {
    dir_stack: Vec<ReadDir>,
    pending: Option<PathBuf>,
    depth_limit: Option<usize>,
}

impl WorldDirScanner {
    pub fn new(path: PathBuf, depth_limit: impl Into<Option<usize>>) -> Self {
        let lim = depth_limit.into();
        Self {
            pending: Some(path),
            dir_stack: Vec::with_capacity(lim.unwrap_or_default()),
            depth_limit: lim,
        }
    }

    fn inspect(p: PathBuf) -> Option<DirOrWorldRoot> {
        match std::fs::symlink_metadata(p.join("level.dat")) {
            Ok(m) if m.is_file() => Some(DirOrWorldRoot::WorldRoot(p)),
            _ => std::fs::read_dir(p).ok().map(DirOrWorldRoot::Dir),
        }
    }
}

impl Iterator for WorldDirScanner {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        let WorldDirScanner {
            dir_stack,
            depth_limit: limit,
            pending,
        } = self;

        if let Some(p) = pending.take()
            && let Some(ldp) = WorldDirScanner::inspect(p)
        {
            match ldp {
                DirOrWorldRoot::WorldRoot(l) => return Some(l),
                DirOrWorldRoot::Dir(read) => dir_stack.push(read),
            };
        }

        loop {
            let read = dir_stack.last_mut()?;
            match read.next() {
                Some(Ok(entry)) => {
                    let Ok(ft) = entry.file_type() else { continue };

                    if ft.is_dir() {
                        if limit.is_some_and(|l| dir_stack.len() >= l) {
                            continue;
                        }

                        let entry_path = entry.path();
                        match WorldDirScanner::inspect(entry_path) {
                            Some(DirOrWorldRoot::WorldRoot(l)) => return Some(l),
                            Some(DirOrWorldRoot::Dir(d)) => dir_stack.push(d),
                            _ => {}
                        }
                    }
                }
                Some(Err(_)) => {}
                None => _ = dir_stack.pop(),
            }
        }
    }
}
