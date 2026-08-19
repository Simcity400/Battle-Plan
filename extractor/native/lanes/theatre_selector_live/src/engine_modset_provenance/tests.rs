use super::*;
use std::cell::Cell;
use std::collections::BTreeMap;

const IMAGE_BASE: u64 = A729_PREFERRED_IMAGE_BASE;

const GAME_ROOT: &str = "d:/games/hoi4";
const USER_ROOT: &str = "c:/users/Example/documents/paradox interactive/the game";

#[derive(Clone, Default)]
struct Image {
    bytes: BTreeMap<u64, u8>,
    next: u64,
}

impl TargetImageMemory for Image {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(length);
        for offset in 0..length as u64 {
            let at = address
                .checked_add(offset)
                .ok_or("synthetic read overflow")?;
            out.push(
                *self
                    .bytes
                    .get(&at)
                    .ok_or_else(|| format!("synthetic address 0x{at:x} is not readable"))?,
            );
        }
        Ok(out)
    }
}

impl Image {
    fn new() -> Self {
        Self {
            bytes: BTreeMap::new(),
            next: 0x7000_0000,
        }
    }

    fn alloc(&mut self, len: u64) -> u64 {
        let at = self.next;
        self.next = (self.next + len + 0x3f) & !0x3f;
        for offset in 0..len {
            self.bytes.insert(at + offset, 0);
        }
        at
    }

    fn put(&mut self, address: u64, data: &[u8]) {
        for (offset, byte) in data.iter().enumerate() {
            self.bytes.insert(address + offset as u64, *byte);
        }
    }

    fn put_u64(&mut self, address: u64, value: u64) {
        self.put(address, &value.to_le_bytes());
    }

    fn put_u32(&mut self, address: u64, value: u32) {
        self.put(address, &value.to_le_bytes());
    }

    fn put_u8(&mut self, address: u64, value: u8) {
        self.put(address, &[value]);
    }

    fn put_string(&mut self, address: u64, text: &str) {
        let bytes = text.as_bytes();
        self.put(address, &[0u8; 32]);
        self.put_u64(address + STD_STRING_SIZE_OFFSET as u64, bytes.len() as u64);
        if bytes.len() > STD_STRING_SSO_CAPACITY as usize {
            let heap = self.alloc(bytes.len() as u64 + 1);
            self.put(heap, bytes);
            self.put_u64(address, heap);
            self.put_u64(
                address + STD_STRING_CAPACITY_OFFSET as u64,
                bytes.len() as u64,
            );
        } else {
            self.put(address, bytes);
            self.put_u64(
                address + STD_STRING_CAPACITY_OFFSET as u64,
                STD_STRING_SSO_CAPACITY,
            );
        }
    }

    fn put_string_vector(&mut self, address: u64, items: &[&str]) {
        self.put(address, &[0u8; 0x18]);
        self.put_u32(address + PDX_VECTOR_CAPACITY, items.len() as u32);
        self.put_u32(address + PDX_VECTOR_SIZE, items.len() as u32);
        if items.is_empty() {
            return;
        }
        let data = self.alloc(items.len() as u64 * STD_STRING_STRIDE);
        for (index, item) in items.iter().enumerate() {
            self.put_string(data + index as u64 * STD_STRING_STRIDE, item);
        }
        self.put_u64(address, data);
    }

    fn put_c_string(&mut self, text: &str) -> u64 {
        let at = self.alloc(text.len() as u64 + 1 + 256);
        self.put(at, text.as_bytes());
        self.put_u8(at + text.len() as u64, 0);
        at
    }

    fn put_unterminated(&mut self, len: u64) -> u64 {
        let at = self.alloc(len);
        for offset in 0..len {
            self.bytes.insert(at + offset, b'a');
        }
        at
    }

    fn put_unterminated_at_page_offset(&mut self, page_offset: u64, len: u64) -> u64 {
        let probe = self.alloc(0);
        let at = (probe & !0xfff) + 0x1000 + page_offset;
        self.next = at + len + 0x1000;
        for offset in 0..len {
            self.bytes.insert(at + offset, b'a');
        }
        at
    }
}

#[derive(Clone)]
struct Entry {
    kind: EngineContentKind,
    key: String,
    descriptor: String,
    canonical: String,
    name: String,
    path: String,
    archive: String,
    base: String,
    dependencies: Vec<String>,
    replace_paths: Vec<String>,
    eligible: bool,
}

impl Entry {
    fn dlc(key: &str, path: &str) -> Self {
        Self::new(EngineContentKind::Dlc, key, path)
    }

    fn mod_(key: &str, path: &str) -> Self {
        Self::new(EngineContentKind::Mod, key, path)
    }

    fn new(kind: EngineContentKind, key: &str, path: &str) -> Self {
        Self {
            kind,
            key: key.to_string(),
            descriptor: key.to_string(),
            canonical: format!("canonical of {key}"),
            name: format!("{key} display"),
            path: path.to_string(),
            archive: String::new(),
            base: String::new(),
            dependencies: Vec::new(),
            replace_paths: Vec::new(),
            eligible: true,
        }
    }

    fn descriptor(mut self, value: &str) -> Self {
        self.descriptor = value.to_string();
        self
    }

    fn based_at(mut self, value: &str) -> Self {
        self.base = value.to_string();
        self
    }

    fn canonical(mut self, value: &str) -> Self {
        self.canonical = value.to_string();
        self
    }

    fn depends_on(mut self, names: &[&str]) -> Self {
        self.dependencies = names.iter().map(|value| value.to_string()).collect();
        self
    }

    fn replaces(mut self, paths: &[&str]) -> Self {
        self.replace_paths = paths.iter().map(|value| value.to_string()).collect();
        self
    }

    fn archive(mut self, value: &str) -> Self {
        self.archive = value.to_string();
        self
    }

    fn ineligible(mut self) -> Self {
        self.eligible = false;
        self
    }
}

#[derive(Clone, Default)]
struct Launch {
    requested_mods: Vec<String>,
    disabled_dlcs: Vec<String>,
    entries: Vec<Entry>,
    search_path: Vec<String>,
    balanced: bool,
}

impl Launch {
    fn requested(mut self, mods: &[&str]) -> Self {
        self.requested_mods = mods.iter().map(|value| value.to_string()).collect();
        self
    }

    fn disabled(mut self, dlcs: &[&str]) -> Self {
        self.disabled_dlcs = dlcs.iter().map(|value| value.to_string()).collect();
        self
    }

