//! Terrain movement costs (`common/terrain`) and the terrain bitmap's byte
//!
//! This deliberately does not use the map renderer's indexed BMP image: native
//! movement samples file rows and the first byte of each stored pixel. Even the
//! 1-bit case uses byte indexing, rather than decoding packed palette pixels.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    content::normalize_relative,
    fields::invalid,
    fixed5::script_fixed5 as parse_fixed5,
    parser::{self, Document, Item, Value},
};

pub(crate) struct TerrainCosts {
    width: i32,
    height: i32,
    components: usize,
    pitch: usize,
    bytes: Vec<u8>,
    costs: Vec<Result<i64, String>>,
    movement_error: Option<String>,
}

impl TerrainCosts {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let default_bytes = layers.read("map/default.map")?;
        refuse_quoted_control_keys(&default_bytes, &[b"terrain"])?;
        let default = parser::parse(&default_bytes)?;
        let path = terrain_path(&default)?;
        let mut terrain = Self::decode(&layers.read(&path)?)?;
        let mut categories = Vec::new();
        // The map palette's file list and order. The native category vector
        // and terrain palette are cumulative; a later palette writer can name
        // an earlier category. `installed_content::terrain::read_definitions` parses
        // the same files for the palette; this parse keeps its own error
        // semantics (quoted control keys refused, a case-mismatched category
        // an error), so only the file list is shared.
        for path in crate::terrain::definition_paths(layers)? {
            let path = path.as_str();
            {
                let bytes = layers.read(path)?;
                refuse_quoted_control_keys(
                    &bytes,
                    &[
                        b"categories",
                        b"terrain",
                        b"type",
                        b"color",
                        b"movement_cost",
                    ],
                )?;
                let document = parser::parse(&bytes)?;
                let variables = file_variables(&document);
                terrain
                    .append_definitions(&document.items, &variables, &mut categories)
                    .map_err(|error| invalid(format!("{path}: {error}")))?;
            }
        }
        Ok(terrain)
    }

    pub fn index(&self, x: i32, y: i32) -> io::Result<u8> {
        if x < 0 || y < 0 {
            return Err(invalid("terrain pixel is outside the written bitmap"));
        }
        if x >= self.width || y >= self.height {
            return Ok(0);
        }
        self.bytes
            .get(y as usize * self.pitch + x as usize * self.components)
            .copied()
            .ok_or_else(|| invalid("terrain pixel is outside the written bitmap"))
    }

    pub fn cost(&self, index: u8) -> io::Result<i64> {
        if let Some(error) = &self.movement_error {
            return Err(invalid(error));
        }
        match self.costs.get(index as usize) {
            Some(Ok(cost)) => Ok(*cost),
            Some(Err(error)) => Err(invalid(error)),
            None => Ok(0),
        }
    }

    fn decode(data: &[u8]) -> io::Result<Self> {
        let head = data
            .get(..54)
            .ok_or_else(|| invalid("terrain BMP has a truncated header"))?;
        if &head[..2] != b"BM" {
            return Err(invalid("terrain BMP has no BM signature"));
        }
        let word = |at| i32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        let width = word(18);
        let height = word(22);
        let bpp = u16::from_le_bytes(head[28..30].try_into().unwrap());
        let components = match bpp {
            1 | 8 => 1usize,
            16 => 2,
            24 => 3,
            _ => return Err(invalid("terrain BMP has unsupported pixel width")),
        };
        if width <= 0 || height <= 0 || word(30) != 0 {
            return Err(invalid(
                "terrain BMP has unsupported dimensions or compression",
            ));
        }
        let row = i64::from(width) * components as i64;
        let pitch = if bpp == 1 { row >> 3 } else { row };
        let padded = if bpp == 1 {
            (row + 3) >> 3
        } else {
            (row + 3) & !3
        };
        let advance = if pitch < 1 { padded } else { pitch };
        let allocation = pitch
            .checked_mul(i64::from(height))
            .and_then(|size| size.checked_add(7))
            .ok_or_else(|| invalid("terrain BMP allocation overflows"))?;
        let written = advance
            .checked_mul(i64::from(height - 1))
            .and_then(|size| size.checked_add(padded))
            .ok_or_else(|| invalid("terrain BMP rows overflow"))?;
        if allocation > i64::from(i32::MAX) || allocation < written || advance < 1 {
            return Err(invalid("terrain BMP exceeds native storage limits"));
        }
        // Native 24-bit loading ignores bfOffBits and starts at byte 54.
        let start = if bpp == 24 {
            54
        } else {
            u32::from_le_bytes(head[10..14].try_into().unwrap()) as usize
        };
        let source_len = (padded as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| invalid("terrain BMP source rows overflow"))?;
        let end = start
            .checked_add(source_len)
            .ok_or_else(|| invalid("terrain BMP source offset overflows"))?;
        if end > data.len() {
            return Err(invalid("terrain BMP has truncated pixel rows"));
        }
        let mut bytes = vec![0; written as usize];
        for y in 0..height as usize {
            let destination = y * advance as usize;
            let source = start + y * padded as usize;
            bytes[destination..destination + padded as usize]
                .copy_from_slice(&data[source..source + padded as usize]);
        }
        Ok(Self {
            width,
            height,
            components,
            pitch: advance as usize,
            bytes,
            costs: Vec::new(),
            movement_error: None,
        })
    }

    fn append_definitions(
        &mut self,
        items: &[Item],
        variables: &BTreeMap<Vec<u8>, Vec<u8>>,
        categories: &mut Vec<(Vec<u8>, Result<i64, String>)>,
    ) -> io::Result<()> {
        for container in items {
            if key_matches(container, b"categories") {
                let Some(entries) = container.value.items() else {
                    self.movement_error = Some("terrain categories is not a block".into());
                    continue;
                };
                for category in entries {
                    let Some(name) = category.key.as_ref() else {
                        self.movement_error = Some("unnamed terrain category".into());
                        continue;
                    };
                    if name.starts_with(b"@") {
                        continue;
                    }
                    let cost = match category.value.items() {
                        Some(fields) => {
                            let mut cost = Ok(0);
                            for field in fields {
                                if key_matches(field, b"movement_cost") {
                                    cost = scalar_with_vars(&field.value, variables)
                                        .as_deref()
                                        .and_then(parse_fixed5)
                                        .map(|raw| raw.max(1_000))
                                        .ok_or_else(|| {
                                            "terrain movement_cost is not native fixed5".into()
                                        });
                                }
                            }
                            cost
                        }
                        None => Err("terrain category is not a block".into()),
                    };
                    categories.push((name.clone(), cost));
                }
            } else if key_matches(container, b"terrain") {
                let entries = container
                    .value
                    .items()
                    .ok_or_else(|| invalid("terrain definitions is not a block"))?;
                for definition in entries {
                    if definition
                        .key
                        .as_ref()
                        .is_some_and(|name| name.starts_with(b"@"))
                    {
                        continue;
                    }
                    let fields = definition
                        .value
                        .items()
                        .ok_or_else(|| invalid("terrain definition is not a block"))?;
                    let mut cost = Ok(0);
                    let mut colors = Vec::new();
                    for field in fields {
                        if key_matches(field, b"type") {
                            cost = match scalar_with_vars(&field.value, variables) {
                                Some(name) => match categories
                                    .iter()
                                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&name))
                                {
                                    Some((candidate, value)) if candidate == &name => value.clone(),
                                    Some(_) => Err("terrain category has different case".into()),
                                    None => Ok(0),
                                },
                                None => Err("terrain type is not a scalar".into()),
                            };
                        } else if key_matches(field, b"color") {
                            let values = field
                                .value
                                .items()
                                .ok_or_else(|| invalid("terrain color is not a block"))?;
                            for item in values {
                                let scalar = scalar_with_vars(&item.value, variables)
                                    .ok_or_else(|| invalid("terrain color is not an integer"))?;
                                // ReadScriptSignedInt32@1424AB970 uses the CRT %i scanner.
                                let index = crate::crt::scan_i32(&scalar)
                                    .stored()
                                    .filter(|index| *index >= 0)
                                    .ok_or_else(|| {
                                        invalid("terrain color is not a nonnegative i32")
                                    })?;
                                colors.push(index as usize);
                            }
                        }
                    }
                    for index in colors {
                        // Indices beyond a byte still extend the witnessed
                        // census only as far as the 256 addressable slots.
                        self.costs.resize(
                            self.costs.len().max(index.saturating_add(1).min(256)),
                            Ok(0),
                        );
                        if let Some(slot) = self.costs.get_mut(index) {
                            *slot = cost.clone();
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn key_matches(item: &Item, key: &[u8]) -> bool {
    item.key
        .as_deref()
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(key))
}

/// The shared parser retains the key text but not whether it was quoted. The
/// native terrain reader treats several quoted control keys differently from
/// bare keys, so refuse such content rather than interpreting it as bare.
/// Callers may reuse this for their own default.map control-key list.
pub(super) fn refuse_quoted_control_keys(bytes: &[u8], keys: &[&[u8]]) -> io::Result<()> {
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'#' => {
                at += 1;
                while at < bytes.len() && !matches!(bytes[at], b'\n' | b'\r') {
                    at += 1;
                }
            }
            b'"' => {
                at += 1;
                let mut literal = Vec::new();
                loop {
                    let Some(&byte) = bytes.get(at) else {
                        return Err(invalid("unterminated quoted content key"));
                    };
                    at += 1;
                    match byte {
                        b'"' => break,
                        b'\\' if matches!(bytes.get(at), Some(b'"' | b'\\')) => {
                            literal.push(bytes[at]);
                            at += 1;
                        }
                        _ => literal.push(byte),
                    }
                }
                let mut next = at;
                loop {
                    while bytes.get(next).is_some_and(u8::is_ascii_whitespace) {
                        next += 1;
                    }
                    if bytes.get(next) != Some(&b'#') {
                        break;
                    }
                    next += 1;
                    while bytes
                        .get(next)
                        .is_some_and(|byte| !matches!(byte, b'\n' | b'\r'))
                    {
                        next += 1;
                    }
                }
                if bytes.get(next) == Some(&b'=')
                    && keys.iter().any(|key| literal.eq_ignore_ascii_case(key))
                {
                    return Err(invalid(format!(
                        "quoted control key {:?} is unsupported for native terrain parsing",
                        String::from_utf8_lossy(&literal)
                    )));
                }
            }
            _ => at += 1,
        }
    }
    Ok(())
}

fn terrain_path(default: &Document) -> io::Result<String> {
    let mut terrain = None;
    for item in &default.items {
        if key_matches(item, b"terrain") {
            terrain = Some(
                item.value
                    .scalar_str()
                    .ok_or_else(|| invalid("map/default.map terrain is not a scalar"))?
                    .to_owned(),
            );
        }
    }
    let terrain = terrain.ok_or_else(|| invalid("map/default.map has no terrain bitmap"))?;
    normalize_relative(&format!("map/{terrain}"))
}

/// File-scoped scalar variables are resolved only inside their source file.
fn file_variables(document: &Document) -> BTreeMap<Vec<u8>, Vec<u8>> {
    document
        .items
        .iter()
        .filter_map(|item| {
            let key = item.key.as_ref()?;
            if !key.starts_with(b"@") {
                return None;
            }
            Some((key.clone(), item.value.scalar()?.to_vec()))
        })
        .collect()
}

fn scalar_with_vars(value: &Value, vars: &BTreeMap<Vec<u8>, Vec<u8>>) -> Option<Vec<u8>> {
    let mut scalar = value.scalar()?.to_vec();
    for _ in 0..=vars.len() {
        if !scalar.starts_with(b"@") {
            return Some(scalar);
        }
        scalar = vars.get(&scalar)?.clone();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain(rows: &[[u8; 4]], bpp: u16) -> TerrainCosts {
        let mut data = vec![0; 54 + rows.len() * 4];
        data[..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[18..22].copy_from_slice(&4i32.to_le_bytes());
        data[22..26].copy_from_slice(&(rows.len() as i32).to_le_bytes());
        data[28..30].copy_from_slice(&bpp.to_le_bytes());
        for (row, values) in rows.iter().enumerate() {
            data[54 + row * 4..58 + row * 4].copy_from_slice(values);
        }
        TerrainCosts::decode(&data).unwrap()
    }

    fn definitions(sources: &[&str]) -> TerrainCosts {
        let mut terrain = terrain(&[[0, 1, 2, 3]], 8);
        let mut categories = Vec::new();
        for source in sources {
            let document = parser::parse(source.as_bytes()).unwrap();
            terrain
                .append_definitions(&document.items, &file_variables(&document), &mut categories)
                .unwrap();
        }
        terrain
    }

    #[test]
    fn category_order_and_last_palette_writer_match_native() {
        let costs = definitions(&[
            "categories={first={movement_cost=1} first={movement_cost=9}} terrain={a={type=first color={2}} early={type=later color={1}}}",
            "categories={later={movement_cost=3}} terrain={b={type=later color={2 4}}}",
        ]);
        assert_eq!(costs.cost(0).unwrap(), 0);
        assert_eq!(costs.cost(1).unwrap(), 0);
        assert_eq!(costs.cost(2).unwrap(), 300_000);
        assert_eq!(costs.cost(4).unwrap(), 300_000);
        assert_eq!(costs.cost(255).unwrap(), 0);
        assert_eq!(definitions(&["categories={x={movement_cost=1} x={movement_cost=9}} terrain={a={type=x color={0}}}"]).cost(0).unwrap(), 100_000);
    }

    #[test]
    fn authored_cost_uses_native_auto_base_fraction_and_clamp() {
        let terrain = definitions(&[
            "categories={octal={movement_cost=010} hex={movement_cost=0x10} suffix={movement_cost=1junk.25} signed={movement_cost=1.-2} clamped={movement_cost=08} invalid={movement_cost=bad}} terrain={a={type=octal color={0}} b={type=hex color={1}} c={type=suffix color={2}} d={type=signed color={3}} e={type=clamped color={4}} f={type=invalid color={5}}}",
        ]);
        for (index, expected) in [
            (0, 800_000),
            (1, 1_600_000),
            (2, 125_000),
            (3, 98_000),
            (4, 1_000),
        ] {
            assert_eq!(terrain.cost(index).unwrap(), expected);
        }
        assert!(terrain.cost(5).is_err());
        assert_eq!(parse_fixed5(b"-0.1234567"), Some(-12_345));
        assert_eq!(parse_fixed5(b"92233720368547.75807"), Some(i64::MAX));
    }

    #[test]
    fn absent_cost_and_case_mismatch_stay_distinct() {
        let terrain = definitions(&[
            "categories={X={movement_cost=2} omitted={} explicit={movement_cost=0}} terrain={a={type=x color={0}} b={type=omitted color={1}} c={type=explicit color={2}} d={color={3}}}",
        ]);
        assert!(terrain.cost(0).is_err());
        assert_eq!(terrain.cost(1).unwrap(), 0);
        assert_eq!(terrain.cost(2).unwrap(), 1_000);
        assert_eq!(terrain.cost(3).unwrap(), 0);
    }

    #[test]
    fn malformed_cost_only_withdraws_the_selected_palette_index() {
        let terrain = definitions(&[
            "categories={bad={movement_cost=broken} good={movement_cost=2}} terrain={a={type=bad color={1}} b={type=good color={2}}}",
        ]);
        assert!(terrain.cost(1).is_err());
        assert_eq!(terrain.cost(2).unwrap(), 200_000);
        assert_eq!(terrain.cost(0).unwrap(), 0);
        // A malformed category which no terrain entry selects stays unused.
        let unused =
            definitions(&["categories={bad={movement_cost=broken}} terrain={a={color={1}}}"]);
        assert_eq!(unused.cost(1).unwrap(), 0);
    }

    #[test]
    fn palette_indices_follow_the_crt_prefix_scan() {
        let terrain = definitions(&[
            "categories={x={movement_cost=2}} terrain={a={type=x color={\"1tail\"}}}",
        ]);
        assert_eq!(terrain.cost(1).unwrap(), 200_000);
    }

    #[test]
    fn file_rows_and_first_component_are_kept() {
        let terrain = terrain(&[[7, 8, 9, 10], [11, 12, 13, 14]], 8);
        assert_eq!(terrain.index(0, 0).unwrap(), 7);
        assert_eq!(terrain.index(0, 1).unwrap(), 11);
        assert_eq!(terrain.index(4, 0).unwrap(), 0);
        assert!(terrain.index(-1, 0).is_err());

        let mut data = vec![0; 58];
        data[..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[18..22].copy_from_slice(&2i32.to_le_bytes());
        data[22..26].copy_from_slice(&1i32.to_le_bytes());
        data[28..30].copy_from_slice(&16u16.to_le_bytes());
        data[54..].copy_from_slice(&[7, 99, 8, 77]);
        assert_eq!(TerrainCosts::decode(&data).unwrap().index(1, 0).unwrap(), 8);
        data[28..30].copy_from_slice(&1u16.to_le_bytes());
        data[18..22].copy_from_slice(&16i32.to_le_bytes());
        assert!(TerrainCosts::decode(&data).unwrap().index(7, 0).is_err());
    }

    #[test]
    fn default_map_last_terrain_field_is_selected_safely() {
        let default = parser::parse(b"terrain=old.bmp terrain=new.bmp").unwrap();
        assert_eq!(terrain_path(&default).unwrap(), "map/new.bmp");
        let bad = parser::parse(b"terrain=../outside.bmp").unwrap();
        assert!(terrain_path(&bad).is_err());
    }

    #[test]
    fn quoted_control_keys_are_refused_without_confusing_comments_or_values() {
        let keys: &[&[u8]] = &[
            b"categories",
            b"terrain",
            b"type",
            b"color",
            b"movement_cost",
        ];
        for quoted in [
            b"\"categories\" = {}".as_slice(),
            b"\"terrain\" = {}",
            b"terrain = { a = { \"type\" = plains } }",
            b"terrain = { a = { \"color\" = { 1 } } }",
            b"categories = { a = { \"movement_cost\" = 2 } }",
        ] {
            assert!(refuse_quoted_control_keys(quoted, keys).is_err());
        }
        assert!(refuse_quoted_control_keys(
            b"# \"terrain\" = comment\n terrain = \"terrain.bmp\" categories = { a = { type = forest } }",
            keys,
        )
        .is_ok());
        assert!(refuse_quoted_control_keys(b"\"other\" = \"type\"", keys).is_ok());
        assert!(
            refuse_quoted_control_keys(b"\"TYPE\" # across a comment\n = forest", keys).is_err()
        );
    }
}
