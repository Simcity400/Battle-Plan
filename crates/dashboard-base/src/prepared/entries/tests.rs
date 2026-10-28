use super::*;
use crate::prepared::series::tests::Fixture;

/// Counts the pages a flush writes.
struct Counted<'a> {
    writer: &'a mut store::Writer,
    writes: usize,
}

impl Pages for Counted<'_> {
    fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
        self.writes += 1;
        Pages::write_at(self.writer, section, range, data)
    }
    fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64> {
        self.writes += 1;
        Pages::write(self.writer, section, data)
    }
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        Pages::read_at(self.writer, section, range)
    }
}

/// A deterministic sequence of pseudo-random numbers.
struct Numbers(u64);

impl Numbers {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % below
    }
}

fn key(number: u64) -> Vec<u8> {
    format!("tag{:03}\u{0}item{:06}", number % 300, number).into_bytes()
}

fn flush(fixture: &Fixture, entries: &mut Entries) -> usize {
    let mut writer = fixture.writer();
    let mut counted = Counted {
        writer: &mut writer,
        writes: 0,
    };
    entries.flush(&mut counted, "catalogue").unwrap();
    let writes = counted.writes;
    writer.commit().unwrap();
    writes
}

#[test]
fn flushed_entries_read_back_as_their_model_across_splits_updates_and_removals() {
    let fixture = Fixture::new();
    let mut model = BTreeMap::<Vec<u8>, Vec<u8>>::new();
    let mut numbers = Numbers(7);
    for round in 0..12_u64 {
        let mut entries = Entries::default();
        for _ in 0..2_500 {
            let number = numbers.next(40_000);
            if round > 2 && numbers.next(5) == 0 {
                entries.remove(key(number));
                model.remove(&key(number));
            } else {
                let value = vec![b'a' + (number % 26) as u8; 20 + (number % 200) as usize];
                entries.put_bytes(key(number), value.clone());
                model.insert(key(number), value);
            }
        }
        flush(&fixture, &mut entries);
        assert!(entries.is_empty());
        let mut reader = read(&fixture.store, &fixture.key, "catalogue")
            .unwrap()
            .unwrap();
        assert_eq!(
            reader.range(&[], None).unwrap(),
            model
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>(),
            "round {round}"
        );
        for number in (0..40_000).step_by(997) {
            assert_eq!(
                reader.get(&key(number)).unwrap().as_ref(),
                model.get(&key(number)),
                "round {round} key {number}"
            );
        }
        let tag = b"tag042\0";
        assert_eq!(
            reader.prefix(tag).unwrap(),
            model
                .range(tag.to_vec()..)
                .take_while(|(key, _)| key.starts_with(tag))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>()
        );
    }
    // The catalogue holds several megabytes in many pages; the root is a branch.
    let mut reader = read(&fixture.store, &fixture.key, "catalogue")
        .unwrap()
        .unwrap();
    assert!(matches!(reader.node(0).unwrap(), Some(Node::Branch(_))));
}

#[test]
fn a_flush_rewrites_only_the_pages_above_its_changed_entries() {
    let fixture = Fixture::new();
    let mut entries = Entries::default();
    for number in 0..30_000 {
        entries.put_bytes(key(number), vec![b'x'; 120]);
    }
    let first = flush(&fixture, &mut entries);
    assert!(first > 50, "{first} pages for the first flush");
    entries.put_bytes(key(12_345), vec![b'y'; 120]);
    let one = flush(&fixture, &mut entries);
    assert!(one <= 3, "{one} pages for one changed entry");
    // An unchanged value rewrites nothing.
    entries.put_bytes(key(12_345), vec![b'y'; 120]);
    assert_eq!(flush(&fixture, &mut entries), 0);
    let mut reader = read(&fixture.store, &fixture.key, "catalogue")
        .unwrap()
        .unwrap();
    assert_eq!(reader.get(&key(12_345)).unwrap(), Some(vec![b'y'; 120]));
    assert_eq!(reader.get(&key(12_346)).unwrap(), Some(vec![b'x'; 120]));
    assert_eq!(reader.get(b"absent").unwrap(), None);
}

#[test]
fn values_larger_than_a_page_keep_their_own_leaves() {
    let fixture = Fixture::new();
    let mut entries = Entries::default();
    let large = |seed: u8| {
        (0..200_000)
            .map(|at| seed.wrapping_add(at as u8))
            .collect::<Vec<u8>>()
    };
    for seed in 0..5 {
        entries.put_bytes(vec![b'k', seed], large(seed));
    }
    entries.put(b"json".to_vec(), &vec![1, 2, 3]).unwrap();
    flush(&fixture, &mut entries);
    let mut written = fixture.writer();
    let mut reader = read_written(&mut written, "catalogue");
    for seed in 0..5 {
        assert_eq!(reader.get(&[b'k', seed]).unwrap(), Some(large(seed)));
    }
    assert_eq!(
        reader.get_json::<Vec<i32>>(b"json").unwrap(),
        Some(vec![1, 2, 3])
    );
    assert_eq!(reader.range(b"k", Some(b"k\x02")).unwrap().len(), 2);
}

#[test]
fn an_unpublished_catalogue_reads_as_empty() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut reader = read_written(&mut writer, "catalogue");
    assert_eq!(reader.get(b"any").unwrap(), None);
    assert!(reader.range(&[], None).unwrap().is_empty());
    assert_eq!(successor(b"a\xff\xff"), Some(b"b".to_vec()));
    assert_eq!(successor(b"\xff"), None);
}

