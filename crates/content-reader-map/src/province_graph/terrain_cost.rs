
use std::io;

use content_reader_layers::{ResolvedLayers, content::normalize_relative};
use content_reader_parser::fields::invalid;
use crate::movement::terrain::TerrainDatabase;
use content_reader_key_names::terrain::DefaultMap;

pub(crate) struct TerrainCosts {
    width: i32,
    height: i32,
    components: usize,
    pitch: usize,
    bytes: Vec<u8>,
    costs: Vec<i64>,
}

impl TerrainCosts {
    pub fn read(layers: &ResolvedLayers, map: &DefaultMap) -> io::Result<Self> {
        let mut terrain = Self::decode(&layers.read(&terrain_path(map)?)?)?;
        terrain.costs = palette_costs(&TerrainDatabase::read(layers)?);
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

    pub fn cost(&self, index: u8) -> i64 {
        self.costs.get(index as usize).copied().unwrap_or(0)
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
        })
    }
}

fn palette_costs(database: &TerrainDatabase<'_>) -> Vec<i64> {
    (0..=u8::MAX).map(|index| database.cost(index)).collect()
}

fn terrain_path(map: &DefaultMap) -> io::Result<String> {
    normalize_relative(&map.file("terrain")?)
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
        content_reader_script::test_game::enter();
        let mut terrain = terrain(&[[0, 1, 2, 3]], 8);
        terrain.costs = palette_costs(&crate::movement::terrain::database(sources));
        terrain
    }

    #[test]
    fn authored_cost_keeps_radix_prefixes_fraction_and_clamp() {
        let terrain = definitions(&[
            "categories={octal={movement_cost=010} hex={movement_cost=0x10} suffix={movement_cost=1junk.25} signed={movement_cost=1.-2} clamped={movement_cost=08} invalid={movement_cost=bad}} terrain={a={type=octal color={0}} b={type=hex color={1}} c={type=suffix color={2}} d={type=signed color={3}} e={type=clamped color={4}} f={type=invalid color={5}}}",
        ]);
        for (index, expected) in [
            (0, 800_000),
            (1, 1_600_000),
            (2, 125_000),
            (3, 98_000),
            (4, 1_000),
            (5, 1_000),
        ] {
            assert_eq!(terrain.cost(index), expected);
        }
        assert_eq!(content_reader_parser::fixed5::script_fixed5(b"-0.1234567"), Some(-12_345));
        assert_eq!(
            content_reader_parser::fixed5::script_fixed5(b"92233720368547.75807"),
            Some(i64::MAX)
        );
    }

    #[test]
    fn absent_cost_zero_cost_and_case_variants_follow_the_category_lookup() {
        let terrain = definitions(&[
            "categories={X={movement_cost=2} x={movement_cost=9} omitted={} explicit={movement_cost=0}} terrain={a={type=x color={0}} b={type=omitted color={1}} c={type=explicit color={2}} d={color={3}}}",
        ]);
        assert_eq!(terrain.cost(0), 200_000);
        assert_eq!(terrain.cost(1), 0);
        assert_eq!(terrain.cost(2), 1_000);
        assert_eq!(terrain.cost(3), 0);
    }

    #[test]
    fn malformed_cost_keeps_the_preceding_value() {
        let terrain = definitions(&[
            "categories={bad={movement_cost=broken} good={movement_cost=2} kept={movement_cost=3 movement_cost=bad}} terrain={a={type=bad color={1}} b={type=good color={2}} c={type=kept color={3}}}",
        ]);
        assert_eq!(terrain.cost(1), 1_000);
        assert_eq!(terrain.cost(2), 200_000);
        assert_eq!(terrain.cost(3), 300_000);
        assert_eq!(terrain.cost(0), 0);
    }

    #[test]
    fn costs_use_variables_as_they_stand_where_they_are_read() {
        let terrain = definitions(&[
            "@a=1 categories={x={movement_cost=@a} y={movement_cost=@b}} @b=3 terrain={a={type=x color={0}} b={type=y color={1}}}",
        ]);
        assert_eq!(terrain.cost(0), 100_000);
        assert_eq!(terrain.cost(1), 1_000);
    }

    #[test]
    fn palette_indices_follow_the_crt_prefix_scan() {
        let terrain = definitions(&[
            "categories={x={movement_cost=2}} terrain={a={type=x color={\"1tail\"}}}",
        ]);
        assert_eq!(terrain.cost(1), 200_000);
    }

    #[test]
    fn default_map_terrain_bitmap_is_read_as_the_game_reads_it() {
        content_reader_script::test_game::enter();
        let map = |terrain: &str| {
            content_reader_key_names::terrain::parse_default_map(
                format!(
                    "{terrain} definitions=d.csv provinces=p.bmp rivers=r.bmp heightmap=h.bmp tree_definition=trees.bmp"
                )
                .as_bytes(),
            )
            .unwrap()
        };
        assert_eq!(
            terrain_path(&map("terrain=old.bmp terrain=new.bmp")).unwrap(),
            "map/new.bmp"
        );
        assert_eq!(
            terrain_path(&map("@t=\"terrain.bmp\" terrain=@t")).unwrap(),
            "map/terrain.bmp"
        );
        assert!(terrain_path(&map("terrain=../outside.bmp")).is_err());
    }
}
