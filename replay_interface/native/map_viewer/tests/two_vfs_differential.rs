//! The permanent guard over the TWO layered-VFS implementations.
//!
//! This repository contains two independent answers to one question — "given a
//! game-relative path and a stack of mod-over-vanilla content layers, which file
//! wins, under Windows case rules":
//!
//! * **A** — `map_viewer::mod_resolver` (`GameFileResolver`/`FileResolver`).
//!   Scans each directory with `fs::read_dir` and compares
//!   `portable_content_manifest::filesystem_case_key` of each entry against the
//!   same fold of the wanted name. Refuses a case-key collision. This is what
//!   the REPLAY side resolves through.
//! * **B** — `portable_content_manifest::effective_vfs::EffectiveVfs`. Pushes
//!   each literal path component and asks the host filesystem
//!   (`fs::symlink_metadata`), so NTFS decides identity. This is what the
//!   RECORDER's attach-time content proof seals a content epoch with.
//!
//! The recorder seals `UsedContentKey` rows through B; replay authenticates them
//! against A in `validate_used_content_match`. If A and B disagree about which
//! file a path names, a valid recording becomes unplayable (or, where the
//! admission map is keyed by a fold that collides, ambiguous). Lane KR recorded
//! the split as residual R3 without closing it; this file is the closure.
//!
//! WHICH ONE IS RIGHT IS NOT A MATTER OF TASTE, though the evidence is not
//! complete either. It was read out of the pinned engine binary
//! (`hoi4.exe 1.19.2.0.a729`,
//! `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`) — see
//! `extractor/native/runs/la_two_vfs/engine_lookup_rule_from_a729.md`:
//! `PHYSFS_stat` (`0x1424e0b00`) walks the search path and takes the FIRST hit
//! through the archiver vtable at `+0x68`;
//! `sanitizePlatformIndependentPath` (`0x1424e3ff0`) copies every byte verbatim
//! and folds NOTHING; `verifyPath` (`0x1424e4250`) and `partOfMountPoint`
//! (`0x1424e3f50`) compare bytes and fold nothing; and
//! `__PHYSFS_platformStat` (`0x1424ff8e0`) converts UTF-8 to UTF-16 and calls
//! `GetFileAttributesExW` on the LITERAL path.
//!
//! **What that establishes:** no step this lane read folds case, and the final
//! filesystem question is asked literally, so the volume decides. B is built on
//! that model; A's full-Unicode fold is a rule the engine does not contain.
//! **What it does NOT establish:** the directory archiver's own `+0x68`
//! function (upstream `DIR_stat`) was not read — it is `static` and has no
//! symbol — so the step that concatenates the layer root with the requested name
//! is inferred from upstream PhysicsFS, not shown. Residual R-LA-1b.
//!
//! So this file does two things, and the second is why it is not vacuous:
//!
//! 1. `agreement_*` — over corpora where the two MUST agree, it requires the
//!    identical FULL identity: winning layer ordinal, layer kind, layer id,
//!    source digest, payload digest and payload length — the exact tuple
//!    `validate_used_content_match` authenticates — or the identical "absent",
//!    and it rejects a refusal on either side. This is the guard proper: a
//!    future edit to either implementation that moves one of them fails here.
//! 2. `pinned_divergence_*` — over the input classes where they provably do NOT
//!    agree, it pins the exact current disagreement, and names which side the
//!    engine backs WHERE THAT IS KNOWN (for case folding it is; for
//!    `replace_path` it is not, and the test says so rather than guessing).
//!    These fail both if a divergence disappears silently and if it changes
//!    shape.
//!
//! Scope of that guarantee, stated honestly: these are finite fixture corpora
//! plus a sampled pass over one local install. They catch a divergence that
//! touches the classes covered here. They cannot prove no other divergence
//! exists, and nothing in this file should be read as claiming they do.

