use super::*;
use crate::prepared::pages::Pages;
use crate::prepared::series::decompress;
use replay_data::rows::CountryProductionModifiersRow;
use std::sync::Weak;

#[derive(Serialize, Deserialize)]
pub(super) struct Page {
    pub lines: SavedLines,
    pub countries: Vec<u64>,
    pub continuation: bool,
}

#[derive(Clone, Default)]
pub(super) struct CountryRows {
    rows: BTreeMap<usize, (Weak<CountryProductionModifiersRow>, u64)>,
    pub next: u64,
    pub continuation: bool,
}

impl CountryRows {
    pub fn id(&mut self, row: &Arc<CountryProductionModifiersRow>) -> u64 {
        let pointer = Arc::as_ptr(row) as usize;
        self.rows
            .entry(pointer)
            .or_insert_with(|| {
                let id = self.next;
                self.next += 1;
                (Arc::downgrade(row), id)
            })
            .1
    }

    pub fn bind(&mut self, row: &Arc<CountryProductionModifiersRow>, id: u64) {
        self.rows
            .insert(Arc::as_ptr(row) as usize, (Arc::downgrade(row), id));
    }
    pub fn page(&mut self, lines: &ScannedLines, cancelled: &AtomicBool) -> io::Result<Page> {
        let saved = SavedLines::save(lines, cancelled)?;
        let mut countries = vec![0; saved.countries.len()];
        for (key, runs) in &lines.modifiers {
            for (run, saved_run) in runs.iter().zip(&saved.modifiers[key]) {
                let pointer = Arc::as_ptr(&run.country) as usize;
                let (_, id) = self.rows.entry(pointer).or_insert_with(|| {
                    let id = self.next;
                    self.next += 1;
                    (Arc::downgrade(&run.country), id)
                });
                countries[saved_run.country] = *id;
            }
        }
        self.rows.retain(|_, (row, _)| row.strong_count() != 0);
        Ok(Page {
            lines: saved,
            countries,
            continuation: self.continuation,
        })
    }
}

pub(super) fn write<W: Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    range: u64,
    value: &impl product_codec::Product,
) -> io::Result<()> {
    let bytes = pack(value)?;
    writer.write_at(section, range, &bytes)?;
    #[cfg(any(test, feature = "test-support"))]
    writer.note_framing(
        section,
        range,
        crate::prepared::pages::framing::compressed(&bytes),
    );
    Ok(())
}

pub(super) fn write_json<W: Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    range: u64,
    value: &impl Serialize,
) -> io::Result<()> {
    crate::prepared::series::write_reply(writer, section, range, value)
}

const BINARY_PAGE: &[u8] = b"PLB1";

pub(super) fn pack(value: &impl product_codec::Product) -> io::Result<Vec<u8>> {
    let mut raw = BINARY_PAGE.to_vec();
    product_codec::Product::encode(value, &mut raw)?;
    crate::prepared::series::compress(&raw, 3)
}

pub(super) fn read<T: product_codec::Product>(bytes: &[u8]) -> io::Result<T> {
    let raw = decompress(bytes)?;
    let payload = raw
        .strip_prefix(BINARY_PAGE)
        .ok_or_else(|| crate::invalid("Invalid production binary page schema"))?;
    product_codec::decode(payload)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn read_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    serde_json::from_slice(&decompress(bytes)?).map_err(io::Error::other)
}

pub(super) fn tail<T>(values: &mut Vec<T>) {
    if values.len() > 1 {
        values.drain(..values.len() - 1);
        values.shrink_to_fit();
    }
}

fn append<T>(to: &mut Vec<T>, mut values: Vec<T>, continuation: bool, at: impl Fn(&T) -> i32) {
    if continuation
        && to
            .last()
            .zip(values.first())
            .is_some_and(|(a, b)| at(a) == at(b))
    {
        to.pop();
    }
    to.append(&mut values);
}

#[derive(Default)]
pub(super) struct Assembler {
    pub result: ScannedLines,
    lines: BTreeMap<String, usize>,
    pub countries: BTreeMap<u64, Arc<CountryProductionModifiersRow>>,
}

