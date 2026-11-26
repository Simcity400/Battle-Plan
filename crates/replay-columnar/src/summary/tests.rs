use super::*;
use crate::{
    manifest::{Manifest, Section},
    section::{Op, Run, TableHour, write},
    state::{Kind, Values},
    walk::State,
};

const KINDS: [Kind; 2] = [Kind::I64, Kind::OptionalI32];

fn columns() -> Shape {
    Shape::Values(vec![
        Column {
            source: Source::Cell(0),
            aggregates: aggregate::ALL,
            rank: 0,
        },
        Column {
            source: Source::Cell(1),
            aggregates: aggregate::FALLS,
            rank: 0,
        },
    ])
}

struct Hour {
    rows: Vec<(u128, i64, Option<i32>)>,
    status: Status,
}

fn hours(count: i32) -> Vec<Hour> {
    (0..count)
        .map(|at| {
            let mut rows = vec![(1, i64::from(at / 7) * 10 - 30, if at % 11 < 5 { None } else { Some(at % 3) })];
            if at % 40 < 25 {
                rows.push((5, 100, Some(1)));
            }
            if at > 30 {
                rows.push((9, -i64::from(at % 13), Some(-1)));
            }
            let status = if (60..64).contains(&at) { Status::NotRecorded } else { Status::Complete };
            Hour { rows, status }
        })
        .collect()
}

fn apply(state: &mut State, at: i32, hour: &Hour) {
    let held = state.table(0).unwrap();
    let live: Vec<(u32, u128)> = held.live().ones().map(|slot| (slot, held.key(slot).unwrap())).collect();
    let deleted: Vec<u32> = if hour.status == Status::Complete {
        live.iter().filter(|(_, key)| !hour.rows.iter().any(|row| row.0 == *key)).map(|(slot, _)| *slot).collect()
    } else {
        Vec::new()
    };
    let mut changed = Vec::new();
    let mut set_values = Vec::new();
    let mut set_options = Vec::new();
    let mut inserted = Vec::new();
    let mut keys = Vec::new();
    let mut insert_values = Vec::new();
    let mut insert_options = Vec::new();
    let mut next = held.slots() as u32;
    let mut free: Vec<u32> = deleted.clone();
    free.sort_unstable();
    if hour.status == Status::Complete {
        for &(key, value, option) in &hour.rows {
            match live.iter().find(|(_, held)| *held == key) {
                Some(&(slot, _)) => {
                    changed.push((slot, value, option));
                }
                None => {
                    let slot = if free.is_empty() {
                        next += 1;
                        next - 1
                    } else {
                        free.remove(0)
                    };
                    inserted.push((slot, key, value, option));
                }
            }
        }
    }
    changed.sort_unstable_by_key(|row| row.0);
    inserted.sort_unstable_by_key(|row| row.0);
    let changed_slots: Vec<u32> = changed.iter().map(|row| row.0).collect();
    for row in &changed {
        set_values.push(row.1);
        set_options.push(row.2);
    }
    let inserted_slots: Vec<u32> = inserted.iter().map(|row| row.0).collect();
    for row in &inserted {
        keys.push(row.1);
        insert_values.push(row.2);
        insert_options.push(row.3);
    }
    let presence = |options: &[Option<i32>]| {
        let mut words = vec![0u64; options.len().div_ceil(64)];
        for (at, option) in options.iter().enumerate() {
            if option.is_some() {
                words[at / 64] |= 1 << (at % 64);
            }
        }
        words
    };
    let set_present = presence(&set_options);
    let set_held: Vec<i32> = set_options.iter().flatten().copied().collect();
    let insert_present = presence(&insert_options);
    let insert_held: Vec<i32> = insert_options.iter().flatten().copied().collect();
    let positions: Vec<u32> = (0..changed_slots.len() as u32).collect();
    let runs = if changed_slots.is_empty() {
        Vec::new()
    } else {
        vec![
            Run {
                column: 0,
                op: Op::Set,
                positions: &positions,
                values: Some(Values::I64(&set_values)),
                except: &[],
                patches: None,
            },
            Run {
                column: 1,
                op: Op::Set,
                positions: &positions,
                values: Some(Values::OptionalI32 {
                    present: &set_present,
                    values: &set_held,
                }),
                except: &[],
                patches: None,
            },
        ]
    };
    let inserts = [
        Values::I64(&insert_values),
        Values::OptionalI32 {
            present: &insert_present,
            values: &insert_held,
        },
    ];
    let mut deleted = deleted;
    deleted.sort_unstable();
    let bytes = write(
        &KINDS,
        &TableHour {
            read: None,
            changed: &changed_slots,
            deleted: &deleted,
            inserted: &inserted_slots,
            keys: &keys,
            inserts: if inserted_slots.is_empty() { &[] } else { &inserts },
            runs: &runs,
            unrecorded: &[],
            touched: &[],
        },
    )
    .unwrap();
    let manifest = Manifest {
        branch: 1,
        hour: at,
        flags: 0,
        total_ns: 0,
        pool_bytes: 0,
        dropped_frames: 0,
        appends: vec![],
        statuses: vec![hour.status],
        read: vec![u64::from(hour.status == Status::Complete)],
        samples: vec![],
        sections: vec![Section {
            table: 0,
            bytes: bytes.len() as u64,
            columns: vec![0b11],
            ops: crate::manifest::INSERTS | crate::manifest::DELETES,
        }],
    };
    state.apply(&manifest, |_| Ok(&bytes)).unwrap();
}

