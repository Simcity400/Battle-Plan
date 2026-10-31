use super::*;
use crate::prepared::series::{self, tests::Fixture};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The catalogue text a query reconstructs from the published ranges.
fn published(fixture: &Fixture, section: &str) -> Vec<u8> {
    let session = fixture.store.session(&fixture.key).unwrap().unwrap();
    read(|range| session.read(section, range)).unwrap().unwrap()
}

/// Ranges before the first empty one: the base and its patches.
fn live(fixture: &Fixture, section: &str) -> usize {
    let session = fixture.store.session(&fixture.key).unwrap().unwrap();
    (0..)
        .map_while(|range| session.read(section, range).unwrap())
        .take_while(|range| !range.is_empty())
        .count()
}

/// First-seen keys whose names spread over the whole sorted order, so new
/// keys land between earlier ones, each with some per-key metadata.
fn add(keys: &mut BTreeMap<String, u64>, metadata: &mut BTreeMap<String, Value>, count: u64) {
    for _ in 0..count {
        let key = keys.len() as u64;
        let name = format!("tag {:04x} / measure {key}", key * 7919 % 65_536);
        keys.insert(name.clone(), key);
        metadata.insert(
            name,
            json!({"first": key, "label": format!("label {key}"), "parts": [key, key + 1]}),
        );
    }
}

#[test]
fn a_flush_writes_only_the_catalogue_keys_it_added() {
    let fixture = Fixture::new();
    let written = |keys: &BTreeMap<String, u64>, metadata: &BTreeMap<String, Value>| {
        let mut writer = fixture.writer();
        series::write_catalogue(&mut writer, "volume", 1, keys, metadata).unwrap();
        let bytes = writer.bytes_written();
        writer.commit().unwrap();
        bytes
    };
    let (mut keys, mut metadata) = (BTreeMap::new(), BTreeMap::new());
    add(&mut keys, &mut metadata, 5_000);
    let whole = written(&keys, &metadata);
    assert!(whole > 20_000, "the whole catalogue is {whole} bytes");
    let mut stamps = series::catalogue_ranges(&fixture.store, &fixture.key, "volume", 1)
        .unwrap()
        .unwrap();
    let mut added = 0;
    for count in [10, 100] {
        add(&mut keys, &mut metadata, count);
        added += count;
        let bytes = written(&keys, &metadata);
        // A flush writes its own keys, and at most rewrites the patch of the
        // keys added since the base; old code wrote the whole catalogue.
        assert!(
            bytes <= 200 * added + 300,
            "{bytes} catalogue bytes for {added} new keys (whole catalogue {whole})"
        );
        let (read_keys, read_metadata) = series::read_catalogue::<BTreeMap<String, Value>>(
            &fixture.store,
            &fixture.key,
            "volume",
            1,
        )
        .unwrap()
        .unwrap();
        assert_eq!((read_keys, read_metadata), (keys.clone(), metadata.clone()));
        assert_eq!(
            published(&fixture, "volume-1-catalogue"),
            serde_json::to_vec(&(&keys, &metadata)).unwrap()
        );
        let now = series::catalogue_ranges(&fixture.store, &fixture.key, "volume", 1)
            .unwrap()
            .unwrap();
        assert_ne!(
            now, stamps,
            "a changed catalogue changes its range identities"
        );
        stamps = now;
    }
    assert_eq!(
        written(&keys, &metadata),
        0,
        "an unchanged catalogue writes nothing"
    );
    assert_eq!(
        series::catalogue_ranges(&fixture.store, &fixture.key, "volume", 1)
            .unwrap()
            .unwrap(),
        stamps
    );
}