    fn discovered(mut self, entries: &[Entry]) -> Self {
        self.entries = entries.to_vec();
        self
    }

    fn mounted(mut self, roots: &[&str]) -> Self {
        self.search_path = roots.iter().map(|value| value.to_string()).collect();
        self
    }

    fn balanced(mut self) -> Self {
        self.balanced = true;
        self
    }

    fn build(&self) -> Image {
        let mut image = Image::new();
        let manager = image.alloc(0xc8);
        image.put_u64(IMAGE_BASE + DLC_MANAGER_RVA, manager);

        let strings: Vec<&str> = self.disabled_dlcs.iter().map(String::as_str).collect();
        image.put_string_vector(manager + MANAGER_REQUESTED_DISABLED_DLCS, &strings);
        let strings: Vec<&str> = self.requested_mods.iter().map(String::as_str).collect();
        image.put_string_vector(manager + MANAGER_REQUESTED_MODS, &strings);

        for (offset, kind) in [
            (MANAGER_DISCOVERED_DLC_MAP, EngineContentKind::Dlc),
            (MANAGER_DISCOVERED_MOD_MAP, EngineContentKind::Mod),
        ] {
            let mut of_kind: Vec<&Entry> = self
                .entries
                .iter()
                .filter(|entry| entry.kind == kind)
                .collect();
            of_kind.sort_by(|left, right| left.key.cmp(&right.key));
            put_map(&mut image, manager + offset, &of_kind, self.balanced);
        }

        put_search_path(&mut image, &self.search_path);
        image
    }
}

fn put_map(image: &mut Image, container: u64, entries: &[&Entry], balanced: bool) {
    let head = image.alloc(0x2e0);
    image.put_u8(head + NODE_IS_NIL, 1);
    image.put_u64(container + MAP_HEAD_POINTER, head);
    image.put_u64(container + MAP_SIZE, entries.len() as u64);
    if entries.is_empty() {
        image.put_u64(head + NODE_LEFT, head);
        image.put_u64(head + NODE_PARENT, head);
        image.put_u64(head + NODE_RIGHT, head);
        return;
    }
    let nodes: Vec<u64> = entries.iter().map(|_| image.alloc(0x2e0)).collect();
    for (index, entry) in entries.iter().enumerate() {
        let node = nodes[index];
        image.put_u8(node + NODE_IS_NIL, 0);
        image.put_string(node + NODE_KEY, &entry.key);
        put_entry(image, node + NODE_VALUE, entry);
    }
    if balanced {
        let root = link_balanced(image, &nodes, head);
        image.put_u64(head + NODE_PARENT, root);
        image.put_u64(root + NODE_PARENT, head);
    } else {
        for index in 0..nodes.len() {
            image.put_u64(nodes[index] + NODE_LEFT, head);
            image.put_u64(
                nodes[index] + NODE_PARENT,
                if index == 0 { head } else { nodes[index - 1] },
            );
            image.put_u64(
                nodes[index] + NODE_RIGHT,
                nodes.get(index + 1).copied().unwrap_or(head),
            );
        }
        image.put_u64(head + NODE_PARENT, nodes[0]);
    }
    image.put_u64(head + NODE_LEFT, nodes[0]);
    image.put_u64(head + NODE_RIGHT, *nodes.last().expect("non-empty"));
}

fn link_balanced(image: &mut Image, nodes: &[u64], nil: u64) -> u64 {
    if nodes.is_empty() {
        return nil;
    }
    let mid = nodes.len() / 2;
    let root = nodes[mid];
    let left = link_balanced(image, &nodes[..mid], nil);
    let right = link_balanced(image, &nodes[mid + 1..], nil);
    image.put_u64(root + NODE_LEFT, left);
    image.put_u64(root + NODE_RIGHT, right);
    if left != nil {
        image.put_u64(left + NODE_PARENT, root);
    }
    if right != nil {
        image.put_u64(right + NODE_PARENT, root);
    }
    root
}

fn put_entry(image: &mut Image, value: u64, entry: &Entry) {
    image.put_string(value + ENTRY_NAME, &entry.canonical);
    image.put_string(value + ENTRY_LOGGED_NAME, &entry.name);
    image.put_string(value + ENTRY_DESCRIPTOR, &entry.descriptor);
    image.put_string(value + ENTRY_PATH, &entry.path);
    image.put_string(value + ENTRY_ARCHIVE, &entry.archive);
    image.put_string(value + ENTRY_BASE_DIRECTORY, &entry.base);
    let deps: Vec<&str> = entry.dependencies.iter().map(String::as_str).collect();
    image.put_string_vector(value + ENTRY_DEPENDENCIES, &deps);
    let replace: Vec<&str> = entry.replace_paths.iter().map(String::as_str).collect();
    image.put_string_vector(value + ENTRY_REPLACE_PATHS, &replace);
    image.put_u32(
        value + ENTRY_KIND,
        match entry.kind {
            EngineContentKind::Dlc => ENTRY_KIND_DLC as u32,
            EngineContentKind::Mod => 2,
        },
    );
    image.put_u8(value + ENTRY_DLC_ELIGIBLE, u8::from(entry.eligible));
}

fn put_search_path(image: &mut Image, roots: &[String]) {
    let list = image.alloc((roots.len() as u64 + 1) * 8);
    for (index, root) in roots.iter().enumerate() {
        let text = image.put_c_string(root);
        image.put_u64(list + index as u64 * 8, text);
    }
    image.put_u64(list + roots.len() as u64 * 8, 0);
    image.put_u64(IMAGE_BASE + VFS_SEARCH_PATH_CACHE_RVA, list);
}

fn read(image: &Image) -> EngineModsetProvenance {
    read_engine_modset_provenance(image, IMAGE_BASE).expect("engine modset provenance")
}

fn refuse(image: &Image) -> String {
    read_engine_modset_provenance(image, IMAGE_BASE).expect_err("this image must be refused")
}

fn entry<'a>(provenance: &'a EngineModsetProvenance, descriptor: &str) -> &'a EngineContentEntry {
    provenance
        .discovered
        .iter()
        .find(|entry| entry.descriptor_entry == descriptor)
        .unwrap_or_else(|| panic!("no discovered entry for {descriptor}"))
}

