use crate::*;
use std::io;

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid compact record")
}

pub(crate) struct Writer<'a> {
    output: Option<&'a mut [u8]>,
    pub len: usize,
}
impl<'a> Writer<'a> {
    pub fn new(output: Option<&'a mut [u8]>) -> Self {
        Self { output, len: 0 }
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        let end = self.len.checked_add(bytes.len()).ok_or_else(bad)?;
        if let Some(output) = self.output.as_mut() {
            output
                .get_mut(self.len..end)
                .ok_or_else(bad)?
                .copy_from_slice(bytes);
        }
        self.len = end;
        Ok(())
    }
    pub fn byte(&mut self, byte: u8) -> io::Result<()> {
        self.bytes(&[byte])
    }
    pub fn uint(&mut self, mut value: u64) -> io::Result<()> {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            self.byte(byte | if value != 0 { 128 } else { 0 })?;
            if value == 0 {
                return Ok(());
            }
        }
    }
    pub fn int(&mut self, value: i64) -> io::Result<()> {
        self.uint(((value << 1) ^ (value >> 63)) as u64)
    }
    fn span(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.uint(bytes.len() as u64)?;
        self.bytes(bytes)
    }
    fn missing(&mut self, value: MissingRef<'_>) -> io::Result<()> {
        self.byte(match value.kind {
            MissingKind::Unknown => 0,
            MissingKind::Refused => 1,
            MissingKind::Lost => 2,
            MissingKind::NotRecorded => 3,
        })?;
        self.int(i64::from(value.hours.first))?;
        self.int(i64::from(value.hours.last))?;
        self.span(value.reason.as_bytes())
    }
    pub fn record(&mut self, value: RecordRef<'_>, base: Hour) -> io::Result<()> {
        self.int(i64::from(value.hour) - i64::from(base))?;
        let kind = match value.value {
            ValueRef::Present(_) => 0,
            ValueRef::Number(_) => 1,
            ValueRef::End => 2,
            ValueRef::Missing(_) => 3,
            ValueRef::Accumulating(_) => 4,
            ValueRef::Fixed(_) => 5,
        };
        let flags = kind
            | if value.break_before { 8 } else { 0 }
            | if value.provenance.supplying_hour.is_some() {
                16
            } else {
                0
            }
            | if value.provenance.holder.is_some() {
                32
            } else {
                0
            }
            | if value.provenance.identity_first_hour.is_some() {
                64
            } else {
                0
            };
        self.byte(flags)?;
        match value.value {
            ValueRef::Present(bytes) => self.span(bytes)?,
            ValueRef::Number(number) => self.bytes(&number.to_le_bytes())?,
            ValueRef::Fixed(number) => self.bytes(&number.to_le_bytes())?,
            ValueRef::End => {}
            ValueRef::Missing(value) => self.missing(value)?,
            ValueRef::Accumulating(value) => {
                match value.total {
                    TotalRef::Known(total) => {
                        self.byte(0)?;
                        self.bytes(&total.to_le_bytes())?;
                    }
                    TotalRef::Missing(missing) => {
                        self.byte(1)?;
                        self.missing(missing)?;
                    }
                }
                self.bytes(&value.rate.to_le_bytes())?;
                self.int(i64::from(value.anchor_hour))?;
                self.uint(value.accepted_at_anchor)?;
                self.int(i64::from(value.rate_began))?;
            }
        }
        if let Some(hour) = value.provenance.supplying_hour {
            self.int(i64::from(hour) - i64::from(base))?;
        }
        if let Some(holder) = value.provenance.holder {
            self.span(holder)?;
        }
        if let Some(hour) = value.provenance.identity_first_hour {
            self.int(i64::from(hour))?;
        }
        Ok(())
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pub offset: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    pub fn bytes(&mut self, len: usize) -> io::Result<&'a [u8]> {
        let end = self.offset.checked_add(len).ok_or_else(bad)?;
        let value = self.bytes.get(self.offset..end).ok_or_else(bad)?;
        self.offset = end;
        Ok(value)
    }
    pub fn byte(&mut self) -> io::Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn uint(&mut self) -> io::Result<u64> {
        let mut value = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = self.byte()?;
            if shift == 63 && byte > 1 {
                return Err(bad());
            }
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(bad())
    }
    pub fn int(&mut self) -> io::Result<i64> {
        let value = self.uint()?;
        Ok(((value >> 1) as i64) ^ -((value & 1) as i64))
    }
    fn span(&mut self) -> io::Result<&'a [u8]> {
        let len = usize::try_from(self.uint()?).map_err(|_| bad())?;
        self.bytes(len)
    }
    fn hour(&mut self) -> io::Result<Hour> {
        Hour::try_from(self.int()?).map_err(|_| bad())
    }
    fn number(&mut self) -> io::Result<f64> {
        Ok(f64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn missing(&mut self) -> io::Result<MissingRef<'a>> {
        let kind = match self.byte()? {
            0 => MissingKind::Unknown,
            1 => MissingKind::Refused,
            2 => MissingKind::Lost,
            3 => MissingKind::NotRecorded,
            _ => return Err(bad()),
        };
        let first = self.hour()?;
        let last = self.hour()?;
        let reason = std::str::from_utf8(self.span()?).map_err(|_| bad())?;
        Ok(MissingRef {
            kind,
            reason,
            hours: Hours::new(first, last)?,
        })
    }
    pub fn record(&mut self, base: Hour) -> io::Result<RecordRef<'a>> {
        let hour = Hour::try_from(self.int()?.checked_add(i64::from(base)).ok_or_else(bad)?)
            .map_err(|_| bad())?;
        let flags = self.byte()?;
        let value = match flags & 7 {
            0 => ValueRef::Present(self.span()?),
            1 => ValueRef::Number(self.number()?),
            2 => ValueRef::End,
            3 => ValueRef::Missing(self.missing()?),
            5 => ValueRef::Fixed(i128::from_le_bytes(self.bytes(16)?.try_into().unwrap())),
            4 => {
                let total = match self.byte()? {
                    0 => TotalRef::Known(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap())),
                    1 => TotalRef::Missing(self.missing()?),
                    _ => return Err(bad()),
                };
                ValueRef::Accumulating(AccumulationRef {
                    total,
                    rate: i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()),
                    anchor_hour: self.hour()?,
                    accepted_at_anchor: self.uint()?,
                    rate_began: self.hour()?,
                })
            }
            _ => return Err(bad()),
        };
        let supplying_hour = if flags & 16 != 0 {
            Some(
                Hour::try_from(self.int()?.checked_add(i64::from(base)).ok_or_else(bad)?)
                    .map_err(|_| bad())?,
            )
        } else {
            None
        };
        let holder = if flags & 32 != 0 {
            Some(self.span()?)
        } else {
            None
        };
        let identity_first_hour = if flags & 64 != 0 {
            Some(self.hour()?)
        } else {
            None
        };
        Ok(RecordRef {
            hour,
            value,
            provenance: ProvenanceRef {
                supplying_hour,
                holder,
                identity_first_hour,
            },
            break_before: flags & 8 != 0,
        })
    }
}