/// A small deterministic generator for the edit sequence.
struct Random(u64);
impl Random {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound.max(1)
    }
    fn leaf(&mut self) -> Value {
        match self.below(8) {
            0 => Value::Null,
            1 => json!(self.below(10_000) as f64 / 7.0),
            2 => json!(format!(
                "quoted \"{}\" \\ é \u{1F600} ,]}}",
                self.below(100)
            )),
            3 => json!(self.below(2) == 0),
            4 => json!({"nested": {"a": [self.below(5), {"b": "c"}]}, "list": [], "map": {}}),
            5 => json!([self.below(10), [self.below(10)], {}]),
            6 => json!(-(self.below(1 << 60) as i64)),
            _ => json!(self.below(u64::MAX)),
        }
    }
    fn name(&mut self) -> String {
        format!("member \"{}\" {}", self.below(50), self.below(1_000))
    }
    /// One edit somewhere in `value`: members and elements added, removed,
    /// inserted, truncated or changed, at any depth.
    fn edit(&mut self, value: &mut Value, depth: u32) {
        if depth > 0 && (value.is_object() || value.is_array()) && self.below(12) == 0 {
            *value = self.leaf();
            return;
        }
        match value {
            Value::Object(map) if !map.is_empty() && depth < 5 && self.below(3) > 0 => {
                let index = self.below(map.len() as u64) as usize;
                let key = map.keys().nth(index).unwrap().clone();
                self.edit(map.get_mut(&key).unwrap(), depth + 1);
            }
            Value::Array(items) if !items.is_empty() && depth < 5 && self.below(3) > 0 => {
                let index = self.below(items.len() as u64) as usize;
                self.edit(&mut items[index], depth + 1);
            }
            Value::Object(map) => {
                if !map.is_empty() && self.below(3) == 0 {
                    let index = self.below(map.len() as u64) as usize;
                    let key = map.keys().nth(index).unwrap().clone();
                    map.remove(&key);
                } else {
                    let name = self.name();
                    let leaf = self.leaf();
                    map.insert(name, leaf);
                }
            }
            Value::Array(items) => match self.below(5) {
                0 if !items.is_empty() => {
                    items.remove(self.below(items.len() as u64) as usize);
                }
                1 => {
                    let at = self.below(items.len() as u64 + 1) as usize;
                    let leaf = self.leaf();
                    items.insert(at, leaf);
                }
                2 => items.truncate(self.below(items.len() as u64 + 1) as usize),
                _ => {
                    let leaf = self.leaf();
                    items.push(leaf);
                }
            },
            scalar => *scalar = self.leaf(),
        }
    }
}

#[test]
fn patches_reproduce_every_catalogue_text_through_append_and_stay_few() {
    let mut fixture = Fixture::new();
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut keys = BTreeMap::new();
    let mut metadata = json!({
        "countries": (0..300).map(|tag| (format!("T{tag:03}"), json!({"first": tag, "names": ["a", "b"]}))).collect::<serde_json::Map<_, _>>(),
        "blob": (0..2_000).map(|byte| byte % 256).collect::<Vec<_>>(),
        "list": [],
        "scalar": 1,
    });
    let mut longest = 0;
    for flush in 0..240 {
        for _ in 0..random.below(4) {
            random.edit(&mut metadata, 0);
        }
        if random.below(3) == 0 {
            let name = random.name();
            let next = keys.len() as u64;
            keys.entry(name).or_insert(next);
        }
        let text = serde_json::to_vec(&(&keys, &metadata)).unwrap();
        let previous = fixture.key.clone();
        let mut writer = if flush == 120 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        if flush == 120 {
            // Restoration after growth reads the earlier key's catalogue.
            let saved = series::read_saved_catalogue::<Value>(
                &fixture.store,
                &fixture.key,
                Some(&previous),
                "random",
                1,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                serde_json::to_vec(&saved).unwrap(),
                published_before(&fixture, &previous)
            );
        }
        let before = published_or_empty(&fixture);
        series::write_catalogue(&mut writer, "random", 1, &keys, &metadata).unwrap();
        let bytes = writer.bytes_written();
        writer.commit().unwrap();
        if before == text {
            assert_eq!(bytes, 0, "flush {flush} repeats its text");
        }
        assert_eq!(
            published(&fixture, "random-1-catalogue"),
            text,
            "flush {flush}"
        );
        let (read_keys, read_metadata) =
            series::read_catalogue::<Value>(&fixture.store, &fixture.key, "random", 1)
                .unwrap()
                .unwrap();
        assert_eq!((&read_keys, &read_metadata), (&keys, &metadata));
        longest = longest.max(live(&fixture, "random-1-catalogue"));
    }
    // Each patch is at least about twice the next, and patches never outweigh
    // the base, so a chain stays logarithmic in the catalogue's size.
    assert!(longest <= 12, "a chain reached {longest} ranges");
    assert!(longest >= 3, "the edits were small enough to be patched");
}