use map_viewer::GameFileResolver;
use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
use portable_content_manifest::{ContentLayerKind, LayerStorageKind};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One implementation's answer for one path, carrying **every field the digest
/// join actually compares**.
///
/// This is deliberately the full tuple from `validate_used_content_match`
/// (`mod_resolver.rs:3475`) — winning layer ordinal, layer KIND, layer ID,
/// source digest, payload digest, payload length — and not a convenient subset.
/// A guard that compared only ordinal and payload hash would pass while the two
/// disagreed about the identity of the layer they had both picked, which is
/// exactly one of the ways the join can refuse a valid recording.
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
    /// The implementation refused. The text is kept for the failure message but
    /// deliberately NOT compared: the two produce different wording for the same
    /// situation, and pinning wording here would make this a spelling test.
    /// Agreement corpora reject refusals outright (see `assert_agrees`), so a
    /// refusal is never silently accepted as agreement there.
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

/// A's layer-kind string -> the portable enum B carries, so the two can be
/// compared at all. Mirrors `mod_resolver::content_layer_kind`, which is private.
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

/// `(winning ordinal, payload digest)` of an outcome that must be a winner.
/// Used where a test needs to assert WHICH layer won and with what bytes,
/// without restating the identity fields the differential already compares.
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

/// Implementation A's answer, in the same field order the join compares.
///
/// `source_sha256` mirrors `validate_used_content_match`'s own choice exactly:
/// the layer's archive digest when it has one, else the payload digest.
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

/// Implementation B's answer.
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

/// Write a `descriptor.mod` in **Latin-1, not UTF-8**.
///
/// `parse_mod_descriptor` reads descriptors through `read_latin1`
/// (`util.rs:22`), which maps each BYTE to the codepoint of the same value.
/// Writing UTF-8 here makes the fixture lie: `µ` (U+00B5) goes out as
/// `0xC2 0xB5` and comes back as `Âµ`, so a declaration silently matches
/// nothing and a test built on it "proves" a divergence that is really this
/// encoding mismatch. That is exactly what happened to the first version of the
/// non-ASCII `replace_path` fixture — see residual R-LA-3c.
///
/// `body` is any extra lines (each already `\n`-terminated); pass "" for a
/// descriptor that declares nothing but its name.
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

/// Refuse to write anywhere outside this process's temp directory.
///
/// Every fixture root comes from [`temp_root`], i.e. lives under the process
/// temp directory. A root outside it is caller-owned content — in practice the
/// player's the game installation — and this file must never modify that. See the
/// comment at the marker loop for why this exists as a check rather than as a
/// convention.
///
/// SCOPE, stated exactly: this proves a root is under the temp directory, not
/// that this test created it. A caller that deliberately pointed a fixture at
/// some other process's temp subdirectory would pass. That is a much narrower
/// hole than the one it closes (the real the game install, which is what actually
/// got written), and closing it properly would mean threading a
/// created-by-this-test token through every fixture; recorded rather than built.
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

/// A matched pair over content this test does NOT own, and therefore must not
/// write to: no descriptors are written and no binding markers are created.
///
/// The ordinal-to-root binding that `matched_pair` proves with markers is not
/// provable this way, so this is restricted to a single base layer, where the
/// binding is not in question: there is exactly one root, it is the one passed
/// in, and `active_sources()` is asserted to contain exactly one layer at
/// ordinal 0.
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

