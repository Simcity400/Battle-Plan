
use map_viewer::GameFileResolver;
use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
use portable_content_manifest::{ContentLayerKind, LayerStorageKind};
use std::collections::BTreeMap;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Winner {
        ordinal: u32,
        kind: ContentLayerKind,
        layer_id: String,
        source_sha256: String,
        payload_sha256: String,
        bytes: u64,
    },
    Absent,
    Refused(String),
}

impl Outcome {
    fn shape(&self) -> &'static str {
        match self {
            Outcome::Winner { .. } => "winner",
            Outcome::Absent => "absent",
            Outcome::Refused(_) => "refused",
        }
    }
}

fn layer_kind_of(kind: &str) -> ContentLayerKind {
    match kind {
        "base" => ContentLayerKind::Base,
        "dlc" => ContentLayerKind::Dlc,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        _ => ContentLayerKind::Mod,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn winner_of(outcome: &Outcome) -> (u32, String) {
    match outcome {
        Outcome::Winner {
            ordinal,
            payload_sha256,
            ..
        } => (*ordinal, payload_sha256.clone()),
        other => panic!("expected a winner, got {other:?}"),
    }
}

fn resolve_a(resolver: &GameFileResolver, path: &str) -> Outcome {
    match resolver.resolve_with_provenance(path) {
        Ok(Some(resolved)) => Outcome::Winner {
            ordinal: resolved.source.ordinal as u32,
            kind: layer_kind_of(&resolved.source.kind),
            layer_id: resolved.source.source_id.clone(),
            source_sha256: resolved
                .source
                .archive_sha256
                .clone()
                .unwrap_or_else(|| resolved.sha256.clone()),
            payload_sha256: resolved.sha256.clone(),
            bytes: resolved.byte_len,
        },
        Ok(None) => Outcome::Absent,
        Err(error) => Outcome::Refused(error.to_string()),
    }
}

fn resolve_b(vfs: &EffectiveVfs, path: &str) -> Outcome {
    match vfs.resolve(path) {
        Ok(Some(file)) => Outcome::Winner {
            ordinal: file.content.winning_layer_ordinal,
            kind: file.content.layer_kind,
            layer_id: file.content.layer_id.clone(),
            source_sha256: file.content.source_sha256.clone(),
            payload_sha256: file.content.payload_sha256.clone(),
            bytes: file.content.payload_bytes,
        },
        Ok(None) => Outcome::Absent,
        Err(error) => Outcome::Refused(error),
    }
}

fn write_bytes(root: &Path, rel: &str, bytes: &[u8]) {
    let target = rel
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part));
    fs::create_dir_all(target.parent().expect("a parent")).expect("create fixture parent");
    fs::write(target, bytes).expect("write fixture file");
}

fn write_descriptor(root: &Path, name: &str, body: &str) {
    let text = format!("name=\"{name}\"\n{body}");
    let latin1 = text
        .chars()
        .map(|ch| {
            let code = ch as u32;
            assert!(
                code <= 0xFF,
                "fixture descriptor char {ch:?} (U+{code:04X}) is not representable in Latin-1, \
                 which is the encoding the resolver reads descriptors in"
            );
            code as u8
        })
        .collect::<Vec<u8>>();
    write_bytes(root, "descriptor.mod", &latin1);
}

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "la_two_vfs_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn matched_pair(vanilla: &Path, mods: &[(&str, PathBuf)]) -> (GameFileResolver, EffectiveVfs) {
    matched_pair_with_replace(vanilla, mods, &[])
}

fn assert_fixture_root(root: &Path) {
    let temp = std::env::temp_dir();
    let canonical_temp = fs::canonicalize(&temp).unwrap_or(temp);
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    assert!(
        canonical_root.starts_with(&canonical_temp),
        "refusing to write a fixture marker into {} — it is not under the process temp root {}. \
         Resolving against caller-owned content (a real the game install) must go through \
         matched_pair_readonly, which never writes.",
        canonical_root.display(),
        canonical_temp.display()
    );
}

