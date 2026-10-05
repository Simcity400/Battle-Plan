use super::*;
use crate::memory::SliceMemory;

#[test]
fn equivalent_mount_notation_preserves_root_membership_and_priority() {
    let text = |bytes: &[u8]| {
        let mut text = Descriptor::default();
        text.bytes[..bytes.len()].copy_from_slice(bytes);
        text.len = bytes.len();
        text
    };
    let actual = [text(b"F:/game/dlc/a/"), text(b"F:/game/")];
    let expected = [r"\\?\F:\game\dlc\a".to_owned(), r"\\?\F:\game".to_owned()];
    assert!(mounts_match(&actual, &expected));
    assert!(mounts_match(&[text(b"F:/")], &[r"\\?\F:\".to_owned()]));
    assert_eq!(mount_path("/"), Some("/".to_owned()));
    assert!(!mounts_match(&[text(b"F:/")], &["F:".to_owned()]));
    assert!(!mounts_match(
        &[text(b"F:/game/")],
        &[r"\\?\C:\game".to_owned()]
    ));
    assert!(!mounts_match(
        &actual,
        &[expected[1].clone(), expected[0].clone()]
    ));
    assert!(mounts_match(
        &[text(b"//server/share/game/")],
        &[r"\\?\UNC\server\share\game".to_owned()]
    ));
    assert!(!mounts_match(
        &[text(b"F:/game\xff")],
        &["F:/game".to_owned()]
    ));
    assert!(!mounts_match(
        &[text(b"F:/game")],
        &["F:/game\0".to_owned()]
    ));
    assert!(mounts_match(&[text(b"F:/game")], &[r"F:\game".to_owned()]));
    assert!(!mounts_match(&[text(b"F:/game")], &["F:/GAME".to_owned()]));
    // Empty lists cannot establish even the vanilla root.
    assert!(!mounts_match(&[], &[]));
}

struct SearchPathMemory<'a> {
    root: usize,
    bytes: &'a [u8],
}

impl Memory for SearchPathMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == SEARCH_PATH_RVA && output.len() == 8 {
            output.copy_from_slice(&(self.root as u64).to_le_bytes());
            true
        } else {
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
}

fn search_path_bytes(paths: &[&[u8]]) -> Vec<u8> {
    let mut bytes = vec![0; 0x200 + paths.len() * 1025];
    for (index, path) in paths.iter().enumerate() {
        let at = 0x200 + index * 1025;
        u64_at(&mut bytes, 0x100 + index * 8, at);
        bytes[at..at + path.len()].copy_from_slice(path);
    }
    bytes
}

fn u64_at(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
}
fn string_at(bytes: &mut [u8], at: usize, text: &[u8]) {
    assert!(text.len() <= 15);
    bytes[at..at + text.len()].copy_from_slice(text);
    u64_at(bytes, at + 16, text.len());
    u64_at(bytes, at + 24, 15);
}
fn node(bytes: &mut [u8], at: usize, left: usize, parent: usize, right: usize, nil: bool) {
    for (offset, pointer) in [
        (tree_node::LEFT, left),
        (tree_node::PARENT, parent),
        (tree_node::RIGHT, right),
    ] {
        u64_at(bytes, at + offset, pointer);
    }
    bytes[at + tree_node::NIL] = u8::from(nil);
}

#[test]
fn active_membership_and_map_order_are_observed_without_claiming_mount_order() {
    let mut bytes = vec![0_u8; 0x1800];
    // This test addresses the maps directly; no live module image is needed.
    let manager = 0x100;
    let head = 0x300;
    let first = 0x400;
    let second = 0x800;
    u64_at(&mut bytes, manager + DLCS, head);
    u64_at(&mut bytes, manager + DLCS + msvc_map::COUNT, 2);
    node(&mut bytes, head, first, first, second, true);
    node(&mut bytes, first, head, head, second, false);
    node(&mut bytes, second, head, first, head, false);
    for (at, name, descriptor) in [
        (first, b"Alpha".as_slice(), b"dlc/a.dlc".as_slice()),
        (second, b"Beta", b"dlc/b.dlc"),
    ] {
        string_at(
            &mut bytes,
            at + content_node::VALUE + content_entry::NAME,
            name,
        );
        string_at(
            &mut bytes,
            at + content_node::VALUE + content_entry::DESCRIPTOR,
            descriptor,
        );
    }
    bytes[second + content_node::VALUE + content_entry::DLC_REPORTED_ACTIVE] = 1;
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut output = [Entry::default(); 2];
    let outcome = walk_map(&memory, manager, DLCS, &mut output, |value, _| {
        Ok(read::<u8>(&memory, value + content_entry::DLC_REPORTED_ACTIVE).unwrap() != 0)
    });
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(output[0].name.as_bytes(), b"Alpha");
    assert!(!output[0].reported_active);
    assert!(output[1].reported_active);
    assert_eq!(
        walk_map(&memory, manager, DLCS, &mut output[..1], |_, _| Ok(false)),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    u64_at(&mut bytes, second + tree_node::PARENT, head);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        walk_map(&memory, manager, DLCS, &mut output, |_, _| Ok(false)),
        Outcome::Unavailable {
            reason: "content map child parent mismatch"
        }
    );
}

#[test]
fn mod_selection_matches_the_engine_suffix_rule_including_separator_priority() {
    assert!(requested_matches(
        b"C:\\users\\Example\\mod\\ugc_1.mod",
        b"mod/ugc_1.mod"
    ));
    assert!(requested_matches(
        b"/users/example/mod/local.mod",
        b"mod/local.mod"
    ));
    assert!(!requested_matches(b"local.mod", b"mod/local.mod"));
    assert!(!requested_matches(b"mod/Local.mod", b"mod/local.mod"));
    assert!(!requested_matches(
        b"C:\\users\\Example/mod/local.mod",
        b"mod/local.mod"
    ));
    assert!(requested_matches(b"mod/", b"mod/"));
}

#[test]
fn partial_requests_cannot_claim_an_empty_active_mod_roster() {
    struct RootMemory<'a>(&'a [u8]);
    impl Memory for RootMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&0x100_u64.to_le_bytes());
                true
            } else {
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
    }
    let mut bytes = vec![0_u8; 0x800];
    for (map, head) in [(DLCS, 0x300), (MODS, 0x400)] {
        u64_at(&mut bytes, 0x100 + map, head);
        node(&mut bytes, head, head, head, head, true);
    }
    u64_at(&mut bytes, 0x100 + REQUESTED_MODS, 0x500);
    bytes[0x100 + REQUESTED_MODS + 8..0x100 + REQUESTED_MODS + 12]
        .copy_from_slice(&1_u32.to_le_bytes());
    bytes[0x100 + REQUESTED_MODS + 12..0x100 + REQUESTED_MODS + 16]
        .copy_from_slice(&1_u32.to_le_bytes());
    string_at(&mut bytes, 0x500, b"mod/a.mod");
    let memory = RootMemory(&bytes);
    let observation = observe(
        &memory,
        0,
        Buffers {
            dlc_filter_strings: &mut [],
            requested_mods: &mut [],
            dlcs: &mut [],
            mods: &mut [],
        },
    );
    assert_eq!(
        observation.requested_mods,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(
        observation.mods,
        Outcome::Unavailable {
            reason: "complete requested mod list unavailable"
        }
    );
    let mut requested = [Descriptor::default(); 1];
    let observation = observe(
        &memory,
        0,
        Buffers {
            dlc_filter_strings: &mut [],
            requested_mods: &mut requested,
            dlcs: &mut [],
            mods: &mut [],
        },
    );
    assert_eq!(observation.requested_mods, Outcome::Rows { len: 1 });
    assert_eq!(requested[0].as_bytes(), b"mod/a.mod");
    // A requested descriptor can be undiscovered. The two lists stay distinct.
    assert_eq!(observation.mods, Outcome::Rows { len: 0 });
}

