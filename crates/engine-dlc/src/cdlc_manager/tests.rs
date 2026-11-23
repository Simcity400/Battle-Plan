use super::*;
use engine_native_memory::memory::SliceMemory;

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
    assert_ne!(mount_path("F:/"), mount_path("F:"));
    assert_ne!(mount_path("F:/game/"), mount_path(r"\\?\C:\game"));
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
    assert_ne!(mount_path("F:/game"), mount_path("F:/GAME"));
    assert!(!mounts_match::<Descriptor>(&[], &[]));
}

fn mount(path: &str) -> Descriptor {
    let mut text = Descriptor::default();
    text.bytes[..path.len()].copy_from_slice(path.as_bytes());
    text.len = path.len();
    text
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "hoi4-engine-dlc-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn junction(link: &std::path::Path, target: &std::path::Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J {} {}: {}{}",
        link.display(),
        target.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn short_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let long: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut short = vec![0_u16; 32768];
    let length =
        unsafe { GetShortPathNameW(long.as_ptr(), short.as_mut_ptr(), short.len() as u32) };
    assert!(
        length > 0 && (length as usize) < short.len(),
        "GetShortPathNameW {}",
        path.display()
    );
    std::ffi::OsString::from_wide(&short[..length as usize]).into()
}

#[test]
fn another_spelling_of_the_same_folder_is_the_same_mount() {
    let root = Scratch::new("mount-spellings");
    let long = root.0.join("a-profile-folder-with-a-long-name");
    let (base, elsewhere) = (long.join("base"), root.0.join("elsewhere").join("base"));
    for folder in [&base, &elsewhere] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let link = root.0.join("junction");
    junction(&link, &long);
    let mut spellings = vec![link.clone()];
    let short = short_path(&long);
    if !short
        .file_name()
        .unwrap()
        .eq_ignore_ascii_case(long.file_name().unwrap())
    {
        spellings.push(short);
    }
    let canonical = |path: &std::path::Path| {
        std::fs::canonicalize(path)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap()
    };
    let game = |path: &std::path::Path| mount(&path.to_str().unwrap().replace('\\', "/"));
    let (expected, other) = (canonical(&base), canonical(&elsewhere));
    let gone = format!(r"{}\gone", canonical(&long));
    for spelling in spellings {
        let spelled = game(&spelling.join("base"));
        let what = spelling.display();
        assert!(
            mounts_match(&[spelled.clone()], std::slice::from_ref(&expected)),
            "{what}"
        );
        assert!(
            mounts_match(
                &[spelled.clone(), game(&elsewhere)],
                &[expected.clone(), other.clone()]
            ),
            "{what}"
        );
        assert!(
            !mounts_match(&[spelled.clone()], std::slice::from_ref(&other)),
            "{what}"
        );
        assert!(
            !mounts_match(
                &[spelled.clone(), game(&elsewhere)],
                &[other.clone(), expected.clone()]
            ),
            "{what}"
        );
        assert!(
            !mounts_match(&[spelled.clone()], &[expected.clone(), other.clone()]),
            "{what}"
        );
        assert!(
            !mounts_match(&[spelled], std::slice::from_ref(&gone)),
            "{what}"
        );
        assert!(
            !mounts_match(&[game(&spelling.join("gone"))], std::slice::from_ref(&gone)),
            "{what}"
        );
    }
}

