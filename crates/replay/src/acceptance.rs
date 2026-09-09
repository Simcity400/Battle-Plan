//! Synthetic source states, faulting spool I/O, and indexed reads share no oracle logic.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{self, Write},
    rc::Rc,
};

use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    branches::{BranchReason, BranchStart, BranchTracker, Gap, Parentage},
    container::{Replay, SeekResult},
    frame::{DecodedFrame, FrameEncoder, Metrics, TableUpdate, TableView},
    identity::{Identity, Roster, SessionStart, Verification},
    rows::{
        CampaignRow, ClockRow, CountryLinkRow, CountryRow, ProvinceRow, Row, StateRow, TableId,
    },
    spool::{
        Record, Recovery, SegmentBackend, SegmentSink, SegmentedSpool, encode_record,
        recover_prefix,
    },
};

const CAMPAIGN: &[u8] = b"synthetic-campaign-native-values";
const HOURS: usize = 2_000;

#[derive(Default)]
struct Disk {
    segments: BTreeMap<u32, Vec<u8>>,
    /// A partial write succeeds up to this many bytes; its next write fails.
    fail_after: Option<usize>,
}

#[derive(Clone, Default)]
struct MemoryBackend(Rc<RefCell<Disk>>);

struct MemorySink {
    disk: Rc<RefCell<Disk>>,
    index: u32,
}

impl SegmentBackend for MemoryBackend {
    type Sink = MemorySink;

    fn create(&mut self, index: u32) -> io::Result<Self::Sink> {
        assert!(
            self.0
                .borrow_mut()
                .segments
                .insert(index, Vec::new())
                .is_none()
        );
        Ok(MemorySink {
            disk: self.0.clone(),
            index,
        })
    }
}

