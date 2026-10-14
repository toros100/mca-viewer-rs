use serde::Deserialize;
use std::error::Error;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::DerefMut;
use std::sync::Arc;

use crate::block_id::BlockId;

type HashMap<K, V> = rustc_hash::FxHashMap<K, V>;

pub trait FxHashExt {
    fn new() -> Self;
}

impl<K, V> FxHashExt for rustc_hash::FxHashMap<K, V> {
    fn new() -> Self {
        rustc_hash::FxHashMap::default()
    }
}

pub trait PaletteCtx {
    fn get_or_intern_block_id(&self, block: &[u8]) -> BlockId;
    fn get_color(&self, id: BlockId) -> [u8; 4];
    fn get_block_name(&self, id: BlockId) -> Option<&[u8]>;
}

#[allow(unused)] // dummy to satisfy signatures in tests where the palette is not relevant
pub(crate) struct DummyPalette;

impl PaletteCtx for DummyPalette {
    fn get_color(&self, _: BlockId) -> [u8; 4] {
        [255, 0, 255, 255]
    }
    fn get_or_intern_block_id(&self, _: &[u8]) -> BlockId {
        BlockId::INVALID
    }
    fn get_block_name(&self, _: BlockId) -> Option<&[u8]> {
        None
    }
}

pub struct BlockPalette {
    pub palette: VecPalette,
    pub interner: BlockInterner,
}

impl BlockPalette {
    pub fn new(toml_palette: TomlPalette, default_color: [u8; 4]) -> Self {
        let mut int = Interner::<Static>::default();

        let mut palette = VecPalette::new(default_color);
        for (k, v) in toml_palette.blocks {
            let id = int.get_or_intern(&k);
            palette.set_color(id, v);
        }

        let block_interner = BlockInterner::new(int);

        BlockPalette {
            palette,
            interner: block_interner,
        }
    }

    pub fn set_color(&mut self, id: BlockId, c: [u8; 4]) {
        self.palette.set_color(id, c);
    }

    pub fn has_color(&self, id: BlockId) -> bool {
        id != BlockId::INVALID && id < self.interner.static_interner.max_exclusive
    }

    // HACK:
    pub fn iter_entries(&self) -> impl Iterator<Item = (BlockId, &'static [u8], [u8; 4], bool)> {
        let default_col = self.palette.static_palette[0];
        let mni = self.palette.max_known_id();

        self.palette
            .iter()
            .filter_map(|(id, col, mapped)| {
                let bs = *self.interner.static_interner.id_to_bytes.get(&id)?;
                Some((id, bs, col, mapped))
            })
            .chain(self.interner.static_interner.id_to_bytes.iter().filter_map(
                move |(&id, &name)| {
                    if id > mni {
                        Some((id, name, default_col, false))
                    } else {
                        None
                    }
                },
            ))
    }

    pub fn fix(&mut self) {
        self.interner.fix();
    }

    pub fn get_block_id(&self, bs: &[u8]) -> Option<BlockId> {
        self.interner.get_block_id(bs)
    }

    pub fn deep_clone(&self) -> Self {
        Self {
            palette: self.palette.clone(),
            interner: self.interner.deep_clone(),
        }
    }
}

impl PaletteCtx for BlockPalette {
    fn get_color(&self, id: BlockId) -> [u8; 4] {
        self.palette.get_color(id)
    }

    fn get_or_intern_block_id(&self, block: &[u8]) -> BlockId {
        self.interner.get_or_intern(block)
    }

    fn get_block_name(&self, id: BlockId) -> Option<&[u8]> {
        self.interner.get_bytes(id)
    }
}

impl<P: PaletteCtx> PaletteCtx for Arc<P> {
    fn get_or_intern_block_id(&self, block: &[u8]) -> BlockId {
        self.as_ref().get_or_intern_block_id(block)
    }
    fn get_color(&self, id: BlockId) -> [u8; 4] {
        self.as_ref().get_color(id)
    }
    fn get_block_name(&self, id: BlockId) -> Option<&[u8]> {
        self.as_ref().get_block_name(id)
    }
}

#[derive(Clone)]
pub struct VecPalette {
    // static_palette and mapped should always have the same nonzero length
    //
    // static_palette[i] is the color for BlockId(i)
    // mapped[i] is true iff the color for BlockId(i) is explicitly set
    static_palette: Vec<[u8; 4]>,
    mapped: Vec<bool>,
}

impl VecPalette {
    fn max_known_id(&self) -> BlockId {
        assert!(!self.static_palette.is_empty());
        ((self.static_palette.len() - 1) as u16).into()
    }

