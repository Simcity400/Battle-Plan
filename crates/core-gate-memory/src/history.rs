use crate::{
    encoding::{self, Output},
    keyset::{Cache, Index},
    manifest::{Selection, invalid},
};
use replay::columns;
use replay_columnar::walk::State;
use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub struct History {
    file: BufWriter<File>,
    rows: Index,
    statuses: Index,
    record: Vec<u8>,
    previous_status: Option<columns::Status>,
    end: u64,
    appending: bool,
}

fn date(hour: i32) -> u128 {
    u128::from((hour as u32) ^ (1 << 31))
}

impl History {
    pub fn new(path: &Path, cache: &Cache) -> io::Result<Self> {
        let suffix = |suffix| {
            path.with_file_name(format!(
                "{}.{suffix}",
                path.file_name().unwrap().to_string_lossy()
            ))
        };
        Ok(Self {
            file: BufWriter::new(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(suffix("history"))?,
            ),
            rows: Index::new(&suffix("history-rows"), cache)?,
            statuses: Index::new(&suffix("history-status"), cache)?,
            record: Vec::new(),
            previous_status: None,
            end: 0,
            appending: true,
        })
    }

    fn save(&mut self) -> io::Result<u64> {
        if !self.appending {
            self.file.seek(SeekFrom::Start(self.end))?;
            self.appending = true;
        }
        let offset = self.end;
        self.end = self
            .end
            .checked_add(8 + self.record.len() as u64)
            .ok_or_else(|| invalid("History offset overflow"))?;
        self.file
            .write_all(&(self.record.len() as u64).to_le_bytes())?;
        self.file.write_all(&self.record)?;
        Ok(offset)
    }

    pub fn capture(
        &mut self,
        state: &State,
        selected: &Selection,
        hour: i32,
        whole: bool,
    ) -> io::Result<()> {
        let status = columns::status(state, selected.table);
        let changed = columns::changes(state, selected.table)?;
        if whole || self.previous_status != Some(status) || changed.truncated {
            encoding::status(state, selected, hour, &mut self.record, None)?;
            let offset = self.save()?;
            self.statuses.put((0, date(hour)), offset)?;
        }
        let reset = whole || self.previous_status != Some(status) || changed.truncated;
        self.previous_status = Some(status);
        let table = columns::table(state, selected.table)?;
        for &key in &changed.deleted_keys {
            encoding::header(&mut self.record, hour, 2, selected.table as u16, key);
            let offset = self.save()?;
            self.rows.put((key, date(hour)), offset)?;
        }
        let mut capture = |slot| -> io::Result<()> {
            if !table.live().get(slot) {
                return Ok(());
            }
            let key = table
                .key(slot)
                .ok_or_else(|| invalid("History row has no key"))?;
            encoding::row(
                state,
                selected,
                hour,
                slot,
                selected.fields.iter().copied(),
                1,
                &mut self.record,
            )?;
            let offset = self.save()?;
            self.rows.put((key, date(hour)), offset)
        };
        if reset {
            for slot in table.live().ones() {
                capture(slot)?;
            }
        } else {
            for &slot in &changed.inserted {
                capture(slot)?;
            }
            for (&slot, &mask) in changed
                .changed
                .iter()
                .zip(&changed.written)
                .chain(changed.fill_slots.iter().zip(&changed.fill_masks))
            {
                if selected
                    .fields
                    .iter()
                    .any(|field| mask & (1_u64 << field) != 0)
                {
                    capture(slot)?;
                }
            }
        }
        Ok(())
    }

    fn load(
        &mut self,
        offset: u64,
        delivered: i32,
        dated: i32,
        filter: Option<(&crate::readset::Filter, usize)>,
        output: &mut Output,
    ) -> io::Result<()> {
        self.appending = false;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut length = [0; 8];
        self.file.get_mut().read_exact(&mut length)?;
        let length = usize::try_from(u64::from_le_bytes(length))
            .map_err(|_| invalid("History record exceeds address space"))?;
        self.record.resize(length, 0);
        self.file.get_mut().read_exact(&mut self.record)?;
        let kind = match self.record[4] {
            0 => 10,
            1 => 9,
            2 => 11,
            _ => return Err(invalid("Invalid historical spool record")),
        };
        if let Some((filter, at)) = filter {
            let mut input = replay_data::codec::Decoder::new(&self.record[23..]);
            match input.u8()? {
                0 | 1 => {}
                2 => {
                    input.u32()?;
                }
                3 => {
                    let length = usize::try_from(input.u64()?)
                        .map_err(|_| invalid("Refusal exceeds address space"))?;
                    input.take(length)?;
                }
                _ => return Err(invalid("Invalid historical table status")),
            }
            if input.u8()? != 0 {
                input.i32()?;
            }
            let count_at = self.record.len() - input.remaining();
            let count = input.u64()?;
            let mut write = count_at + 8;
            let mut kept = 0_u64;
            for index in 0..count {
                let start = count_at + 8 + index as usize * 16;
                let key = u128::from_le_bytes(self.record[start..start + 16].try_into().unwrap());
                if filter.historical_contains(at, key)? {
                    self.record.copy_within(start..start + 16, write);
                    write += 16;
                    kept += 1;
                }
            }
            self.record.truncate(write);
            self.record[count_at..count_at + 8].copy_from_slice(&kept.to_le_bytes());
        }
        self.record[..4].copy_from_slice(&delivered.to_le_bytes());
        self.record[4] = kind;
        self.record.splice(23..23, dated.to_le_bytes());
        output.record(&self.record)
    }

    pub fn hydrate(
        &mut self,
        table: u16,
        delivered: i32,
        filter: (&crate::readset::Filter, usize),
        output: &mut Output,
    ) -> io::Result<()> {
        if !filter.0.has_historical(filter.1)? {
            return Ok(());
        }
        let dated = delivered
            .checked_sub(1)
            .ok_or_else(|| invalid("Previous hour underflow"))?;
        if let Some((_, offset)) = self.statuses.last((0, date(dated)))? {
            self.load(offset, delivered, dated, Some(filter), output)?;
        } else {
            return Err(invalid("Previous lookup has no recorded status"));
        }
        let mut next = Some(0);
        while let Some(start) = next {
            let Some((key, present)) = filter.0.next_historical(filter.1, start)? else {
                break;
            };
            if present {
                let (_, offset) = self
                    .rows
                    .last((key, date(dated)))?
                    .filter(|(row, _)| row.0 == key)
                    .ok_or_else(|| invalid("Historical closure row has no recorded value"))?;
                self.load(offset, delivered, dated, None, output)?;
            } else {
                encoding::header(&mut self.record, delivered, 11, table, key);
                self.record.extend_from_slice(&dated.to_le_bytes());
                output.record(&self.record)?;
            }
            next = key.checked_add(1);
        }
        Ok(())
    }
}