/// The text the previous key published, read as a growth restoration does.
fn published_before(fixture: &Fixture, previous: &store::SourceKey) -> Vec<u8> {
    read(|range| {
        fixture
            .store
            .read_prefix(previous, &fixture.key, "random-1-catalogue", range)
    })
    .unwrap()
    .unwrap()
}

fn published_or_empty(fixture: &Fixture) -> Vec<u8> {
    fixture
        .store
        .session(&fixture.key)
        .unwrap()
        .and_then(|session| read(|range| session.read("random-1-catalogue", range)).unwrap())
        .unwrap_or_default()
}

#[test]
fn a_reader_refuses_patches_that_do_not_reproduce_their_text() {
    let before = br#"[{"a":1},{"list":[1,2,3],"name":"x"}]"#;
    let after = br#"[{"a":1,"b":2},{"list":[1,2,3,4],"name":"y"}]"#;
    let mut range = patch(before, after).unwrap().unwrap();
    let origin = before.to_vec();
    let decoded = Patch::decode(&range).unwrap();
    assert_eq!(&*rebuild(&origin, &[decoded]).unwrap(), &after[..]);
    // A patch for another text, or a damaged hash, is refused.
    let other = br#"[{"a":7},{"list":[1,2,3],"name":"x"}]"#;
    assert_eq!(other.len(), before.len());
    assert!(rebuild(other, &[Patch::decode(&range).unwrap()]).is_err());
    // The first hash byte: both lengths are one-byte integers.
    range[3] ^= 1;
    assert!(
        Patch::decode(&range)
            .and_then(|patch| rebuild(&origin, &[patch]).map(|text| text.into_owned()))
            .is_err()
    );
}

#[test]
fn owned_catalogue_checks_identities_without_rereading_current_payloads() {
    use crate::prepared::pages::Pages;
    struct Counted<'a> {
        writer: &'a mut store::Writer,
        reads: usize,
    }
    impl Pages for Counted<'_> {
        fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
            Pages::write_at(self.writer, section, range, data)
        }
        fn write(&mut self, section: &str, data: &[u8]) -> io::Result<u64> {
            Pages::write(self.writer, section, data)
        }
        fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
            self.reads += 1;
            Pages::read_at(self.writer, section, range)
        }
        fn range_ids(
            &mut self,
            section: &str,
            count: u64,
        ) -> io::Result<Option<Vec<Option<store::RangeId>>>> {
            Pages::range_ids(self.writer, section, count)
        }
    }
    let mut fixtures = [Fixture::new(), Fixture::new()];
    let mut cached = series::CatalogueWriter::default();
    let (mut keys, mut metadata) = (BTreeMap::new(), BTreeMap::new());
    add(&mut keys, &mut metadata, 128);
    for step in 0..16 {
        if step % 3 == 0 {
            add(&mut keys, &mut metadata, 3);
        }
        if step % 3 == 1 {
            let name = metadata.keys().next().unwrap().clone();
            metadata.get_mut(&name).unwrap()["label"] = json!(format!("renamed {step}"));
        }
        if step == 8 {
            cached = Default::default();
        }
        for (index, fixture) in fixtures.iter_mut().enumerate() {
            let mut writer = if step == 12 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            let mut pages = Counted {
                writer: &mut writer,
                reads: 0,
            };
            if index == 0 {
                series::write_catalogue(&mut pages, "owned", 1, &keys, &metadata).unwrap();
            } else {
                let current = match cached.0.0.as_ref() {
                    Some(held) => {
                        pages
                            .range_ids("owned-1-catalogue", held.stored.len() as u64 + 1)
                            .unwrap()
                            == held.ids
                    }
                    None => false,
                };
                cached
                    .write(&mut pages, "owned", 1, &keys, &metadata)
                    .unwrap();
                if current {
                    assert_eq!(pages.reads, 0, "step {step} reread an owned chain");
                } else {
                    assert!(pages.reads > 0, "cold/resumed publication reads its chain");
                }
            }
            writer.commit().unwrap();
        }
        assert_eq!(
            fixtures[0].payloads(),
            fixtures[1].payloads(),
            "step {step}"
        );
        assert_eq!(
            published(&fixtures[1], "owned-1-catalogue"),
            serde_json::to_vec(&(&keys, &metadata)).unwrap()
        );
    }
}