    pub fn len(&self) -> usize {
        self.static_palette.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (BlockId, [u8; 4], bool)> {
        self.static_palette
            .iter()
            .enumerate()
            .zip(self.mapped.iter())
            .skip(1)
            .map(|((i, c), b)| {
                let id = (i as u16).into();
                (id, *c, *b)
            })
    }

    fn set_color(&mut self, id: BlockId, c: [u8; 4]) {
        if id == BlockId::INVALID {
            // not meaningful
            return;
        }

        let i = usize::from(id);

        // id = BlockId(i) goes to index i
        // -> need length i + 1

        if self.static_palette.len() > i {
            self.static_palette[i] = c;
            self.mapped[i] = true;
        } else {
            let default_col = self.static_palette[0];

            let d = (i + 1).saturating_sub(self.static_palette.len());
            self.static_palette
                .extend(std::iter::repeat_n(default_col, d));
            self.mapped.extend(std::iter::repeat_n(false, d));

            self.static_palette[i] = c;
            self.mapped[i] = true;
        }
    }

    pub fn new(default_color: [u8; 4]) -> Self {
        Self {
            mapped: vec![false], // entry for BlockId::INVALID, mapped[0] not really meaningful
            static_palette: vec![default_color],
        }
    }

    /// returns the color for a BlockId, default color if id > self.max_known_id()
    pub fn get_color(&self, id: BlockId) -> [u8; 4] {
        let i = usize::from(id);
        let j = if i < self.static_palette.len() { i } else { 0 };
        self.static_palette[j]
    }
}

#[derive(Debug, Clone)]
pub struct Static;
#[derive(Debug, Clone)]
pub struct Dynamic;

impl Default for Interner<Static> {
    fn default() -> Self {
        Self {
            min: BlockId::MIN_VALID,
            max_exclusive: BlockId::MIN_VALID,
            bytes_to_id: Default::default(),
            id_to_bytes: Default::default(),
            _phantom: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct Interner<T> {
    pub min: BlockId,
    pub max_exclusive: BlockId,
    pub id_to_bytes: HashMap<BlockId, &'static [u8]>,
    pub bytes_to_id: HashMap<&'static [u8], BlockId>,
    _phantom: PhantomData<T>,
}

impl Interner<Static> {
    fn drain_from(&mut self, dyn_interner: &mut Interner<Dynamic>) {
        debug_assert_eq!(self.max_exclusive, dyn_interner.min);

        // WARN: assert/check self has enough room left

        debug_assert!(dyn_interner.len() <= self.remaining_capacity());

        for (k, v) in dyn_interner.id_to_bytes.drain() {
            let prev = self.id_to_bytes.insert(k, v);
            debug_assert!(prev.is_none());
            let prev = self.bytes_to_id.insert(v, k);
            debug_assert!(prev.is_none());
        }
        dyn_interner.bytes_to_id.clear();

        dyn_interner.min = dyn_interner.max_exclusive;

        self.max_exclusive = dyn_interner.max_exclusive;
    }
}

impl<T> Interner<T> {
    fn len(&self) -> usize {
        let mi = self.min.into_raw();
        let ma = self.max_exclusive.into_raw();
        debug_assert!(ma == 0 || mi < ma);
        ma.wrapping_sub(mi) as usize
    }

    fn remaining_capacity(&self) -> usize {
        // TODO: how do we encode: len 0 AND remaining_capacity 0
        if self.max_exclusive == BlockId::INVALID {
            0
        } else {
            usize::from(BlockId::MAX) - usize::from(self.max_exclusive) + 1
        }
    }

    fn get_id(&self, bs: &[u8]) -> Option<BlockId> {
        self.bytes_to_id.get(bs).copied()
    }
    fn get_bytes(&self, id: BlockId) -> Option<&'static [u8]> {
        self.id_to_bytes.get(&id).copied()
    }
    fn get_or_intern(&mut self, bs: &[u8]) -> BlockId {
        if let Some(i) = self.get_id(bs) {
            return i;
        };

        if self.max_exclusive == BlockId::INVALID {
            panic!("interner overflowed")
        }

        let next_id = self.max_exclusive;
        self.max_exclusive = self.max_exclusive.next();

        // NOTE: could look into leaking from an arena or other chunked memory rather than
        // individual allocations, but i doubt the performance would change measurably
        let bs_static = bs.to_vec().leak();

        let prev = self.bytes_to_id.insert(bs_static, next_id);
        debug_assert!(prev.is_none());

        let prev = self.id_to_bytes.insert(next_id, bs_static);
        debug_assert!(prev.is_none());

        next_id
    }
}

impl Interner<Dynamic> {
    fn new(s: &Interner<Static>) -> Self {
        Self {
            min: s.max_exclusive,
            max_exclusive: s.max_exclusive,
            id_to_bytes: Default::default(),
            bytes_to_id: Default::default(),
            _phantom: Default::default(),
        }
    }
}

pub struct BlockInterner {
    pub static_interner: Interner<Static>,
    pub dyn_interner: crossbeam_utils::CachePadded<parking_lot::RwLock<Interner<Dynamic>>>,
}

impl BlockInterner {
    pub fn fix(&mut self) {
        let BlockInterner {
            static_interner,
            dyn_interner,
        } = self;

        let mut w = dyn_interner.write();
        static_interner.drain_from(w.deref_mut());
    }

    fn get_block_id(&self, bs: &[u8]) -> Option<BlockId> {
        if let Some(s) = self.static_interner.get_id(bs) {
            return Some(s);
        };
        self.dyn_interner.read().get_id(bs)
    }

    fn new(static_interner: Interner<Static>) -> Self {
        let d = Interner::new(&static_interner);
        Self {
            static_interner,
            dyn_interner: crossbeam_utils::CachePadded::new(parking_lot::RwLock::new(d)),
        }
    }

    pub fn deep_clone(&self) -> Self {
        let st = self.static_interner.clone();
        let r = self.dyn_interner.read();
        let dy = r.clone();

        Self {
            static_interner: st,
            dyn_interner: crossbeam_utils::CachePadded::new(parking_lot::RwLock::new(dy)),
        }
    }

    pub fn get_or_intern(&self, bs: &[u8]) -> BlockId {
        if let Some(i) = self.static_interner.get_id(bs) {
            return i;
        };

        {
            let r = self.dyn_interner.read();
            if let Some(id) = r.get_id(bs) {
                return id;
            }
        }

        let mut w = self.dyn_interner.write();
        w.get_or_intern(bs)
    }

    pub fn get_bytes(&self, idx: BlockId) -> Option<&[u8]> {
        match idx {
            BlockId::INVALID => None,
            id if id < self.static_interner.max_exclusive => self.static_interner.get_bytes(id),
            id => {
                let r = self.dyn_interner.read();
                r.get_bytes(id)
            }
        }
    }
}

#[derive(serde::Deserialize)]
pub struct TomlPalette {
    #[serde(deserialize_with = "deserialize_palette_blocks")]
    pub blocks: HashMap<Vec<u8>, [u8; 4]>,
}

impl TryFrom<&str> for TomlPalette {
    type Error = toml::de::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        toml::from_str(value)
    }
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

impl Error for ParseError {}
impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParseError {
    InvalidHexDigit,
    NonAsciiInput,
    BadFormat,
}

impl ParseError {
    pub const fn message(self) -> &'static str {
        match self {
            ParseError::InvalidHexDigit => "invalid hex digit",
            ParseError::NonAsciiInput => "non-ASCII input",
            ParseError::BadFormat => "bad format",
        }
    }
}

macro_rules! bail_err {
    ($expression:expr) => {
        match $expression {
            Ok(v) => v,
            Err(e) => return Err(e),
        }
    };
}

const fn parse_hex_rgb(src: &str) -> Result<[u8; 3], ParseError> {
    if !src.is_ascii() {
        return Err(ParseError::NonAsciiInput);
    }

    let bs = src.as_bytes();

    if bs.len() != 7 || bs[0] != b'#' {
        return Err(ParseError::BadFormat);
    }

    let r = bail_err!(u8_from_hex(bs[1], bs[2]));
    let g = bail_err!(u8_from_hex(bs[3], bs[4]));
    let b = bail_err!(u8_from_hex(bs[5], bs[6]));

    Ok([r, g, b])
}

const fn u8_from_hex(u1: u8, u2: u8) -> Result<u8, ParseError> {
    let l = bail_err!(nibble_from_hex(u2));
    let h = bail_err!(nibble_from_hex(u1));
    Ok((h << 4) | l)
}

const fn nibble_from_hex(src: u8) -> Result<u8, ParseError> {
    match src.to_ascii_uppercase() {
        b @ b'0'..=b'9' => Ok(b - b'0'),
        b @ b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(ParseError::InvalidHexDigit),
    }
}
#[cfg(test)]
mod tests {