/// Build B over the same roots, in the same order, that A was given, and prove
/// the two stacks really are the same stack before comparing any resolution.
///
/// Without this the whole differential could pass while comparing two DIFFERENT
/// layer stacks — the most obvious way for this file to be worthless.
///
/// B's mount identities are taken FROM A's `active_sources()` rather than
/// invented here. That is not a way of making the comparison pass: the two
/// implementations must be describing the same stack for a differential over it
/// to mean anything, and A is the side that derives layer identity from the mod
/// descriptors. Inventing B's ids independently (an earlier version of this file
/// used `mod/{name}.mod` while A uses the directory basename) guarantees a
/// mismatch on every full-key comparison and so would have forced the identity
/// fields back out of `Outcome` — hiding a real class of disagreement.
///
/// `replace` supplies `replace_paths` for the mod layers by index, since A takes
/// them from descriptors and this fixture has none.
fn matched_pair_with_replace(
    vanilla: &Path,
    mods: &[(&str, PathBuf)],
    replace: &[(usize, &str)],
) -> (GameFileResolver, EffectiveVfs) {
    // OWNERSHIP FIRST. Everything below this line may write, so every root is
    // checked before ANY of it runs — the descriptor writes included. (Round 4
    // caught the check sitting further down, after the descriptors: the
    // structural guarantee was advertised but not delivered for that path.)
    assert_fixture_root(vanilla);
    for (_, path) in mods {
        assert_fixture_root(path);
    }

    // A reads a mod's replace_path from its `descriptor.mod`; B is told directly.
    // Writing the descriptor is what makes the two stacks the SAME stack. (The
    // first version of this helper passed `replace` to B only, and the resulting
    // "disagreement" was this fixture's bug, not the implementations'.)
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

    // PROVE the ordinal-to-root binding rather than assuming it. Each root gets
    // a marker file naming its own index; if A's ordinal N did not come from the
    // root this fixture put at position N, the marker resolves to the wrong
    // payload and the whole differential would be comparing a mislabelled stack.
    //
    // THIS WRITES, so it may only ever touch roots under the process temp
    // directory (which is where every fixture root comes from; the check does
    // NOT prove this test created them — see `assert_fixture_root`).
    // The first version of this helper did not check, and the installed-corpus
    // gate — which passes the REAL game directory as `vanilla` — dropped a
    // marker file into the player's the game installation and never removed it.
    // `assert_fixture_root` makes that structurally impossible rather than a
    // thing to remember: a root outside the process temp directory is refused,
    // loudly, before anything is written. Callers that legitimately resolve
    // against caller-owned content use `matched_pair_readonly`, which never
    // reaches this block.
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

/// Compare both implementations over one corpus and return every disagreement.
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

/// Require exact agreement, and additionally require that NEITHER side refused.
///
/// Treating refusal-vs-refusal as agreement would let two entirely different
/// causes — "unsafe path grammar" on one side and "case collision" or an
/// unrelated I/O fault on the other — pass as if the implementations agreed.
/// No corpus in this file intends a refusal, so the honest rule is simply that a
/// refusal anywhere in an agreement corpus is a failure. Deliberate refusals are
/// asserted individually, by cause, in the `pinned_divergence_*` tests.
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

/// NON-VACUITY. "Both said absent for everything" agrees perfectly and proves
/// nothing, and so does "both refused everything". Require that the corpus
/// actually produced winners on both sides, and that they are real payloads.
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

// ---------------------------------------------------------------------------
// 1. AGREEMENT — the guard proper.
// ---------------------------------------------------------------------------

/// The ordinary layered cases the product actually depends on: a mod overriding
/// a vanilla file, a mod adding a file, a vanilla-only file, deep ancestors,
/// wrong ASCII case, and genuine absence. These must agree exactly, forever.
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

    // A mod overrides one vanilla file and adds one of its own.
    write_bytes(&mod_low, "common/countries/colors.txt", b"low-colors");
    write_bytes(&mod_low, "common/countries/low_only.txt", b"low-only");

    // A higher mod overrides the same file again: the highest layer must win in
    // both implementations, which is the whole point of the layer stack.
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
        // Wrong ASCII case, which NTFS folds and both implementations must too.
        "MAP/DEFINITION.CSV",
        "interface/onlyvanilla.gfx",
        "Common/Countries/Colors.TXT",
        // Absent, at several depths.
        "map/no_such_file.csv",
        "no_such_dir/at_all.txt",
        "gfx/interface/deep/deeper/no_such.dds",
    ];

    assert_corpus_resolved(&resolver, &vfs, &corpus, 8);
    assert_agrees(&resolver, &vfs, &corpus);

    // And the layered answers are the ones the product needs, not merely equal.
    assert_eq!(
        winner_of(&resolve_b(&vfs, "common/countries/colors.txt")),
        (2, sha256_hex(b"high-colors")),
        "the highest mod layer must win"
    );

    let _ = fs::remove_dir_all(&root);
}