#[test]
fn catalogue_revalidates_other_publishers_replacement_clear_and_abandoned_writes() {
    let mut fixtures = [Fixture::new(), Fixture::new()];
    let mut cache = series::CatalogueWriter::default();
    let (mut keys, mut metadata) = (BTreeMap::new(), BTreeMap::new());
    add(&mut keys, &mut metadata, 128);
    let mut other = metadata.clone();
    let name = other.keys().next().unwrap().clone();
    other.get_mut(&name).unwrap()["label"] = json!("another Prepared for this family and branch");
    for event in [
        "initial",
        "other publisher",
        "replace",
        "abandon",
        "clear",
        "append",
    ] {
        for (index, fixture) in fixtures.iter_mut().enumerate() {
            if event == "abandon" {
                let mut writer = fixture.writer();
                if index == 0 {
                    series::write_catalogue(&mut writer, "current", 1, &keys, &other).unwrap();
                } else {
                    cache
                        .write(&mut writer, "current", 1, &keys, &other)
                        .unwrap();
                }
                // Dropped without commit: the cache must not trust its staged ranges.
            }
            if event == "clear" {
                fixture.store.clear().unwrap();
            }
            let mut writer = match event {
                "replace" => fixture.store.replace(fixture.key.clone()).unwrap(),
                "append" => fixture.append_writer(),
                _ => fixture.writer(),
            };
            if event == "other publisher" {
                series::write_catalogue(&mut writer, "current", 1, &keys, &other).unwrap();
            }
            if index == 0 {
                series::write_catalogue(&mut writer, "current", 1, &keys, &metadata).unwrap();
            } else {
                cache
                    .write(&mut writer, "current", 1, &keys, &metadata)
                    .unwrap();
            }
            writer.commit().unwrap();
        }
        assert_eq!(fixtures[0].payloads(), fixtures[1].payloads(), "{event}");
        assert_eq!(
            published(&fixtures[1], "current-1-catalogue"),
            serde_json::to_vec(&(&keys, &metadata)).unwrap(),
            "{event}"
        );
    }
}