fn expected(hours: &[Hour], first: i32, key: u128, column: usize, level: u8) -> HashMap<i64, Fold> {
    let mut out: HashMap<i64, Fold> = HashMap::new();
    let mut before: Option<i64> = None;
    for (at, hour) in hours.iter().enumerate() {
        let game = i64::from(first) + at as i64;
        let value = (hour.status == Status::Complete)
            .then(|| hour.rows.iter().find(|row| row.0 == key))
            .flatten()
            .and_then(|row| if column == 0 { Some(row.1) } else { row.2.map(i64::from) });
        let bucket = bucket_of(game, level);
        if let Some(value) = value {
            let fold = out.entry(bucket).or_insert(Fold {
                first: value,
                min: value,
                max: value,
                ..Fold::default()
            });
            fold.hours += 1;
            fold.last = value;
            fold.min = fold.min.min(value);
            fold.max = fold.max.max(value);
            fold.sum += i128::from(value);
            fold.abs += u128::from(value.unsigned_abs());
            if let Some(before) = before
                && before != value
            {
                fold.changes += 1;
                fold.falls += u64::from(value < before);
            }
        }
        before = value;
    }
    out
}

fn pieces(hours: &[Hour], first: i32, block: usize) -> Vec<Vec<u8>> {
    pieces_of(&columns(), hours, first, block)
}

fn pieces_of(shape: &Shape, hours: &[Hour], first: i32, block: usize) -> Vec<Vec<u8>> {
    let mut state = State::new(vec![KINDS.to_vec()]).unwrap();
    let mut out = Vec::new();
    for (start, chunk) in hours.chunks(block).enumerate() {
        let first_hour = first + (start * block) as i32;
        let mut builder = Builder::open(shape, state.table(0).unwrap(), state.status(0).unwrap(), state.truncated_view(0), first_hour);
        for (at, hour) in chunk.iter().enumerate() {
            let game = first_hour + at as i32;
            apply(&mut state, game, hour);
            builder.hour(game, state.table(0).unwrap(), &hour_log(state.changes(0).unwrap()), state.status(0).unwrap(), None);
        }
        if let Some(piece) = builder.finish(first_hour + chunk.len() as i32).unwrap() {
            out.push(piece.assemble(3).unwrap());
        }
    }
    out
}

fn check(piece: &[u8], hours: &[Hour], first: i32) {
    let read = Piece::read(piece).unwrap();
    let level = read.directory.level;
    for column in 0..2 {
        let frame = read.column(column).unwrap();
        for (at, &key) in read.keys.iter().enumerate() {
            for (bucket, fold) in frame.buckets(&read, at).unwrap().into_iter().enumerate() {
                let index = read.directory.first_bucket + bucket as i64;
                let piece_first = read.directory.first_hour.max(index * bucket_hours(level));
                let piece_end = (read.directory.first_hour + i64::from(read.directory.hours)).min((index + 1) * bucket_hours(level));
                let inside = expected(
                    &hours[(piece_first - i64::from(first)) as usize..(piece_end - i64::from(first)) as usize],
                    piece_first as i32,
                    key,
                    column,
                    level,
                );
                let want = inside.get(&index).filter(|fold| fold.hours > 0);
                let got = fold.filter(|fold| fold.hours > 0);
                match (want, got) {
                    (None, None) => {}
                    (Some(want), Some(got)) => {
                        let fields = |fold: &Fold| (fold.hours, fold.first, fold.last, fold.min, fold.max);
                        assert_eq!(fields(&got), fields(want), "key {key} column {column} bucket {index}");
                        if column == 0 {
                            assert_eq!((got.sum, got.abs), (want.sum, want.abs), "key {key} bucket {index}");
                        }
                    }
                    (want, got) => panic!("key {key} column {column} bucket {index}: {want:?} against {got:?}"),
                }
            }
        }
    }
}

