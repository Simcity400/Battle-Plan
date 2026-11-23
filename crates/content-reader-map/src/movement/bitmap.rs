
use std::io;

use content_reader_parser::fields::invalid;

pub(crate) struct NativeBitmap {
    pub width: usize,
    pub height: usize,
    pub bits: u16,
    components: usize,
    pitch: usize,
    bytes: Vec<u8>,
}

impl NativeBitmap {
    pub(crate) fn decode(data: &[u8]) -> io::Result<Self> {
        let head = data
            .get(..54)
            .ok_or_else(|| invalid("BMP is shorter than the native 54-byte header"))?;
        if &head[..2] != b"BM" {
            return Err(invalid("BMP has no BM signature"));
        }
        let word = |at: usize| i32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        let (width, height) = (word(18), word(22));
        let bits = u16::from_le_bytes(head[28..30].try_into().unwrap());
        let components: i64 = match bits {
            1 | 8 => 1,
            16 => 2,
            24 => 3,
            _ => {
                return Err(invalid(format!(
                    "the native map loader rejects {bits}-bit BMP"
                )));
            }
        };
        if word(30) != 0 {
            return Err(invalid(
                "the native map loader does not decode compressed BMP rows",
            ));
        }
        if width <= 0 || height <= 0 {
            return Err(invalid(format!(
                "the native map layout of a {width}x{height} BMP is unsettled"
            )));
        }
        let row = i64::from(width) * components;
        let pitch = if bits == 1 { row >> 3 } else { row };
        let padded = if bits == 1 {
            (row + 3) >> 3
        } else {
            (row + 3) & !3
        };
        let advance = if pitch < 1 { padded } else { pitch };
        let allocation = pitch * i64::from(height) + 7;
        let written = advance * i64::from(height - 1) + padded;
        if allocation > i64::from(i32::MAX) || allocation < written {
            return Err(invalid(format!(
                "a {width}x{height} {bits}-bit BMP exceeds the native row storage"
            )));
        }
        let start = if bits == 24 {
            54
        } else {
            u32::from_le_bytes(head[10..14].try_into().unwrap()) as usize
        };
        let (advance, padded, height) = (advance as usize, padded as usize, height as usize);
        if start + padded * height > data.len() {
            return Err(invalid("BMP is short of its native pixel rows"));
        }
        let mut bytes = vec![0; written as usize];
        if padded > 0 {
            for y in 0..height {
                let source = start + y * padded;
                bytes[y * advance..y * advance + padded]
                    .copy_from_slice(&data[source..source + padded]);
            }
        }
        Ok(Self {
            width: width as usize,
            height,
            bits,
            components: components as usize,
            pitch: advance,
            bytes,
        })
    }

    pub(crate) fn require_rgb(&self) -> io::Result<()> {
        let last = (self.height - 1)
            .checked_mul(self.pitch)
            .and_then(|row| (self.width - 1).checked_mul(self.components)?.checked_add(row))
            .and_then(|at| at.checked_add(3));
        match last {
            Some(end) if end <= self.bytes.len() => Ok(()),
            _ => Err(invalid("pixel reads outside the native bitmap storage")),
        }
    }

    fn at(&self, x: usize, y: usize, length: usize) -> io::Result<&[u8]> {
        let at = y * self.pitch + x * self.components;
        self.bytes
            .get(at..at + length)
            .ok_or_else(|| invalid("pixel reads outside the native bitmap storage"))
    }

    pub(super) fn byte(&self, x: usize, y: usize) -> io::Result<u8> {
        Ok(self.at(x, y, 1)?[0])
    }

    pub(crate) fn rgb(&self, x: usize, y: usize) -> io::Result<[u8; 3]> {
        let [b, g, r] = self.at(x, y, 3)?.try_into().unwrap();
        Ok([r, g, b])
    }
}

#[cfg(test)]
pub(crate) fn test_bmp(width: i32, rows: &[&[u8]], bits: u16) -> Vec<u8> {
    let mut data = vec![0; 54];
    data[..2].copy_from_slice(b"BM");
    data[10..14].copy_from_slice(&54u32.to_le_bytes());
    data[14..18].copy_from_slice(&40u32.to_le_bytes());
    data[18..22].copy_from_slice(&width.to_le_bytes());
    data[22..26].copy_from_slice(&(rows.len() as i32).to_le_bytes());
    data[26..28].copy_from_slice(&1u16.to_le_bytes());
    data[28..30].copy_from_slice(&bits.to_le_bytes());
    for row in rows {
        data.extend_from_slice(row);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_rows_and_the_first_component_are_kept() {
        let bitmap =
            NativeBitmap::decode(&test_bmp(4, &[&[7, 8, 9, 10], &[11, 12, 13, 14]], 8)).unwrap();
        assert_eq!(bitmap.byte(0, 0).unwrap(), 7);
        assert_eq!(bitmap.byte(0, 1).unwrap(), 11);
        let wide = NativeBitmap::decode(&test_bmp(2, &[&[7, 99, 8, 77]], 16)).unwrap();
        assert_eq!(wide.byte(1, 0).unwrap(), 8);
        let packed = NativeBitmap::decode(&test_bmp(16, &[&[1, 2, 3, 4]], 1)).unwrap();
        assert!(packed.byte(7, 0).is_err());
    }

    #[test]
    fn padded_rows_keep_their_first_width_bytes() {
        let bitmap =
            NativeBitmap::decode(&test_bmp(3, &[&[1, 2, 3, 0xee], &[4, 5, 6, 0xee]], 8)).unwrap();
        let rows: Vec<Vec<u8>> = (0..2)
            .map(|y| (0..3).map(|x| bitmap.byte(x, y).unwrap()).collect())
            .collect();
        assert_eq!(rows, [[1, 2, 3], [4, 5, 6]]);
        let colour =
            NativeBitmap::decode(&test_bmp(1, &[&[1, 2, 3, 0xee], &[4, 5, 6, 0xee]], 24)).unwrap();
        assert_eq!(colour.rgb(0, 1).unwrap(), [6, 5, 4]);
    }

    #[test]
    fn unsupported_or_truncated_storage_is_refused() {
        assert!(NativeBitmap::decode(&test_bmp(4, &[&[0; 4]], 32)).is_err());
        assert!(NativeBitmap::decode(&test_bmp(8, &[&[0; 4]], 8)).is_err());
        let mut compressed = test_bmp(4, &[&[0; 4]], 8);
        compressed[30] = 1;
        assert!(NativeBitmap::decode(&compressed).is_err());
        let mut top_down = test_bmp(4, &[&[0; 4]], 8);
        top_down[22..26].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(NativeBitmap::decode(&top_down).is_err());
    }
}