pub(crate) fn record_len(record: RecordRef<'_>, base: Hour) -> usize {
    let mut writer = Writer::new(None);
    writer.record(record, base).unwrap();
    writer.len
}
pub(crate) fn owned(record: RecordRef<'_>, memory: &Memory) -> io::Result<Record> {
    let missing = |value: MissingRef<'_>| -> io::Result<Missing> {
        Ok(Missing {
            kind: value.kind,
            reason: Text::copy_from(memory, Owner::Transient, value.reason)?,
            hours: value.hours,
        })
    };
    Ok(Record {
        hour: record.hour,
        value: match record.value {
            ValueRef::Present(bytes) => {
                Value::Present(Bytes::copy_from(memory, Owner::Transient, bytes)?)
            }
            ValueRef::Number(number) => Value::Number(number),
            ValueRef::Fixed(number) => Value::Fixed(number),
            ValueRef::End => Value::End,
            ValueRef::Missing(value) => Value::Missing(missing(value)?),
            ValueRef::Accumulating(value) => Value::Accumulating(Accumulation {
                total: match value.total {
                    TotalRef::Known(total) => Total::Known(total),
                    TotalRef::Missing(value) => Total::Missing(missing(value)?),
                },
                rate: value.rate,
                anchor_hour: value.anchor_hour,
                accepted_at_anchor: value.accepted_at_anchor,
                rate_began: value.rate_began,
            }),
        },
        provenance: Provenance {
            supplying_hour: record.provenance.supplying_hour,
            holder: record
                .provenance
                .holder
                .map(|bytes| Key::from_varint_in(memory, bytes))
                .transpose()?,
            identity_first_hour: record.provenance.identity_first_hour,
        },
        break_before: record.break_before,
    })
}
