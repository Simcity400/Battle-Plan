use crate::prepared::driver::native::{measure, status};
use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records, source};
use replay::{
    columnar_frames::{
        Damage, Recording, Segment, fixtures, schema,
        synthetic::{Input, Sample},
    },
    campaign::Replay,
    segments::State,
};
use replay_data::{
    rows::{CampaignRow, TABLE_COUNT, TableId, TableRows},
    scope::{self, CaptureMode},
};
use replay_frame::frame::{FrameInput, Metrics, TableInput, TableView};
use std::{cell::Cell, io, sync::atomic::AtomicBool};

thread_local! {
    /// The hours [`super::check`] walked on this thread.
    pub(super) static WALKED: Cell<u64> = const { Cell::new(0) };
}

/// One run of branch 1 from hour 0, two units long: a restart ending its first starts a second
/// segment.
const RUNS: [(u64, i32, u32); 1] = [(1, 0, 2)];
const HOURS: i32 = 1_440;
/// An ordinal of the second segment, from which the Campaign row may name another campaign.
const CHANGED: u32 = 1_000;

/// A native recording of [`RUNS`] whose envelope names [`CAMPAIGN`], as its Campaign row does
/// until [`CHANGED`], and `later` from there.
fn native(later: &[u8]) -> Recording {
    let (bytes, _) = fixtures::written(
        CAMPAIGN,
        &RUNS,
        1,
        &[TableId::Campaign],
        |synthetic, _, ordinal| {
            let id = if ordinal < CHANGED { CAMPAIGN } else { later };
            synthetic
                .table(
                    TableId::Campaign,
                    Input::Snapshot {
                        rows: &[CampaignRow { id: id.to_vec() }],
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
        },
    );
    Recording::open(bytes).unwrap()
}

/// What a native preparation of no family made of [`native`]`(later)`, and whether it published.
fn prepared(later: &[u8]) -> (io::Result<()>, bool) {
    let frames = (0..HOURS).map(|hour| source(1, hour, None)).collect();
    let replay = Replay::from_records(CAMPAIGN.to_vec(), &[records(1, frames)]).unwrap();
    let (source, root) = file_source(replay);
    let cancelled = AtomicBool::new(false);
    let made = measure(
        &source,
        None,
        native(later),
        None,
        Some(&[][..]),
        1,
        false,
        &cancelled,
    )
    .map(drop);
    let (store, _) = crate::prepared::files(&source, None).unwrap();
    let published = status::held(&store).unwrap().is_some();
    let _ = store.clear();
    let _ = std::fs::remove_dir_all(store.directory());
    let _ = std::fs::remove_dir_all(&root);
    (made, published)
}

/// A complete Campaign row naming another campaign than the envelope, past the recording's first
/// segment, refuses the native preparation whole with the row path's refusal, publishing nothing;
/// one naming the envelope's campaign throughout prepares.
#[test]
fn a_campaign_row_naming_another_campaign_refuses_the_native_preparation() {
    let (made, published) = prepared(b"another campaign");
    refused(made);
    assert!(!published);
    let (made, published) = prepared(CAMPAIGN);
    made.unwrap();
    assert!(published);
}

/// A columnar block's hours: one index page each.
const UNIT: u32 = 720;
const TABLE: u32 = TableId::Campaign.index() as u32;
const OTHER: &[u8] = b"another campaign";

fn complete(id: &[u8]) -> TableView {
    TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: id.to_vec() }]))
}

/// The one row `id`, read short of a row it misses.
fn truncated(id: &[u8]) -> TableView {
    TableView::Truncated {
        rows: TableRows::Campaign(vec![CampaignRow { id: id.to_vec() }]),
        missing: 1,
    }
}

/// One run of branch 1 from hour 0, `units` units long and a restart ending every
/// `restart_units`th, as the native recorder writes it: Campaign read as `campaign(unit)` every
/// hour of each unit, every other table it records unavailable.
fn recorded(units: u32, restart_units: u32, campaign: impl Fn(u32) -> TableView) -> Vec<u8> {
    let bytes = fixtures::recorded(CAMPAIGN, units * UNIT, restart_units, |ordinal| {
        let mut tables: [TableInput; TABLE_COUNT] = std::array::from_fn(|index| {
            TableInput::Sampled(if scope::records(CaptureMode::Full, TableId::ALL[index]) {
                TableView::Unavailable {
                    reason: "not in this test".into(),
                }
            } else {
                TableView::NotRecorded {
                    mode: CaptureMode::Full,
                }
            })
        });
        tables[TABLE as usize] = TableInput::Sampled(campaign(ordinal / UNIT));
        let mut table_ns = [0; TABLE_COUNT];
        table_ns[TABLE as usize] = 1;
        FrameInput {
            branch: 1,
            hour: ordinal as i32,
            tables,
            metrics: Metrics {
                total_ns: 1,
                table_ns,
                pool_bytes: 0,
                dropped_frames: 0,
            },
        }
    });
    let recording = opened(&bytes);
    assert_eq!(recording.page(0, 0).unwrap().hours, UNIT);
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    bytes
}

fn opened(bytes: &[u8]) -> Recording {
    Recording::open(bytes.to_vec()).unwrap()
}

/// Whether page `at` of `recording` sections Campaign, and whether it changes its status.
fn touches(recording: &Recording, at: usize) -> (bool, bool) {
    let page = recording.page(0, at).unwrap();
    (page.sectioned.get(TABLE), page.status_changed.get(TABLE))
}