fn ordinary() -> Launch {
    Launch::default()
        .requested(&["mod/alpha.mod"])
        .disabled(&["dlc/quiet/quiet.dlc"])
        .discovered(&[
            Entry::dlc("dlc/loud/loud.dlc", "dlc/loud").based_at(GAME_ROOT),
            Entry::dlc("dlc/quiet/quiet.dlc", "dlc/quiet").based_at(GAME_ROOT),
            Entry::mod_("mod/alpha.mod", "mod/alpha")
                .based_at(USER_ROOT)
                .depends_on(&["canonical of dlc/loud/loud.dlc"])
                .replaces(&["common/units"]),
            Entry::mod_("mod/beta.mod", "mod/beta").based_at(USER_ROOT),
        ])
        .mounted(&[
            GAME_ROOT,
            "d:/games/hoi4/dlc/loud",
            "c:/users/Example/documents/paradox interactive/the game/mod/alpha",
        ])
}

#[test]
fn requested_discovered_and_mounted_are_carried_as_three_separate_readings() {
    let provenance = read(&ordinary().build());

    assert_eq!(provenance.requested_mods, ["mod/alpha.mod"]);
    assert_eq!(provenance.requested_disabled_dlcs, ["dlc/quiet/quiet.dlc"]);
    assert_eq!(provenance.discovered.len(), 4);
    assert_eq!(provenance.mounted_roots.len(), 3);
    assert_eq!(provenance.mounted_roots[0], "d:/games/hoi4");

    let alpha = entry(&provenance, "mod/alpha.mod");
    assert!(alpha.selected && alpha.mounted);
    let beta = entry(&provenance, "mod/beta.mod");
    assert!(
        !beta.selected && !beta.mounted,
        "a mod on disk that dlc_load.json did not ask for is discovered, not selected"
    );
    let loud = entry(&provenance, "dlc/loud/loud.dlc");
    assert!(loud.selected && loud.mounted);
    let quiet = entry(&provenance, "dlc/quiet/quiet.dlc");
    assert!(
        quiet.selected == false && !quiet.mounted,
        "a disabled DLC stays discovered so its absence is visible rather than silent"
    );
    assert!(!provenance.has_disagreement());
}

#[test]
fn the_two_name_fields_and_the_key_are_read_from_three_distinct_offsets() {
    let image = ordinary()
        .discovered(&[Entry::mod_("mod/alpha.mod", "mod/alpha").canonical("Alpha Mod")])
        .build();
    let provenance = read(&image);
    let alpha = entry(&provenance, "mod/alpha.mod");
    assert_eq!(alpha.map_key, "mod/alpha.mod");
    assert_eq!(alpha.name, "Alpha Mod");
    assert_eq!(alpha.logged_name, "mod/alpha.mod display");
    assert_eq!(alpha.descriptor_entry, "mod/alpha.mod");
}

#[test]
fn every_portable_per_entry_field_is_a_digest_input_on_its_own() {
    let base = Entry::mod_("mod/probe.mod", "mod/probe")
        .based_at("d:/p")
        .canonical("Probe")
        .depends_on(&["Some Prerequisite"])
        .replaces(&["common/units"]);
    let launch = |entry: Entry| {
        Launch::default()
            .requested(&["mod/probe.mod"])
            .discovered(&[entry])
            .mounted(&["d:/p/mod/probe"])
    };
    let baseline = read(&launch(base.clone()).build()).provenance_sha256;

    let mut variants: Vec<(&str, Entry)> = Vec::new();
    let mut with = base.clone();
    with.key = "mod/other_key".into();
    variants.push(("the map key", with));
    variants.push(("the canonical name", base.clone().canonical("Different")));
    let mut with = base.clone();
    with.name = "different logged name".into();
    variants.push(("the logged name", with));
    variants.push((
        "the dependency list",
        base.clone().depends_on(&["Some Other Prerequisite"]),
    ));
    variants.push((
        "the replace-path list",
        base.clone().replaces(&["common/units", "common/countries"]),
    ));

    for (what, entry) in variants {
        assert_ne!(
            read(&launch(entry).build()).provenance_sha256,
            baseline,
            "the digest must move when {what} changes"
        );
    }

    let renamed = Launch::default()
        .requested(&["mod/probe.mod"])
        .discovered(&[base.descriptor("mod/renamed.mod")])
        .mounted(&["d:/p/mod/probe"]);
    assert_ne!(read(&renamed.build()).provenance_sha256, baseline);
}

#[test]
fn a_vector_header_with_a_negative_capacity_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let vector = manager + MANAGER_REQUESTED_MODS;
    image.put_u32(vector + PDX_VECTOR_CAPACITY, u32::MAX);
    image.put_u32(vector + PDX_VECTOR_SIZE, 1);
    assert!(refuse(&image).contains("negative size or capacity"));
}

#[test]
fn a_search_path_root_terminating_just_past_the_bound_is_refused() {
    let mut image = ordinary().build();
    let list = image.alloc(16);
    let text = image.put_unterminated_at_page_offset(4000, MAX_TEXT_BYTES as u64 + 512);
    image.put_u8(text + MAX_TEXT_BYTES as u64 + 8, 0);
    image.put_u64(list, text);
    image.put_u64(list + 8, 0);
    image.put_u64(IMAGE_BASE + VFS_SEARCH_PATH_CACHE_RVA, list);
    assert!(refuse(&image).contains("not NUL-terminated"));
}

#[test]
fn selection_matches_the_descriptor_entry_and_not_the_map_key() {
    let image = Launch::default()
        .requested(&["mod/real_entry.mod"])
        .discovered(&[Entry::mod_("some_other_key", "mod/thing")
            .based_at("d:/p")
            .descriptor("mod/real_entry.mod")])
        .mounted(&["d:/p/mod/thing"])
        .build();
    let provenance = read(&image);
    let found = entry(&provenance, "mod/real_entry.mod");
    assert_eq!(found.map_key, "some_other_key");
    assert!(
        found.selected,
        "selection must key off the descriptor entry the engine itself compares"
    );
    assert!(provenance.requested_mods_not_discovered.is_empty());
}

#[test]
fn a_requested_mod_the_engine_never_discovered_is_reported_as_a_gap() {
    let image = ordinary()
        .requested(&["mod/alpha.mod", "mod/vanished.mod"])
        .build();
    let provenance = read(&image);

    assert_eq!(
        provenance.requested_mods_not_discovered,
        ["mod/vanished.mod"]
    );
    assert!(
        provenance
            .discovered
            .iter()
            .all(|entry| entry.descriptor_entry != "mod/vanished.mod"),
        "the gap must be visible as a request with no discovered counterpart"
    );
    assert!(provenance.has_disagreement());
    assert!(provenance
        .requested_mods
        .contains(&"mod/vanished.mod".to_string()));
}

