//! Synthetic version 4 recordings, written as the converter writes them, for tests.

use super::*;
use replay_columnar::{
    envelope::{Identity, RestartDirectory, RestartTable},
    leaf::Writer,
    restart::{self, Acquisition},
    state::Bits,
};

/// One extent of `raw` appended to `file`, compressed as the converter stores it.
pub(crate) fn append(
    file: &mut Vec<u8>,
    kind: Kind,
    (run, stream, first, hours): (u32, u32, u32, u32),
    raw: &[u8],
) -> Extent {
    let stored = zstd::bulk::compress(raw, 1).unwrap();
    let extent = Extent {
        kind,
        run,
        stream,
        first,
        hours,
        offset: file.len() as u64,
        stored: stored.len() as u64,
        raw: raw.len() as u64,
        digest: envelope::digest(&stored),
    };
    file.extend_from_slice(&stored);
    extent
}

/// A recording of `runs` (branch, first hour, ordinals) as the converter writes one, of
/// manifests alone: ordinal `n` of run `r` is the hour `hour(r, n)` gives (none leaves it
/// out), appending one dictionary string, and every `restart_units`th full page ends in a
/// restart. `tamper` changes each page before it is written. The file, and each run's page
/// extents.
#[cfg(test)]
pub(crate) fn recording(
    runs: &[(u64, i32, u32)],
    hour: impl Fn(u32, u32) -> Option<i32>,
    restart_units: u32,
    tamper: impl Fn(&mut IndexPage),
) -> (Vec<u8>, Vec<Vec<Extent>>) {
    let schema = schema().unwrap();
    let columns: Vec<usize> = schema.iter().map(Vec::len).collect();
    let identity = Identity {
        schema: envelope::schema_digest(&schema),
        campaign: b"campaign".to_vec(),
        content: Vec::new(),
        capture_mode: 0,
    };
    let none = Bits::from_words(vec![0; TABLE_COUNT.div_ceil(64)]).unwrap();
    let mut file = envelope::MAGIC.to_vec();
    file.extend_from_slice(&envelope::VERSION.to_le_bytes());
    let (mut directories, mut every_page) = (Vec::new(), Vec::new());
    for (run, &(branch, first_hour, ordinals)) in (0..).zip(runs) {
        let mut chain = envelope::chain_start(&identity.chain_bytes().unwrap(), run);
        let (mut pages, mut restarts, mut next_id) = (Vec::new(), 0, 0);
        // The strings first used since the run's last restart.
        let mut pending: Vec<Vec<u8>> = Vec::new();
        for first in (0..ordinals).step_by(envelope::UNIT_ORDINALS as usize) {
            let hours = (ordinals - first).min(envelope::UNIT_ORDINALS);
            let strings: Vec<Vec<u8>> = (first..first + hours)
                .map(|ordinal| format!("run {run} ordinal {ordinal}").into_bytes())
                .collect();
            pending.extend(strings.iter().cloned());
            let mut unit = Writer::new();
            for (ordinal, string) in (first..).zip(&strings) {
                let Some(hour) = hour(run, ordinal) else {
                    continue;
                };
                let manifest = Manifest {
                    branch,
                    hour,
                    flags: 0,
                    total_ns: 1,
                    pool_bytes: 0,
                    dropped_frames: 0,
                    appends: vec![string],
                    statuses: vec![Status::NotRecorded; TABLE_COUNT],
                    read: vec![0; TABLE_COUNT.div_ceil(64)],
                    samples: Vec::new(),
                    sections: Vec::new(),
                }
                .write(&columns)
                .unwrap();
                framing::put_manifest(&mut unit, &manifest).unwrap();
                chain = envelope::chain_hour(&chain, &manifest, std::iter::empty());
            }
            let units = vec![append(
                &mut file,
                Kind::Unit,
                (run, framing::MANIFESTS, first, hours),
                unit.bytes(),
            )];
            let restarts_here = hours == envelope::UNIT_ORDINALS
                && (first / envelope::UNIT_ORDINALS + 1) % restart_units == 0;
            let restart = restarts_here.then(|| {
                let after = first + hours - 1;
                let strings = std::mem::take(&mut pending);
                let mut chunk = Writer::new();
                chunk.varint(strings.len() as u64).unwrap();
                for string in &strings {
                    chunk.varint(string.len() as u64).unwrap();
                    chunk.raw(string).unwrap();
                }
                let chunk = append(
                    &mut file,
                    Kind::DictionaryChunk,
                    (run, 0, after, 0),
                    chunk.bytes(),
                );
                let tables = (0..)
                    .zip(&schema)
                    .map(|(table, kinds)| {
                        let bytes = restart::write(
                            &Table::new(kinds).unwrap(),
                            Status::NotRecorded,
                            &Acquisition::default(),
                            &[],
                        )
                        .unwrap();
                        RestartTable {
                            table,
                            extent: append(
                                &mut file,
                                Kind::RestartTable,
                                (run, table, after, 0),
                                &bytes,
                            ),
                            chunks: Vec::new(),
                        }
                    })
                    .collect();
                let restart = RestartDirectory {
                    after,
                    hour: first_hour + after as i32,
                    ordinal: restarts,
                    chunk,
                    ids: (next_id, strings.len() as u32),
                    tables,
                };
                restarts += 1;
                next_id += strings.len() as u32;
                restart
            });
            let mut page = IndexPage {
                run,
                first,
                hours,
                first_hour: first_hour + first as i32,
                last_hour: first_hour + (first + hours - 1) as i32,
                units,
                restart,
                sectioned: none.clone(),
                read: none.clone(),
                status_changed: none.clone(),
                chain,
            };
            tamper(&mut page);
            let raw = page.write().unwrap();
            pages.push(append(
                &mut file,
                Kind::IndexPage,
                (run, 0, first, hours),
                &raw,
            ));
        }
        let directory = RunDirectory {
            branch,
            epoch: envelope::epoch::START,
            first_hour,
            ordinals,
            chain,
            pages: pages.clone(),
        }
        .write(run)
        .unwrap();
        directories.push(append(
            &mut file,
            Kind::RunDirectory,
            (run, 0, 0, 0),
            &directory,
        ));
        every_page.push(pages);
    }
    let root = Root {
        identity,
        revision: 1,
        runs: directories,
        controls: Vec::new(),
    }
    .write()
    .unwrap();
    let root = append(&mut file, Kind::Root, (0, 0, 0, 0), &root);
    file.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
    (file, every_page)
}

