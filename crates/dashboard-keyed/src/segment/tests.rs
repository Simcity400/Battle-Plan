use super::*;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::Operator as _,
};
use replay_data::{
    rows::{ShipRow, TableRows, TableSchema, VariantDefinitionRow, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};

/// A family reading its ships' strength and the designs' reads.
struct Toy;

/// What the toy reads of an hour: each ship's strength, the hour the designs were last sampled,
/// whether the designs' table is complete, and the ships the hour wrote, or none where it gave
/// them whole.
type Seen = (Vec<(u64, i64)>, Option<i32>, bool, Option<Vec<u128>>);

impl Segmented for Toy {
    const TABLES: &'static [TableId] = &[TableId::Ships, TableId::VariantDefinitions];
    const READ: TableId = TableId::Ships;
    type Read = Seen;

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![TableFields::named(TableId::Ships, &["strength"])?])
    }

    fn changed(changes: &FrameChanges) -> bool {
        Self::TABLES
            .iter()
            .any(|table| changes.rows_changed[table.index()])
            || changes.slot_read[TableId::VariantDefinitions.index()]
    }

    fn reader(_: FormatVersion) -> Box<dyn Reader<Self>> {
        Box::new(Seeing)
    }
}

struct Seeing;

impl Reader<Toy> for Seeing {
    fn read(
        &mut self,
        frame: &ColumnFrame<'_>,
        _: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(Seen, usize)> {
        let strength = replay_data::rows::column::<ShipRow>("strength");
        let ships = match frame.complete(TableId::Ships) {
            Ok(rows) => rows
                .slots()
                .iter()
                .map(|&slot| Ok((rows.key(slot) as u64, rows.value::<i64>(slot, strength)?)))
                .collect::<io::Result<_>>()?,
            Err(_) => Vec::new(),
        };
        let complete = frame.view(TableId::VariantDefinitions).is_complete();
        let touch = touched.of(TableId::Ships);
        let written = (!touch.restated).then(|| touch.written.iter().map(|&(key, _)| key).collect());
        let designs = frame.sampled_at(TableId::VariantDefinitions);
        Ok(((ships, designs, complete, written), 1))
    }
}

/// An hour of the toy's tables: ships at `strength` but where the ships are not recorded, and
/// the designs read when `read`.
fn frame(hour: i32, strength: i64, ships: bool, read: bool) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    *frame.tables[TableId::Ships.index()] = if ships {
        TableView::Complete(TableRows::Ships(vec![
            ShipRow {
                id: 1,
                strength,
                organisation: hour.into(),
                ..Default::default()
            },
            ShipRow {
                id: 2,
                strength: 7,
                ..Default::default()
            },
        ]))
    } else {
        TableView::Unavailable {
            reason: "Synthetic ships refusal".into(),
        }
    };
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 5,
            ..Default::default()
        }]));
    frame.metrics.table_ns[TableId::VariantDefinitions.index()] = u64::from(read);
    frame
}

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        _ => unreachable!("the toy writes complete or unavailable tables"),
    };
    walk.table(T::ID, input).unwrap();
}

/// The hours `frames` give the toy's operators, a fresh one at each of `restarts` (ordinals).
fn operate(frames: &[DecodedFrame], restarts: &[usize]) -> Vec<Hour<Toy>> {
    let format = FormatVersion::CURRENT;
    let mut walk = Synthetic::new(Toy::TABLES, 1, frames[0].hour).unwrap();
    let mut world = None;
    let needs = needs::<Toy>().unwrap();
    let mut operator = Operator::<Toy>::new(format).unwrap();
    let mut hours = Vec::new();
    for (ordinal, frame) in frames.iter().enumerate() {
        let opens = ordinal == 0 || restarts.contains(&ordinal);
        if opens {
            operator = Operator::new(format).unwrap();
        }
        write::<tables::Ships>(&mut walk, frame);
        write::<tables::VariantDefinitions>(&mut walk, frame);
        walk.end(|state, manifest, decoded, starts| {
            let world = World::at(
                &mut world,
                starts,
                &needs,
                (format, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            operator
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts: opens,
                        opens,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
    }
    hours
}

/// The hours from 100: a ship's strength moving, the ships refused for two hours and back, a
/// field the toy does not watch moving every hour, and the designs read off their slot at 110
/// and on it at 118, nothing else changing there.
fn scenario() -> Vec<DecodedFrame> {
    (100..=121)
        .map(|hour| {
            let strength = match hour {
                ..103 => 10,
                103..112 => 20,
                _ => 30,
            };
            frame(hour, strength, !(105..107).contains(&hour), hour == 110)
        })
        .collect()
}

/// A segment's operator reads an hour where a field its family reads changed, a table's status
/// moved or a table its family dates was read, from the world's frame of the family's tables,
/// each with its sample as the recording holds it, and hands on the rows the hour wrote; every
/// other hour is quiet.
#[test]
fn an_operator_reads_the_hours_its_family_observes_from_its_own_frame() {
    let frames = scenario();
    for restarts in [&[][..], &[3, 9], &[5, 6, 7, 18]] {
        let hours = operate(&frames, restarts);
        assert_eq!(hours.len(), frames.len());
        let mut read = Vec::new();
        for (hour, frame) in hours.iter().zip(&frames) {
            assert_eq!(hour.hour, frame.hour);
            assert!(hour.dated, "the ships are read every hour");
            let Some((ships, sampled, complete, written)) = &hour.read else {
                continue;
            };
            read.push(hour.hour);
            let expected: Vec<(u64, i64)> = frame
                .complete::<tables::Ships>()
                .map(|rows| rows.iter().map(|row| (row.id, row.strength)).collect())
                .unwrap_or_default();
            assert_eq!(ships, &expected, "{restarts:?} at {}", hour.hour);
            assert!(complete);
            assert_eq!(*sampled, (hour.hour >= 110).then_some(110), "at {}", hour.hour);
            // The ships give their rows whole where they are refused or come back, and where
            // a segment starts; else only the first ship's strength moves.
            let restated = [100, 105, 107].contains(&hour.hour)
                || restarts.iter().any(|&ordinal| 100 + ordinal as i32 == hour.hour);
            let moved = [103, 112].contains(&hour.hour);
            let expected = (!restated).then(|| if moved { vec![1] } else { Vec::new() });
            assert_eq!(written, &expected, "{restarts:?} at {}", hour.hour);
        }
        let mut expected = vec![100, 103, 105, 107, 110, 112, 118];
        expected.extend(restarts.iter().map(|&ordinal| 100 + ordinal as i32));
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(read, expected, "{restarts:?}");
    }
}