impl Assembler {
    pub fn push(
        &mut self,
        page: Page,
        localisation: Option<Arc<installed_content_localisation::localisation::Localisation>>,
        language: Arc<str>,
    ) -> io::Result<()> {
        let continuation = page.continuation;
        let ids = page.countries;
        let country_indexes: BTreeMap<_, _> = page
            .lines
            .modifiers
            .iter()
            .map(|(key, runs)| {
                let ids = runs
                    .iter()
                    .map(|run| {
                        ids.get(run.country)
                            .copied()
                            .ok_or_else(|| crate::invalid("Invalid production page country index"))
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                Ok((key.clone(), ids))
            })
            .collect::<io::Result<_>>()?;
        let mut next = page
            .lines
            .restore(localisation, language)?
            .ok_or_else(|| crate::invalid("Unsupported production page"))?;
        for (key, runs) in &mut next.modifiers {
            for (run, id) in runs.iter_mut().zip(&country_indexes[key]) {
                run.country = Arc::clone(
                    self.countries
                        .entry(*id)
                        .or_insert_with(|| Arc::clone(&run.country)),
                );
            }
        }
        let result = &mut self.result;
        for mut line in next.lines {
            let position = match self.lines.get(&line.key) {
                Some(&position) => position,
                None => {
                    let position = result.lines.len();
                    self.lines.insert(line.key.clone(), position);
                    result.lines.push(line);
                    continue;
                }
            };
            let old = &mut result.lines[position];
            append(
                &mut old.runs,
                std::mem::take(&mut line.runs),
                continuation,
                |run| run.first_hour,
            );
            append(
                &mut old.output,
                std::mem::take(&mut line.output),
                continuation,
                |run| run.first_hour,
            );
            std::mem::swap(&mut old.runs, &mut line.runs);
            std::mem::swap(&mut old.output, &mut line.output);
            *old = line;
        }
        for (key, runs) in next.modifiers {
            append(
                result.modifiers.entry(key).or_default(),
                runs,
                continuation,
                |run| run.first_hour,
            );
        }
        for (key, runs) in next.designs {
            append(
                result.designs.entry(key).or_default(),
                runs,
                continuation,
                |run| run.first_hour,
            );
        }
        for (key, runs) in next.icons {
            append(
                result.icons.entry(key).or_default(),
                runs,
                continuation,
                |run| run.0,
            );
        }
        for (key, runs) in next.organisations {
            append(
                result.organisations.entry(key).or_default(),
                runs,
                continuation,
                |run| run.first_hour,
            );
        }
        result.reads.extend(next.reads);
        result.reads.dedup();
        for (to, spans) in [
            (&mut result.line_hours, next.line_hours),
            (&mut result.output_hours, next.output_hours),
            (&mut result.organisation_hours, next.organisation_hours),
            (&mut result.unread_output, next.unread_output),
        ] {
            for span in spans {
                mark_span(to, span.first_hour, span.last_hour);
            }
        }
        result.modifier_layout_refusals = next.modifier_layout_refusals;
        result.unknown_kinds = next.unknown_kinds;
        result.unlisted_output = next.unlisted_output;
        result.unread_output_reads = next.unread_output_reads;
        result.unreadable_stretches = next.unreadable_stretches;
        result.unlinked_rows = next.unlinked_rows;
        Ok(())
    }
}

// The JSON charge also covers inline slots in nested string and tuple vectors.
pub(super) fn record_bytes<T: Serialize>(value: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    if serde_json::to_writer(&mut count, value).is_err() {
        return crate::prepared::pages::BATCH_BYTES;
    }
    (std::mem::size_of::<T>() + count.0).saturating_mul(8)
}

// Only immutable page references live here; a cursor never retains decoded history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PageLog {
    root: private::Root,
    pub len: u64,
}

pub(super) struct PageCursor {
    log: PageLog,
    next: u64,
}

impl PageLog {
    pub fn append<W: Pages + ?Sized>(
        &mut self,
        facts: &mut private::Store<'_, W>,
        section: &str,
        value: &impl product_codec::Product,
    ) -> io::Result<()> {
        let next = self
            .len
            .checked_add(1)
            .ok_or_else(|| crate::invalid("Production page ordinal overflow"))?;
        let bytes = pack(value)?;
        let range = facts.pages.write(section, &bytes)?;
        private::put(facts, &mut self.root, &format!("{:016x}", self.len), &range)?;
        self.len = next;
        Ok(())
    }

    pub fn cursor(&self, first: u64) -> io::Result<PageCursor> {
        if first > self.len {
            return Err(crate::invalid("Invalid production page cursor"));
        }
        Ok(PageCursor {
            log: *self,
            next: first,
        })
    }
}

impl PageCursor {
    pub fn next<T: product_codec::Product>(
        &mut self,
        facts: &mut private::Store<'_, impl Pages + ?Sized>,
        section: &str,
    ) -> io::Result<Option<T>> {
        if self.next == self.log.len {
            return Ok(None);
        }
        let range: u64 = private::get(facts, self.log.root, &format!("{:016x}", self.next))?
            .ok_or_else(|| crate::invalid("Missing production page ordinal"))?;
        let bytes = facts
            .pages
            .read_at(section, range)?
            .ok_or_else(|| crate::invalid("Missing production history page"))?;
        let page = read(&bytes)?;
        self.next += 1;
        Ok(Some(page))
    }
}

product_codec::product! { impl Page {
    lines: SavedLines,
    countries: Vec<u64>,
    continuation: bool,
} default {} }
product_codec::product! { impl PageLog {
    root: private::Root,
    len: u64,
} default {} }
