use rustc_hash::FxHasher;
use serde::Deserialize;
use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
    sync::OnceLock,
};

use crate::block_id::{BlockId, BlockIdVariants, DynId, SeqId, StaticId};

type HashMap<K, V> = rustc_hash::FxHashMap<K, V>;

pub trait FxHashExt {
    fn new() -> Self;
}

impl<K, V> FxHashExt for rustc_hash::FxHashMap<K, V> {
    fn new() -> Self {
        rustc_hash::FxHashMap::default()
    }
}

// NOTE: should accept palette as arg
const PALETTE_TOML_STR: &str = include_str!("../palette.toml");

static PALETTE: OnceLock<Palette> = OnceLock::new();
static INTERNER: OnceLock<BlockInterner> = OnceLock::new();
static WATER_COL: OnceLock<[u8; 4]> = OnceLock::new();

pub struct Palette {
    static_palette: Vec<[u8; 4]>,
    default_col: [u8; 4],
    interner: &'static BlockInterner,
}

impl Palette {
    pub fn new(p: Vec<[u8; 4]>, default: [u8; 4]) -> Self {
        Self {
            static_palette: p,
            default_col: default,
            interner: get_interner(),
        }
    }
    pub fn get_static_color_by_id(&self, id: BlockId) -> [u8; 4] {
        match id.into_specialized() {
            // NOTE: this abuses the internals of StaticId
            BlockIdVariants::Static(id) => self.static_palette[usize::from(id) - 1],
            _ => self.default_col,
        }
    }
    pub fn get_static_color_by_bytes(&self, bs: &[u8]) -> [u8; 4] {
        self.interner
            .get_idx_static(bs)
            .map(|i| self.static_palette[u16::from(i) as usize - 1])
            .unwrap_or(self.default_col)
    }
    pub fn get(&self, id: BlockId) -> [u8; 4] {
        match id.into_specialized() {
            BlockIdVariants::Static(id) => self.static_palette[u16::from(id) as usize - 1],
            _ => self.default_col,
        }
    }
}

pub struct Interner<I: SeqId> {
    next: I,
    idx_to_bs: HashMap<I, &'static [u8]>,
    pub bs_to_idx: HashMap<&'static [u8], I>,
}

impl<I: SeqId + Hash> Default for Interner<I> {
    fn default() -> Self {
        Self {
            next: I::MIN,
            idx_to_bs: HashMap::new(),
            bs_to_idx: HashMap::new(),
        }
    }
}

impl<I: Copy + SeqId + PartialOrd + Eq + Hash> Interner<I> {
    fn get_idx(&self, bs: &[u8]) -> Option<I> {
        self.bs_to_idx.get(&bs).copied()
    }

    fn get_or_intern(&mut self, bs: &[u8]) -> I {
        if let Some(i) = self.get_idx(bs) {
            return i;
        };

        let idx = self.next;

        if idx == I::MAX {
            // should never happen
            panic!("interner overflowed");
        };
        self.next = idx.next();

        let bs_static = Box::leak(Vec::from(bs).into_boxed_slice());

        let prev = self.bs_to_idx.insert(bs_static, idx);
        debug_assert!(prev.is_none());

        let prev = self.idx_to_bs.insert(idx, bs_static);
        debug_assert!(prev.is_none());

        idx
    }

    fn get_bs(&self, idx: I) -> Option<&'static [u8]> {
        self.idx_to_bs.get(&idx).copied()
    }
}

pub struct BlockInterner {
    pub static_interner: Interner<StaticId>,
    pub dyn_interner: std::sync::RwLock<Interner<DynId>>,
}

impl BlockInterner {
    fn new(static_interner: Interner<StaticId>) -> Self {
        Self {
            static_interner,
            dyn_interner: std::sync::RwLock::new(Interner::default()),
        }
    }

    pub fn get_idx_static(&self, bs: &[u8]) -> Option<StaticId> {
        self.static_interner.get_idx(bs)
    }

    #[allow(unused)]
    fn get_idx_dyn(&self, bs: &[u8]) -> Option<BlockId> {
        let r = self.dyn_interner.read().unwrap();
        r.get_idx(bs).map(Into::into)
    }

    pub fn get_or_intern(&self, bs: &[u8]) -> BlockId {
        if let Some(i) = self.get_idx_static(bs) {
            return i.into();
        };

        let mut w = self.dyn_interner.write().unwrap();
        w.get_or_intern(bs).into()
    }

    pub fn get_bytes(&self, idx: BlockId) -> Option<&'static [u8]> {
        match idx.into_specialized() {
            BlockIdVariants::Invalid => None,
            BlockIdVariants::Dyn(i) => {
                let r = self.dyn_interner.read().unwrap();
                r.get_bs(i)
            }
            BlockIdVariants::Static(i) => self.static_interner.get_bs(i),
        }
    }
}