#[test]
fn a_differing_mount_is_named_by_its_position_paths_and_what_the_disk_says() {
    let root = Scratch::new("mount-difference");
    let (one, two) = (root.0.join("one"), root.0.join("two"));
    for folder in [&one, &two] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let link = root.0.join("link-to-one");
    junction(&link, &one);
    let text = |path: &std::path::Path| path.to_str().unwrap().to_owned();
    let (one, two, link, gone) = (
        text(&one),
        text(&two),
        text(&link),
        text(&root.0.join("gone")),
    );
    let observed = [mount(&link), mount(&two)];
    assert_eq!(
        mount_difference(&observed, &[one.clone(), two.clone()]),
        None
    );
    assert_eq!(
        mount_difference(&observed, &[two.clone(), one.clone()]),
        Some(MountDifference::Differs {
            position: 0,
            observed: link.as_bytes(),
            expected: &two,
            folders: Folders::Different,
        })
    );
    assert_eq!(
        mount_difference(&observed, &[one.clone(), gone.clone()]),
        Some(MountDifference::Differs {
            position: 1,
            observed: two.as_bytes(),
            expected: &gone,
            folders: Folders::Unopened {
                observed: true,
                expected: false,
            },
        })
    );
    let unmounted = format!("{link}/gone");
    assert_eq!(
        mount_difference(&[mount(&unmounted)], std::slice::from_ref(&gone)),
        Some(MountDifference::Differs {
            position: 0,
            observed: unmounted.as_bytes(),
            expected: &gone,
            folders: Folders::Unopened {
                observed: false,
                expected: false,
            },
        })
    );
    assert_eq!(
        mount_difference(&observed[..1], &[one.clone(), two.clone()]),
        Some(MountDifference::Missing {
            position: 1,
            expected: &two,
        })
    );
    assert_eq!(
        mount_difference(&observed, std::slice::from_ref(&one)),
        Some(MountDifference::Extra {
            position: 1,
            observed: two.as_bytes(),
        })
    );
    let mut unreadable = mount(&one);
    unreadable.bytes[unreadable.len] = 0xff;
    unreadable.len += 1;
    assert_eq!(
        mount_difference(&[unreadable], std::slice::from_ref(&one)),
        Some(MountDifference::Differs {
            position: 0,
            observed: &unreadable.bytes[..unreadable.len],
            expected: &one,
            folders: Folders::Unreadable,
        })
    );
    assert_eq!(
        mount_difference::<Descriptor>(&[], &[]),
        Some(MountDifference::NoneExpected)
    );
}

struct SearchPathMemory<'a> {
    root: usize,
    bytes: &'a [u8],
}

impl Memory for SearchPathMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == SEARCH_PATH_RVA.0 && output.len() == 8 {
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
            if address == SINGLETON_RVA.0 && output.len() == 8 {
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
    assert_eq!(observation.mods, Outcome::Rows { len: 0 });
}

#[test]
fn counts_read_headers_and_the_mount_terminator_only() {
    struct Rooted<'a>(&'a [u8]);
    impl Memory for Rooted<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let root = match address {
                a if a == SINGLETON_RVA.0 => 0x100_u64,
                a if a == SEARCH_PATH_RVA.0 => 0x1000,
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
    u64_at(&mut bytes, manager + MODS + msvc_map::COUNT, 5_001);
    for index in 0..80 {
        u64_at(&mut bytes, 0x1000 + index * 8, 0x1800);
    }
    let rooted = Rooted(&bytes);
    let memory = engine_native_memory::read_census::ReadCensus::new(&rooted);
    assert_eq!(
        observe_counts(&memory, 0),
        Counts {
            dlc_filter_strings: 70,
            requested_mods: 100,
            dlcs: 90,
            mods: 5_001,
            mounts: 80,
        }
    );
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
    let memory = engine_native_memory::read_census::ReadCensus::new(&memory);
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
    let empty = engine_native_memory::read_census::ReadCensus::new(&empty);
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
fn search_path_rejects_overflow_and_reads_until_native_terminator_or_fault() {
    let memory = SearchPathMemory {
        root: usize::MAX - 7,
        bytes: &[],
    };
    assert_eq!(
        observe_search_path(&memory, usize::MAX, &mut []),
        Outcome::Unavailable {
            reason: "search path root address withheld"
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
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address != SEARCH_PATH_RVA.0 && address >= 0x100 + 5_001 * 8 {
                return false;
            }
            assert_eq!(output.len(), 8);
            output.copy_from_slice(&0x100_u64.to_le_bytes());
            true
        }
    }
    assert_eq!(
        observe_search_path(&Unterminated, 0, &mut []),
        Outcome::Unavailable {
            reason: "search path entry pointer unreadable"
        }
    );
}