/// A file present in one layer only, at a name that differs from another layer's
/// file by ASCII case alone. NTFS folds these, so the higher layer's spelling
/// must win in both — this is the case that silently swaps a payload if the two
/// implementations pick different winners.
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

    // All three spellings must reach the MOD's payload: one file, one winner.
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

/// A directory in one layer whose name collides, case-insensitively, with a FILE
/// in another layer, plus a lookup that walks THROUGH a file component. Both are
/// shapes a mod can create by accident.
#[test]
fn agreement_on_directory_versus_file_name_collisions() {
    let root = temp_root("dir_file");
    let vanilla = root.join("vanilla");
    let mod_dir = root.join("mod");

    // `common/traits` is a directory in vanilla and a FILE in the mod.
    write_bytes(&vanilla, "common/traits/leader.txt", b"vanilla-leader");
    write_bytes(&mod_dir, "common/traits", b"mod-made-traits-a-file");
    write_bytes(&vanilla, "common/ideas/idea.txt", b"vanilla-idea");

    let (resolver, vfs) = matched_pair(&vanilla, &[("collide", mod_dir.clone())]);

    let corpus = [
        // Resolving THROUGH the mod's file component: the mod cannot satisfy it
        // (a file is not a directory), so vanilla's copy must win in both.
        "common/traits/leader.txt",
        // The colliding name itself, as a file: only the mod has it as a file,
        // so the mod must win in both.
        "common/traits",
        "common/ideas/idea.txt",
    ];
    // Every path here is a winner; the floor is the corpus length, not a
    // fraction of it, so no path can silently degrade to "absent on both sides"
    // and still pass.
    assert_corpus_resolved(&resolver, &vfs, &corpus, corpus.len());
    assert_agrees(&resolver, &vfs, &corpus);

    // ...and they are won by the layers the layout dictates, not merely equal.
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

/// Non-ASCII names that NTFS folds together (measured in
/// `windows_collision_key`'s doc comment: `Ą`/`ą`, `É`/`é`, Cyrillic `Г`/`г`,
/// fullwidth `Ａ`/`ａ`). Both implementations must fold them, for different
/// reasons — A because `filesystem_case_key` does, B because the host does — and
/// they must land on the same file.
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

/// `replace_path`, the other axis on which the two fold names — and the one the
/// first version of this file missed entirely.
///
/// A stores a mod's declared replace paths folded with `filesystem_case_key`
/// (`mod_resolver.rs:1523`) and tests them against `immediate_parent` of the
/// FOLDED requested path (`mod_resolver.rs:1563`). B stores them canonicalized
/// by `normalize_game_relative_path` (ASCII lowercase) and tests them against an
/// ASCII-lowercased `logical_parent` (`effective_vfs.rs:178`). For an ordinary
/// ASCII directory the two agree; the folds differ only where
/// `filesystem_case_key` and ASCII-lowercasing differ.
///
/// This covers the ordinary case — a mod declaring `common/traits` must suppress
/// the lower layer's files under exactly that directory in both, and must NOT
/// suppress a sibling or a subdirectory. The non-ASCII case, where the two folds
/// genuinely part company, is pinned separately in
/// `pinned_divergence_replace_path_folding_differs_on_a_non_ascii_directory`.
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
        // Suppressed by the replace_path: the mod replaces this directory.
        "common/traits/leader.txt",
        // The mod's own file in the replaced directory still resolves.
        "common/traits/own.txt",
        // A SUBDIRECTORY of the replaced directory is not the replaced parent.
        "common/traits/sub/deep.txt",
        // An untouched sibling directory.
        "common/ideas/idea.txt",
    ];
    assert_agrees(&resolver, &vfs, &corpus);

    // The three paths that must SURVIVE are named individually. A corpus-wide
    // floor cannot do this job: with a floor of two, any one of the three could
    // silently become Absent on both sides and the test would still pass — which
    // is exactly the failure a replace_path bug produces.
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

    // And the replace actually bit, or this proves nothing about replace_path.
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

