use super::*;
use crate::template_changes::Watches;
use engine_native::memory::SliceMemory;

/// Two templates whose literal names and keys are inline MSVC strings.
const FIRST: usize = 0x1000;
const SECOND: usize = 0x2000;

fn world() -> Vec<u8> {
    let mut bytes = vec![0; 0x3000];
    for (template, literal, key) in [(FIRST, b"Alpha", b"key_a"), (SECOND, b"Beta_", b"key_b")] {
        put(&mut bytes, template + native::NAME, literal);
        put(&mut bytes, template + native::NAME_KEY, key);
    }
    bytes
}

/// An inline string of up to 15 bytes.
fn put(bytes: &mut [u8], at: usize, text: &[u8]) {
    bytes[at..at + 16].fill(0);
    bytes[at..at + text.len()].copy_from_slice(text);
    bytes[at + 16..at + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
    bytes[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
}

/// A template's pointer, literal name and the key beside an empty literal.
type Template<'a> = (usize, &'a [u8], Option<&'a [u8]>);

/// The icon walk's rows for `templates`, with their literals in `names` and
/// an empty literal's key beside it.
fn icons(templates: &[Template<'_>]) -> (Vec<native::IconRow>, Vec<u8>) {
    let mut names = Vec::new();
    let rows = templates
        .iter()
        .map(|&(pointer, literal, key)| {
            let literal = append(&mut names, literal).unwrap();
            let fallback_key = key.map(|key| append(&mut names, key).unwrap());
            native::IconRow {
                template: ((pointer as u64) << 32) | 0x34,
                pointer,
                owner: 7,
                unit_type: 3,
                display_name: Some(native::DisplayName {
                    literal,
                    fallback_key,
                }),
                ..native::IconRow::default()
            }
        })
        .collect();
    (rows, names)
}

struct Hour {
    outcome: Outcome,
    rows: Vec<native::Row>,
    bytes: Vec<u8>,
}

impl Hour {
    fn keys(&self) -> Vec<&[u8]> {
        self.rows
            .iter()
            .map(|row| text(&self.bytes, row.name_key).unwrap())
            .collect()
    }
}

fn hour(
    world: &[u8],
    icons: &(Vec<native::IconRow>, Vec<u8>),
    store: &mut KeyStore,
    watches: &Watches,
    check: Check,
) -> Hour {
    let memory = SliceMemory {
        base: 0,
        bytes: world,
    };
    let (mut rows, mut bytes) = (Vec::new(), Vec::new());
    let outcome = derive(
        &memory,
        &icons.0,
        Outcome::Rows { len: icons.0.len() },
        &icons.1,
        store,
        watches,
        check,
        &mut rows,
        &mut bytes,
    );
    Hour {
        outcome,
        rows,
        bytes,
    }
}

/// A quiet hour carries every key without reading a template; a hooked write
/// to one template reads its names again; a reordered roster still finds
/// each held key.
#[test]
fn quiet_hours_carry_keys_and_a_written_template_is_read_again() {
    let mut world = world();
    let watches = Watches::enabled(16);
    let mut store = KeyStore::default();
    let roster = icons(&[(FIRST, &b"Alpha"[..], None), (SECOND, &b"Beta_"[..], None)]);
    let first = hour(&world, &roster, &mut store, &watches, Check::Nothing);
    assert_eq!(first.outcome, Outcome::Rows { len: 2 });
    assert_eq!(first.keys(), [b"key_a", b"key_b"]);
    assert_eq!(text(&first.bytes, first.rows[1].name), Some(&b"Beta_"[..]));
    assert_eq!((first.rows[0].owner, first.rows[0].unit_type), (7, 3));
    // Quiet: an unreadable template is not read.
    world[FIRST + native::NAME_KEY + 16..][..8].copy_from_slice(&99_u64.to_le_bytes());
    let quiet = hour(&world, &roster, &mut store, &watches, Check::Nothing);
    assert_eq!(quiet.keys(), [b"key_a", b"key_b"]);
    // A hooked assignment to the second template (named through its data).
    put(&mut world, FIRST + native::NAME_KEY, b"key_a");
    put(&mut world, SECOND + native::NAME_KEY, b"key_c");
    drop(watches.notify(SECOND, 0));
    // An unobserved change to the first stays carried.
    put(&mut world, FIRST + native::NAME_KEY, b"key_z");
    let written = hour(&world, &roster, &mut store, &watches, Check::Nothing);
    assert_eq!(written.keys(), [b"key_a", b"key_c"]);
    // Reordered, each key is still found by its template.
    let reordered = icons(&[(SECOND, &b"Beta_"[..], None), (FIRST, &b"Alpha"[..], None)]);
    drop(watches.notify(0xdead, 3));
    let moved = hour(&world, &reordered, &mut store, &watches, Check::Nothing);
    assert_eq!(moved.keys(), [b"key_c", b"key_a"]);
    assert!(!watches.poisoned());
}

/// The verified slice reads carryable keys afresh and publishes them; a
/// carried key that differs poisons the domain.
#[test]
fn verified_keys_are_read_and_a_difference_poisons() {
    let mut world = world();
    let watches = Watches::enabled(16);
    let mut store = KeyStore::default();
    let roster = icons(&[(FIRST, &b"Alpha"[..], None), (SECOND, &b"Beta_"[..], None)]);
    hour(&world, &roster, &mut store, &watches, Check::Nothing);
    hour(&world, &roster, &mut store, &watches, Check::Every);
    assert!(!watches.poisoned());
    put(&mut world, SECOND + native::NAME_KEY, b"key_c");
    let verified = hour(&world, &roster, &mut store, &watches, Check::Every);
    assert_eq!(verified.keys(), [b"key_a", b"key_c"]);
    assert!(watches.poisoned());
}

/// A hooked assignment enters after the sampled key's certificate was taken
/// and before its header read. The fresh key (or fault) remains the outcome;
/// the overlapped observation cannot poison or become a held capture.
#[test]
fn sampled_key_writer_during_fresh_names_withdraws_without_poison() {
    use std::cell::Cell;
    struct Writer<'a> {
        world: &'a [u8],
        watches: &'a Watches,
        fired: Cell<bool>,
        fail: bool,
    }
    impl engine_native::Memory for Writer<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == FIRST + native::NAME_KEY && !self.fired.replace(true) {
                drop(self.watches.notify(FIRST, 0));
                if self.fail {
                    return false;
                }
            }
            SliceMemory {
                base: 0,
                bytes: self.world,
            }
            .copy(address, output)
        }
    }
    for fail in [false, true] {
        let mut world = world();
        let watches = Watches::enabled(16);
        let mut store = KeyStore::default();
        let roster = icons(&[(FIRST, &b"Alpha"[..], None)]);
        hour(&world, &roster, &mut store, &watches, Check::Nothing);
        put(&mut world, FIRST + native::NAME_KEY, b"key_c");
        let writer = Writer {
            world: &world,
            watches: &watches,
            fired: Cell::new(false),
            fail,
        };
        let (mut rows, mut bytes) = (Vec::new(), Vec::new());
        let outcome = derive(
            &writer,
            &roster.0,
            Outcome::Rows { len: 1 },
            &roster.1,
            &mut store,
            &watches,
            Check::Every,
            &mut rows,
            &mut bytes,
        );
        assert!(writer.fired.get());
        assert!(!watches.poisoned(), "failed read {fail}");
        assert!(store.held.is_empty() && store.quiet.is_none());
        if fail {
            assert!(matches!(outcome, Outcome::Unavailable { .. }));
        } else {
            assert_eq!(outcome, Outcome::Rows { len: 1 });
            assert_eq!(text(&bytes, rows[0].name_key), Some(&b"key_c"[..]));
        }
        assert_eq!(
            hour(&world, &roster, &mut store, &watches, Check::Nothing).keys(),
            [b"key_c"]
        );
    }
}

