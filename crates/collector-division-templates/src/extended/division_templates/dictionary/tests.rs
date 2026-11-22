use super::*;
use engine_native::memory::SliceMemory;

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

fn put(bytes: &mut [u8], at: usize, text: &[u8]) {
    bytes[at..at + 16].fill(0);
    bytes[at..at + text.len()].copy_from_slice(text);
    bytes[at + 16..at + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
    bytes[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
}

type Template<'a> = (usize, Option<(&'a [u8], &'a [u8])>);

fn icons(templates: &[Template<'_>]) -> (Vec<native::IconRow>, Vec<u8>) {
    let mut names = Vec::new();
    let rows = templates
        .iter()
        .map(|&(pointer, walked)| {
            let spans = walked.map(|(literal, key)| {
                let literal = append(&mut names, literal).unwrap();
                (literal, append(&mut names, key).unwrap())
            });
            native::IconRow {
                template: ((pointer as u64) << 32) | 0x34,
                pointer,
                owner: 7,
                unit_type: 3,
                display_name: spans.map(|(literal, key)| native::DisplayName {
                    literal,
                    fallback_key: (literal.len == 0).then_some(key),
                }),
                name_key: spans.map(|(_, key)| key),
                ..native::IconRow::default()
            }
        })
        .collect();
    (rows, names)
}

struct Hour {
    outcome: Outcome,
    rereads: u64,
    rows: Vec<native::Row>,
    bytes: Vec<u8>,
}

impl Hour {
    fn names(&self) -> Vec<(&[u8], &[u8])> {
        self.rows
            .iter()
            .map(|row| {
                (
                    text(&self.bytes, row.name).unwrap(),
                    text(&self.bytes, row.name_key).unwrap(),
                )
            })
            .collect()
    }
}

fn hour(memory: &impl engine_native::Memory, icons: &(Vec<native::IconRow>, Vec<u8>)) -> Hour {
    let (mut rows, mut bytes) = (Vec::new(), Vec::new());
    let (outcome, rereads) = derive(
        memory,
        &icons.0,
        Outcome::Rows { len: icons.0.len() },
        &icons.1,
        &mut rows,
        &mut bytes,
    );
    Hour {
        outcome,
        rereads,
        rows,
        bytes,
    }
}

#[test]
fn the_walks_names_are_published_without_a_read() {
    struct Unreadable;
    impl engine_native::Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("the walk supplied every name")
        }
    }
    let roster = icons(&[
        (FIRST, Some((&b"Alpha"[..], &b"key_a"[..]))),
        (SECOND, Some((&b""[..], &b"walk_key"[..]))),
    ]);
    let read = hour(&Unreadable, &roster);
    assert_eq!((read.outcome, read.rereads), (Outcome::Rows { len: 2 }, 0));
    assert_eq!(
        read.names(),
        [(&b"Alpha"[..], &b"key_a"[..]), (&b""[..], &b"walk_key"[..])]
    );
    assert_eq!((read.rows[0].owner, read.rows[0].unit_type), (7, 3));
}

#[test]
fn unread_names_are_read_again_and_failures_refuse_the_dictionary() {
    let mut world = world();
    let roster = icons(&[
        (FIRST, Some((&b"Alpha"[..], &b"key_a"[..]))),
        (SECOND, None),
    ]);
    let reread = hour(
        &SliceMemory {
            base: 0,
            bytes: &world,
        },
        &roster,
    );
    assert_eq!((reread.outcome, reread.rereads), (Outcome::Rows { len: 2 }, 1));
    assert_eq!(reread.names()[1], (&b"Beta_"[..], &b"key_b"[..]));
    world[SECOND + native::NAME_KEY + 16..][..8].copy_from_slice(&99_u64.to_le_bytes());
    let failed = hour(
        &SliceMemory {
            base: 0,
            bytes: &world,
        },
        &roster,
    );
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }) && failed.rows.is_empty());
    let mut unknown = roster.clone();
    unknown.0[0].unit_type_failure = Some("subunit category unreadable");
    let unknown = hour(
        &SliceMemory {
            base: 0,
            bytes: &world,
        },
        &unknown,
    );
    assert_eq!(
        unknown.outcome,
        Outcome::Unavailable {
            reason: "subunit category unreadable"
        }
    );
    let (mut rows, mut bytes) = (Vec::new(), Vec::new());
    let (unavailable, _) = derive(
        &SliceMemory {
            base: 0,
            bytes: &world,
        },
        &[],
        Outcome::Unavailable {
            reason: "icon walk failed",
        },
        &[],
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
