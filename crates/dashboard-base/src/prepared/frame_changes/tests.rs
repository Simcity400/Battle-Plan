use super::*;
use crate::test_fixtures::source;
use replay::frame::{FrameDecoder, FrameEncoder, READ_TIME_NOT_RECORDED, TableUpdate, TableView};
use replay::rows::{TableRows, tables};

#[test]
fn frame_changes_distinguish_unchanged_rows_fresh_equal_reads_and_status_changes() {
    let format = FormatVersion::CURRENT;
    let mut tracker = FrameChangeTracker::new();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let countries = TableId::Countries.index();
    let mut input = source(1, 10, Some(7));
    input.metrics.table_ns[countries] = READ_TIME_NOT_RECORDED;
    let wire = encoder.encode(input, true).unwrap();
    let decoded = decoder.advance(&wire).unwrap();
    let first = tracker.observe(decoded, format);
    assert!(first.rows_changed.iter().all(|changed| *changed));
    assert_eq!(decoded.tables[countries].sampled_at(), Some(10));
    assert!(first.table_read[countries]);
    let identity = decoded.tables[countries].identity();

    let wire = encoder.encode(source(1, 11, Some(7)), false).unwrap();
    assert!(matches!(
        wire.tables[countries],
        TableUpdate::Unchanged { .. }
    ));
    let decoded = decoder.advance(&wire).unwrap();
    let carried = tracker.observe(decoded, format);
    assert!(identity.is(&decoded.tables[countries]));
    assert!(!carried.rows_changed[countries]);
    assert_eq!(decoded.tables[countries].sampled_at(), Some(10));
    assert!(!carried.table_read[countries]);

    let mut input = source(1, 12, Some(7));
    input.metrics.table_ns[countries] = READ_TIME_NOT_RECORDED;
    let wire = encoder.encode(input, false).unwrap();
    assert!(matches!(
        wire.tables[countries],
        TableUpdate::Unchanged { .. }
    ));
    let decoded = decoder.advance(&wire).unwrap();
    let fresh = tracker.observe(decoded, format);
    assert!(identity.is(&decoded.tables[countries]));
    assert!(!fresh.rows_changed[countries]);
    assert_eq!(decoded.tables[countries].sampled_at(), Some(12));
    assert!(fresh.table_read[countries]);
    // Same metric sentinel at hours 10 and 12 still denotes a fresh hour.

    let mut input = source(1, 13, Some(8));
    input.metrics.table_ns[countries] = 1;
    let wire = encoder.encode(input, false).unwrap();
    assert!(matches!(wire.tables[countries], TableUpdate::Delta { .. }));
    let decoded = decoder.advance(&wire).unwrap();
    let changed = tracker.observe(decoded, format);
    assert!(
        !identity.is(&decoded.tables[countries]),
        "decoder mutation must replace the allocation"
    );
    assert!(changed.rows_changed[countries]);
    assert_eq!(decoded.tables[countries].sampled_at(), Some(13));

    let mut input = source(1, 14, Some(8));
    *input.tables[countries] = TableView::Unavailable {
        reason: "synthetic acquisition failure".into(),
    };
    input.metrics.table_ns[countries] = 1;
    let decoded = decoder
        .advance(&encoder.encode(input, false).unwrap())
        .unwrap();
    let unavailable = tracker.observe(decoded, format);
    assert!(unavailable.rows_changed[countries]);
    assert!(unavailable.table_read[countries]);
    assert_eq!(
        decoded.tables[countries].sampled_at(),
        None,
        "failed read is not a complete acquisition"
    );
}

#[test]
fn frame_changes_invalidate_branch_gap_and_actual_decoder_resume_boundaries() {
    let format = FormatVersion::CURRENT;
    let mut tracker = FrameChangeTracker::new();
    let mut observed = source(1, 10, Some(7));
    tracker.observe(&observed, format);
    observed.hour = 11;
    assert!(!tracker.observe(&observed, format).rows_changed[TableId::Countries.index()]);
    // Shared unchanged rows still need all native joins after missing hours.
    observed.hour = 15;
    assert!(
        tracker
            .observe(&observed, format)
            .rows_changed
            .iter()
            .all(|changed| *changed)
    );
    observed.branch = 2;
    observed.hour = 16;
    assert!(
        tracker
            .observe(&observed, format)
            .rows_changed
            .iter()
            .all(|changed| *changed)
    );

    // The saved accumulator genuinely enters the public selected resume API.
    let saved_identity = observed.tables[TableId::Countries.index()].identity();
    let mut decoder = FrameDecoder::resume_for_tables(&TableId::ALL, observed).unwrap();
    tracker.invalidate();
    let mut encoder = FrameEncoder::new();
    encoder.encode(source(2, 16, Some(7)), true).unwrap();
    let mut next = source(2, 17, Some(7));
    next.metrics.table_ns[TableId::Countries.index()] = 0;
    let wire = encoder.encode(next, false).unwrap();
    let decoded = decoder.advance(&wire).unwrap();
    assert!(saved_identity.is(&decoded.tables[TableId::Countries.index()]));
    let resumed = tracker.observe(decoded, format);
    assert!(resumed.rows_changed.iter().all(|changed| *changed));
    assert!(!resumed.table_read[TableId::Countries.index()]);

    // Same-branch restore forces changes even without decoding a new allocation.
    tracker.invalidate();
    let mut saved = source(2, 18, Some(7));
    tracker.observe(&saved, format);
    saved.hour = 19;
    tracker.invalidate();
    assert!(
        tracker
            .observe(&saved, format)
            .rows_changed
            .iter()
            .all(|changed| *changed)
    );
}