fn matched_pair_readonly(vanilla: &Path) -> (GameFileResolver, EffectiveVfs) {
    let resolver = GameFileResolver::vanilla(vanilla.to_path_buf()).expect("build implementation A");
    let sources = resolver.active_sources();
    assert_eq!(
        sources.len(),
        1,
        "read-only pairing supports a single base layer only: {sources:#?}"
    );
    assert_eq!(sources[0].ordinal, 0, "base layer is not ordinal zero");

    let vfs = EffectiveVfs::new(
        "a".repeat(64),
        [EffectiveVfsLayerMount {
            ordinal: sources[0].ordinal as u32,
            layer_kind: layer_kind_of(&sources[0].kind),
            layer_id: sources[0].source_id.clone(),
            storage_kind: LayerStorageKind::Directory,
            local_source: vanilla.to_path_buf(),
            source_container_sha256: None,
            replace_paths: Vec::new(),
        }],
    )
    .expect("build implementation B");
    (resolver, vfs)
}

fn matched_pair_with_replace(
    vanilla: &Path,
    mods: &[(&str, PathBuf)],
    replace: &[(usize, &str)],
) -> (GameFileResolver, EffectiveVfs) {
    // OWNERSHIP FIRST. Everything below this line may write, so every root is
    assert_fixture_root(vanilla);
    for (_, path) in mods {
        assert_fixture_root(path);
    }

    for (index, (name, path)) in mods.iter().enumerate() {
        let declared = replace
            .iter()
            .filter(|(at, _)| *at == index)
            .map(|(_, rel)| format!("replace_path=\"{rel}\"\n"))
            .collect::<String>();
        if !declared.is_empty() {
            write_descriptor(path, name, &declared);
        }
    }

    let resolver = GameFileResolver::with_mod_paths(
        vanilla.to_path_buf(),
        mods.iter().map(|(_, path)| path.clone()).collect(),
    )
    .expect("build implementation A");

    let sources = resolver.active_sources();
    assert_eq!(
        sources.len(),
        mods.len() + 1,
        "implementation A mounted {} layers for base + {} mods: {sources:#?}",
        sources.len(),
        mods.len()
    );
    assert_eq!(sources[0].ordinal, 0, "base layer is not ordinal zero");
    for (index, _) in mods.iter().enumerate() {
        assert_eq!(
            sources[index + 1].ordinal,
            index + 1,
            "implementation A reordered or renumbered the mod layers: {sources:#?}"
        );
    }

    let mut mounts = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let local_source = if index == 0 {
            vanilla.to_path_buf()
        } else {
            mods[index - 1].1.clone()
        };
        let replace_paths = replace
            .iter()
            .filter(|(at, _)| *at + 1 == index)
            .map(|(_, path)| (*path).to_string())
            .collect::<Vec<_>>();
        mounts.push(EffectiveVfsLayerMount {
            ordinal: source.ordinal as u32,
            layer_kind: layer_kind_of(&source.kind),
            layer_id: source.source_id.clone(),
            storage_kind: LayerStorageKind::Directory,
            local_source,
            source_container_sha256: None,
            replace_paths,
        });
    }
    let vfs = EffectiveVfs::new("a".repeat(64), mounts).expect("build implementation B");

    for (index, root) in std::iter::once(vanilla)
        .chain(mods.iter().map(|(_, path)| path.as_path()))
        .enumerate()
    {
        assert_fixture_root(root);
        let marker = format!("la_layer_marker_{index}.txt");
        let payload = format!("layer-{index}");
        write_bytes(root, &marker, payload.as_bytes());
        let a = resolve_a(&resolver, &marker);
        let b = resolve_b(&vfs, &marker);
        let expected_payload = sha256_hex(payload.as_bytes());
        match (&a, &b) {
            (
                Outcome::Winner {
                    ordinal: ao,
                    payload_sha256: ap,
                    ..
                },
                Outcome::Winner {
                    ordinal: bo,
                    payload_sha256: bp,
                    ..
                },
            ) => {
                assert_eq!(
                    (*ao as usize, ap.as_str()),
                    (index, expected_payload.as_str()),
                    "implementation A bound ordinal {ao} to the wrong root for layer {index}"
                );
                assert_eq!(
                    (*bo as usize, bp.as_str()),
                    (index, expected_payload.as_str()),
                    "implementation B bound ordinal {bo} to the wrong root for layer {index}"
                );
            }
            _ => panic!("layer {index} marker did not resolve in both: A={a:?} B={b:?}"),
        }
    }

    (resolver, vfs)
}