#[derive(serde::Deserialize)]
pub struct TomlPalette {
    #[serde(deserialize_with = "deserialize_palette_blocks")]
    pub blocks: HashMap<Vec<u8>, [u8; 4]>,
}

fn parse_hex_rgb(s: &str) -> Result<[u8; 3], String> {
    if s.len() != 7 || !s.is_ascii() || !s.starts_with("#") {
        return Err("expected hex color with format #RRGGBB".to_string());
    }
    // NOTE: slicing can not panic here because of the ASCII check
    let hex = &s[1..];
    let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
    Ok([r, g, b])
}

fn deserialize_palette_blocks<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<HashMap<Vec<u8>, [u8; 4]>, D::Error> {
    let raw = HashMap::<String, String>::deserialize(d)?;
    raw.into_iter()
        .map(|(k, v)| {
            let [r, g, b] = parse_hex_rgb(&v).map_err(serde::de::Error::custom)?;
            Ok((k.as_bytes().to_vec(), [r, g, b, 255]))
        })
        .collect()
}

// HACK: for convenience in dev, should not just unwrap
pub fn get_palette() -> &'static Palette {
    PALETTE.get_or_init(|| {
        let palette: TomlPalette = toml::from_str(PALETTE_TOML_STR).unwrap();

        let mut int = Interner::<StaticId>::default();

        let mut pal = Vec::new();
        // let mut vv = Vec::new();
        for (k, v) in palette.blocks {
            _ = int.get_or_intern(&k);
            pal.push(v);
        }

        let block_interner = BlockInterner::new(int);

        let water_id = block_interner
            .get_idx_static(b"minecraft:water")
            .expect("should have water id");

        let water_col = pal[u16::from(water_id) as usize - 1];

        _ = WATER_COL.get_or_init(|| water_col);
        _ = INTERNER.get_or_init(move || block_interner);

        Palette::new(pal, [0, 0, 255, 255])
    })
}

pub fn get_water_col() -> [u8; 4] {
    *WATER_COL.get().unwrap()
}

pub fn get_interner() -> &'static BlockInterner {
    INTERNER.get().unwrap()
}

pub struct UnhingedHashMap<K, V> {
    inner: rustc_hash::FxHashMap<u64, V>,
    _phantom: PhantomData<K>,
}

impl<K, V> Default for UnhingedHashMap<K, V> {
    fn default() -> Self {
        Self {
            inner: rustc_hash::FxHashMap::default(),
            _phantom: PhantomData,
        }
    }
}

impl<K: Hash + Eq, V> UnhingedHashMap<K, V> {
    const SEED: usize = 123;
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        let mut hasher = FxHasher::with_seed(Self::SEED);
        k.hash(&mut hasher);
        let h = hasher.finish();
        self.inner.insert(h, v)
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        let mut hasher = FxHasher::with_seed(Self::SEED);
        k.hash(&mut hasher);
        let h = hasher.finish();
        self.inner.get(&h)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity_check() {
        let _ = get_palette();
        let int = get_interner();
        let i1 = int.get_idx_static(b"minecraft:water").unwrap();
        let i2 = int.get_idx_static(b"minecraft:water").unwrap();
        assert_eq!(i1, i2);
        let j1 = int.get_or_intern(b"xyz");
        let j2 = int.get_or_intern(b"xyz");
        assert_eq!(j1, j2);

        let s = j1.into_specialized();
        assert!(matches!(s, BlockIdVariants::Dyn(_)));
    }

    #[test]
    fn parse_hex_colors() {
        let c = parse_hex_rgb("#FF00FF").expect("should be Ok");
        assert_eq!(c, [255, 0, 255]);

        let c = parse_hex_rgb("#ff00FF").expect("should be Ok");
        assert_eq!(c, [255, 0, 255]);

        assert!(parse_hex_rgb("").is_err());
        assert!(parse_hex_rgb("AABBCC").is_err());
        assert!(parse_hex_rgb("FF00FX").is_err());
        assert!(parse_hex_rgb("FF00F").is_err());
        assert!(parse_hex_rgb("-FF00F").is_err());
    }

    #[test]
    fn nohash_thing() {
        let palette: TomlPalette = toml::from_str(PALETTE_TOML_STR).unwrap();

        let mut m = UnhingedHashMap::default();

        let mut next = StaticId::MIN;
        for k in palette.blocks.keys() {
            m.insert(k.clone(), next);
            let v = m.get(k);
            assert!(v.is_some());
            assert_eq!(*v.unwrap(), next);
            next = next.next();
        }

        assert_eq!(m.len(), palette.blocks.len())
    }
}