/// What [`super::check`] made of `bytes` against a replay of [`CAMPAIGN`], and the hours it
/// walked.
fn checked(bytes: &[u8]) -> (io::Result<()>, u64) {
    let replay =
        Replay::from_records(CAMPAIGN.to_vec(), &[records(1, vec![source(1, 0, None)])]).unwrap();
    WALKED.with(|walked| walked.set(0));
    let made = super::check(&mut opened(bytes), &replay, &AtomicBool::new(false));
    (made, WALKED.with(Cell::get))
}

fn refused(made: io::Result<()>) {
    let error = made.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "frame campaign identity differs from replay header"
    );
}

/// `bytes` with one byte flipped inside the extent at `(offset, stored)`.
fn damaged(bytes: &[u8], (offset, stored): (u64, u64)) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    bytes[(offset + stored / 2) as usize] ^= 0xff;
    bytes
}

/// Page `at`'s Campaign block.
fn unit(bytes: &[u8], at: usize) -> (u64, u64) {
    let page = opened(bytes).page(0, at).unwrap();
    let block = page.tables.iter().find(|block| block.stream == TABLE).unwrap();
    (block.offset, block.stored)
}

/// Whether walking `segment` of `bytes` meets damage.
fn meets_damage(bytes: &[u8], segment: usize) -> bool {
    let recording = opened(bytes);
    let mut state = State::new(schema().unwrap()).unwrap();
    let walked = recording.walk_segment(&recording.segments()[segment], &mut state, |_, _, _| {
        Ok(true)
    });
    walked.is_err_and(|error| Damage::of(&error).is_some())
}

/// Segments whose pages never touch Campaign are not walked: a recording whose Campaign is the
/// envelope's throughout walks only the hours through its first page.
#[test]
fn a_campaign_left_alone_is_walked_only_where_a_page_touches_it() {
    let bytes = recorded(4, 1, |_| complete(CAMPAIGN));
    let recording = opened(&bytes);
    assert_eq!(recording.segments().len(), 4);
    assert_eq!(touches(&recording, 0), (true, true));
    for at in 1..4 {
        assert_eq!(touches(&recording, at), (false, false), "page {at}");
    }
    let (made, walked) = checked(&bytes);
    made.unwrap();
    assert_eq!(walked, u64::from(UNIT));
}

/// Past segments that never touch Campaign, one reads it short holding another campaign's row
/// and then whole without a new row: the page changing only its status still makes the check
/// walk to it, and refuse it there.
#[test]
fn a_campaign_completed_by_its_status_alone_is_refused_past_untouched_segments() {
    let bytes = recorded(8, 2, |unit| match unit {
        0..=5 => complete(CAMPAIGN),
        6 => truncated(OTHER),
        _ => complete(OTHER),
    });
    let recording = opened(&bytes);
    let branch = recording.run(0).unwrap().branch;
    assert_eq!(
        recording.segments(),
        [
            Segment { branch, run: 0, restart: None, first: 0, last: 2 * UNIT - 1 },
            Segment { branch, run: 0, restart: Some(1), first: 2 * UNIT, last: 4 * UNIT - 1 },
            Segment { branch, run: 0, restart: Some(3), first: 4 * UNIT, last: 6 * UNIT - 1 },
            Segment { branch, run: 0, restart: Some(5), first: 6 * UNIT, last: 8 * UNIT - 1 },
        ]
    );
    for at in 1..6 {
        assert_eq!(touches(&recording, at), (false, false), "page {at}");
    }
    assert!(touches(&recording, 6).0);
    assert_eq!(touches(&recording, 7), (false, true));
    let (made, walked) = checked(&bytes);
    refused(made);
    // The first page, then the last segment from its restart to its second page's first hour.
    assert_eq!(walked, u64::from(2 * UNIT + 1));
}

/// A damaged Campaign unit hides the hours it changes Campaign in, but not the restart after
/// it: the quiet segment from that restart, holding another campaign's row, is refused.
#[test]
fn a_quiet_segment_after_a_damaged_campaign_unit_is_checked_from_its_restart() {
    let bytes = recorded(3, 1, |unit| complete(if unit == 0 { CAMPAIGN } else { OTHER }));
    assert_eq!(touches(&opened(&bytes), 2), (false, false));
    let bytes = damaged(&bytes, unit(&bytes, 1));
    assert!(meets_damage(&bytes, 1));
    refused(checked(&bytes).0);
}

/// A segment touching Campaign whose restart is damaged is walked from the restart before it,
/// through the segment that reads another campaign's row short: its completion is refused.
#[test]
fn a_damaged_restart_is_rebuilt_from_the_segments_before_it() {
    let bytes = recorded(3, 1, |unit| match unit {
        0 => complete(CAMPAIGN),
        1 => truncated(OTHER),
        _ => complete(OTHER),
    });
    let restart = opened(&bytes).page(0, 1).unwrap().restart.unwrap();
    let table = restart.tables.iter().find(|table| table.table == TABLE).unwrap();
    let bytes = damaged(&bytes, (table.extent.offset, table.extent.stored));
    assert!(meets_damage(&bytes, 2));
    refused(checked(&bytes).0);
}

/// Damage hiding the only hours another campaign's row is complete in, with no restart after
/// it, leaves Campaign unknown there: not refused, as the row path does not refuse a Campaign
/// it does not hold whole.
#[test]
fn damage_leaving_campaign_unknown_is_not_refused() {
    let bytes = recorded(2, 1, |unit| complete(if unit == 0 { CAMPAIGN } else { OTHER }));
    refused(checked(&bytes).0);
    let bytes = damaged(&bytes, unit(&bytes, 1));
    assert!(meets_damage(&bytes, 1));
    checked(&bytes).0.unwrap();
}