#[test]
fn a_blocks_piece_holds_what_its_hours_held() {
    let first = 1_000_003;
    let hours = hours(1500);
    let built = pieces(&hours, first, 720);
    assert_eq!(built.len(), 3);
    for piece in &built {
        check(piece, &hours, first);
        let again = recompress(piece, 19).unwrap();
        check(&again, &hours, first);
        let (before, after) = (Piece::read(piece).unwrap(), Piece::read(&again).unwrap());
        assert_eq!(after.keys, before.keys);
        for at in 0..before.directory.frames.len() {
            assert_eq!(after.frame(at).unwrap(), before.frame(at).unwrap(), "frame {at}");
        }
        assert!(recompress(&[piece.as_slice(), b"tail"].concat(), 19).is_err(), "bytes past its frames are refused");
    }
}

#[test]
fn pieces_fold_into_the_level_above() {
    let first = 1_000_003;
    let hours = hours(1500);
    let built = pieces(&hours, first, 720);
    let refs: Vec<&[u8]> = built.iter().map(Vec::as_slice).collect();
    let folded = fold(&refs, i64::from(first), 1500).unwrap().unwrap().assemble(3).unwrap();
    check(&folded, &hours, first);
    let read = Piece::read(&folded).unwrap();
    assert_eq!(read.directory.level, 1);
    let at = read.key_at(1).unwrap();
    let frame = read.column(0).unwrap();
    let wanted = expected(&hours, first, 1, 0, 1);
    for (bucket, fold) in frame.buckets(&read, at).unwrap().into_iter().enumerate() {
        let index = read.directory.first_bucket + bucket as i64;
        if let (Some(got), Some(want)) = (fold, wanted.get(&index)) {
            assert_eq!((got.falls, got.changes), (want.falls, want.changes), "bucket {index}");
        }
    }
}

fn ranked_hours(count: i32) -> Vec<Hour> {
    (0..count)
        .map(|at| {
            let rows = (1..=12_i32)
                .filter(|&key| (at / (37 + key * 11)) % 4 != 0)
                .map(|key| (key as u128, i64::from((at / 24 + key * 7) % 9) * 10 - i64::from(at % 5 == 0 && key == 3), Some(key)))
                .collect();
            let status = if (2000..2100).contains(&at) { Status::NotRecorded } else { Status::Complete };
            Hour { rows, status }
        })
        .collect()
}

fn check_ranks(piece: &[u8], hours: &[Hour], first: i32) -> usize {
    let read = Piece::read(piece).unwrap();
    let level = read.directory.level;
    let ranks = read.ranks(0).unwrap().unwrap();
    assert_eq!(read.ranks(1).unwrap(), None);
    let mut nonempty = 0;
    for bucket in 0..read.directory.buckets {
        let index = read.directory.first_bucket + i64::from(bucket);
        let piece_first = read.directory.first_hour.max(index * bucket_hours(level));
        let piece_end = (read.directory.first_hour + i64::from(read.directory.hours)).min((index + 1) * bucket_hours(level));
        let inside = &hours[(piece_first - i64::from(first)) as usize..(piece_end - i64::from(first)) as usize];
        let mut held: Vec<(i64, u128)> = (1..=12)
            .filter_map(|key| expected(inside, piece_first as i32, key, 0, level).get(&index).filter(|fold| fold.hours > 0).map(|fold| (fold.last, key)))
            .collect();
        held.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let want: Vec<u128> = held.into_iter().take(3).map(|(_, key)| key).collect();
        let got: Vec<u128> = ranks.bucket(bucket).iter().map(|&at| read.keys[at as usize]).collect();
        assert_eq!(got, want, "level {level} bucket {index}");
        nonempty += usize::from(!got.is_empty());
    }
    nonempty
}

