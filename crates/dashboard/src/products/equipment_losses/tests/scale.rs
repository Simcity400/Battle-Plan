//! Scan benchmark, including selected wire-table decoding and reconstruction.

use super::*;
use std::{hint::black_box, time::Instant};

#[test]
#[ignore]
fn equipment_losses_observer_scale_including_decode() {
    const HOURS: i32 = 240;
    let mut worst_ms = 0.0_f64;
    for countries in [1_500_u32, 5_000] {
        // Every synthetic country has a major-sized census, including majors.
        // Change motion/strength on a tenth of it each hour; grouping stays fixed.
        let mut source = observer_frame(countries, 100);
        let divisions = source.complete::<tables::Divisions>().unwrap().len();
        let log_rows = source
            .complete::<tables::CombatLogEquipment>()
            .unwrap()
            .len();
        let mut reads: Vec<_> = (1..=countries)
            .map(|owner| SliceReadRow {
                table: TableId::CombatLogEquipment.index() as u32,
                owner,
                sample_hour: -1,
            })
            .collect();
        let mut encoder = FrameEncoder::new();
        let mut records = vec![
            Record::SessionStart(session()),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: None,
                first_hour: 0,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        for hour in 0..HOURS {
            source.hour = hour;
            for read in &mut reads {
                if hour == 0 || read.owner % 24 == hour as u32 % 24 {
                    read.sample_hour = hour;
                }
            }
            *source.tables[TableId::SliceReads.index()] =
                TableView::Complete(TableRows::SliceReads(reads.clone()));
            let TableView::Complete(TableRows::Divisions(rows)) =
                &mut *source.tables[TableId::Divisions.index()]
            else {
                unreachable!()
            };
            for row in rows.iter_mut().skip(hour as usize % 10).step_by(10) {
                row.province = hour as u32;
                row.hit_points = i64::from(hour) * 100;
                row.organisation = i64::from(hour) * 200;
            }
            records.push(Record::Frame(
                encoder.encode(source.clone(), false).unwrap(),
            ));
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();

        // No fixture construction, encoding or builds inside either interval.
        let start = Instant::now();
        let mut parsed_hours = 0;
        for block in &replay.index {
            let frames = replay
                .read_block_selected(black_box(block), &SELECTED)
                .unwrap();
            parsed_hours += frames.len();
            black_box(frames);
        }
        let parse_ms = start.elapsed().as_secs_f64() * 1_000.0 / f64::from(HOURS);
        assert_eq!(parsed_hours, HOURS as usize);
        let start = Instant::now();
        let scanned = scan(black_box(&replay), 1, "C00007", &AtomicBool::new(false)).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1_000.0 / f64::from(HOURS);
        worst_ms = worst_ms.max(ms);
        assert_eq!(scanned.coverage.len(), 1);
        assert_eq!(scanned.coverage[0].coverage, Coverage::Logged);
        assert_eq!(scanned.events.len(), 20);
        black_box(scanned);
        let combat_tables = [TableId::CombatLogEquipment, TableId::SliceReads];
        let start = Instant::now();
        for block in &replay.index {
            let mut decoder = FrameDecoder::for_tables(&combat_tables);
            for frame in replay.read_block_selected(block, &combat_tables).unwrap() {
                black_box(decoder.advance(&frame).unwrap());
            }
        }
        let combat_ms = start.elapsed().as_secs_f64() * 1_000.0 / f64::from(HOURS);
        println!(
            "observer countries={countries}, divisions={divisions}, log_rows={log_rows}, hours={HOURS}, selected_parse_ms_per_hour={parse_ms:.6}, scan_ms_per_hour={ms:.6}, combat_decode_ms_per_hour={combat_ms:.6} (including decoding/decompression/reconstruction/coverage; combat pass includes wire decompression and SliceReads)"
        );
    }
    assert!(
        worst_ms < 5.5,
        "decode-inclusive scan exceeded 5.5 ms per built hour: {worst_ms:.6}"
    );
}