/// Every ordinal of `runs` at its run's hour.
#[cfg(test)]
pub(crate) fn every(runs: &[(u64, i32, u32)]) -> impl Fn(u32, u32) -> Option<i32> + '_ {
    |run, ordinal| Some(runs[run as usize].1 + ordinal as i32)
}

/// Two live countries, one retaining its first string through unique-string churn.
#[cfg(test)]
pub(crate) fn native_recording(units: u32, tamper: impl Fn(&mut IndexPage)) -> Vec<u8> {
    use super::synthetic::{Input, Sample, Synthetic};
    use crate::rows::CountryRow;
    let schema = schema().unwrap();
    let columns: Vec<_> = schema.iter().map(Vec::len).collect();
    let identity = Identity {
        schema: envelope::schema_digest(&schema),
        campaign: b"native synthetic".to_vec(),
        content: Vec::new(),
        capture_mode: 0,
    };
    let mut chain = envelope::chain_start(&identity.chain_bytes().unwrap(), 0);
    let mut file = envelope::MAGIC.to_vec();
    file.extend_from_slice(&envelope::VERSION.to_le_bytes());
    let mut synthetic = Synthetic::new(&[TableId::Countries], 7, 100).unwrap();
    let (mut pages, mut chunk_ends, mut next_id) = (Vec::new(), Vec::new(), 0_usize);
    for unit in 0..units {
        let first = unit * envelope::UNIT_ORDINALS;
        let mut manifests = Writer::new();
        let mut table_bytes = Writer::new();
        for ordinal in first..first + envelope::UNIT_ORDINALS {
            let rows = [
                CountryRow {
                    id: 1,
                    tag: b"old string".to_vec(),
                    stability_raw: 123,
                    ..Default::default()
                },
                CountryRow {
                    id: 2,
                    tag: format!("new string {ordinal}").into_bytes(),
                    stability_raw: i64::from(ordinal),
                    ..Default::default()
                },
            ];
            synthetic
                .table(
                    TableId::Countries,
                    Input::Snapshot {
                        rows: &rows,
                        partial: None,
                        read: ordinal % 2 == 0,
                        sample: if ordinal % 2 == 0 {
                            Sample::FromReadBit
                        } else {
                            Sample::Carried(100 + ordinal as i32 - 1)
                        },
                    },
                )
                .unwrap();
            synthetic
                .end_encoded(|_, manifest, _, _, sections| {
                    let bytes = manifest.write(&columns).unwrap();
                    framing::put_manifest(&mut manifests, &bytes).unwrap();
                    for (_, bytes) in sections {
                        framing::put_section(&mut table_bytes, bytes).unwrap();
                    }
                    chain = envelope::chain_hour(
                        &chain,
                        &bytes,
                        sections.iter().map(|(_, bytes)| bytes.as_slice()),
                    );
                })
                .unwrap();
        }
        let after = first + envelope::UNIT_ORDINALS - 1;
        let state = synthetic.state();
        let chunk_raw = state.dictionary().chunk(next_id).unwrap();
        let count = state.dictionary().len() - next_id;
        let chunk = append(
            &mut file,
            Kind::DictionaryChunk,
            (0, 0, after, 0),
            &chunk_raw,
        );
        let ids = (next_id as u32, count as u32);
        next_id += count;
        chunk_ends.push(next_id);
        let mut tables = Vec::new();
        for table in 0..schema.len() {
            let held = state.table(table).unwrap();
            let acquisition = Acquisition {
                last_read: state.sampled_at(table),
                ..Default::default()
            };
            let raw = restart::write(
                held,
                state.status(table).unwrap(),
                &acquisition,
                state.truncated_view(table).unwrap_or(&[]),
            )
            .unwrap();
            let extent = append(
                &mut file,
                Kind::RestartTable,
                (0, table as u32, after, 0),
                &raw,
            );
            let mut chunks = Vec::new();
            state
                .text_ids(table, |id| {
                    chunks.push(chunk_ends.partition_point(|&end| end <= id as usize) as u32);
                    Ok(())
                })
                .unwrap();
            chunks.sort_unstable();
            chunks.dedup();
            tables.push(RestartTable {
                table: table as u32,
                extent,
                chunks,
            });
        }
        let units = vec![
            append(
                &mut file,
                Kind::Unit,
                (0, 0, first, envelope::UNIT_ORDINALS),
                manifests.bytes(),
            ),
            append(
                &mut file,
                Kind::Unit,
                (
                    0,
                    TableId::Countries.index() as u32 + 1,
                    first,
                    envelope::UNIT_ORDINALS,
                ),
                table_bytes.bytes(),
            ),
        ];
        let mut bits = vec![0; TABLE_COUNT.div_ceil(64)];
        bits[TableId::Countries.index() / 64] |= 1 << (TableId::Countries.index() % 64);
        let bits = Bits::from_words(bits).unwrap();
        let mut page = IndexPage {
            run: 0,
            first,
            hours: envelope::UNIT_ORDINALS,
            first_hour: 100 + first as i32,
            last_hour: 100 + after as i32,
            units,
            restart: Some(RestartDirectory {
                after,
                hour: 100 + after as i32,
                ordinal: unit,
                chunk,
                ids,
                tables,
            }),
            sectioned: bits.clone(),
            read: bits.clone(),
            status_changed: bits,
            chain,
        };
        tamper(&mut page);
        pages.push(append(
            &mut file,
            Kind::IndexPage,
            (0, 0, first, envelope::UNIT_ORDINALS),
            &page.write().unwrap(),
        ));
    }
    let directory = RunDirectory {
        branch: 7,
        epoch: envelope::epoch::START,
        first_hour: 100,
        ordinals: units * envelope::UNIT_ORDINALS,
        chain,
        pages,
    };
    let directory = append(
        &mut file,
        Kind::RunDirectory,
        (0, 0, 0, 0),
        &directory.write(0).unwrap(),
    );
    let root = Root {
        identity,
        revision: 1,
        runs: vec![directory],
        controls: Vec::new(),
    };
    let root = append(&mut file, Kind::Root, (0, 0, 0, 0), &root.write().unwrap());
    file.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
    file
}