    use super::*;

    struct TestCase {
        input: &'static str,
        res: Result<[u8; 3], ParseError>,
    }

    const TEST_CASES: &[TestCase] = &[
        TestCase {
            input: "FF00FF",
            res: Err(ParseError::BadFormat),
        },
        TestCase {
            input: "#FF00FF",
            res: Ok([255, 0, 255]),
        },
        TestCase {
            input: "FF00FF",
            res: Err(ParseError::BadFormat),
        },
        TestCase {
            input: "",
            res: Err(ParseError::BadFormat),
        },
        TestCase {
            input: "#fF00fFa",
            res: Err(ParseError::BadFormat),
        },
        TestCase {
            input: "#fF00fx",
            res: Err(ParseError::InvalidHexDigit),
        },
        TestCase {
            input: "#aabbcð",
            res: Err(ParseError::NonAsciiInput),
        },
    ];

    #[test]
    fn cases() {
        for (i, t) in TEST_CASES.iter().enumerate() {
            let res = parse_hex_rgb(t.input);

            assert_eq!(t.res, res, "test case {i}: input={:?}", t.input)
        }
    }

    #[test]
    fn roundtrip_u8_from_hex() {
        for u in 0u8..=255 {
            let s = format!("{:02x}", u);
            let bs: [u8; 2] = s.as_bytes().try_into().unwrap();
            let r = u8_from_hex(bs[0], bs[1]);
            assert_eq!(r, Ok(u));
        }
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
}
