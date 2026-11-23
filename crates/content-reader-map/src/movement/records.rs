
use std::{collections::HashMap, io};

use super::bitmap::NativeBitmap;
pub(crate) use crate::map::ProvinceKind as Kind;
use content_reader_parser::fields::invalid;
use crate::map::DefinitionRow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub kind: Kind,
    pub coastal: bool,
    pub terrain: Vec<u8>,
}

pub(crate) struct Provinces {
    pub width: usize,
    pub height: usize,
    pub ids: Vec<u32>,
    pub rows: Vec<Option<Record>>,
    pub count: usize,
}

impl Provinces {
    pub(super) fn kind(&self, id: u32) -> Kind {
        match self.rows.get(id as usize) {
            Some(Some(record)) => record.kind,
            _ => Kind::Land,
        }
    }
}

pub(crate) fn engine_number(field: &[u8]) -> i64 {
    let start = field
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | 0x0b | 0x0c | b'\r' | b'\n'))
        .unwrap_or(field.len());
    let bytes = &field[start..];
    let (negative, digits) = match bytes.first() {
        Some(b'-') => (true, &bytes[1..]),
        Some(b'+') => (false, &bytes[1..]),
        _ => (false, bytes),
    };
    let mut value = 0_i64;
    for byte in digits.iter().take_while(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'));
    }
    let signed = if negative { -value } else { value };
    signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX))
}

fn record(row: &DefinitionRow) -> io::Result<Record> {
    let kind = row.kind().ok_or_else(|| {
        invalid(format!(
            "definition.csv line {} has an unsupported province kind {:?}",
            row.line,
            String::from_utf8_lossy(&row.kind)
        ))
    })?;
    Ok(Record {
        kind,
        coastal: row.coastal,
        terrain: row.terrain.clone(),
    })
}

fn colour_key([red, green, blue]: [u8; 3]) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

pub(crate) fn read(definition_csv: &[u8], provinces_bmp: &[u8]) -> io::Result<Provinces> {
    let mut rows = Vec::new();
    let mut colours = HashMap::new();
    let definitions = crate::map::definition_rows(definition_csv)?;
    crate::map::require_positions(&definitions)?;
    for row in definitions {
        let id = usize::from(row.id);
        if id != 0 {
            colours.insert(colour_key(row.rgb), id as u32);
        }
        rows.push(if id == 0 { None } else { Some(record(&row)?) });
    }
    if rows.is_empty() {
        return Err(invalid("definition.csv declares no province records"));
    }
    let bitmap = NativeBitmap::decode(provinces_bmp)
        .map_err(|error| invalid(format!("province bitmap: {error}")))?;
    if bitmap.width > usize::from(u16::MAX) || bitmap.height > usize::from(u16::MAX) {
        return Err(invalid(format!(
            "a {}x{} province bitmap exceeds the native u16 run fields",
            bitmap.width, bitmap.height
        )));
    }
    let mut count = rows.len();
    let mut ids = Vec::with_capacity(bitmap.width * bitmap.height);
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let colour = colour_key(bitmap.rgb(x, y)?);
            let id = *colours.entry(colour).or_insert_with(|| {
                count += 1;
                (count - 1) as u32
            });
            ids.push(id);
        }
    }
    if count > 1 << 16 {
        return Err(invalid(format!(
            "{count} province records exceed the native u16 province IDs"
        )));
    }
    Ok(Provinces {
        width: bitmap.width,
        height: bitmap.height,
        ids,
        rows,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::super::bitmap::test_bmp;
    use super::*;

    const RED: [u8; 3] = [0, 0, 255];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [255, 0, 0];

    fn rows(file_rows: [[[u8; 3]; 2]; 2]) -> Vec<u8> {
        let rows: Vec<Vec<u8>> = file_rows
            .iter()
            .map(|row| {
                let mut bytes = row.concat();
                bytes.resize(8, 0);
                bytes
            })
            .collect();
        test_bmp(2, &[&rows[0], &rows[1]], 24)
    }

    #[test]
    fn file_order_invents_unclaimed_colours_and_keeps_unpainted_records() {
        let definitions = b"0;0;0;0;land;false;unknown;0\n\
            1;255;0;0;land;false;plains;1\r\n\
            2;0;255;0;sea;true;ocean;0\n\
            3;1;2;3;lake;false;lakes;0\n";
        let provinces = read(definitions, &rows([[RED, BLUE], [RED, GREEN]])).unwrap();
        assert_eq!(provinces.ids, [1, 4, 1, 2]);
        assert_eq!(provinces.count, 5);
        assert_eq!(provinces.kind(2), Kind::Sea);
        assert_eq!(provinces.kind(4), Kind::Land);
        let record = provinces.rows[1].as_ref().unwrap();
        assert_eq!(record.terrain, b"plains");
        assert!(provinces.rows[2].as_ref().unwrap().coastal);
    }

    #[test]
    fn records_whose_native_identity_or_kind_is_unsettled_are_refused() {
        let bitmap = rows([[RED; 2]; 2]);
        for definitions in [
            b"0;0;0;0;land\n2;255;0;0;land;false;plains\n".as_slice(),
            b"0;0;0;0;land\n1;255;0;0;hills;false;plains\n",
            b"0;0;0\n",
            b"",
        ] {
            assert!(read(definitions, &bitmap).is_err());
        }
        let provinces = read(b"0;0;0;0;land\n1;255;0;0;sea;maybe\n", &bitmap).unwrap();
        let record = provinces.rows[1].as_ref().unwrap();
        assert_eq!(
            (record.coastal, record.terrain.as_slice()),
            (false, &b"unknown"[..])
        );
        assert_eq!(engine_number(b" +12suffix"), 12);
        assert_eq!(engine_number(b"-2147483649"), i64::from(i32::MIN));
    }
}