impl Write for MemorySink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if disk.fail_after == Some(0) {
            disk.fail_after = None;
            return Err(io::Error::other("injected disk write failure"));
        }
        let length = disk
            .fail_after
            .map_or(bytes.len(), |left| bytes.len().min(left));
        if let Some(left) = &mut disk.fail_after {
            *left -= length;
        }
        disk.segments
            .get_mut(&self.index)
            .unwrap()
            .extend_from_slice(&bytes[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SegmentSink for MemorySink {
    fn sync_all(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: 100 + id as u32,
        process_start_time: u64::MAX - id,
        qpc_frequency: 10_000_000,
        identity: Identity {
            game_version: "synthetic build".into(),
            exe_hash: [11; 32],
            exe_size: 123,
            vanilla_hash: [12; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: format!("test-session-{id}"),
            collector_hash: [id as u8; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

fn random(index: usize, salt: u64) -> u64 {
    let mut value = (index as u64)
        .wrapping_add(salt)
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Independent direct reads: no previous rows, field masks, or format reader participate.
fn source(index: usize, branch: BranchId, hour: Hour, signal: i64) -> DecodedFrame {
    let raw = match index % 31 {
        0 => i64::MIN,
        1 => i64::MAX,
        _ => (1_i64 << 40) - index as i64,
    };
    let country = |id, canonical_slot| {
        Row::Country(CountryRow {
            id,
            canonical_slot,
            tag: if id == 7 {
                b"GER".to_vec()
            } else {
                b"TAG".to_vec()
            },
            name: vec![0xff, id as u8, 0, b'N'],
            political_power_raw: (index % 11 != 3).then_some(raw),
            command_power_raw: -((index / 7) as i64) * 100_000,
            stability_raw: 78_901,
            war_support_raw: 54_321,
            convoys_raw: (index % 19 != 4).then_some(1_i64 << 42),
        })
    };
    let mut countries = vec![country(7, 1), country(19, 3)];
    let mut links = vec![
        Row::CountryLink(CountryLinkRow {
            id: 2,
            canonical_slot: 1,
        }),
        Row::CountryLink(CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        }),
        Row::CountryLink(CountryLinkRow {
            id: 19,
            canonical_slot: 3,
        }),
    ];
    if index % 13 < 7 {
        countries.push(country(29, 5));
        links.push(Row::CountryLink(CountryLinkRow {
            id: 29,
            canonical_slot: 5,
        }));
    }
    let mut provinces: Vec<_> = (1..=8)
        .map(|id| {
            Row::Province(ProvinceRow {
                id,
                state: id / 3,
                owner: 2,
                controller: if id == 1 {
                    if index.is_multiple_of(2) { 7 } else { 2 }
                } else {
                    [0, 2, 19][(index / 9 + id as usize) % 3]
                },
            })
        })
        .collect();
    if index % 17 < 5 {
        provinces.push(Row::Province(ProvinceRow {
            id: 9,
            state: u32::MAX,
            owner: 0,
            controller: 0,
        }));
    }
    let mut states: Vec<_> = (0..=2)
        .map(|id| {
            Row::State(StateRow {
                id,
                owner: [2, 19][(index / 37 + id as usize) % 2],
                controller: [0, 7, 19][(index / 5 + id as usize) % 3],
            })
        })
        .collect();
    if index % 17 < 5 {
        states.push(Row::State(StateRow {
            id: u32::MAX,
            owner: 0,
            controller: 0,
        }));
    }
    let mut tables = [
        TableView::Complete(vec![Row::Clock(ClockRow {
            hour,
            play_time: (1_u64 << 40) + index as u64,
            play_session_started_at: signal,
            seed: i32::MIN,
        })]),
        TableView::Complete(vec![Row::Campaign(CampaignRow {
            id: CAMPAIGN.to_vec(),
        })]),
        TableView::Complete(countries),
        TableView::Complete(links),
        TableView::Complete(provinces),
        TableView::Complete(states),
    ];
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Provinces,
        TableId::States,
    ] {
        let draw = random(index, 91 + table.index() as u64);
        let force_partial = matches!(index, 0 | 38 | 758 | 830)
            && matches!(table, TableId::Countries | TableId::States);
        if draw.is_multiple_of(47) || (index == 780 && table == TableId::Provinces) {
            tables[table.index()] = TableView::Unavailable {
                reason: format!("synthetic pointer unavailable for {table:?} at {hour}"),
            };
        } else if draw.is_multiple_of(43) || force_partial {
            let TableView::Complete(rows) = &tables[table.index()] else {
                unreachable!()
            };
            tables[table.index()] = TableView::Truncated {
                rows: rows[..1].to_vec(),
                missing: (rows.len() - 1) as u32,
            };
        }
    }
    DecodedFrame {
        branch,
        hour,
        tables,
        metrics: Metrics {
            total_ns: 100_000 + index as u64,
            table_ns: [1, 2, 3, 4, 5, 6],
            pool_bytes: 1 << 25,
            dropped_frames: (index / 61) as u64,
        },
    }
}

/// Protect scenario boundaries and known parent witnesses from random queue drops.
fn may_drop(index: usize) -> bool {
    ![
        0_usize, 37, 38, 399, 448, 758, 780, 828, 830, 880, 929, 930, 999, 1000, 1499, 1500, 1999,
    ]
    .iter()
    .any(|anchor| index.abs_diff(*anchor) <= 2)
        && random(index, 7).is_multiple_of(61)
}

#[derive(Default)]
struct Coverage {
    keyframes: usize,
    periodic_keyframes: usize,
    delta_tables: usize,
    deletions: usize,
    insertions: usize,
    unchanged_tables: usize,
    unavailable_tables: usize,
    truncated_tables: usize,
    partial_keyframes: usize,
    state_delta_tables: usize,
    state_unavailable_tables: usize,
    state_truncated_tables: usize,
    state_partial_keyframes: usize,
}

struct Campaign {
    segments: Vec<(u32, Recovery)>,
    expected: BTreeMap<(BranchId, Hour), DecodedFrame>,
    missing: BTreeMap<(BranchId, Hour), String>,
    starts: Vec<BranchStart>,
    coverage: Coverage,
}

fn synthesize() -> Campaign {
    let backend = MemoryBackend::default();
    let disk = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let mut tracker = BranchTracker::new();
    let mut encoder = FrameEncoder::new();
    let mut expected = BTreeMap::new();
    let mut missing = BTreeMap::new();
    let mut starts = Vec::new();
    let mut pending_gaps = Vec::new();
    let mut coverage = Coverage::default();
    let mut current_session = session(1);
    let mut hour = 10_000;
    let mut signal = 1_000;
    let mut last_keyframe = None;
    let mut failed_length = None;
    let mut final_frame_bytes = 0;
    let mut final_hour = (0, 0);

    for index in 0..HOURS {
        if matches!(index, 0 | 1000 | 1500) {
            if index != 0 {
                spool
                    .append(&Record::Detach {
                        session: current_session.id,
                        last_hour: Some(hour),
                    })
                    .unwrap();
                spool.finish_segment().unwrap();
            }
            current_session = session(if index == 0 {
                1
            } else if index == 1000 {
                2
            } else {
                3
            });
            tracker.begin_session(current_session.id).unwrap();
            if index == 1500 {
                hour = 10_449;
            }
        }
        match index {
            780 => {
                hour = 10_400;
                signal += 1;
            } // Earlier recorded hour, one possible parent.
            830 => {
                hour = 10_449;
                signal += 1;
            } // Repeat last sampled hour, ambiguous parent.
            880 => {
                hour = 12_000;
                signal += 1;
            } // Later save outside any recorded branch.
            930 => {
                signal += 1;
            } // Ordinary next-hour sample; only the load signal exposes it.
            _ => {}
        }
        let loaded_hour = hour;
        hour += 1;
        if may_drop(index) {
            let branch = tracker.current_branch().unwrap();
            let reason = "bounded queue overflow".to_owned();
            pending_gaps.push(Gap {
                branch,
                first_hour: hour,
                last_hour: hour,
                reason: reason.clone(),
            });
            assert!(missing.insert((branch, hour), reason).is_none());
            continue;
        }
        let observed = tracker
            .observe(Some(loaded_hour), hour, Some(signal), &pending_gaps)
            .unwrap();
        if let Some(start) = &observed.start {
            // Calculate ancestry from direct successfully written witnesses, not the tracker.
            let parents: Vec<_> = expected
                .keys()
                .filter_map(|&(branch, recorded)| (recorded == loaded_hour).then_some(branch))
                .collect();
            let parent = match parents.as_slice() {
                [branch] => Parentage::Probable {
                    branch: *branch,
                    hour: loaded_hour,
                },
                _ => Parentage::Unknown,
            };
            assert_eq!(start.parent, parent);
            starts.push(start.clone());
        }
        let source = source(index, observed.branch, hour, signal);
        let force = !spool.is_open() || observed.start.is_some();
        let periodic = !force
            && last_keyframe.is_some_and(|previous| {
                i64::from(hour) - i64::from(previous) >= i64::from(KEYFRAME_INTERVAL)
            });
        let keyframe = force || periodic;
        let frame = encoder.encode(&source, force).unwrap();
        assert_eq!(
            frame.keyframe, keyframe,
            "keyframe cadence at source {index}"
        );
        if keyframe {
            last_keyframe = Some(hour);
        }
        coverage.keyframes += usize::from(keyframe);
        coverage.periodic_keyframes += usize::from(periodic);
        for (table, update) in TableId::ALL.into_iter().zip(&frame.tables) {
            match update {
                TableUpdate::Delta {
                    changed, deleted, ..
                } => {
                    coverage.delta_tables += 1;
                    coverage.state_delta_tables += usize::from(table == TableId::States);
                    coverage.deletions += deleted.len();
                    coverage.insertions += changed
                        .iter()
                        .filter(|row| row.mask == table.full_mask())
                        .count();
                }
                TableUpdate::Unchanged { .. } => coverage.unchanged_tables += 1,
                TableUpdate::Unavailable { .. } => {
                    coverage.unavailable_tables += 1;
                    coverage.state_unavailable_tables += usize::from(table == TableId::States);
                }
                TableUpdate::Truncated { .. } => {
                    coverage.truncated_tables += 1;
                    coverage.state_truncated_tables += usize::from(table == TableId::States);
                }
                TableUpdate::Snapshot(_) => {}
            }
        }

        if spool.is_open() {
            for gap in pending_gaps.drain(..) {
                spool.append(&Record::Gap(gap)).unwrap();
            }
            if let Some(start) = &observed.start {
                spool.append(&Record::BranchStart(start.clone())).unwrap();
            }
            if index == HOURS - 1 {
                spool
                    .append(&Record::ClockWitness {
                        session: current_session.id,
                        qpc: u64::MAX,
                        hour,
                        load_signal: Some(signal),
                    })
                    .unwrap();
            }
            if index == 37 {
                disk.borrow_mut().fail_after =
                    Some(encode_record(&Record::Frame(frame.clone())).unwrap().len() / 2);
                assert!(spool.append(&Record::Frame(frame.clone())).is_err());
                assert!(!spool.is_open());
                let length = disk.borrow().segments[&0].len();
                assert!(spool.append(&Record::Frame(frame)).is_err());
                assert_eq!(disk.borrow().segments[&0].len(), length);
                failed_length = Some(length);
                let reason = "injected disk write failure".to_owned();
                pending_gaps.push(Gap {
                    branch: observed.branch,
                    first_hour: hour,
                    last_hour: hour,
                    reason: reason.clone(),
                });
                missing.insert((observed.branch, hour), reason);
                continue;
            }
            final_frame_bytes = encode_record(&Record::Frame(frame.clone())).unwrap().len();
            spool.append(&Record::Frame(frame)).unwrap();
        } else {
            let mut context = vec![
                Record::SessionStart(current_session.clone()),
                Record::BranchStart(tracker.branch_start(observed.branch).unwrap().clone()),
            ];
            context.extend(pending_gaps.drain(..).map(Record::Gap));
            spool.start_segment(&context, &frame).unwrap();
        }
        tracker.record_hour(observed.branch, hour).unwrap();
        let mut direct = source;
        if keyframe {
            for (id, table) in TableId::ALL.into_iter().zip(&mut direct.tables) {
                if let TableView::Truncated { missing, .. } = table {
                    coverage.partial_keyframes += 1;
                    coverage.state_partial_keyframes += usize::from(id == TableId::States);
                    *table = TableView::Unavailable {
                        reason: format!("truncated at keyframe: {missing} rows not read"),
                    };
                }
            }
        }
        final_hour = (observed.branch, hour);
        assert!(expected.insert(final_hour, direct).is_none());
    }
    spool.finish_segment().unwrap();
    assert!(pending_gaps.is_empty());
    let mut disk = disk.borrow_mut();
    assert_eq!(
        disk.segments[&0].len(),
        failed_length.unwrap(),
        "abandoned segment was appended to"
    );
    assert_eq!(disk.segments.len(), 4);
    // Model forced termination partway through the last record, after its clock witness.
    let last = disk.segments.last_entry().unwrap().into_mut();
    last.truncate(last.len() - final_frame_bytes / 2);
    expected.remove(&final_hour).unwrap();
    let mut segments = Vec::new();
    for (&number, bytes) in &disk.segments {
        let mut recovered = recover_prefix(bytes);
        assert_eq!(recovered.damage.is_some(), matches!(number, 0 | 3));
        if recovered.damage.is_some() {
            assert!(recovered.valid_bytes < bytes.len());
        }
        if number == 3 {
            let witness = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::ClockWitness { hour, .. } => Some(*hour),
                    _ => None,
                })
                .unwrap();
            assert_eq!(witness, final_hour.1);
            let reason = recovered.damage.clone().unwrap();
            recovered.records.push(Record::Gap(Gap {
                branch: final_hour.0,
                first_hour: witness,
                last_hour: witness,
                reason: reason.clone(),
            }));
            missing.insert(final_hour, reason);
        }
        segments.push((number, recovered));
    }
    Campaign {
        segments,
        expected,
        missing,
        starts,
        coverage,
    }
}

fn assert_aliases(frame: &DecodedFrame) {
    let [
        _,
        _,
        TableView::Complete(countries),
        TableView::Complete(links),
        TableView::Complete(provinces),
        _,
    ] = &frame.tables
    else {
        return;
    };
    let Some(Row::Province(province)) = provinces.first() else {
        return;
    };
    assert_eq!(province.owner, 2);
    let slot = links
        .iter()
        .find_map(|row| match row {
            Row::CountryLink(link) if link.id == province.owner => Some(link.canonical_slot),
            _ => None,
        })
        .unwrap();
    let country = countries
        .iter()
        .find_map(|row| match row {
            Row::Country(country) if country.canonical_slot == slot => Some(country),
            _ => None,
        })
        .unwrap();
    assert_eq!((country.id, country.canonical_slot), (7, 1));
    assert_eq!(country.name, [0xff, 7, 0, b'N']);
    assert_ne!(province.owner, country.id);
}

#[test]
fn two_thousand_hours_survive_faults_and_append_with_exact_direct_state() {
    let campaign = synthesize();
    assert_eq!(campaign.expected.len() + campaign.missing.len(), HOURS);
    assert!(campaign.missing.len() > 20);
    assert_eq!(campaign.starts.len(), 7);
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.session)
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 2, 3]
    );
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.reason)
            .collect::<Vec<_>>(),
        [
            BranchReason::SessionStart,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::SessionStart,
            BranchReason::SessionStart,
        ]
    );
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.parent.clone())
            .collect::<Vec<_>>(),
        [
            Parentage::Unknown,
            Parentage::Probable {
                branch: 1,
                hour: 10_400
            },
            Parentage::Unknown,
            Parentage::Unknown,
            Parentage::Probable {
                branch: 4,
                hour: 12_050
            },
            Parentage::Probable {
                branch: 5,
                hour: 12_120
            },
            Parentage::Unknown,
        ]
    );
    let coverage = &campaign.coverage;
    assert!(coverage.keyframes >= 9 && coverage.periodic_keyframes > 0);
    assert!(coverage.delta_tables > 1_000 && coverage.unchanged_tables > 1_000);
    assert!(coverage.deletions > 100 && coverage.insertions > 100);
    assert!(coverage.unavailable_tables > 50 && coverage.truncated_tables > 50);
    assert!(coverage.partial_keyframes >= 3);
    assert!(coverage.state_delta_tables > 100);
    assert!(coverage.state_unavailable_tables > 0 && coverage.state_truncated_tables > 0);
    assert!(coverage.state_partial_keyframes > 0);

    let first = Replay::from_recovered(CAMPAIGN.to_vec(), &campaign.segments[..2]).unwrap();
    assert_eq!(first.sessions.len(), 1);
    let old_bytes = first.bytes().to_vec();
    let appended = first.append_recovered(&campaign.segments[2..]).unwrap();
    assert_eq!(first.bytes(), old_bytes);
    for block in &first.index {
        let start = block.offset as usize;
        let end = start + block.length as usize;
        assert_eq!(&appended.bytes()[start..end], &old_bytes[start..end]);
        assert!(appended.index.contains(block));
    }
    let replay = Replay::open(appended.bytes().to_vec()).unwrap();
    assert_eq!(replay.sessions, [session(1), session(2), session(3)]);
    assert_eq!(replay.branches, campaign.starts);
    assert_eq!(replay.campaign_id, CAMPAIGN);
    assert_eq!(
        replay
            .recovery_notes
            .iter()
            .map(|note| note.segment)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    for note in &replay.recovery_notes {
        let recovery = &campaign.segments[note.segment as usize].1;
        assert_eq!(note.valid_bytes, recovery.valid_bytes as u64);
        assert_eq!(Some(note.reason.as_str()), recovery.damage.as_deref());
    }
    assert!(replay.index.iter().all(|block| i64::from(block.last_hour)
        - i64::from(block.first_hour)
        < i64::from(KEYFRAME_INTERVAL)));

    // Every target is an independently generated source state, including partial rows and metrics.
    for (&(branch, hour), expected) in &campaign.expected {
        match replay.seek(branch, hour).unwrap() {
            SeekResult::Frame(actual) => {
                assert_eq!(actual.as_ref(), expected, "branch {branch}, hour {hour}");
                assert_aliases(&actual);
            }
            other => panic!("readable branch {branch}, hour {hour}: {other:?}"),
        }
    }
    for (&(branch, hour), expected_reason) in &campaign.missing {
        match replay.seek(branch, hour).unwrap() {
            SeekResult::Gap {
                branch: actual_branch,
                hour: actual_hour,
                reason,
            } => {
                assert_eq!((actual_branch, actual_hour), (branch, hour));
                assert!(
                    reason.contains(expected_reason),
                    "{reason:?} lacks {expected_reason:?}"
                );
            }
            other => panic!("lost branch {branch}, hour {hour} was invented: {other:?}"),
        }
    }
}