#[test]
fn ranks_are_each_buckets_keys_greatest_in_their_last_value() {
    let shape = Shape::Values(vec![
        Column { source: Source::Cell(0), aggregates: aggregate::ALL, rank: 3 },
        Column { source: Source::Cell(1), aggregates: aggregate::FALLS, rank: 0 },
    ]);
    let first = 1_000_003;
    let hours = ranked_hours(5000);
    let zero = pieces_of(&shape, &hours, first, 250);
    assert_eq!(zero.len(), 20);
    let mut nonempty = 0;
    for piece in &zero {
        check(piece, &hours, first);
        nonempty += check_ranks(piece, &hours, first);
    }
    let mut ones = Vec::new();
    for (group, chunk) in zero.chunks(5).enumerate() {
        let refs: Vec<&[u8]> = chunk.iter().map(Vec::as_slice).collect();
        let one = fold(&refs, i64::from(first) + group as i64 * 1250, 1250).unwrap().unwrap().assemble(3).unwrap();
        nonempty += check_ranks(&one, &hours, first);
        ones.push(one);
    }
    let refs: Vec<&[u8]> = ones.iter().map(Vec::as_slice).collect();
    let two = fold(&refs, i64::from(first), 5000).unwrap().unwrap().assemble(3).unwrap();
    assert_eq!(Piece::read(&two).unwrap().directory.level, 2);
    nonempty += check_ranks(&two, &hours, first);
    assert!(nonempty > 700, "{nonempty} buckets ranked a key");
}

#[test]
fn a_fold_of_merged_keys_is_the_fold_of_gathered_keys() {
    let shape = Shape::Values(vec![
        Column { source: Source::Cell(0), aggregates: aggregate::ALL, rank: 3 },
        Column { source: Source::Cell(1), aggregates: aggregate::FALLS, rank: 0 },
    ]);
    let first = 1_000_003;
    let mut hours = ranked_hours(5000);
    for (at, hour) in hours.iter_mut().enumerate() {
        hour.rows.retain(|&(key, _, _)| (at as u128 / 250 + key) % 3 != 0);
    }
    let gathered = |pieces: &[Vec<u8>], first_hour: i64, hours: u32| -> Vec<u8> {
        let refs: Vec<&[u8]> = pieces.iter().map(Vec::as_slice).collect();
        let read: Vec<Piece<'_>> = refs.iter().map(|bytes| Piece::read(bytes).unwrap()).collect();
        let mut keys: Vec<u128> = read.iter().flat_map(|piece| piece.keys.iter().copied()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(merged(&read).unwrap(), keys);
        let old = fold_keys(&read, &keys, first_hour, hours).unwrap().unwrap().assemble(3).unwrap();
        let new = fold(&refs, first_hour, hours).unwrap().unwrap().assemble(3).unwrap();
        assert_eq!(new, old, "the folded piece from {first_hour} differs");
        new
    };
    let zero = pieces_of(&shape, &hours, first, 250);
    assert_eq!(zero.len(), 20);
    assert!(zero.iter().all(|piece| Piece::read(piece).unwrap().keys.len() == 8), "a piece holds other than two thirds of the keys");
    let ones: Vec<Vec<u8>> = zero.chunks(5).enumerate().map(|(group, chunk)| gathered(chunk, i64::from(first) + group as i64 * 1250, 1250)).collect();
    assert!(ones.iter().all(|piece| Piece::read(piece).unwrap().keys.len() == 12));
    gathered(&ones, i64::from(first), 5000);
}

#[test]
fn a_log_counts_its_inserts_per_bucket() {
    let shape = Shape::Log(vec![Facet::Column(1)]);
    let mut state = State::new(vec![KINDS.to_vec()]).unwrap();
    let first = 1_000_003;
    let hours = hours(100);
    let mut builder = Builder::open(&shape, state.table(0).unwrap(), Status::NotRecorded, None, first);
    for (at, hour) in hours.iter().enumerate() {
        apply(&mut state, first + at as i32, hour);
        builder.hour(first + at as i32, state.table(0).unwrap(), &hour_log(state.changes(0).unwrap()), state.status(0).unwrap(), None);
    }
    let piece = builder.finish(first + 100).unwrap().unwrap().assemble(3).unwrap();
    let read = Piece::read(&piece).unwrap();
    let frame = read.column(0).unwrap();
    let total: u64 = (0..read.keys.len()).map(|at| frame.counts(&read, at).unwrap().iter().sum::<u64>()).sum();
    assert_eq!(total, 5);
}

fn hour_log(changes: &crate::section::Changes) -> crate::store::TableHour {
    let mut log = crate::store::TableHour::default();
    log.inserted = changes.inserted.clone();
    log.deleted = changes.deleted.iter().copied().zip(changes.deleted_keys.iter().copied()).collect();
    for (slots, masks) in [(&changes.changed, &changes.masks), (&changes.fill_slots, &changes.fill_masks)] {
        for (&slot, &mask) in slots.iter().zip(masks) {
            let mut rest = mask;
            while rest != 0 {
                log.written.push((slot, rest.trailing_zeros() as u16));
                rest &= rest - 1;
            }
        }
    }
    log.written.sort_unstable();
    log.written.dedup();
    log
}

struct Grid {
    width: usize,
    values: Vec<i64>,
}

impl Grid {
    fn keys(&self) -> u32 {
        (self.values.len() / self.width) as u32
    }
}

impl Cells for Grid {
    fn number(&self, column: usize, slot: u32) -> Option<i64> {
        self.values.get(slot as usize * self.width + column).copied()
    }

    fn key(&self, slot: u32) -> Option<u128> {
        (slot < self.keys()).then(|| u128::from(slot) * 7 + 1)
    }

    fn slot(&self, key: u128) -> Option<u32> {
        let slot = u32::try_from(key.checked_sub(1)? / 7).ok()?;
        (self.key(slot) == Some(key)).then_some(slot)
    }

    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        (0..self.keys()).for_each(each);
    }
}