fn differential(
    resolver: &GameFileResolver,
    vfs: &EffectiveVfs,
    corpus: &[&str],
) -> BTreeMap<String, (Outcome, Outcome)> {
    let mut disagreements = BTreeMap::new();
    for path in corpus {
        let a = resolve_a(resolver, path);
        let b = resolve_b(vfs, path);
        if a != b {
            disagreements.insert((*path).to_string(), (a, b));
        }
    }
    disagreements
}

fn assert_agrees(resolver: &GameFileResolver, vfs: &EffectiveVfs, corpus: &[&str]) {
    let disagreements = differential(resolver, vfs, corpus);
    assert!(
        disagreements.is_empty(),
        "the two layered-VFS implementations disagree on {} of {} paths:\n{}",
        disagreements.len(),
        corpus.len(),
        disagreements
            .iter()
            .map(|(path, (a, b))| format!(
                "  {path}\n    A (map_viewer::mod_resolver) = {a:?}\n    B (effective_vfs)          = {b:?}"
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let refused = corpus
        .iter()
        .filter_map(|path| {
            let a = resolve_a(resolver, path);
            let b = resolve_b(vfs, path);
            (a.shape() == "refused" || b.shape() == "refused")
                .then(|| format!("  {path}\n    A = {a:?}\n    B = {b:?}"))
        })
        .collect::<Vec<_>>();
    assert!(
        refused.is_empty(),
        "an agreement corpus must not contain refusals — two refusals with different causes are \
         not agreement:\n{}",
        refused.join("\n")
    );
}

fn assert_corpus_resolved(
    resolver: &GameFileResolver,
    vfs: &EffectiveVfs,
    corpus: &[&str],
    minimum_winners: usize,
) {
    let mut a_winners = 0usize;
    let mut b_winners = 0usize;
    for path in corpus {
        if matches!(resolve_a(resolver, path), Outcome::Winner { .. }) {
            a_winners += 1;
        }
        if matches!(resolve_b(vfs, path), Outcome::Winner { .. }) {
            b_winners += 1;
        }
    }
    assert!(
        a_winners >= minimum_winners && b_winners >= minimum_winners,
        "differential corpus is vacuous: implementation A resolved {a_winners} and B resolved \
         {b_winners} of {} paths to a winner, need at least {minimum_winners} on each side",
        corpus.len()
    );
}


#[test]
fn agreement_on_the_ordinary_layered_corpus() {
    let root = temp_root("ordinary");
    let vanilla = root.join("vanilla");
    let mod_low = root.join("mod_low");
    let mod_high = root.join("mod_high");

    write_bytes(&vanilla, "map/definition.csv", b"vanilla-definition");
    write_bytes(&vanilla, "common/countries/colors.txt", b"vanilla-colors");
    write_bytes(
        &vanilla,
        "gfx/interface/deep/deeper/deepest/icon.dds",
        b"vanilla-deep",
    );
    write_bytes(&vanilla, "interface/OnlyVanilla.gfx", b"vanilla-only");

    write_bytes(&mod_low, "common/countries/colors.txt", b"low-colors");
    write_bytes(&mod_low, "common/countries/low_only.txt", b"low-only");

    write_bytes(&mod_high, "common/countries/colors.txt", b"high-colors");
    write_bytes(
        &mod_high,
        "gfx/interface/deep/deeper/deepest/icon.dds",
        b"high-deep",
    );

    let (resolver, vfs) = matched_pair(
        &vanilla,
        &[("low", mod_low.clone()), ("high", mod_high.clone())],
    );

    let corpus = [
        "map/definition.csv",
        "common/countries/colors.txt",
        "common/countries/low_only.txt",
        "gfx/interface/deep/deeper/deepest/icon.dds",
        "interface/OnlyVanilla.gfx",
        "MAP/DEFINITION.CSV",
        "interface/onlyvanilla.gfx",
        "Common/Countries/Colors.TXT",
        "map/no_such_file.csv",
        "no_such_dir/at_all.txt",
        "gfx/interface/deep/deeper/no_such.dds",
    ];

    assert_corpus_resolved(&resolver, &vfs, &corpus, 8);
    assert_agrees(&resolver, &vfs, &corpus);

    assert_eq!(
        winner_of(&resolve_b(&vfs, "common/countries/colors.txt")),
        (2, sha256_hex(b"high-colors")),
        "the highest mod layer must win"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn agreement_when_layers_spell_one_name_with_different_ascii_case() {
    let root = temp_root("ascii_case");
    let vanilla = root.join("vanilla");
    let mod_dir = root.join("mod");

    write_bytes(&vanilla, "gfx/interface/Banner.dds", b"vanilla-banner");
    write_bytes(&mod_dir, "gfx/interface/BANNER.DDS", b"mod-banner");
    write_bytes(&vanilla, "gfx/interface/keep.dds", b"vanilla-keep");

    let (resolver, vfs) = matched_pair(&vanilla, &[("case", mod_dir.clone())]);

    let corpus = [
        "gfx/interface/Banner.dds",
        "gfx/interface/BANNER.DDS",
        "gfx/interface/banner.dds",
        "gfx/interface/keep.dds",
    ];
    assert_corpus_resolved(&resolver, &vfs, &corpus, 4);
    assert_agrees(&resolver, &vfs, &corpus);

    for spelling in [
        "gfx/interface/Banner.dds",
        "gfx/interface/BANNER.DDS",
        "gfx/interface/banner.dds",
    ] {
        assert_eq!(
            winner_of(&resolve_b(&vfs, spelling)),
            (1, sha256_hex(b"mod-banner")),
            "{spelling} did not reach the mod layer's payload"
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn agreement_on_directory_versus_file_name_collisions() {
    let root = temp_root("dir_file");
    let vanilla = root.join("vanilla");
    let mod_dir = root.join("mod");

    write_bytes(&vanilla, "common/traits/leader.txt", b"vanilla-leader");
    write_bytes(&mod_dir, "common/traits", b"mod-made-traits-a-file");
    write_bytes(&vanilla, "common/ideas/idea.txt", b"vanilla-idea");

    let (resolver, vfs) = matched_pair(&vanilla, &[("collide", mod_dir.clone())]);

    let corpus = [
        "common/traits/leader.txt",
        "common/traits",
        "common/ideas/idea.txt",
    ];
    assert_corpus_resolved(&resolver, &vfs, &corpus, corpus.len());
    assert_agrees(&resolver, &vfs, &corpus);

    assert_eq!(
        winner_of(&resolve_b(&vfs, "common/traits/leader.txt")).0,
        0,
        "a file component in the mod must not satisfy a directory walk"
    );
    assert_eq!(
        winner_of(&resolve_b(&vfs, "common/traits")).0,
        1,
        "the mod's file must win for the colliding name itself"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn agreement_on_non_ascii_case_pairs_that_ntfs_really_folds() {
    let root = temp_root("nonascii_folded");
    let vanilla = root.join("vanilla");

    write_bytes(&vanilla, "localisation/Ärger.yml", b"umlaut");
    write_bytes(&vanilla, "localisation/Голова.yml", b"cyrillic");
    write_bytes(&vanilla, "localisation/Éclair.yml", b"acute");

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    let corpus = [
        "localisation/Ärger.yml",
        "localisation/ärger.yml",
        "localisation/ÄRGER.yml",
        "localisation/Голова.yml",
        "localisation/голова.yml",
        "localisation/Éclair.yml",
        "localisation/éclair.yml",
    ];
    assert_corpus_resolved(&resolver, &vfs, &corpus, 7);
    assert_agrees(&resolver, &vfs, &corpus);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn agreement_on_an_ordinary_replace_path() {
    let root = temp_root("replace_path");
    let vanilla = root.join("vanilla");
    let mod_dir = root.join("mod");

    write_bytes(&vanilla, "common/traits/leader.txt", b"vanilla-leader");
    write_bytes(&vanilla, "common/traits/sub/deep.txt", b"vanilla-deep");
    write_bytes(&vanilla, "common/ideas/idea.txt", b"vanilla-idea");
    write_bytes(&mod_dir, "common/traits/own.txt", b"mod-own");

    let (resolver, vfs) =
        matched_pair_with_replace(&vanilla, &[("repl", mod_dir.clone())], &[(0, "common/traits")]);

    let corpus = [
        "common/traits/leader.txt",
        "common/traits/own.txt",
        "common/traits/sub/deep.txt",
        "common/ideas/idea.txt",
    ];
    assert_agrees(&resolver, &vfs, &corpus);

    for (path, expected_ordinal, payload) in [
        ("common/traits/own.txt", 1u32, &b"mod-own"[..]),
        ("common/traits/sub/deep.txt", 0, &b"vanilla-deep"[..]),
        ("common/ideas/idea.txt", 0, &b"vanilla-idea"[..]),
    ] {
        assert_eq!(
            winner_of(&resolve_a(&resolver, path)),
            (expected_ordinal, sha256_hex(payload)),
            "{path} must survive the replace_path in A"
        );
        assert_eq!(
            winner_of(&resolve_b(&vfs, path)),
            (expected_ordinal, sha256_hex(payload)),
            "{path} must survive the replace_path in B"
        );
    }

    assert_eq!(
        resolve_a(&resolver, "common/traits/leader.txt"),
        Outcome::Absent,
        "the replace_path did not suppress the lower layer in A"
    );
    assert_eq!(
        resolve_b(&vfs, "common/traits/leader.txt"),
        Outcome::Absent,
        "the replace_path did not suppress the lower layer in B"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn pinned_divergence_replace_path_folding_differs_on_a_non_ascii_directory() {
    let root = temp_root("replace_nonascii");
    let vanilla = root.join("vanilla");
    let mod_dir = root.join("mod");

    write_bytes(&vanilla, "common/\u{3bc}/unit.txt", b"vanilla-mu");
    write_bytes(&vanilla, "common/keep/unit.txt", b"vanilla-keep");
    write_bytes(&mod_dir, "common/unrelated.txt", b"mod-unrelated");

    let (resolver, vfs) = matched_pair_with_replace(
        &vanilla,
        &[("repl", mod_dir.clone())],
        &[(0, "common/\u{b5}")],
    );

    assert_eq!(
        winner_of(&resolve_b(&vfs, "common/\u{3bc}/unit.txt")),
        (0, sha256_hex(b"vanilla-mu")),
        "B no longer keeps the two replace-path spellings distinct"
    );
    assert_eq!(
        resolve_a(&resolver, "common/\u{3bc}/unit.txt"),
        Outcome::Absent,
        "A no longer suppresses a differently-spelled directory via replace_path"
    );

    let control_root = temp_root("replace_nonascii_control");
    let control_vanilla = control_root.join("vanilla");
    let control_mod = control_root.join("mod");
    write_bytes(&control_vanilla, "common/\u{3bc}/unit.txt", b"vanilla-mu");
    write_bytes(&control_mod, "common/unrelated.txt", b"mod-unrelated");
    write_descriptor(&control_mod, "repl", "");
    let (control_a, control_b) =
        matched_pair(&control_vanilla, &[("repl", control_mod.clone())]);
    assert_eq!(
        control_a.active_sources()[1].source_id,
        "descriptor.mod",
        "the control's mod layer did not pick up its descriptor"
    );
    assert!(
        control_a.active_sources()[1].replace_path.is_empty(),
        "the control's descriptor must declare no replace_path"
    );
    assert_agrees(&control_a, &control_b, &["common/\u{3bc}/unit.txt"]);
    assert_eq!(
        winner_of(&resolve_a(&control_a, "common/\u{3bc}/unit.txt")),
        (0, sha256_hex(b"vanilla-mu")),
        "without the declaration the lower layer must survive in A too — if it \
         does not, this fixture is not isolating replace_path"
    );
    let _ = fs::remove_dir_all(&control_root);

    assert_agrees(&resolver, &vfs, &["common/keep/unit.txt"]);

    let _ = fs::remove_dir_all(&root);
}


#[test]
fn agreement_on_unicode_case_neighbours_the_volume_keeps_apart() {
    let root = temp_root("overfold");
    let vanilla = root.join("vanilla");

    write_bytes(&vanilla, "common/\u{b5}nit.txt", b"micro-sign");
    write_bytes(&vanilla, "common/\u{3bc}nit.txt", b"greek-mu");
    write_bytes(&vanilla, "common/ordinary.txt", b"ordinary");

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    let micro = resolve_b(&vfs, "common/\u{b5}nit.txt");
    let mu = resolve_b(&vfs, "common/\u{3bc}nit.txt");
    let payload_of = |outcome: &Outcome| match outcome {
        Outcome::Winner { payload_sha256, .. } => payload_sha256.clone(),
        other => panic!("expected a winner, got {other:?}"),
    };
    assert_eq!(
        payload_of(&micro),
        sha256_hex(b"micro-sign"),
        "the host filesystem did not keep MICRO SIGN distinct; the premise is gone"
    );
    assert_eq!(
        payload_of(&mu),
        sha256_hex(b"greek-mu"),
        "the host filesystem did not keep GREEK SMALL MU distinct; the premise is gone"
    );

    assert_agrees(
        &resolver,
        &vfs,
        &["common/\u{b5}nit.txt", "common/\u{3bc}nit.txt"],
    );
    assert_eq!(
        payload_of(&resolve_a(&resolver, "common/\u{b5}nit.txt")),
        sha256_hex(b"micro-sign"),
        "MICRO SIGN must resolve to the micro-sign payload in A"
    );
    assert_eq!(
        payload_of(&resolve_a(&resolver, "common/\u{3bc}nit.txt")),
        sha256_hex(b"greek-mu"),
        "GREEK SMALL MU must resolve to the greek-mu payload in A"
    );

    assert_agrees(&resolver, &vfs, &["common/ordinary.txt"]);
    assert_eq!(
        payload_of(&resolve_a(&resolver, "common/ordinary.txt")),
        sha256_hex(b"ordinary"),
        "the collision must not poison an unrelated sibling's point lookup"
    );

    assert_ne!(micro, mu, "the two payloads must not be the same file");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn agreement_across_the_win32_normalization_class() {
    let root = temp_root("win32_class");
    let vanilla = root.join("vanilla");
    write_bytes(&vanilla, "common/ideas.txt", b"ideas");
    write_bytes(&vanilla, "gfx/unit.dds", b"unit");

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    assert_corpus_resolved(&resolver, &vfs, &["common/ideas.txt", "gfx/unit.dds"], 2);

    let payload_of = |outcome: &Outcome| match outcome {
        Outcome::Winner { payload_sha256, .. } => payload_sha256.clone(),
        other => panic!("expected a winner, got {other:?}"),
    };

    enum Expect {
        Resolves(&'static str),
        Absent,
        Refused,
    }
    use Expect::*;

    let rows: &[(&str, Expect)] = &[
        ("common/ideas.txt", Resolves("ideas")),
        ("COMMON/IDEAS.TXT", Resolves("ideas")),
        ("common/ideas.txt.", Resolves("ideas")),
        ("common/ideas.txt...", Resolves("ideas")),
        ("common./ideas.txt", Resolves("ideas")),
        ("common/ideas.txt ", Resolves("ideas")),
        ("common//ideas.txt", Resolves("ideas")),
        ("common/./ideas.txt", Refused),
        ("common/../common/ideas.txt", Refused),
        ("common /ideas.txt", Absent),
        ("common/missing.txt", Absent),
    ];

    for (spelling, expect) in rows {
        let a = resolve_a(&resolver, spelling);
        let b = resolve_b(&vfs, spelling);
        match expect {
            Refused => {
                for (label, outcome) in [("A", &a), ("B", &b)] {
                    let Outcome::Refused(message) = outcome else {
                        panic!(
                            "{spelling:?} must refuse on both sides; {label} answered {outcome:?} \
                             while the other side gave A={a:?} B={b:?}"
                        );
                    };
                    assert!(
                        message.contains(spelling),
                        "{label}'s refusal of {spelling:?} does not name it: {message:?}"
                    );
                    let grammar = ["unsafe", "non-canonical", "engine path grammar"]
                        .iter()
                        .any(|marker| message.contains(marker));
                    assert!(
                        grammar,
                        "{label} refused {spelling:?} for a reason that is not the path grammar, \
                         so this row is not pinning the cause it claims: {message:?}"
                    );
                }
            }
            _ => {
                assert_eq!(
                    a, b,
                    "the two implementations disagree on {spelling:?}: A={a:?} B={b:?}. \
                     Agreement across this class is what the digest join rests on."
                );
                match expect {
                    Resolves(payload) => assert_eq!(
                        payload_of(&a),
                        sha256_hex(payload.as_bytes()),
                        "{spelling:?} must resolve to the {payload:?} payload, and both sides \
                         answered {a:?}"
                    ),
                    Absent => assert_eq!(a, Outcome::Absent, "{spelling:?} must be absent"),
                    Refused => unreachable!("handled above"),
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&root);
}

#[cfg(windows)]
#[test]
fn win32_normalization_still_applies_just_below_max_path() {
    let root = temp_root("near_max_path");
    let vanilla = root.join("vanilla");

    let base_len = vanilla.as_os_str().encode_wide().count();
    let dotted_target = 258usize;
    let tail = "/ideas.txt";
    let head = "common/";
    let fixed = base_len + 1 + head.len() + tail.len() + 1;
    assert!(
        dotted_target > fixed,
        "the temp root is {base_len} units, leaving no room to build a path inside the window"
    );
    let rel = format!("{head}{}{tail}", "d".repeat(dotted_target - fixed));
    let full_len = base_len + 1 + rel.encode_utf16().count();
    assert!(
        (248..259).contains(&full_len),
        "the fixture path is {full_len} units, which is outside the window this test exists for; \
         with the trailing dot it must still be under 260"
    );

    write_bytes(&vanilla, &rel, b"ideas");
    let on_disk = rel
        .split('/')
        .fold(vanilla.clone(), |path, part| path.join(part));
    assert!(on_disk.is_file(), "the fixture file was not created");

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    assert_agrees(&resolver, &vfs, &[rel.as_str()]);

    let dotted = format!("{rel}.");
    let a = resolve_a(&resolver, &dotted);
    let b = resolve_b(&vfs, &dotted);
    assert_eq!(a, b, "the two disagree just below MAX_PATH: A={a:?} B={b:?}");
    assert_eq!(
        a.shape(),
        "winner",
        "at {full_len} units the engine still gets ordinary Win32 parsing and opens {dotted:?}, \
         so a resolver that answers {a:?} has been promoted into the verbatim namespace"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn pinned_divergence_archive_layers_match_names_by_ascii_case_only() {
    use std::io::Write as _;

    let root = temp_root("archive_parity");
    assert_fixture_root(&root);
    let stored = "gfx/\u{c4}.dds";
    let looked_up = "gfx/\u{e4}.dds";

    let dir_layer = root.join("dir_layer");
    write_bytes(&dir_layer, stored, b"aumlaut");
    assert!(
        dir_layer
            .join("gfx")
            .join("\u{e4}.dds")
            .is_file(),
        "this host does not fold \u{c4}/\u{e4}, so the premise of this test is gone"
    );

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(stored, zip::write::SimpleFileOptions::default())
        .expect("start archive member");
    writer.write_all(b"aumlaut").expect("write archive member");
    let archive_bytes = writer.finish().expect("finish archive").into_inner();
    let archive_sha = sha256_hex(&archive_bytes);
    let archive_path = root.join("layer.zip");
    fs::create_dir_all(&root).expect("create fixture root");
    fs::write(&archive_path, &archive_bytes).expect("write archive");

    let build = |storage_kind, local_source, container| {
        EffectiveVfs::new(
            "a".repeat(64),
            vec![EffectiveVfsLayerMount {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".to_string(),
                storage_kind,
                local_source,
                source_container_sha256: container,
                replace_paths: Vec::new(),
            }],
        )
        .expect("build implementation B")
    };
    let as_directory = build(LayerStorageKind::Directory, dir_layer.clone(), None);
    let as_archive = build(
        LayerStorageKind::ZipArchive,
        archive_path.clone(),
        Some(archive_sha),
    );

    assert_eq!(
        resolve_b(&as_directory, stored).shape(),
        "winner",
        "the directory fixture does not carry {stored:?}"
    );
    assert_eq!(
        resolve_b(&as_archive, stored).shape(),
        "winner",
        "the archive fixture does not carry {stored:?}"
    );

    let ascii_variant = "GFX/\u{c4}.DDS";
    assert_eq!(
        resolve_b(&as_directory, ascii_variant).shape(),
        "winner",
        "a directory layer must fold ASCII case"
    );
    assert_eq!(
        resolve_b(&as_archive, ascii_variant).shape(),
        "winner",
        "the archive index folds ASCII case, so {ascii_variant:?} must resolve; if this now \
         misses, the index became case-SENSITIVE and the gap this test pins is a different one"
    );

    assert_eq!(
        resolve_b(&as_directory, looked_up).shape(),
        "winner",
        "a directory layer must resolve {looked_up:?} through the volume's own folding"
    );
    assert_eq!(
        resolve_b(&as_archive, looked_up).shape(),
        "absent",
        "PINNED GAP CHANGED: an archive layer resolved {looked_up:?}. If archive name matching \
         was brought under a volume-equivalent rule this pin should be replaced by an agreement \
         assertion, not deleted."
    );

    let _ = fs::remove_dir_all(&root);
}

#[cfg(windows)]
#[test]
fn a_path_past_max_path_is_absent_because_the_engine_is_not_long_path_aware() {
    let root = temp_root("max_path");
    let vanilla = root.join("vanilla");
    write_bytes(&vanilla, "common/ideas.txt", b"ideas");

    let base_len = vanilla.to_string_lossy().chars().count();
    let segment = "d".repeat(60);
    let mut deep = String::from("common");
    while base_len + deep.chars().count() + 1 + 12 < 300 {
        deep.push('/');
        deep.push_str(&segment);
    }
    let long_rel = format!("{deep}/ideas.txt");
    let full_len = base_len + 1 + long_rel.chars().count();
    assert!(
        full_len > 260,
        "the fixture path is only {full_len} characters, so it does not exercise the limit"
    );

    write_bytes(&vanilla, &long_rel, b"ideas");
    let on_disk = long_rel
        .split('/')
        .fold(vanilla.clone(), |path, part| path.join(part));
    assert!(
        on_disk.is_file(),
        "the fixture file was not created, so 'absent' below would be meaningless"
    );

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    assert_agrees(&resolver, &vfs, &["common/ideas.txt"]);

    let a = resolve_a(&resolver, &long_rel);
    let b = resolve_b(&vfs, &long_rel);
    assert_eq!(a, b, "the two disagree past MAX_PATH: A={a:?} B={b:?}");
    assert_eq!(
        a,
        Outcome::Absent,
        "a path the engine cannot name must be absent, not {a:?}"
    );

    let _ = fs::remove_dir_all(&root);
}


#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
)]
#[test]
fn gate_two_vfs_agree_on_the_installed_vanilla_corpus() {
    let game_dir = replay_test_gates::require_vanilla_game_root();
    let (resolver, vfs) = matched_pair_readonly(&game_dir);

    let mut a_paths = std::collections::BTreeSet::new();
    let mut b_paths = std::collections::BTreeSet::new();
    for (dir, extension) in [
        ("interface", Some("gfx")),
        ("common/abilities", None),
        ("gfx/interface/abilitylist", None),
        ("localisation/english", Some("yml")),
    ] {
        match resolver.resolve_dir_files(dir, extension) {
            Ok(rows) => a_paths.extend(rows.into_iter().map(|(rel, _)| rel)),
            Err(error) => assert!(
                error.to_string().contains("missing directory"),
                "implementation A failed to list {dir}: {error}"
            ),
        }
        match vfs.list_recursive(dir, extension) {
            Ok(files) => b_paths.extend(files.into_iter().map(|file| file.logical_path)),
            Err(error) => panic!("implementation B failed to list {dir}: {error}"),
        }
    }

    let only_a = a_paths.difference(&b_paths).cloned().collect::<Vec<_>>();
    let only_b = b_paths.difference(&a_paths).cloned().collect::<Vec<_>>();
    let (from_a, from_b) = (a_paths.len(), b_paths.len());

    let mut corpus = std::collections::BTreeSet::new();
    corpus.extend(only_a.iter().cloned());
    corpus.extend(only_b.iter().cloned());
    {
        let union = a_paths.union(&b_paths).cloned().collect::<Vec<_>>();
        for (index, path) in union.into_iter().enumerate() {
            if index % 7 == 0 {
                corpus.insert(path);
            }
        }
    }
    assert!(
        from_a > 0 && from_b > 0,
        "the installed corpus is not a union: implementation A's listings yielded {from_a} unique \
         paths and B's yielded {from_b}"
    );
    let mut corpus = corpus.into_iter().collect::<Vec<_>>();

    let variants = corpus
        .iter()
        .take(12)
        .flat_map(|rel| [rel.to_uppercase(), rel.to_lowercase()])
        .collect::<Vec<_>>();
    corpus.extend(variants);
    corpus.push("interface/no_such_file_lane_la.gfx".to_string());
    corpus.push("no_such_directory_lane_la/file.txt".to_string());

    assert!(
        corpus.len() >= 30,
        "installed differential corpus is only {} paths",
        corpus.len()
    );

    let paths = corpus.iter().map(String::as_str).collect::<Vec<_>>();
    assert_corpus_resolved(&resolver, &vfs, &paths, 20);
    assert_agrees(&resolver, &vfs, &paths);
}