#[test]
fn frame_changes_weak_stamps_detect_mutation_without_retaining_rows() {
    let mut tracker = FrameChangeTracker::new();
    let mut observed = source(1, 10, Some(7));
    let index = TableId::Countries.index();
    let identity = observed.tables[index].identity();
    tracker.observe(&observed, FormatVersion::CURRENT);
    observed.hour = 11;
    let TableView::Complete(TableRows::Countries(rows)) = &mut *observed.tables[index] else {
        unreachable!()
    };
    rows[0].political_power_raw = Some(8);
    assert!(
        !identity.is(&observed.tables[index]),
        "mutation must dissociate even a Weak-only stamp"
    );
    assert!(
        tracker
            .observe(&observed, FormatVersion::CURRENT)
            .rows_changed[index]
    );
    let view = std::mem::replace(
        &mut observed.tables[index],
        TableView::Unavailable {
            reason: "moved out in synthetic test".into(),
        }
        .into(),
    );
    assert!(
        view.try_unwrap().is_ok(),
        "tracker must not retain a strong table reference"
    );
}

#[test]
fn frame_changes_preserve_every_table_slot_read_and_slice_read_dependency() {
    let mut observed = source(1, 17, Some(7));
    // A read charged to an anchor in a slot also witnesses its companion tables;
    // CountryIntel is deliberately excluded from that cross-table evidence.
    observed.metrics.table_ns[TableId::CountryIntel.index()] = READ_TIME_NOT_RECORDED;
    let mut tracker = FrameChangeTracker::new();
    let first = tracker.observe(&observed, FormatVersion::CURRENT);
    for table in TableId::ALL {
        assert_eq!(
            first.slot_read[table.index()],
            crate::metadata::slot_read(
                FormatVersion::CURRENT,
                &observed.metrics,
                table,
                observed.hour
            )
        );
        assert_eq!(
            first.table_read[table.index()],
            observed.metrics.table_ns[table.index()] != 0
        );
    }
    observed.hour += 1;
    *observed.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(Vec::new()));
    let changed = tracker.observe(&observed, FormatVersion::CURRENT);
    assert!(changed.rows_changed[TableId::SliceReads.index()]);
    assert!(!changed.rows_changed[TableId::Countries.index()]);
    assert!(
        observed
            .complete::<tables::SliceReads>()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn frame_changes_keep_shared_slot_reads_separate_from_own_reads_and_row_changes() {
    let format = FormatVersion::CURRENT;
    let (anchor, companion, slot) = TableId::ALL
        .into_iter()
        .find_map(|anchor| {
            if anchor == TableId::CountryIntel {
                return None;
            }
            let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence_in(anchor, format)
            else {
                return None;
            };
            TableId::ALL
                .into_iter()
                .find(|companion| {
                    *companion != anchor
                        && *companion != TableId::CountryIntel
                        && replay::cadence::cadence_in(*companion, format)
                            == replay::cadence::Cadence::Daily(slot)
                })
                .map(|companion| (anchor, companion, slot))
        })
        .expect("the recorded cadence has shared daily slots");
    let mut observed = source(
        1,
        (slot as i32 + 1) % replay::cadence::STAGGER_PERIOD,
        Some(7),
    );
    observed.metrics.table_ns[TableId::CountryIntel.index()] = READ_TIME_NOT_RECORDED;
    let mut tracker = FrameChangeTracker::new();
    let first = tracker.observe(&observed, format);
    assert!(
        !first.slot_read[companion.index()],
        "CountryIntel does not witness companions"
    );
    observed.hour += 1;
    observed.metrics.table_ns[anchor.index()] = READ_TIME_NOT_RECORDED;
    let next = tracker.observe(&observed, format);
    assert!(!next.rows_changed[companion.index()]);
    assert!(!next.table_read[companion.index()]);
    assert!(
        next.slot_read[companion.index()],
        "the slot's own anchor witnesses this hour"
    );
}