/// A read after whose first `pages` pages `between`, a flush, publishes.
struct Interrupted<'a, F> {
    published: Published<'a>,
    pages: usize,
    between: Option<F>,
    renewed: usize,
}

impl<F: FnOnce()> PageSource for Interrupted<'_, F> {
    fn page(&mut self, range: u64) -> io::Result<Option<Vec<u8>>> {
        let page = self.published.page(range);
        self.pages = self.pages.saturating_sub(1);
        if self.pages == 0
            && let Some(between) = self.between.take()
        {
            between();
        }
        page
    }

    fn renew(&mut self) -> io::Result<bool> {
        let renewed = self.published.renew()?;
        self.renewed += usize::from(renewed);
        Ok(renewed)
    }
}

/// Two published flushes whose root and second leaf share the second's pack
/// while the first leaf stays in the first's, and the model they hold.
fn two_publications(fixture: &Fixture, numbers: &mut Numbers) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut model = BTreeMap::new();
    let mut entries = Entries::default();
    // 64 entries fill the first leaf and the last 6 make a second.
    for number in 0..70 {
        let bytes = value(numbers);
        entries.put_bytes(name(number), bytes.clone());
        model.insert(name(number), bytes);
    }
    flush(fixture, &mut entries);
    let bytes = value(numbers);
    entries.put_bytes(name(69), bytes.clone());
    model.insert(name(69), bytes);
    flush(fixture, &mut entries);
    assert_eq!(children(fixture), 2);
    model
}

fn value(numbers: &mut Numbers) -> Vec<u8> {
    (0..1_000).map(|_| numbers.next(256) as u8).collect()
}

fn name(number: u32) -> Vec<u8> {
    format!("k{number:06}").into_bytes()
}

fn children(fixture: &Fixture) -> usize {
    let mut reader = read(&fixture.store, &fixture.key, "catalogue")
        .unwrap()
        .unwrap();
    match reader.node(0).unwrap() {
        Some(Node::Branch(children)) => children.len(),
        _ => 0,
    }
}

/// Forty entries inside the first leaf, which split it, and `changed`'s new value.
fn splitting(
    numbers: &mut Numbers,
    model: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    changed: Option<u32>,
) -> Entries {
    let mut split = Entries::default();
    for number in 0..40 {
        let key = format!("k000010-{number:02}").into_bytes();
        let bytes = value(numbers);
        split.put_bytes(key.clone(), bytes.clone());
        model.insert(key, bytes);
    }
    if let Some(number) = changed {
        let bytes = value(numbers);
        split.put_bytes(name(number), bytes.clone());
        model.insert(name(number), bytes);
    }
    split
}

#[test]
fn a_read_overtaken_by_a_flush_splitting_its_leaf_reads_the_newer_publication_whole() {
    let fixture = Fixture::new();
    let mut numbers = Numbers(11);
    let mut model = two_publications(&fixture, &mut numbers);
    let mut split = splitting(&mut numbers, &mut model, None);
    let mut reader = Reader {
        source: Interrupted {
            published: read(&fixture.store, &fixture.key, "catalogue")
                .unwrap()
                .unwrap()
                .source,
            pages: 1,
            between: Some(|| {
                flush(&fixture, &mut split);
                // A writer's start removes the packs no publication lists.
                drop(fixture.writer());
            }),
            renewed: 0,
        },
    };
    let cancelled = AtomicBool::new(false);
    assert_eq!(
        reader
            .whole(&cancelled, |reader| reader.range(&[], None))
            .unwrap(),
        model.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(reader.source.renewed, 1);
    assert_eq!(children(&fixture), 3);
}

#[test]
fn a_composed_read_overtaken_after_an_earlier_read_reads_everything_again() {
    let fixture = Fixture::new();
    let mut numbers = Numbers(13);
    let mut model = two_publications(&fixture, &mut numbers);
    // The flush also changes the entry the composition reads first.
    let mut split = splitting(&mut numbers, &mut model, Some(66));
    let mut reader = Reader {
        source: Interrupted {
            published: read(&fixture.store, &fixture.key, "catalogue")
                .unwrap()
                .unwrap()
                .source,
            // The root and the second leaf: the first read completes.
            pages: 2,
            between: Some(|| {
                flush(&fixture, &mut split);
                drop(fixture.writer());
            }),
            renewed: 0,
        },
    };
    let cancelled = AtomicBool::new(false);
    let mut runs = 0;
    let (first, all) = reader
        .whole(&cancelled, |reader| {
            runs += 1;
            Ok((reader.get(&name(66))?, reader.range(&[], None)?))
        })
        .unwrap();
    assert_eq!(runs, 2);
    assert_eq!(reader.source.renewed, 1);
    assert_eq!(first.as_ref(), model.get(&name(66)));
    assert_eq!(all, model.into_iter().collect::<Vec<_>>());
}

#[test]
fn a_cancelled_composition_reads_nothing() {
    let fixture = Fixture::new();
    let mut entries = Entries::default();
    entries.put_bytes(b"key".to_vec(), b"value".to_vec());
    flush(&fixture, &mut entries);
    let cancelled = AtomicBool::new(true);
    let mut runs = 0;
    let result = compose(&fixture.store, &fixture.key, "catalogue", &cancelled, |reader| {
        runs += 1;
        reader.get(b"key")
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(runs, 0);
}