#[test]
fn counts_read_headers_and_the_mount_terminator_only() {
    struct Rooted<'a>(&'a [u8]);
    impl Memory for Rooted<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let root = match address {
                SINGLETON_RVA => 0x100_u64,
                SEARCH_PATH_RVA => 0x1000,
                _ => {
                    return SliceMemory {
                        base: 0,
                        bytes: self.0,
                    }
                    .copy(address, output);
                }
            };
            output.copy_from_slice(&root.to_le_bytes());
            true
        }
    }
    let mut bytes = vec![0_u8; 0x2000];
    let manager = 0x100;
    for (offset, count) in [(DLC_FILTER_STRINGS, 70_u32), (REQUESTED_MODS, 100)] {
        u64_at(&mut bytes, manager + offset, 0x4000);
        bytes[manager + offset + 8..manager + offset + 12].copy_from_slice(&count.to_le_bytes());
        bytes[manager + offset + 12..manager + offset + 16].copy_from_slice(&count.to_le_bytes());
    }
    u64_at(&mut bytes, manager + DLCS + msvc_map::COUNT, 90);
    // Above the reader's node bound: `observe` refuses the map, so no hint.
    u64_at(
        &mut bytes,
        manager + MODS + msvc_map::COUNT,
        MAX_MAP_NODES as usize + 1,
    );
    for index in 0..80 {
        u64_at(&mut bytes, 0x1000 + index * 8, 0x1800);
    }
    let rooted = Rooted(&bytes);
    let memory = crate::read_census::ReadCensus::new(&rooted);
    assert_eq!(
        observe_counts(&memory, 0),
        Counts {
            dlc_filter_strings: 70,
            requested_mods: 100,
            dlcs: 90,
            mods: 0,
            mounts: 80,
        }
    );
    // Singleton, two headers, two map counts, the root and 81 list entries.
    assert_eq!(memory.calls(), 1 + 2 + 2 + 1 + 81);
    assert_eq!(observe_counts(&Rooted(&[]), 0), Counts::default());
}