/// What a synthetic writer saw of an hour: its frame, and each table's sample hour.
pub type Seen = (DecodedFrame, Vec<Option<i32>>);

/// Countries and world tension over `runs` (branch, first hour, units), written as the native
/// recorder writes them and stored as the converter stores them ([`written`]). Strings come
/// back pages after their last use, as text cells and as an unavailable table's reasons, and
/// a row leaves and comes back with the string it held.
pub fn churned(runs: &[(u64, i32, u32)], restart_units: u32) -> (Vec<u8>, Vec<Seen>) {
    use super::synthetic::{Input, Sample};
    use crate::rows::{CountryRow, WorldTensionRow};
    let streams = [TableId::Countries, TableId::WorldTension];
    written(
        b"churned synthetic",
        runs,
        restart_units,
        &streams,
        |synthetic, _, ordinal| {
            let mut rows = vec![
                CountryRow {
                    id: 1,
                    tag: b"old string".to_vec(),
                    stability_raw: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 2,
                    tag: format!("new string {ordinal}").into_bytes(),
                    stability_raw: i64::from(ordinal),
                    ..Default::default()
                },
                // The same 40 strings, each back 2,000 hours after it was last held.
                CountryRow {
                    id: 3,
                    tag: format!("cycle {}", ordinal / 50 % 40).into_bytes(),
                    ..Default::default()
                },
            ];
            if ordinal / 100 % 2 == 0 {
                rows.push(CountryRow {
                    id: 4,
                    tag: b"returning".to_vec(),
                    ..Default::default()
                });
            }
            synthetic
                .table(
                    TableId::Countries,
                    Input::Snapshot {
                        rows: &rows,
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
            let block = ordinal / 300;
            let tension = if block % 2 == 0 {
                synthetic.table(
                    TableId::WorldTension,
                    Input::Snapshot {
                        rows: &[WorldTensionRow {
                            total_raw: i64::from(block),
                            source_count: 1,
                            country_aggregate_count: 2,
                        }],
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
            } else {
                synthetic.table::<WorldTensionRow>(
                    TableId::WorldTension,
                    Input::Unavailable {
                        reason: &format!("refused {}", block / 2 % 3),
                        read: false,
                        sample: Sample::FromReadBit,
                    },
                )
            };
            tension.unwrap();
        },
    )
}

/// `streams` over `runs` (branch, first hour, units), `write(synthetic, run, ordinal)` writing
/// each hour's tables as the native recorder writes them, stored as the converter stores them:
/// each unit's manifests and sections, and a restart ending every `restart_units`th unit. The
/// file, and what the writer saw of each hour, run after run.
pub fn written(
    campaign: &[u8],
    runs: &[(u64, i32, u32)],
    restart_units: u32,
    streams: &[TableId],
    write: impl FnMut(&mut super::synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    written_captured(campaign, None, runs, restart_units, streams, write)
}

/// One run of `ordinals` hours, `frame(ordinal)` each, written as the native recorder writes
/// them: through the frame writer into the columnar file writer, a restart ending every
/// `restart_units`th unit. The run is the first frame's branch from its hour.
pub fn recorded(
    campaign: &[u8],
    ordinals: u32,
    restart_units: u32,
    mut frame: impl FnMut(u32) -> crate::frame::FrameInput,
) -> Vec<u8> {
    use crate::{
        columnar_file::{FileWriter, Hour, Position, Settings},
        native_frame::FrameWriter,
    };
    let mut writer = FrameWriter::new().unwrap();
    let identity = Identity {
        schema: envelope::schema_digest(&schema().unwrap()),
        campaign: campaign.to_vec(),
        content: Vec::new(),
        capture_mode: CaptureMode::Full.byte(),
    };
    let settings = Settings {
        restart_units,
        ..Settings::RECORDING
    };
    let mut file = FileWriter::create(
        std::io::Cursor::new(Vec::new()),
        identity,
        writer.columns().to_vec(),
        settings,
    )
    .unwrap();
    for ordinal in 0..ordinals {
        let input = frame(ordinal);
        if ordinal == 0 {
            file.start_run(input.branch, envelope::epoch::START, input.hour).unwrap();
        }
        let prepared = writer.hour(&input).unwrap();
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        let position = file
            .hour(Hour {
                hour: input.hour,
                manifest: &prepared.manifest,
                sections: &sections,
                statuses: prepared.statuses(),
                read: prepared.read(),
            })
            .unwrap();
        drop(sections);
        writer.commit(prepared).unwrap();
        match position {
            Position::Open => {}
            Position::Full { restart: false } => {
                file.close_position().unwrap();
            }
            Position::Full { restart: true } => {
                let (strings, tables) = writer.restart().unwrap();
                file.close_with_restart(&strings, &tables).unwrap();
            }
        }
    }
    file.finish(|_| Ok(())).unwrap().file.into_inner()
}

/// [`written`], its root's control records holding `capture` when given.
pub fn written_captured(
    campaign: &[u8],
    capture: Option<&crate::columnar_file::Capture>,
    runs: &[(u64, i32, u32)],
    restart_units: u32,
    streams: &[TableId],
    mut write: impl FnMut(&mut super::synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    use super::synthetic::Synthetic;
    let schema = schema().unwrap();
    let columns: Vec<_> = schema.iter().map(Vec::len).collect();
    let identity = Identity {
        schema: envelope::schema_digest(&schema),
        campaign: campaign.to_vec(),
        content: Vec::new(),
        capture_mode: 0,
    };
    let mut written = vec![0; TABLE_COUNT.div_ceil(64)];
    for table in streams {
        written[table.index() / 64] |= 1 << (table.index() % 64);
    }
    let written = Bits::from_words(written).unwrap();
    let mut file = envelope::MAGIC.to_vec();
    file.extend_from_slice(&envelope::VERSION.to_le_bytes());
    let (mut directories, mut seen) = (Vec::new(), Vec::new());
    for (run, &(branch, first_hour, units)) in (0..).zip(runs) {
        let mut synthetic = Synthetic::new(streams, branch, first_hour).unwrap();
        let mut chain = envelope::chain_start(&identity.chain_bytes().unwrap(), run);
        let (mut pages, mut chunk_ends) = (Vec::new(), Vec::new());
        for unit in 0..units {
            let first = unit * envelope::UNIT_ORDINALS;
            let after = first + envelope::UNIT_ORDINALS - 1;
            let mut manifests = Writer::new();
            let mut sections: Vec<Writer> = streams.iter().map(|_| Writer::new()).collect();
            for ordinal in first..=after {
                write(&mut synthetic, run, ordinal);
                synthetic
                    .end_encoded(|_, manifest, frame, _, encoded| {
                        let bytes = manifest.write(&columns).unwrap();
                        framing::put_manifest(&mut manifests, &bytes).unwrap();
                        for (section, raw) in encoded {
                            let at = streams
                                .iter()
                                .position(|table| table.index() == section.table)
                                .unwrap();
                            framing::put_section(&mut sections[at], raw).unwrap();
                        }
                        chain = envelope::chain_hour(
                            &chain,
                            &bytes,
                            encoded.iter().map(|(_, raw)| raw.as_slice()),
                        );
                        seen.push((
                            frame.clone(),
                            frame.tables.iter().map(SharedView::sampled_at).collect(),
                        ));
                    })
                    .unwrap();
            }
            let mut extents = vec![append(
                &mut file,
                Kind::Unit,
                (run, framing::MANIFESTS, first, envelope::UNIT_ORDINALS),
                manifests.bytes(),
            )];
            for (table, bytes) in streams.iter().zip(&sections) {
                if !bytes.bytes().is_empty() {
                    let stream = framing::table_stream(table.index());
                    extents.push(append(
                        &mut file,
                        Kind::Unit,
                        (run, stream, first, envelope::UNIT_ORDINALS),
                        bytes.bytes(),
                    ));
                }
            }
            extents.sort_by_key(|extent| extent.stream);
            let restart = if (unit + 1) % restart_units == 0 {
                let state = synthetic.state();
                let next_id = chunk_ends.last().copied().unwrap_or(0);
                let count = state.dictionary().len() - next_id;
                let chunk = append(
                    &mut file,
                    Kind::DictionaryChunk,
                    (run, 0, after, 0),
                    &state.dictionary().chunk(next_id).unwrap(),
                );
                chunk_ends.push(next_id + count);
                let mut tables = Vec::new();
                for table in 0..TABLE_COUNT {
                    let raw = restart::write(
                        state.table(table).unwrap(),
                        state.status(table).unwrap(),
                        &Acquisition {
                            last_read: state.sampled_at(table),
                            ..Default::default()
                        },
                        state.truncated_view(table).unwrap_or(&[]),
                    )
                    .unwrap();
                    let mut chunks = Vec::new();
                    state
                        .text_ids(table, |id| {
                            chunks
                                .push(chunk_ends.partition_point(|&end| end <= id as usize) as u32);
                            Ok(())
                        })
                        .unwrap();
                    chunks.sort_unstable();
                    chunks.dedup();
                    tables.push(RestartTable {
                        table: table as u32,
                        extent: append(
                            &mut file,
                            Kind::RestartTable,
                            (run, table as u32, after, 0),
                            &raw,
                        ),
                        chunks,
                    });
                }
                Some(RestartDirectory {
                    after,
                    hour: first_hour + after as i32,
                    ordinal: chunk_ends.len() as u32 - 1,
                    chunk,
                    ids: (next_id as u32, count as u32),
                    tables,
                })
            } else {
                None
            };
            let page = IndexPage {
                run,
                first,
                hours: envelope::UNIT_ORDINALS,
                first_hour: first_hour + first as i32,
                last_hour: first_hour + after as i32,
                units: extents,
                restart,
                sectioned: written.clone(),
                read: written.clone(),
                status_changed: written.clone(),
                chain,
            };
            pages.push(append(
                &mut file,
                Kind::IndexPage,
                (run, 0, first, envelope::UNIT_ORDINALS),
                &page.write().unwrap(),
            ));
        }
        let directory = RunDirectory {
            branch,
            epoch: envelope::epoch::START,
            first_hour,
            ordinals: units * envelope::UNIT_ORDINALS,
            chain,
            pages,
        };
        directories.push(append(
            &mut file,
            Kind::RunDirectory,
            (run, 0, 0, 0),
            &directory.write(run).unwrap(),
        ));
    }
    let controls = capture
        .map(|capture| {
            append(
                &mut file,
                Kind::Controls,
                (0, 0, 0, 0),
                &capture.write().unwrap(),
            )
        })
        .into_iter()
        .collect();
    let root = Root {
        identity,
        revision: 1,
        runs: directories,
        controls,
    };
    let root = append(&mut file, Kind::Root, (0, 0, 0, 0), &root.write().unwrap());
    file.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
    (file, seen)
}
