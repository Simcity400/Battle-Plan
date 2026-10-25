use super::*;
use std::sync::Arc;

#[test]
fn diagnostics_validates_the_whole_section_but_retains_one_metrics_value() {
    let (replay, _) = direct_tests::fixture(Layout::Chosen);
    let index = &replay.index[0];
    let block = replay.block_bytes(index).unwrap();
    let directory = Directory::parse(&block, index).unwrap();
    let present = Present::new(replay.format.version(), replay.capture_mode);
    let raw = directory.raw(&block, DIAGNOSTICS).unwrap().unwrap();
    let expected =
        read_diagnostics_prefix(&raw, directory.frames, &present, directory.frames).unwrap();
    let mut cursor = cursor::DiagnosticsCursor::new(
        raw.clone(),
        &present,
        directory.frames,
        &mut Check::disabled(),
    )
    .unwrap();
    for metrics in expected {
        assert_eq!(cursor.next(&mut Check::disabled()).unwrap(), metrics);
    }
    let mut bad = raw;
    bad.push(0);
    assert_eq!(
        cursor::DiagnosticsCursor::new(bad, &present, directory.frames, &mut Check::disabled())
            .err()
            .unwrap()
            .to_string(),
        "trailing bytes"
    );
}

#[test]
fn cursor_keeps_predictor_keys_masks_and_range_model_across_every_hour() {
    for layout in direct_tests::LAYOUTS {
        let (replay, _) = direct_tests::fixture(layout);
        let tables = [
            TableId::Countries,
            TableId::ArmyOrders,
            TableId::LexicalTokens,
            TableId::Clock,
        ];
        let mut anchors = tables.map(|_| anchors::TableAnchors::default());
        for position in 0..replay.index.len() {
            let selected = [true; TABLE_COUNT];
            let credits = Arc::new(chunks::Credits::default());
            let mut block = chunks::BlockChunks::load(
                &replay,
                position,
                &selected,
                true,
                credits.acquire(1).unwrap(),
                &mut Check::disabled(),
            )
            .unwrap();
            let reference = replay.read_block(&replay.index[position]).unwrap();
            for (table, anchors) in tables.into_iter().zip(&mut anchors) {
                anchors.admit_ordinal(replay.ordinals[position]);
                let (mut cursor, update, _) = cursor::TableCursor::begin(
                    &replay,
                    &mut block,
                    table,
                    anchors,
                    &mut Check::disabled(),
                )
                .unwrap();
                assert_eq!(update, reference[0].tables[table.index()]);
                for at in 1..block.hours.len() {
                    cursor
                        .prepare(&replay, &mut block, at, None, &mut Check::disabled())
                        .unwrap();
                    assert_eq!(
                        cursor
                            .next(
                                block.hours[at - 1],
                                block.entered(table, at),
                                &mut Check::disabled()
                            )
                            .unwrap(),
                        reference[at].tables[table.index()],
                        "{layout:?} {table:?} hour {}",
                        block.hours[at]
                    );
                }
                cursor.finish().unwrap();
                assert_eq!(anchors.bootstrap_reads.sections, 0);
            }
        }
    }
}

#[test]
fn incremental_inflation_checks_cancellation_and_exact_extent() {
    let bytes = vec![7; 200_000];
    let compressed = compress(&bytes).unwrap();
    let mut calls = 0;
    let mut cancel = || {
        calls += 1;
        if calls == 3 {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Ok(())
        }
    };
    let error =
        chunks::inflate(&compressed, bytes.len(), true, &mut Check::new(&mut cancel)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    for keep in [false, true] {
        let error = chunks::inflate(&compressed, bytes.len() - 1, keep, &mut Check::disabled())
            .unwrap_err();
        assert_eq!(error.to_string(), "grouped section raw length mismatch");
        let mut joined = compressed.clone();
        joined.extend_from_slice(&compressed);
        let error =
            chunks::inflate(&joined, bytes.len() * 2, keep, &mut Check::disabled()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "grouped section contains trailing or concatenated zstd frames"
        );
    }
}