/// A key behind an empty literal comes from the walk; an unknown unit type
/// or an unavailable walk fails the dictionary and holds nothing.
#[test]
fn the_walks_key_is_published_and_failures_hold_nothing() {
    let world = world();
    let watches = Watches::enabled(16);
    let mut store = KeyStore::default();
    let roster = icons(&[(FIRST, &b""[..], Some(&b"walk_key"[..]))]);
    let read = hour(&world, &roster, &mut store, &watches, Check::Nothing);
    assert_eq!(read.keys(), [b"walk_key"]);
    assert!(store.quiet.is_some());
    let mut failed = roster.clone();
    failed.0[0].unit_type_failure = Some("subunit category unreadable");
    let failure = hour(&world, &failed, &mut store, &watches, Check::Nothing);
    assert_eq!(
        failure.outcome,
        Outcome::Unavailable {
            reason: "subunit category unreadable"
        }
    );
    assert!(failure.rows.is_empty() && store.quiet.is_none() && store.held.is_empty());
    let memory = SliceMemory {
        base: 0,
        bytes: &world,
    };
    let (mut rows, mut bytes) = (Vec::new(), Vec::new());
    let unavailable = derive(
        &memory,
        &[],
        Outcome::Unavailable {
            reason: "icon walk failed",
        },
        &[],
        &mut store,
        &watches,
        Check::Nothing,
        &mut rows,
        &mut bytes,
    );
    assert_eq!(
        unavailable,
        Outcome::Unavailable {
            reason: "icon walk failed"
        }
    );
}
