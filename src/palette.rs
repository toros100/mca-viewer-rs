use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

static PALETTE: OnceLock<HashMap<String, [u8; 4]>> = OnceLock::new();

// NOTE: should accept palette as arg
const PALETTE_TOML_STR: &str = include_str!("../palette.toml");

#[derive(serde::Deserialize)]
pub struct Palette {
    #[serde(deserialize_with = "deserialize_palette_blocks")]
    blocks: std::collections::HashMap<String, [u8; 4]>,
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
) -> Result<HashMap<String, [u8; 4]>, D::Error> {
    // TODO: is there an easy way to do this without the intermediate <String, String> map?
    let raw = HashMap::<String, String>::deserialize(d)?;
    raw.into_iter()
        .map(|(k, v)| {
            let [r, g, b] = parse_hex_rgb(&v).map_err(serde::de::Error::custom)?;
            Ok((k, [r, g, b, 255]))
        })
        .collect()
}

// HACK: for convenience in dev, should not just unwrap
pub fn get_palette() -> &'static HashMap<String, [u8; 4]> {
    PALETTE.get_or_init(|| {
        let palette: Palette = toml::from_str(PALETTE_TOML_STR).unwrap();
        palette.blocks
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_palette() {
        // this is just so that if the palette is broken, it will show up with cargo test
        let p = get_palette();
        _ = p.get("minecraft:grass_block").unwrap();
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