fn grid_log(keys: u32, width: usize, at: i32) -> crate::store::TableHour {
    let mut log = crate::store::TableHour::default();
    for slot in (0..keys).filter(|&slot| (slot as i32 + at) % 24 == 0) {
        log.written.extend((0..width as u16).map(|column| (slot, column)));
    }
    log
}

fn grid_step(grid: &mut Grid, log: &crate::store::TableHour) {
    for &(slot, column) in &log.written {
        grid.values[slot as usize * grid.width + usize::from(column)] += i64::from(slot % 13) * 1_000 + i64::from(column) - 3;
    }
}

fn grid_shape(width: usize) -> Shape {
    Shape::Values((0..width).map(|column| Column { source: Source::Cell(column), aggregates: aggregate::FALLS, rank: if column == 0 { 32 } else { 0 } }).collect())
}

#[test]
fn a_grids_piece_holds_each_rows_value_at_every_bucket() {
    const KEYS: u32 = 40;
    const WIDTH: usize = 3;
    let first = 1_000_003;
    let mut grid = Grid { width: WIDTH, values: (0..KEYS as usize * WIDTH).map(|at| at as i64 * 17).collect() };
    let mut builder = Builder::open(&grid_shape(WIDTH), &grid, Status::Complete, None, first);
    let mut history = vec![grid.values.clone()];
    for at in 0..720 {
        let log = grid_log(KEYS, WIDTH, at);
        grid_step(&mut grid, &log);
        builder.hour(first + at, &grid, &log, Status::Complete, None);
        history.push(grid.values.clone());
    }
    let piece = builder.finish(first + 720).unwrap().unwrap().assemble(3).unwrap();
    let read = Piece::read(&piece).unwrap();
    assert_eq!(read.keys.len(), KEYS as usize);
    for column in 0..WIDTH {
        let frame = read.column(column).unwrap();
        for (at, &key) in read.keys.iter().enumerate() {
            let slot = grid.slot(key).unwrap() as usize;
            for (bucket, fold) in frame.buckets(&read, at).unwrap().into_iter().enumerate() {
                let index = read.directory.first_bucket + bucket as i64;
                let hours: Vec<i64> = (0..720).filter(|&hour| bucket_of(i64::from(first) + hour, 0) == index).collect();
                let values: Vec<i64> = hours.iter().map(|&hour| history[hour as usize + 1][slot * WIDTH + column]).collect();
                let fold = fold.unwrap();
                assert_eq!(fold.hours as usize, values.len(), "key {key} bucket {index}");
                assert_eq!((fold.first, fold.last), (values[0], *values.last().unwrap()), "key {key} column {column} bucket {index}");
                assert_eq!((fold.min, fold.max), (*values.iter().min().unwrap(), *values.iter().max().unwrap()));
            }
        }
    }
}

#[cfg(feature = "memory-labels")]
mod labelled {
    use super::*;
    use engine_allocation::labels;

    #[global_allocator]
    static COUNTING: labels::Counting<std::alloc::System> = labels::Counting(std::alloc::System);

    fn measured<T>(name: &'static str, work: impl FnOnce() -> T) -> (T, Vec<labels::Phase>) {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        labels::phases();
        labels::phase(name);
        let out = work();
        (out, labels::phases())
    }

}

#[test]
fn summary_numbers_hold_128_bits_and_refuse_bits_past_them() {
    let mut widest = Vec::new();
    put_u128(&mut widest, u128::MAX);
    assert_eq!(get_u128(&mut Reader::new(&widest)).unwrap(), u128::MAX);
    let over = |last: u8| [vec![0x80; 18], vec![last]].concat();
    assert_eq!(get_u128(&mut Reader::new(&over(3))).unwrap(), 3 << 126);
    assert!(get_u128(&mut Reader::new(&over(4))).is_err());
}