#[test]
fn a_selected_mod_absent_from_the_search_path_is_reported_as_not_mounted() {
    let image = ordinary()
        .mounted(&["d:/games/hoi4", "d:/games/hoi4/dlc/loud"])
        .build();
    let provenance = read(&image);

    let alpha = entry(&provenance, "mod/alpha.mod");
    assert!(
        alpha.selected && !alpha.mounted,
        "selected and discovered but never mounted is exactly the reportable gap"
    );
    assert_eq!(provenance.selected_not_mounted, ["mod/alpha.mod"]);
    assert!(provenance.has_disagreement());
    assert!(!provenance
        .selected_not_mounted
        .contains(&"mod/beta.mod".to_string()));
}

#[test]
fn a_disabled_dlc_is_discovered_and_not_selected() {
    let provenance = read(&ordinary().disabled(&["dlc/loud/loud.dlc"]).build());
    assert!(!entry(&provenance, "dlc/loud/loud.dlc").selected);
    assert!(entry(&provenance, "dlc/quiet/quiet.dlc").selected);
}

#[test]
fn a_dlc_the_engine_marked_ineligible_is_not_selected_even_when_nothing_disabled_it() {
    let image = ordinary()
        .disabled(&[])
        .discovered(&[Entry::dlc("dlc/loud/loud.dlc", "dlc/loud").ineligible()])
        .build();
    let provenance = read(&image);
    let loud = entry(&provenance, "dlc/loud/loud.dlc");
    assert!(!loud.dlc_eligible);
    assert!(
        !loud.selected,
        "the engine's own eligibility flag must gate selection"
    );
    assert_eq!(provenance.selected_not_mounted, Vec::<String>::new());
}

#[test]
fn a_root_naming_a_different_directory_does_not_mount_an_entry() {
    let image = Launch::default()
        .requested(&["mod/alpha.mod"])
        .discovered(&[Entry::mod_("mod/alpha.mod", "mod/alpha").based_at("d:/p")])
        .mounted(&["d:/p/mod/superalpha"])
        .build();
    assert!(!entry(&read(&image), "mod/alpha.mod").mounted);

    let image = Launch::default()
        .requested(&["mod/alpha.mod"])
        .discovered(&[Entry::mod_("mod/alpha.mod", "mod/alpha").based_at("d:/p")])
        .mounted(&["d:/p/mod/alpha"])
        .build();
    assert!(entry(&read(&image), "mod/alpha.mod").mounted);
}

#[test]
fn an_archive_mod_matches_on_its_archive_rather_than_its_folder() {
    let image = Launch::default()
        .requested(&["mod/zipped.mod"])
        .discovered(&[Entry::mod_("mod/zipped.mod", "mod/never_mounted")
            .based_at("d:/p")
            .archive("mod/zipped.zip")])
        .mounted(&["d:/p/mod/zipped.zip"])
        .build();
    assert!(entry(&read(&image), "mod/zipped.mod").mounted);
}

#[test]
fn an_entry_with_neither_archive_nor_folder_can_never_be_mounted() {
    let image = Launch::default()
        .discovered(&[Entry::dlc("dlc/hollow/hollow.dlc", "")])
        .mounted(&["d:/games/hoi4"])
        .build();
    let provenance = read(&image);
    let hollow = entry(&provenance, "dlc/hollow/hollow.dlc");
    assert!(hollow.path.is_none() && hollow.archive.is_none());
    assert!(!hollow.mounted);
    assert_eq!(provenance.selected_not_mounted, ["dlc/hollow/hollow.dlc"]);
}

#[test]
fn mount_matching_ignores_case_and_separator_style() {
    let image = Launch::default()
        .requested(&["mod/Alpha.mod"])
        .discovered(&[Entry::mod_("mod/Alpha.mod", "Mod\\Alpha").based_at("D:/Games")])
        .mounted(&["D:/Games/Mod/Alpha/"])
        .build();
    assert!(entry(&read(&image), "mod/Alpha.mod").mounted);
}

#[test]
fn the_digest_excludes_machine_local_paths() {
    let here = Launch::default()
        .requested(&["mod/alpha.mod"])
        .discovered(&[Entry::mod_("mod/alpha.mod", "mod/alpha").based_at("c:/one")])
        .mounted(&["c:/one/mod/alpha"])
        .build();
    let there = Launch::default()
        .requested(&["mod/alpha.mod"])
        .discovered(&[Entry::mod_("mod/alpha.mod", "z:/somewhere/else/mod/alpha")])
        .mounted(&["z:/somewhere/else/mod/alpha", "z:/somewhere/else"])
        .build();
    let here = read(&here);
    let there = read(&there);
    assert_ne!(here.mounted_roots, there.mounted_roots);
    assert_ne!(
        here.discovered[0].path, there.discovered[0].path,
        "the local paths must actually differ or this test proves nothing"
    );
    assert!(here.discovered[0].mounted && there.discovered[0].mounted);
    assert_eq!(here.provenance_sha256, there.provenance_sha256);
}

#[test]
fn the_digest_moves_for_every_portable_field_it_covers() {
    let baseline = read(&ordinary().build()).provenance_sha256;
    let cases: Vec<(&str, Launch)> = vec![
        (
            "a different requested mod",
            ordinary().requested(&["mod/other.mod"]),
        ),
        (
            "requested mods in a different order",
            ordinary().requested(&["mod/alpha.mod", "mod/second.mod"]),
        ),
        (
            "a different disabled DLC",
            ordinary().disabled(&["dlc/loud/loud.dlc"]),
        ),
        (
            "one fewer discovered entry",
            ordinary().discovered(&[
                Entry::dlc("dlc/loud/loud.dlc", "dlc/loud"),
                Entry::mod_("mod/alpha.mod", "mod/alpha"),
            ]),
        ),
        (
            "a mod that no longer mounts",
            ordinary().mounted(&["d:/games/hoi4", "d:/games/hoi4/dlc/loud"]),
        ),
    ];
    for (what, launch) in cases {
        assert_ne!(
            read(&launch.build()).provenance_sha256,
            baseline,
            "the digest must move when {what}"
        );
    }
}