#[test]
fn framing_reports_only_the_ranges_a_catalogue_actually_writes() {
    use crate::prepared::pages::{Committed, HeldPages, Pages, framing as written};

    const TEXT: &[u8] = br#"[{},{"fixed":"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ","value":1}]"#;
    const UPDATED: &[u8] = br#"[{},{"fixed":"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ","value":2}]"#;
    // Base marker, zstd magic, descriptor, one-byte content size, one block header.
    const BASE_FRAMING: u64 = 1 + (4 + 1 + 1 + 3);
    // Patch marker, two one-byte lengths, 16-byte hash, then the same zstd envelope.
    const PATCH_FRAMING: u64 = 1 + 1 + 1 + 16 + (4 + 1 + 1 + 3);

    fn assert_frame(frame: &[u8]) {
        assert_eq!(&frame[..4], &[0x28, 0xb5, 0x2f, 0xfd]);
        assert_eq!(
            frame[4], 0x20,
            "single segment, one-byte content size, no dictionary/window/checksum"
        );
        let block = u32::from_le_bytes([frame[6], frame[7], frame[8], 0]);
        assert_eq!(block & 7, 1, "one final raw block");
        assert_eq!(block >> 3, u32::from(frame[5]));
        assert_eq!(frame.len(), 9 + usize::from(frame[5]));
    }

    let fixture = Fixture::new();
    let mut held = HeldPages::default();
    let keys = BTreeMap::new();
    let mut metadata = json!({
        "fixed": "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "value": 1,
    });
    let section = series::catalogue_section("framing", 1);
    let before = written::bytes();
    {
        let mut view = held.view(Committed {
            store: &fixture.store,
            key: &fixture.key,
            saved: None,
        });
        series::write_catalogue(&mut view, "framing", 1, &keys, &json!({})).unwrap();
        series::write_catalogue(&mut view, "framing", 1, &keys, &metadata).unwrap();
        let base = view.read_at(&section, 0).unwrap().unwrap();
        assert_eq!(base[0], b'B', "the tiny first base was replaced");
        assert_frame(&base[1..]);
        assert_eq!(decompress(&base[1..]).unwrap(), TEXT);
        assert!(view.read_at(&section, 1).unwrap().is_none());
    }
    assert_eq!(written::bytes(), before);
    let mut writer = fixture.writer();
    held.write(&mut writer).unwrap();
    assert_eq!(written::bytes() - before, BASE_FRAMING);
    let unchanged = written::bytes();
    held.write(&mut writer).unwrap();
    assert_eq!(written::bytes(), unchanged, "held framing is not repeated");
    series::write_catalogue(&mut writer, "framing", 1, &keys, &metadata).unwrap();
    assert_eq!(
        written::bytes(),
        unchanged,
        "unchanged catalogue writes nothing"
    );
    metadata["value"] = json!(2);
    series::write_catalogue(&mut writer, "framing", 1, &keys, &metadata).unwrap();
    let patch = Pages::read_at(&mut writer, &section, 1).unwrap().unwrap();
    assert_eq!(TEXT.len(), 89);
    assert_eq!(UPDATED.len(), 89);
    assert_eq!(&patch[..3], &[b'P', 89, 89]);
    assert_eq!(&patch[3..19], &blake3::hash(UPDATED).as_bytes()[..16]);
    assert_frame(&patch[19..]);
    assert_eq!(written::bytes() - unchanged, PATCH_FRAMING);
    let after_patch = written::bytes();
    series::write_catalogue(&mut writer, "framing", 1, &keys, &metadata).unwrap();
    assert_eq!(written::bytes(), after_patch);
    writer.commit().unwrap();
    assert_eq!(published(&fixture, &section), UPDATED);
}

#[test]
fn catalogue_pending_counts_the_exact_escaped_publication_text() {
    let keys = BTreeMap::from([("a".to_owned(), 0)]);
    let metadata = BTreeMap::from([("note", "x\ny")]);
    let expected = br#"[{"a":0},{"note":"x\ny"}]"#;
    assert_eq!(series::catalogue_bytes(&keys, &metadata), expected.len());
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    series::write_catalogue(&mut writer, "sized", 1, &keys, &metadata).unwrap();
    writer.commit().unwrap();
    assert_eq!(published(&fixture, "sized-1-catalogue"), expected.as_slice());
}

#[test]
fn a_shared_catalogue_is_parsed_once_for_its_pages_and_again_when_they_change() {
    let fixture = Fixture::new();
    let publish = |note: &str| {
        let mut writer = fixture.writer();
        let keys = BTreeMap::from([("a".to_owned(), 0)]);
        series::write_catalogue(&mut writer, "shared", 1, &keys, &note.to_owned()).unwrap();
        writer.commit().unwrap();
    };
    let read = || {
        series::read_shared_catalogue::<String>(
            &fixture.store,
            &fixture.key,
            "shared",
            1,
            |(keys, note)| series::catalogue_key_bytes(keys) + note.capacity(),
        )
        .unwrap()
        .unwrap()
    };
    publish("first");
    let scope = series::cache_scope(None);
    let first = read();
    let held = scope.finish();
    let scope = series::cache_scope(Some(&held));
    assert!(
        std::sync::Arc::ptr_eq(&first, &read()),
        "the next ask reuses the parse"
    );
    let held = scope.finish();
    publish("second");
    let _scope = series::cache_scope(Some(&held));
    let second = read();
    assert_eq!(second.1, "second", "new pages are parsed afresh");
    assert_eq!(second.0, first.0);
}
