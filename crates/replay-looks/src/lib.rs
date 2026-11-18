//! Recorded country appearance and naming operands, shared by capture and playback.

use installed_content_country_names::country_names::Operands;
use replay_data::rows::Recorded;
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// A country's flag inputs, as recorded: its appearance's cosmetic tag and original tag (with
/// the reference choosing it), its ruling party and that party's ideology group token. A part
/// it has no row of is None.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlagInputs {
    pub appearance: Option<(Vec<u8>, Recorded<Option<i32>>, Recorded<Option<Vec<u8>>>)>,
    pub ruling: Option<Option<u32>>,
    pub group: Option<Vec<u8>>,
}

/// What a tag shows at an hour: its holder, colour, naming chain and flag inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub slot: u32,
    pub id: u32,
    pub colour: Option<[u8; 3]>,
    /// The naming operands of the holder's slot and every overlord its name reaches, in order.
    pub chain: Vec<(u32, Result<Operands, String>)>,
    pub built: Option<u8>,
    pub flag: FlagInputs,
}

fn put(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn take(&mut self) -> io::Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self.0.split_first().ok_or_else(|| invalid("A look ends early"))?;
            self.0 = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("A look's number is too long"))
    }

    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let length = self.take()? as usize;
        if length > self.0.len() {
            return Err(invalid("A look ends early"));
        }
        let (bytes, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(bytes.to_vec())
    }

    fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid("A look's text is not UTF-8"))
    }
}

fn put_recorded<T>(out: &mut Vec<u8>, value: &Recorded<Option<T>>, each: impl Fn(&mut Vec<u8>, &T)) {
    match value {
        Recorded::NotRecorded => out.push(0),
        Recorded::Value(None) => out.push(1),
        Recorded::Value(Some(value)) => {
            out.push(2);
            each(out, value);
        }
    }
}

fn take_recorded<T>(cursor: &mut Cursor<'_>, each: impl Fn(&mut Cursor<'_>) -> io::Result<T>) -> io::Result<Recorded<Option<T>>> {
    Ok(match cursor.take()? {
        0 => Recorded::NotRecorded,
        1 => Recorded::Value(None),
        _ => Recorded::Value(Some(each(cursor)?)),
    })
}

impl Look {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![1];
        put(&mut out, u64::from(self.slot));
        put(&mut out, u64::from(self.id));
        match self.colour {
            None => out.push(0),
            Some(rgb) => {
                out.push(1);
                for value in rgb {
                    put(&mut out, u64::from(value));
                }
            }
        }
        put(&mut out, self.built.map_or(0, |built| u64::from(built) + 1));
        put(&mut out, self.chain.len() as u64);
        for (slot, operands) in &self.chain {
            put(&mut out, u64::from(*slot));
            match operands {
                Ok(operands) => {
                    out.push(0);
                    let (overlord, texts) = operands.parts();
                    put(&mut out, overlord.map_or(0, |slot| u64::from(slot) + 1));
                    for text in texts {
                        put_bytes(&mut out, text.as_bytes());
                    }
                }
                Err(reason) => {
                    out.push(1);
                    put_bytes(&mut out, reason.as_bytes());
                }
            }
        }
        match &self.flag.appearance {
            None => out.push(0),
            Some((cosmetic, reference, original)) => {
                out.push(1);
                put_bytes(&mut out, cosmetic);
                put_recorded(&mut out, reference, |out, value| put(out, u64::from(*value as u32)));
                put_recorded(&mut out, original, |out, value| put_bytes(out, value));
            }
        }
        put(&mut out, match self.flag.ruling {
            None => 0,
            Some(None) => 1,
            Some(Some(party)) => u64::from(party) + 2,
        });
        match &self.flag.group {
            None => out.push(0),
            Some(group) => {
                out.push(1);
                put_bytes(&mut out, group);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor(bytes);
        if cursor.take()? != 1 {
            return Err(invalid("A look of another version"));
        }
        let slot = cursor.take()? as u32;
        let id = cursor.take()? as u32;
        let colour = match cursor.take()? {
            0 => None,
            _ => {
                let mut rgb = [0u8; 3];
                for value in &mut rgb {
                    *value = cursor.take()? as u8;
                }
                Some(rgb)
            }
        };
        let built = match cursor.take()? {
            0 => None,
            built => Some((built - 1) as u8),
        };
        let mut chain = Vec::new();
        for _ in 0..cursor.take()? {
            let slot = cursor.take()? as u32;
            let operands = match cursor.take()? {
                0 => {
                    let overlord = match cursor.take()? {
                        0 => None,
                        slot => Some((slot - 1) as u32),
                    };
                    let mut texts: [String; 6] = Default::default();
                    for text in &mut texts {
                        *text = cursor.text()?;
                    }
                    Ok(Operands::from_parts(overlord, texts))
                }
                _ => Err(cursor.text()?),
            };
            chain.push((slot, operands));
        }
        let appearance = match cursor.take()? {
            0 => None,
            _ => {
                let cosmetic = cursor.bytes()?;
                let reference = take_recorded(&mut cursor, |cursor| Ok(cursor.take()? as u32 as i32))?;
                let original = take_recorded(&mut cursor, |cursor| cursor.bytes())?;
                Some((cosmetic, reference, original))
            }
        };
        let ruling = match cursor.take()? {
            0 => None,
            1 => Some(None),
            party => Some(Some((party - 2) as u32)),
        };
        let group = match cursor.take()? {
            0 => None,
            _ => Some(cursor.bytes()?),
        };
        Ok(Self { slot, id, colour, chain, built, flag: FlagInputs { appearance, ruling, group } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_reads_back_as_it_was_written() {
        let look = Look {
            slot: 7,
            id: 1234,
            colour: Some([1, 2, 255]),
            chain: vec![
                (7, Ok(Operands::from_parts(Some(3), ["GER".into(), "".into(), "_sub".into(), "".into(), "_fascism".into(), "".into()]))),
                (3, Err("CountryAppearance is incomplete this hour".into())),
            ],
            built: Some(2),
            flag: FlagInputs {
                appearance: Some((b"GER_cos".to_vec(), Recorded::Value(Some(-1)), Recorded::NotRecorded)),
                ruling: Some(Some(5)),
                group: Some(b"fascism".to_vec()),
            },
        };
        assert_eq!(Look::decode(&look.encode()).unwrap(), look);
        let bare = Look { flag: FlagInputs { ruling: Some(None), ..FlagInputs::default() }, ..Look::default() };
        assert_eq!(Look::decode(&bare.encode()).unwrap(), bare);
    }
}