#[test]
fn requested_order_is_in_the_preimage_and_tree_shape_is_not() {
    let one = ordinary().requested(&["mod/alpha.mod", "mod/beta.mod"]);
    let mut swapped = one.clone();
    swapped.requested_mods.swap(0, 1);
    assert_ne!(
        read(&one.build()).provenance_sha256,
        read(&swapped.build()).provenance_sha256
    );

    let balanced = one.clone().balanced();
    assert_eq!(
        read(&one.build()).provenance_sha256,
        read(&balanced.build()).provenance_sha256,
        "roster identity must not depend on how the tree happens to be shaped"
    );
    assert_eq!(
        read(&one.build()).discovered,
        read(&balanced.build()).discovered
    );
}

#[test]
fn a_balanced_and_a_spine_shaped_map_read_the_same_entries() {
    let spine = read(&ordinary().build());
    let balanced = read(&ordinary().balanced().build());
    assert_eq!(spine.discovered.len(), 4);
    assert_eq!(spine.discovered, balanced.discovered);
}

#[test]
fn a_map_whose_in_order_walk_is_not_strictly_increasing_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let head = u64::from_le_bytes(
        image
            .read(manager + MANAGER_DISCOVERED_DLC_MAP + MAP_HEAD_POINTER, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let first = u64::from_le_bytes(image.read(head + NODE_LEFT, 8).unwrap().try_into().unwrap());
    image.put_string(first + NODE_KEY, "zzz_last");
    assert!(refuse(&image).contains("not strictly increasing"));
}

#[test]
fn a_sentinel_that_does_not_bracket_its_nodes_is_refused() {
    for link in [NODE_LEFT, NODE_RIGHT] {
        let mut image = ordinary().build();
        let manager = u64::from_le_bytes(
            image
                .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let head = u64::from_le_bytes(
            image
                .read(manager + MANAGER_DISCOVERED_DLC_MAP + MAP_HEAD_POINTER, 8)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        image.put_u64(head + link, head + 0x2e0);
        assert!(
            refuse(&image).contains("does not bracket"),
            "sentinel link at +0x{link:x} must be checked"
        );
    }
}

#[test]
fn a_dlc_and_a_mod_sharing_a_key_are_two_entries_not_a_collision() {
    let image = Launch::default()
        .discovered(&[
            Entry::dlc("shared", "dlc/shared").based_at("d:/p"),
            Entry::mod_("shared", "mod/shared").based_at("d:/p"),
        ])
        .mounted(&["d:/p"])
        .build();
    assert_eq!(read(&image).discovered.len(), 2);
}

#[test]
fn a_manager_that_does_not_exist_yet_is_refused() {
    let mut image = ordinary().build();
    image.put_u64(IMAGE_BASE + DLC_MANAGER_RVA, 0);
    assert!(refuse(&image).contains("not constructed"));
}

#[test]
fn an_unpopulated_search_path_is_refused_rather_than_read_as_nothing_mounted() {
    let mut image = ordinary().build();
    image.put_u64(IMAGE_BASE + VFS_SEARCH_PATH_CACHE_RVA, 0);
    assert!(refuse(&image).contains("has not been populated"));
}

#[test]
fn a_map_that_counts_more_nodes_than_it_holds_is_refused() {
    let launch = ordinary();
    let mut image = launch.build();
    let manager = image.read(IMAGE_BASE + DLC_MANAGER_RVA, 8).unwrap();
    let manager = u64::from_le_bytes(manager.try_into().unwrap());
    image.put_u64(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_SIZE, 9);
    assert!(refuse(&image).contains("counts"));
}

fn mod_map_root(image: &Image) -> u64 {
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let head = u64::from_le_bytes(
        image
            .read(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_HEAD_POINTER, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    u64::from_le_bytes(
        image
            .read(head + NODE_PARENT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

#[test]
fn a_right_cycle_is_refused() {
    let mut image = ordinary().build();
    let root = mod_map_root(&image);
    image.put_u64(root + NODE_RIGHT, root);
    let error = refuse(&image);
    assert!(
        error.contains("does not point back at its parent")
            || error.contains("more nodes than it counts"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn a_map_holding_more_nodes_than_it_counts_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u64(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_SIZE, 1);
    assert!(refuse(&image).contains("more nodes than it counts"));
}

#[test]
fn a_left_cycle_is_refused_by_the_stack_bound() {
    let mut image = ordinary().build();
    let root = mod_map_root(&image);
    image.put_u64(root + NODE_LEFT, root);
    assert!(refuse(&image).contains("exceeded its node count"));
}

#[test]
fn a_heap_string_pointing_nowhere_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let vector = manager + MANAGER_REQUESTED_MODS;
    let data = u64::from_le_bytes(image.read(vector, 8).unwrap().try_into().unwrap());
    image.put_u64(data + STD_STRING_CAPACITY_OFFSET as u64, 64);
    image.put_u64(data + STD_STRING_SIZE_OFFSET as u64, 20);
    image.put_u64(data, 0);
    assert!(refuse(&image).contains("null heap buffer"));
}

#[test]
fn a_vector_whose_size_exceeds_its_capacity_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u32(manager + MANAGER_REQUESTED_MODS + PDX_VECTOR_SIZE, 7);
    assert!(refuse(&image).contains("exceeds its capacity"));
}

#[test]
fn a_search_path_entry_that_never_terminates_is_refused() {
    let mut image = ordinary().build();
    let list = image.alloc(16);
    let text = image.put_unterminated(MAX_TEXT_BYTES as u64 + 1_024);
    image.put_u64(list, text);
    image.put_u64(list + 8, 0);
    image.put_u64(IMAGE_BASE + VFS_SEARCH_PATH_CACHE_RVA, list);
    assert!(refuse(&image).contains("not NUL-terminated"));
}

#[test]
fn a_search_path_longer_than_the_bound_is_refused() {
    let mut image = ordinary().build();
    let count = MAX_SEARCH_PATH_ROOTS as u64 + 8;
    let list = image.alloc((count + 1) * 8);
    let text = image.put_c_string("d:/p");
    for index in 0..count {
        image.put_u64(list + index * 8, text);
    }
    image.put_u64(list + count * 8, 0);
    image.put_u64(IMAGE_BASE + VFS_SEARCH_PATH_CACHE_RVA, list);
    assert!(refuse(&image).contains("unbounded"));
}

#[test]
fn an_sso_string_and_a_heap_string_of_the_same_text_read_identically() {
    let short = "0123456789abcde";
    let long = "0123456789abcdef";
    for text in [short, long] {
        let image = Launch::default()
            .requested(&[text])
            .discovered(&[])
            .mounted(&["d:/p"])
            .build();
        assert_eq!(read(&image).requested_mods, [text.to_string()]);
    }
}

struct Flipping {
    first: Image,
    second: Image,
    passes: Cell<u32>,
}

impl TargetImageMemory for Flipping {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if address == IMAGE_BASE + DLC_MANAGER_RVA {
            self.passes.set(self.passes.get() + 1);
        }
        if self.passes.get() <= 1 {
            self.first.read(address, length)
        } else {
            self.second.read(address, length)
        }
    }
}

#[test]
fn a_roster_that_changes_between_the_two_passes_is_refused() {
    let flipping = Flipping {
        first: ordinary().build(),
        second: ordinary()
            .requested(&["mod/alpha.mod", "mod/late.mod"])
            .build(),
        passes: Cell::new(0),
    };
    let error = read_engine_modset_provenance(&flipping, IMAGE_BASE)
        .expect_err("a roster that moved mid-read must be refused");
    assert!(
        error.contains("changed between two consecutive reads"),
        "got: {error}"
    );
    assert_eq!(flipping.passes.get(), 2, "both passes must actually run");
}

#[test]
fn two_passes_over_an_unchanging_roster_agree() {
    let stable = ordinary().build();
    let flipping = Flipping {
        first: stable.clone(),
        second: stable,
        passes: Cell::new(0),
    };
    assert!(read_engine_modset_provenance(&flipping, IMAGE_BASE).is_ok());
    assert_eq!(flipping.passes.get(), 2);
}

#[test]
fn the_rvas_are_the_pinned_absolute_addresses_less_the_image_base() {
    assert_eq!(DLC_MANAGER_VA_A729, 0x1_4343_1608);
    assert_eq!(VFS_SEARCH_PATH_CACHE_VA_A729, 0x1_435c_af10);
    assert_eq!(DLC_MANAGER_RVA, 0x343_1608);
    assert_eq!(VFS_SEARCH_PATH_CACHE_RVA, 0x35c_af10);
    assert_eq!(
        A729_PREFERRED_IMAGE_BASE + DLC_MANAGER_RVA,
        DLC_MANAGER_VA_A729
    );
}

#[test]
fn the_kind_tag_follows_the_engines_own_equality_test() {
    assert_eq!(EngineContentKind::from_raw(1), EngineContentKind::Dlc);
    assert_eq!(EngineContentKind::from_raw(0), EngineContentKind::Mod);
    assert_eq!(EngineContentKind::from_raw(2), EngineContentKind::Mod);
    assert_eq!(EngineContentKind::from_raw(-9), EngineContentKind::Mod);
}

#[test]
fn two_entries_sharing_a_relative_path_under_different_bases_are_told_apart() {
    let image = Launch::default()
        .requested(&["mod/one.mod", "mod/two.mod"])
        .discovered(&[
            Entry::mod_("mod/one.mod", "shared").based_at("d:/one"),
            Entry::mod_("mod/two.mod", "shared").based_at("d:/two"),
        ])
        .mounted(&["d:/one/shared"])
        .build();
    let provenance = read(&image);
    assert!(entry(&provenance, "mod/one.mod").mounted);
    assert!(
        !entry(&provenance, "mod/two.mod").mounted,
        "an entry under a different base has a different mount location"
    );
    assert_eq!(provenance.selected_not_mounted, ["mod/two.mod"]);
}

#[test]
fn the_engine_join_reproduces_its_three_branches() {
    assert_eq!(engine_join("", "mod/alpha"), "mod/alpha");
    assert_eq!(engine_join("d:/profile", ""), "d:/profile");
    assert_eq!(
        engine_join("d:/profile", "mod/alpha"),
        "d:/profile/mod/alpha"
    );
    assert_eq!(
        engine_join("d:/profile/", "mod/alpha"),
        "d:/profile/mod/alpha"
    );
    assert_eq!(
        engine_join("d:\\profile\\", "mod/alpha"),
        "d:\\profile\\mod/alpha",
        "a trailing backslash is already a separator and must not gain a second"
    );
}

#[test]
fn a_map_counting_zero_while_its_sentinel_still_points_somewhere_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u64(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_SIZE, 0);
    assert!(refuse(&image).contains("sentinel is not self-referential"));
}

#[test]
fn an_empty_map_whose_sentinel_links_are_not_all_self_referential_is_refused() {
    for link in [NODE_LEFT, NODE_PARENT, NODE_RIGHT] {
        let mut image = Launch::default()
            .discovered(&[Entry::dlc("dlc/only/only.dlc", "dlc/only").based_at("d:/p")])
            .mounted(&["d:/p/dlc/only"])
            .build();
        let manager = u64::from_le_bytes(
            image
                .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let head = u64::from_le_bytes(
            image
                .read(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_HEAD_POINTER, 8)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        image.put_u64(head + link, head + 0x2e0);
        assert!(
            refuse(&image).contains("sentinel is not self-referential"),
            "link at +0x{link:x} must be checked"
        );
    }
}

#[test]
fn a_root_that_merely_ends_with_a_location_does_not_mount_it() {
    let image = Launch::default()
        .requested(&["mod/short.mod"])
        .discovered(&[Entry::mod_("mod/short.mod", "shared").based_at("a")])
        .mounted(&["x/a/shared"])
        .build();
    let provenance = read(&image);
    assert!(!entry(&provenance, "mod/short.mod").mounted);
    assert_eq!(provenance.selected_not_mounted, ["mod/short.mod"]);
}

#[test]
fn an_absolute_folder_is_matched_unjoined_as_the_engine_mounts_it() {
    let image = Launch::default()
        .requested(&["mod/abs.mod"])
        .discovered(&[Entry::mod_("mod/abs.mod", "c:/mods/absolute").based_at("d:/base")])
        .mounted(&["c:/mods/absolute"])
        .build();
    let provenance = read(&image);
    assert!(entry(&provenance, "mod/abs.mod").mounted);
    assert!(provenance.selected_not_mounted.is_empty());
}

#[test]
fn a_relative_folder_is_matched_joined_onto_its_base() {
    let image = Launch::default()
        .requested(&["mod/rel.mod"])
        .discovered(&[Entry::mod_("mod/rel.mod", "mod/rel").based_at("d:/profile")])
        .mounted(&["d:/profile/mod/rel"])
        .build();
    assert!(entry(&read(&image), "mod/rel.mod").mounted);
}

#[test]
fn two_entries_at_byte_identical_locations_are_both_mounted() {
    let image = Launch::default()
        .requested(&["mod/first.mod", "mod/second.mod"])
        .discovered(&[
            Entry::mod_("mod/first.mod", "shared").based_at("d:/p"),
            Entry::mod_("mod/second.mod", "shared").based_at("d:/p"),
        ])
        .mounted(&["d:/p/shared"])
        .build();
    let provenance = read(&image);
    assert!(entry(&provenance, "mod/first.mod").mounted);
    assert!(entry(&provenance, "mod/second.mod").mounted);
    assert!(provenance.selected_not_mounted.is_empty());
}

#[test]
fn a_map_head_that_is_not_a_sentinel_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let head = u64::from_le_bytes(
        image
            .read(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_HEAD_POINTER, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u8(head + NODE_IS_NIL, 0);
    assert!(refuse(&image).contains("head is not a sentinel"));
}

#[test]
fn an_empty_map_with_a_self_referential_sentinel_is_accepted() {
    let image = Launch::default()
        .discovered(&[Entry::dlc("dlc/only/only.dlc", "dlc/only")])
        .mounted(&["d:/p/dlc/only"])
        .build();
    let provenance = read(&image);
    assert_eq!(
        provenance.discovered.len(),
        1,
        "the mod map is legitimately empty"
    );
}

#[test]
fn the_entry_kind_is_a_digest_input() {
    let as_dlc = Launch::default()
        .discovered(&[Entry::dlc("thing", "loc").based_at("d:/p")])
        .mounted(&["d:/p/loc"]);
    let as_mod = Launch::default()
        .discovered(&[Entry::mod_("thing", "loc").based_at("d:/p")])
        .mounted(&["d:/p/loc"]);
    let dlc = read(&as_dlc.build());
    let modded = read(&as_mod.build());
    assert_eq!(dlc.discovered[0].map_key, modded.discovered[0].map_key);
    assert_eq!(dlc.discovered[0].mounted, modded.discovered[0].mounted);
    assert_ne!(dlc.discovered[0].kind, modded.discovered[0].kind);
    assert_ne!(dlc.provenance_sha256, modded.provenance_sha256);
}

#[test]
fn a_roster_exceeding_the_whole_read_text_budget_is_refused() {
    let per_string = MAX_TEXT_BYTES;
    let each = (MAX_TOTAL_TEXT_BYTES * 3 / 5) / per_string;
    let deps: Vec<String> = (0..each).map(|_| "x".repeat(per_string)).collect();
    let dep_refs: Vec<&str> = deps.iter().map(String::as_str).collect();
    let root_len = per_string - 256;
    let root_count = (MAX_TOTAL_TEXT_BYTES * 3 / 5) / root_len;
    let roots: Vec<String> = (0..root_count).map(|_| "r".repeat(root_len)).collect();
    let root_refs: Vec<&str> = roots.iter().map(String::as_str).collect();

    let mut heavy = Entry::mod_("mod/heavy.mod", "heavy").based_at("d:/p");
    heavy.dependencies = dep_refs.iter().map(|value| value.to_string()).collect();
    let image = Launch::default()
        .discovered(&[heavy])
        .mounted(&root_refs)
        .build();
    let error = refuse(&image);
    assert!(
        error.contains("whole-read text budget"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn the_budget_counts_both_strings_and_bytes() {
    let budget = Budget::default();
    assert!(budget.charge(16, "probe").is_ok());
    assert!(
        budget.charge(MAX_TOTAL_TEXT_BYTES, "probe").is_err(),
        "the byte cap is cumulative, not per-string"
    );

    let budget = Budget::default();
    for _ in 0..MAX_TOTAL_STRINGS {
        budget.charge(0, "probe").expect("within the string cap");
    }
    assert!(
        budget.charge(0, "probe").is_err(),
        "zero-length strings still count against the string cap"
    );
}

#[test]
fn an_ordinary_roster_stays_far_inside_the_budget() {
    let entries: Vec<Entry> = (0..64)
        .map(|index| {
            Entry::mod_(&format!("mod/ugc_{index}.mod"), &format!("mod/ugc_{index}"))
                .based_at("d:/p")
                .depends_on(&["Some Prerequisite", "Another One"])
                .replaces(&["common/units", "common/countries", "history/states"])
        })
        .collect();
    let roots: Vec<String> = (0..64)
        .map(|index| format!("d:/p/mod/ugc_{index}"))
        .collect();
    let root_refs: Vec<&str> = roots.iter().map(String::as_str).collect();
    let image = Launch::default()
        .discovered(&entries)
        .mounted(&root_refs)
        .build();
    assert_eq!(read(&image).discovered.len(), 64);
}

#[test]
fn an_unselected_entrys_mount_state_still_moves_the_roster_identity() {
    let unmounted = Launch::default()
        .discovered(&[Entry::mod_("mod/spare.mod", "mod/spare").based_at("d:/p")])
        .mounted(&["d:/p"]);
    let mounted = unmounted.clone().mounted(&["d:/p", "d:/p/mod/spare"]);
    let before = read(&unmounted.build());
    let after = read(&mounted.build());

    assert!(!before.discovered[0].selected && !before.discovered[0].mounted);
    assert!(!after.discovered[0].selected && after.discovered[0].mounted);
    assert_eq!(
        (
            &before.selected_not_mounted,
            &before.requested_mods_not_discovered
        ),
        (
            &after.selected_not_mounted,
            &after.requested_mods_not_discovered
        ),
        "the summary lists must be identical, or this proves nothing about the per-entry byte"
    );
    assert_ne!(before.provenance_sha256, after.provenance_sha256);
}

#[test]
fn the_engines_eligibility_flag_moves_the_roster_identity_on_its_own() {
    let disabled = &["dlc/x/x.dlc"];
    let eligible = Launch::default()
        .disabled(disabled)
        .discovered(&[Entry::dlc("dlc/x/x.dlc", "dlc/x").based_at("d:/p")])
        .mounted(&["d:/p/dlc/x"]);
    let ineligible = Launch::default()
        .disabled(disabled)
        .discovered(&[Entry::dlc("dlc/x/x.dlc", "dlc/x")
            .ineligible()
            .based_at("d:/p")])
        .mounted(&["d:/p/dlc/x"]);
    let before = read(&eligible.build());
    let after = read(&ineligible.build());

    assert!(before.discovered[0].dlc_eligible && !after.discovered[0].dlc_eligible);
    assert_eq!(
        (before.discovered[0].selected, before.discovered[0].mounted),
        (after.discovered[0].selected, after.discovered[0].mounted),
        "selection and mount state must be identical or this proves nothing about eligibility"
    );
    assert_eq!(
        before.requested_disabled_dlcs,
        after.requested_disabled_dlcs
    );
    assert_ne!(before.provenance_sha256, after.provenance_sha256);
}

#[test]
fn selection_is_recomputable_from_what_the_digest_already_covers() {
    let provenance = read(&ordinary().build());
    assert!(provenance.discovered.iter().any(|entry| entry.selected));
    assert!(provenance.discovered.iter().any(|entry| !entry.selected));
    for entry in &provenance.discovered {
        let expected = match entry.kind {
            EngineContentKind::Dlc => {
                entry.dlc_eligible
                    && !provenance
                        .requested_disabled_dlcs
                        .contains(&entry.descriptor_entry)
            }
            EngineContentKind::Mod => provenance.requested_mods.contains(&entry.descriptor_entry),
        };
        assert_eq!(entry.selected, expected, "for {}", entry.descriptor_entry);
    }
}

fn mod_map_head(image: &Image) -> u64 {
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    u64::from_le_bytes(
        image
            .read(manager + MANAGER_DISCOVERED_MOD_MAP + MAP_HEAD_POINTER, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

#[test]
fn a_root_not_linked_back_to_its_sentinel_is_refused() {
    let mut image = ordinary().build();
    let head = mod_map_head(&image);
    let root = u64::from_le_bytes(
        image
            .read(head + NODE_PARENT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u64(root + NODE_PARENT, root);
    assert!(refuse(&image).contains("not linked back to its sentinel"));
}

#[test]
fn a_child_not_pointing_back_at_its_parent_is_refused() {
    let mut image = ordinary().build();
    let head = mod_map_head(&image);
    let root = u64::from_le_bytes(
        image
            .read(head + NODE_PARENT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let child = u64::from_le_bytes(
        image
            .read(root + NODE_RIGHT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u64(child + NODE_PARENT, head);
    assert!(refuse(&image).contains("does not point back at its parent"));
}

#[test]
fn a_node_colour_outside_red_and_black_is_refused() {
    let mut image = ordinary().build();
    let head = mod_map_head(&image);
    let root = u64::from_le_bytes(
        image
            .read(head + NODE_PARENT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u8(root + NODE_COLOUR, 0x7f);
    assert!(refuse(&image).contains("colour is not red or black"));
}

#[test]
fn the_absoluteness_predicate_matches_the_engines_own_test() {
    for absolute in [
        "/mods/x",
        "\\\\mods\\\\x",
        "c:/mods/x",
        "Z:\\\\mods",
        "?:/odd",
    ] {
        assert!(is_engine_absolute(absolute), "{absolute:?} is absolute");
    }
    for relative in ["", "mod/rel", "c:mods", "c/:mods", ":", ":/x", "cc:/x"] {
        assert!(
            !is_engine_absolute(relative),
            "{relative:?} is not absolute"
        );
    }
}

#[test]
fn a_relative_folder_is_not_offered_as_an_unjoined_candidate() {
    let image = Launch::default()
        .requested(&["mod/rel.mod"])
        .discovered(&[Entry::mod_("mod/rel.mod", "mod/rel").based_at("d:/profile")])
        .mounted(&["mod/rel"])
        .build();
    let provenance = read(&image);
    assert!(!entry(&provenance, "mod/rel.mod").mounted);
    assert_eq!(provenance.selected_not_mounted, ["mod/rel.mod"]);
}

#[test]
fn an_absolute_dlc_folder_is_only_ever_matched_joined() {
    let image = Launch::default()
        .discovered(&[Entry::dlc("dlc/abs/abs.dlc", "c:/dlc/abs").based_at("d:/games")])
        .mounted(&["c:/dlc/abs"])
        .build();
    let provenance = read(&image);
    assert!(!entry(&provenance, "dlc/abs/abs.dlc").mounted);
    assert_eq!(provenance.selected_not_mounted, ["dlc/abs/abs.dlc"]);

    let image = Launch::default()
        .discovered(&[Entry::dlc("dlc/abs/abs.dlc", "c:/dlc/abs").based_at("d:/games")])
        .mounted(&["d:/games/c:/dlc/abs"])
        .build();
    assert!(entry(&read(&image), "dlc/abs/abs.dlc").mounted);
}

#[test]
fn a_relative_mod_archive_is_matched_unjoined_through_the_native_fallback() {
    let image = Launch::default()
        .requested(&["mod/rel.mod"])
        .discovered(&[Entry::mod_("mod/rel.mod", "unused")
            .based_at("d:/profile")
            .archive("mod/rel.zip")])
        .mounted(&["mod/rel.zip"])
        .build();
    let provenance = read(&image);
    assert!(entry(&provenance, "mod/rel.mod").mounted);
    assert!(provenance.selected_not_mounted.is_empty());
}

#[test]
fn a_null_child_link_is_refused_rather_than_read_as_absent() {
    let mut image = ordinary().build();
    let head = mod_map_head(&image);
    let root = u64::from_le_bytes(
        image
            .read(head + NODE_PARENT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_u64(root + NODE_LEFT, 0);
    assert!(refuse(&image).contains("neither the sentinel nor a live node"));
}

#[test]
fn a_child_link_to_some_other_nil_node_is_refused() {
    let mut image = ordinary().build();
    let head = mod_map_head(&image);
    let root = u64::from_le_bytes(
        image
            .read(head + NODE_PARENT, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let impostor = image.alloc(0x2e0);
    image.put_u8(impostor + NODE_IS_NIL, 1);
    image.put_u64(root + NODE_LEFT, impostor);
    assert!(refuse(&image).contains("neither the sentinel nor a live node"));
}

#[test]
fn a_nil_flag_outside_zero_and_one_is_refused() {
    let mut image = ordinary().build();
    let head = mod_map_head(&image);
    image.put_u8(head + NODE_IS_NIL, 0x5a);
    assert!(refuse(&image).contains("nil flag is not a boolean"));
}

#[test]
fn an_engine_string_with_an_embedded_nul_is_refused() {
    let mut image = ordinary().build();
    let manager = u64::from_le_bytes(
        image
            .read(IMAGE_BASE + DLC_MANAGER_RVA, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let data = u64::from_le_bytes(
        image
            .read(manager + MANAGER_REQUESTED_MODS, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    image.put_string(data, "mod/alpha.mod\0tail");
    assert!(refuse(&image).contains("embedded NUL"));
}