/// The `replace_path` split, MEASURED and pinned as open.
///
/// A folds a declared replace path with `filesystem_case_key`
/// (`mod_resolver.rs:1523`) and matches it against the folded parent of the
/// request (`mod_resolver.rs:1563`); B canonicalizes with
/// `normalize_game_relative_path` (ASCII lowercase) and matches an
/// ASCII-lowercased `logical_parent` (`effective_vfs.rs:178`). So a mod
/// declaring `common/µ` (U+00B5) is expected to suppress a lower layer's
/// `common/μ/...` (U+03BC) in A — one folded key — and not in B, which keeps
/// them as two names, matching the two directories NTFS actually has.
///
/// **Which side is right is NOT decided here.** The engine's `replace_path`
/// `theatre_selector_live/src/engine_modset_provenance.rs`) and this lane did
/// not read it. Recording a preference as a fact is the error this lane spent a
/// review round removing elsewhere, so this test asserts only WHAT THE TWO DO,
/// and `RESIDUALS.md` R-LA-3b carries the open question.
///
/// It is pinned rather than merely described because a described divergence is
/// one nobody notices changing. This fails if either side moves.
#[test]
fn pinned_divergence_replace_path_folding_differs_on_a_non_ascii_directory() {
    let root = temp_root("replace_nonascii");
    let vanilla = root.join("vanilla");
    let mod_dir = root.join("mod");

    // The lower layer holds the GREEK MU spelling.
    write_bytes(&vanilla, "common/\u{3bc}/unit.txt", b"vanilla-mu");
    write_bytes(&vanilla, "common/keep/unit.txt", b"vanilla-keep");
    // The mod holds NOTHING under either spelling. This is the whole point of
    // the fixture and the first version of it got this wrong: if the mod also
    // shipped `common/µ/unit.txt`, A would select that file through its ORDINARY
    // lookup over-fold (`find_dir_in_layer` folds µ onto μ, mod_resolver.rs:3281)
    // and the A-vs-B difference would appear with or without the declaration —
    // i.e. it would be divergence #1 wearing a replace_path costume. With the
    // mod empty, the ONLY thing that can suppress the lower layer is the
    // replace_path declaration itself.
    write_bytes(&mod_dir, "common/unrelated.txt", b"mod-unrelated");

    let (resolver, vfs) = matched_pair_with_replace(
        &vanilla,
        &[("repl", mod_dir.clone())],
        &[(0, "common/\u{b5}")],
    );

    // EXACT outcomes on both sides, not merely "they differ". `assert_ne!` would
    // stay green if A drifted to a refusal, or to some other wrong winner.
    //
    // B: the declared replace path `common/µ` and the request's parent
    // `common/μ` are different ASCII-lowercased strings, so the declaration does
    // not reach this directory and the lower layer SURVIVES.
    assert_eq!(
        winner_of(&resolve_b(&vfs, "common/\u{3bc}/unit.txt")),
        (0, sha256_hex(b"vanilla-mu")),
        "B no longer keeps the two replace-path spellings distinct"
    );
    // A: `filesystem_case_key` folds the declaration and the parent onto one
    // key, so the declaration DOES reach this directory and suppresses it. The
    // mod has nothing to put in its place, so the path goes absent.
    assert_eq!(
        resolve_a(&resolver, "common/\u{3bc}/unit.txt"),
        Outcome::Absent,
        "A no longer suppresses a differently-spelled directory via replace_path"
    );

    // The control: the SAME layout with the SAME descriptor, differing ONLY in
    // the presence of the replace_path line. The descriptor is written here
    // explicitly because `matched_pair` writes none, and a control that also
    // lacked the descriptor would vary two things at once — descriptor presence
    // as well as the declaration — which establishes nothing about which caused
    // the divergence.
    let control_root = temp_root("replace_nonascii_control");
    let control_vanilla = control_root.join("vanilla");
    let control_mod = control_root.join("mod");
    write_bytes(&control_vanilla, "common/\u{3bc}/unit.txt", b"vanilla-mu");
    write_bytes(&control_mod, "common/unrelated.txt", b"mod-unrelated");
    write_descriptor(&control_mod, "repl", "");
    let (control_a, control_b) =
        matched_pair(&control_vanilla, &[("repl", control_mod.clone())]);
    // The control really did mount a descriptor, so the only difference from the
    // fixture above is the replace_path line.
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

    // An untouched sibling directory is unaffected in both, so the divergence
    // is about the replace_path and not about the fixture being broken.
    assert_agrees(&resolver, &vfs, &["common/keep/unit.txt"]);

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 2. PINNED DIVERGENCES — the split that is real, with the engine's answer named.
// ---------------------------------------------------------------------------

/// **A over-folds where NTFS — and therefore the engine — keeps two files.**
///
/// `filesystem_case_key` is full-Unicode `to_uppercase` then `to_lowercase`, so
/// MICRO SIGN (U+00B5) and GREEK SMALL LETTER MU (U+03BC) fold to one key. NTFS
/// does not fold them (measured, `windows_collision_key` doc comment), and the
/// engine adds no folding of its own — `__PHYSFS_platformStat` (`0x1424ff8e0`)
/// hands the literal UTF-16 path to `GetFileAttributesExW`.
///
/// So with BOTH spellings present, A refuses **each colliding name** while the
/// engine happily opens either. **B is engine-correct; A is not.**
///
/// BLAST RADIUS, stated precisely, because an earlier draft of this file got it
/// wrong: a case collision refuses only a requested name whose folded key has
/// two matches. An uncolliding sibling in the same directory still resolves —
/// asserted below, and asserted in production too by
/// `memoized_session_reproduces_refusals_and_their_order`
/// (`mod_resolver.rs:4021`). What DOES refuse wholesale is the directory
/// LISTING, and therefore any complete-census admission over that directory.
/// The "one bad sibling poisons every point lookup" behaviour is a different
/// defect (lane KR's residual R1) and belongs to the layer-ESCAPE check, not to
/// case collision.
///
/// This is pinned rather than repaired here because repairing it means replacing
/// A's selection rule wholesale — see the lane report — and a pinned divergence
/// that names the engine's side is worth more than an unpinned one.
#[test]
fn pinned_divergence_a_over_folds_micro_sign_against_greek_mu() {
    let root = temp_root("overfold");
    let vanilla = root.join("vanilla");

    // U+00B5 MICRO SIGN and U+03BC GREEK SMALL LETTER MU: two files on NTFS.
    write_bytes(&vanilla, "common/\u{b5}nit.txt", b"micro-sign");
    write_bytes(&vanilla, "common/\u{3bc}nit.txt", b"greek-mu");
    write_bytes(&vanilla, "common/ordinary.txt", b"ordinary");

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    // Both really are distinct files on this volume; if the host ever folded
    // them the premise of this test would be gone, so prove it rather than
    // assume it.
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

    // A refuses each COLLIDING NAME, because its fold makes them one ambiguous
    // identity, and it names that cause rather than failing for some other
    // reason.
    for spelling in ["common/\u{b5}nit.txt", "common/\u{3bc}nit.txt"] {
        match resolve_a(&resolver, spelling) {
            Outcome::Refused(text) => assert!(
                text.contains("ambiguous case-colliding file identity"),
                "implementation A refused {spelling} for the wrong reason: {text}"
            ),
            other => panic!(
                "implementation A no longer refuses {spelling}; it answered {other:?}. If A \
                 adopted the engine's rule this pin should become an agreement case."
            ),
        }
    }

    // ...and an UNCOLLIDING sibling still resolves in both, identically. Without
    // this the test above is indistinguishable from "the whole directory is
    // broken", which is a materially different and much larger defect.
    assert_agrees(&resolver, &vfs, &["common/ordinary.txt"]);
    assert_eq!(
        payload_of(&resolve_a(&resolver, "common/ordinary.txt")),
        sha256_hex(b"ordinary"),
        "the collision must not poison an unrelated sibling's point lookup"
    );

    // The engine-correct answer, for the record: whatever the host says.
    assert_ne!(micro, mu, "the two payloads must not be the same file");

    let _ = fs::remove_dir_all(&root);
}

/// A trailing-dot component: the two AGREE (both absent), and this test pins
/// that — but it is filed here rather than above because the agreed answer is
/// probably not the ENGINE's answer, and that is worth stating where someone
/// will read it.
///
/// This lane predicted a divergence here and the differential refuted it, which
/// is the whole reason to run one. The prediction was: Win32 strips trailing
/// dots per component (measured, `windows_collision_key` doc comment —
/// `b.txt.` overwrote `b.txt`, and interior `gfx./u.asset` opened
/// `gfx/u.asset`), so B — which asks the host — would open the file while A's
/// `filesystem_case_key` would not model the stripping and report absent.
///
/// **Measured: BOTH report absent.** B does not reach `GetFileAttributesExW`
/// with the raw spelling; it goes through `std::fs::symlink_metadata`, whose
/// Windows path handling does not reproduce the Win32 trailing-dot strip that
/// the `windows_collision_key` probe measured against the API directly.
///
/// So the digest join is SAFE here (the two agree, which is all this file
/// guards), while both may be refusing a path the engine would open. That is a
/// residual, not a defect this guard can close: closing it needs the
/// trailing-dot behaviour of `std::fs` measured against a direct
/// `GetFileAttributesExW` on the same path, and a decision about whether the
/// recorder should ever seal such a spelling in the first place.
#[test]
fn agreement_on_a_trailing_dot_component_both_report_absent() {
    let root = temp_root("trailing_dot");
    let vanilla = root.join("vanilla");
    write_bytes(&vanilla, "common/ideas.txt", b"ideas");

    let (resolver, vfs) = matched_pair(&vanilla, &[]);

    // The plain spelling resolves in both, so the fixture is real.
    assert_corpus_resolved(&resolver, &vfs, &["common/ideas.txt"], 1);
    assert_agrees(&resolver, &vfs, &["common/ideas.txt"]);

    // The trailing-dot spelling: agreement, on "absent".
    let a = resolve_a(&resolver, "common/ideas.txt.");
    let b = resolve_b(&vfs, "common/ideas.txt.");
    assert_eq!(a, Outcome::Absent, "implementation A now answers {a:?}");
    assert_eq!(
        b,
        Outcome::Absent,
        "implementation B now answers {b:?} for a trailing-dot component while A answers {a:?}. \
         If B started stripping trailing dots the two would disagree and the digest join would \
         break; see this test's doc comment."
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 3. THE REAL INSTALL.
// ---------------------------------------------------------------------------

/// The same differential over Paradox's own corpus, where the names are theirs
/// rather than this file's. Bounded to a deterministic slice so it stays a check
/// rather than a soak.
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
)]
#[test]
fn gate_two_vfs_agree_on_the_installed_vanilla_corpus() {
    let game_dir = replay_test_gates::require_vanilla_game_root();
    // READ-ONLY. This is the player's installation, not a fixture: nothing here
    // may create, modify or delete a file inside it.
    let (resolver, vfs) = matched_pair_readonly(&game_dir);

    // Draw the corpus from BOTH implementations' own listings, and compare the
    // union.
    //
    // Drawing it from A alone would bias the differential in the one direction
    // that matters: a path A cannot enumerate would never be asked about, so
    // exactly the files A is wrong about could be the files the test never
    // sees. Taking the union means a path EITHER side believes exists gets
    // resolved through both, and a file one of them cannot see becomes a
    // disagreement rather than a silence.
    // A listing FAILURE is never swallowed. Discarding it silently is the one
    // way a pathological directory can vanish from the corpus while every
    // remaining assertion stays green — which is precisely the case the guard
    // exists to catch.
    let mut a_paths = std::collections::BTreeSet::new();
    let mut b_paths = std::collections::BTreeSet::new();
    for (dir, extension) in [
        ("interface", Some("gfx")),
        ("common/abilities", None),
        ("gfx/interface/abilitylist", None),
        ("localisation/english", Some("yml")),
    ] {
        // A's view: non-recursive listing of the directory.
        match resolver.resolve_dir_files(dir, extension) {
            Ok(rows) => a_paths.extend(rows.into_iter().map(|(rel, _)| rel)),
            // "missing directory" is a legitimate answer for an install that
            // does not ship it; any other error is a real failure to report.
            Err(error) => assert!(
                error.to_string().contains("missing directory"),
                "implementation A failed to list {dir}: {error}"
            ),
        }
        // B's view: recursive listing under the same prefix. The two listings
        // are deliberately different operations (A's is non-recursive), so this
        // is a union of two honest views, not a duplicate of one.
        match vfs.list_recursive(dir, extension) {
            Ok(files) => b_paths.extend(files.into_iter().map(|file| file.logical_path)),
            Err(error) => panic!("implementation B failed to list {dir}: {error}"),
        }
    }

    // Paths only ONE listing produced are taken in FULL rather than sampled.
    //
    // Being honest about what that set contains: MOST of it is structural, not a
    // capability difference. A's `resolve_dir_files` is non-recursive and B's
    // `list_recursive` is recursive, so everything in a subdirectory is
    // "B-only" by construction. Taking the asymmetric set whole is still the
    // right move — it costs little, it widens coverage into subdirectories, and
    // it means a genuine one-sided path (the case this guard exists for) can
    // never be sampled away — but this set is not by itself evidence of
    // disagreement, and nothing here treats it as such. The disagreement test is
    // `assert_agrees` below, which resolves every path through BOTH.
    let only_a = a_paths.difference(&b_paths).cloned().collect::<Vec<_>>();
    let only_b = b_paths.difference(&a_paths).cloned().collect::<Vec<_>>();
    let (from_a, from_b) = (a_paths.len(), b_paths.len());

    // Sample AFTER the union, so a path is never dropped merely because of where
    // it fell in one implementation's ordering.
    let mut corpus = std::collections::BTreeSet::new();
    corpus.extend(only_a.iter().cloned());
    corpus.extend(only_b.iter().cloned());
    {
        let union = a_paths.union(&b_paths).cloned().collect::<Vec<_>>();
        // Every 7th row, so the slice spans the corpus rather than sampling one
        // alphabetical corner of it.
        for (index, path) in union.into_iter().enumerate() {
            if index % 7 == 0 {
                corpus.insert(path);
            }
        }
    }
    // The union is only worth taking if BOTH sides actually contributed. If one
    // listing silently produced nothing, this degenerates into the single-source
    // corpus whose bias the union exists to remove.
    assert!(
        from_a > 0 && from_b > 0,
        "the installed corpus is not a union: implementation A's listings yielded {from_a} unique \
         paths and B's yielded {from_b}"
    );
    let mut corpus = corpus.into_iter().collect::<Vec<_>>();

    // Case variants of real paths: the engine is case-insensitive through NTFS,
    // so these must resolve to the same bytes in both implementations.
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
