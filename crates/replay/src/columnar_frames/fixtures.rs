//! Synthetic version 3 recordings, written as the converter writes them, for tests.

use super::*;
use replay_columnar::{
    envelope::{Identity, RestartDirectory, RestartTable},
    leaf::Writer,
    restart::{self, Acquisition},
    state::Bits,
};

/// One extent of `raw` appended to `file`, compressed as the converter stores it.
fn append(
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
pub(crate) fn every(runs: &[(u64, i32, u32)]) -> impl Fn(u32, u32) -> Option<i32> + '_ {
    |run, ordinal| Some(runs[run as usize].1 + ordinal as i32)
}