#[test]
fn search_path_order_raw_bytes_and_omitted_count_are_preserved() {
    let paths: &[&[u8]] = &[b"dlc/z.zip", b"dlc/\xff/", b"C:\\game/"];
    let bytes = search_path_bytes(paths);
    let memory = SearchPathMemory {
        root: 0x100,
        bytes: &bytes,
    };
    let memory = crate::read_census::ReadCensus::new(&memory);
    let mut output = [Descriptor::default(); 3];
    assert_eq!(
        observe_search_path(&memory, 0, &mut output),
        Outcome::Rows { len: 3 }
    );
    for (row, expected) in output.iter().zip(paths) {
        assert_eq!(row.as_bytes(), *expected);
    }
    let string_bytes: usize = paths.iter().map(|path| path.len() + 1).sum();
    assert_eq!(memory.calls(), 1 + paths.len() + 1 + string_bytes);
    assert_eq!(memory.bytes(), 8 + (paths.len() + 1) * 8 + string_bytes);
    memory.reset();
    assert_eq!(
        observe_search_path(&memory, 0, &mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    assert_eq!(memory.calls(), 1 + paths.len() + 1 + paths[0].len() + 1);
    memory.reset();
    assert_eq!(
        observe_search_path(&memory, 0, &mut []),
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
    assert_eq!(memory.calls(), 1 + paths.len() + 1);
    let empty_bytes = search_path_bytes(&[]);
    let empty = SearchPathMemory {
        root: 0x100,
        bytes: &empty_bytes,
    };
    let empty = crate::read_census::ReadCensus::new(&empty);
    assert_eq!(
        observe_search_path(&empty, 0, &mut []),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(empty.calls(), 2);
    let null = SearchPathMemory {
        root: 0,
        bytes: &[],
    };
    assert_eq!(
        observe_search_path(&null, 0, &mut output),
        Outcome::Unavailable {
            reason: "search path cache unreadable or not constructed"
        }
    );
    assert_eq!(
        observe_search_path(
            &SliceMemory {
                base: 0,
                bytes: &[]
            },
            0,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "search path cache unreadable or not constructed"
        }
    );
}

#[test]
fn search_path_string_capacity_and_missing_terminators_are_explicit() {
    let mut bytes = search_path_bytes(&[&[b'x'; 1024]]);
    let memory = SearchPathMemory {
        root: 0x100,
        bytes: &bytes,
    };
    let mut output = [Descriptor::default(); 1];
    assert_eq!(
        observe_search_path(&memory, 0, &mut output),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(output[0].as_bytes(), &[b'x'; 1024]);
    bytes[0x200 + 1024] = b'x';
    let memory = SearchPathMemory {
        root: 0x100,
        bytes: &bytes,
    };
    assert_eq!(
        observe_search_path(&memory, 0, &mut output),
        Outcome::Unavailable {
            reason: "search path string exceeds capacity or has no terminator"
        }
    );
    let memory = SearchPathMemory {
        root: 0x100,
        bytes: &bytes[..0x203],
    };
    assert_eq!(
        observe_search_path(&memory, 0, &mut output),
        Outcome::Unavailable {
            reason: "search path string unreadable"
        }
    );
    let memory = SearchPathMemory {
        root: 0x100,
        bytes: &bytes[..0x108],
    };
    assert_eq!(
        observe_search_path(&memory, 0, &mut []),
        Outcome::Unavailable {
            reason: "search path entry pointer unreadable"
        }
    );
}

#[test]
fn search_path_rejects_overflow_and_bounds_an_unterminated_pointer_list() {
    let memory = SearchPathMemory {
        root: usize::MAX - 7,
        bytes: &[],
    };
    assert_eq!(
        observe_search_path(&memory, usize::MAX, &mut []),
        Outcome::Unavailable {
            reason: "search path root address overflow"
        }
    );
    assert_eq!(
        observe_search_path(&memory, 0, &mut []),
        Outcome::Unavailable {
            reason: "search path entry pointer unreadable"
        }
    );
    struct Unterminated;
    impl Memory for Unterminated {
        fn copy(&self, _: usize, output: &mut [u8]) -> bool {
            assert_eq!(output.len(), 8);
            output.copy_from_slice(&0x100_u64.to_le_bytes());
            true
        }
    }
    assert_eq!(
        observe_search_path(&Unterminated, 0, &mut []),
        Outcome::Unavailable {
            reason: "search path list exceeds reader limit or has no terminator"
        }
    );
}
