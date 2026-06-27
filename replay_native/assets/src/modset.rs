//! Mod-awareness for the native replay UI (GAME_UI_REPLICA_PLAN §K).
//!
//! Two responsibilities, both pure asset-RESOLUTION concerns — the engine
//! (`gui_runtime` / `render` / `bind`) is unchanged and renders whatever bundle it
//! is handed:
//!
//! 1. **Mod-selection.** The app picks an ACTIVE MOD-SET and loads THAT mod's asset
//!    bundle (the offline extractor's per-mod `gui/*.json` + `sprites.json` + fonts
//!    + loc — same FORMAT, mod content). A [`BundleConfig`] resolves which bundle
//!    dir to use from a CLI flag / env / config file, DEFAULTING to the bundle's
//!    `active_modset.json` manifest when present, else treating the bundle as
//!    vanilla.
//!
//! 2. **Auto-match + mismatch warning.** A replay records the mod-set it was
//!    captured under in `provenance.mod` (written capture-side from the host's
//!    `dlc_load.json`; see `live_capture/mod_identity.py`). On load,
//!    [`match_replay`] compares the replay's recorded mod-set to the selected
//!    bundle's mod-set and returns a [`MatchOutcome`] that the UI surfaces — a CLEAR
//!    WARNING when they differ (incl. version / load-order / composition deltas), so
//!    vanilla-UI-over-a-mod-replay is structurally prevented.
//!
//! NO FABRICATION: a replay whose `provenance.mod` is absent/`null` was captured
//! WITHOUT a readable launcher config (vanilla/SP, or no access). We do NOT invent a
//! match or a mismatch from absence — [`MatchOutcome::ReplayUnknown`] says "the
//! replay's mod identity is unknown", and the UI shows an advisory, not a false
//! all-clear and not a false alarm.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One mod in a mod-set, mirroring the Python ModSet entry
/// (`schema.empty_modset` / `mod_identity.read_modset`). Every field is optional so
/// a local mod with no workshop id, or a partial record, deserializes cleanly.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ModEntry {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub workshop_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub supported_version: Option<String>,
    #[serde(default)]
    pub descriptor: Option<String>,
}

impl ModEntry {
    /// The stable IDENTITY token for one mod, NAMESPACED `workshop:<id>` vs
    /// `name:<name>` (matches the Python `schema.modset_id` rule exactly, Codex
    /// review A): a local mod named `12345` can never collide with a Workshop mod
    /// whose id is `12345`. A mod with neither tokenises to `name:` (empty). Two
    /// mods are "the same mod" (for identity) iff their `ident()` AND `version`
    /// agree.
    pub fn ident(&self) -> String {
        let wid = self.workshop_id.as_deref().unwrap_or("").trim();
        if !wid.is_empty() {
            format!("workshop:{wid}")
        } else {
            format!("name:{}", self.name.as_deref().unwrap_or("").trim())
        }
    }
    fn norm_version(&self) -> String {
        self.version.as_deref().unwrap_or("").trim().to_string()
    }
    /// A human label for diffs/warnings (never a path).
    pub fn label(&self) -> String {
        let id = self
            .name
            .as_deref()
            .or(self.workshop_id.as_deref())
            .or(self.descriptor.as_deref())
            .unwrap_or("<unnamed mod>");
        match self.version.as_deref() {
            Some(v) if !v.is_empty() => format!("{id} v{v}"),
            _ => id.to_string(),
        }
    }
}

/// A mod-set: the ordered list of mods layered over vanilla, plus a stable id.
///
/// Deserializes BOTH producer shapes (a custom `Deserialize`, Codex-confirmed over a
/// brittle whole-document untagged enum):
///   * the REPLAY's `provenance.mod` (`mod_identity.read_modset`): an explicit
///     `mods: [ModEntry]` + `load_order: [String]` + `modset_id`,
///   * the BUNDLE manifest `active_modset.json` (the offline extractor): a
///     `load_order: [layer-object]` where each layer has `kind` (`vanilla`/`mod`),
///     `label`, `name`, `workshop_id`, `version`, …; the MODS are the `kind == "mod"`
///     layers (the vanilla base layer is NOT a mod — including it would poison the
///     identity digest, Codex-flagged).
/// Identity is computed ONLY from the ordered `kind == "mod"` entries; `load_order`
/// keeps the human labels for display, kept SEPARATE from the identity-bearing list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModSet {
    /// Stable, order-sensitive digest (or the `"vanilla"` sentinel). May be empty
    /// if a hand-written manifest omitted it — [`ModSet::resolved_id`] recomputes.
    pub modset_id: String,
    pub source: Option<String>,
    /// Display load-order labels (e.g. `["vanilla","AMSB - DEV"]` for a bundle, or the
    /// replay's recorded strings). NOT used for identity.
    pub load_order: Vec<String>,
    /// The identity-bearing mod list (the `kind == "mod"` layers, vanilla excluded).
    pub mods: Vec<ModEntry>,
}

/// One raw `load_order` element: either a plain STRING (replay shape) or a layer
/// OBJECT (bundle `active_modset.json` shape). An element-level untagged enum is safe
/// here — `String` vs object are unambiguous — unlike a whole-document untagged enum.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawLoadOrderItem {
    Label(String),
    Layer(BundleLayer),
}

/// A bundle `active_modset.json` `load_order` layer (the extractor's per-layer record).
#[derive(Debug, Clone, Deserialize)]
struct BundleLayer {
    #[serde(default)]
    label: Option<String>,
    /// `"vanilla"` | `"mod"` (the vanilla layer is skipped for identity).
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    workshop_id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    supported_version: Option<String>,
    #[serde(default)]
    descriptor: Option<String>,
}

/// The intermediate parse target: tolerant of BOTH shapes' top-level keys.
#[derive(Debug, Clone, Deserialize, Default)]
struct RawModSet {
    #[serde(default)]
    modset_id: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    load_order: Vec<RawLoadOrderItem>,
    /// Present only in the replay shape; when present it is the authoritative mod list.
    #[serde(default)]
    mods: Option<Vec<ModEntry>>,
}

impl<'de> Deserialize<'de> for ModSet {
    fn deserialize<D>(deserializer: D) -> Result<ModSet, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawModSet::deserialize(deserializer)?;

        // Display labels: a Label element keeps its string; a Layer element uses its
        // label/name.
        let load_order: Vec<String> = raw
            .load_order
            .iter()
            .map(|it| match it {
                RawLoadOrderItem::Label(s) => s.clone(),
                RawLoadOrderItem::Layer(l) => l
                    .label
                    .clone()
                    .or_else(|| l.name.clone())
                    .unwrap_or_default(),
            })
            .collect();

        // Identity-bearing mods: prefer an explicit `mods` (replay shape); else derive
        // from the `kind == "mod"` layers (bundle shape), SKIPPING the vanilla base.
        let mods: Vec<ModEntry> = match raw.mods {
            Some(m) => m,
            None => raw
                .load_order
                .iter()
                .filter_map(|it| match it {
                    RawLoadOrderItem::Layer(l)
                        if l.kind
                            .as_deref()
                            .map(|k| k.eq_ignore_ascii_case("mod"))
                            .unwrap_or(false) =>
                    {
                        Some(ModEntry {
                            name: l.name.clone(),
                            workshop_id: l.workshop_id.clone(),
                            version: l.version.clone(),
                            supported_version: l.supported_version.clone(),
                            descriptor: l.descriptor.clone(),
                        })
                    }
                    _ => None,
                })
                .collect(),
        };

        Ok(ModSet {
            modset_id: raw.modset_id.unwrap_or_default(),
            source: raw.source,
            load_order,
            mods,
        })
    }
}

impl ModSet {
    /// The vanilla (no-mod) mod-set.
    pub fn vanilla() -> ModSet {
        ModSet {
            modset_id: "vanilla".to_string(),
            source: None,
            load_order: Vec::new(),
            mods: Vec::new(),
        }
    }

    pub fn is_vanilla(&self) -> bool {
        self.mods.is_empty()
    }

    /// Recompute the identity digest from the mods, byte-for-byte matching the
    /// Python `schema.modset_id` (canonical-JSON of ordered `[ident, version]`
    /// pairs, SHA-1, first 16 hex chars; empty -> `"vanilla"`). Used so a bundle
    /// manifest authored WITHOUT a precomputed id, or with a stale one, is still
    /// compared on its true content (Codex review A: the digest is an index, the
    /// list is the evidence — but the index must be derivable, never trusted blind).
    pub fn resolved_id(&self) -> String {
        if self.mods.is_empty() {
            return "vanilla".to_string();
        }
        // Canonical JSON of [[ident, version], ...] — identical to the Python side.
        let pairs: Vec<[String; 2]> = self
            .mods
            .iter()
            .map(|m| [m.ident(), m.norm_version()])
            .collect();
        let canonical = serde_json::to_string(&pairs).unwrap_or_else(|_| String::from("[]"));
        sha1_hex16(canonical.as_bytes())
    }

    /// Load a mod-set from an `active_modset.json` file. Returns `Ok(None)` if the
    /// file does not exist (a bundle with no manifest is treated as vanilla by the
    /// caller); `Err` only on a present-but-unreadable / malformed file.
    pub fn load_manifest(path: &Path) -> Result<Option<ModSet>, ModError> {
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path).map_err(ModError::Io)?;
        let ms: ModSet = serde_json::from_str(&text).map_err(ModError::Json)?;
        Ok(Some(ms))
    }

    /// Parse a replay's `provenance.mod` value (a `serde_json::Value`). `Null` /
    /// absent -> `None` (the replay's mod identity is UNKNOWN — no fabrication).
    pub fn from_provenance(v: Option<&serde_json::Value>) -> Option<ModSet> {
        match v {
            None | Some(serde_json::Value::Null) => None,
            Some(val) => serde_json::from_value::<ModSet>(val.clone()).ok(),
        }
    }
}

/// Errors loading a mod-set manifest.
#[derive(Debug)]
pub enum ModError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for ModError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModError::Io(e) => write!(f, "io: {e}"),
            ModError::Json(e) => write!(f, "json: {e}"),
        }
    }
}
impl std::error::Error for ModError {}

// =============================================================================
// Bundle selection config (deliverable 1).
// =============================================================================

/// Which asset BUNDLE the app renders with, and the mod-set it represents.
///
/// A "bundle" is a directory the offline extractor produced: `gui/*.json` +
/// `sprites.json` + the font map + loc, RESOLVED against a specific mod-set. The
/// engine loads assets from `bundle_dir`; `modset` is that bundle's identity (its
/// `active_modset.json`, or vanilla when the manifest is absent).
#[derive(Debug, Clone)]
pub struct BundleConfig {
    /// The asset-bundle directory (the `game_assets`-shaped dir the engine reads).
    pub bundle_dir: PathBuf,
    /// The mod-set this bundle was extracted from (vanilla if no manifest present).
    pub modset: ModSet,
    /// True iff an `active_modset.json` manifest was actually found + loaded (so the
    /// caller can distinguish "vanilla bundle, manifest said so" from "no manifest,
    /// assumed vanilla").
    pub manifest_present: bool,
}

impl BundleConfig {
    /// The conventional manifest filename inside a bundle dir.
    pub const MANIFEST_NAME: &'static str = "active_modset.json";

    /// Resolve the active bundle from a bundle directory, loading its
    /// `active_modset.json` manifest if present (else treating it as vanilla).
    ///
    /// This is the DEFAULT path the requirement names: "DEFAULT to the modset
    /// manifest `game_assets/active_modset.json` if present, else vanilla."
    pub fn from_bundle_dir(bundle_dir: impl Into<PathBuf>) -> Result<BundleConfig, ModError> {
        let bundle_dir = bundle_dir.into();
        let manifest = bundle_dir.join(Self::MANIFEST_NAME);
        match ModSet::load_manifest(&manifest)? {
            Some(ms) => Ok(BundleConfig {
                bundle_dir,
                modset: ms,
                manifest_present: true,
            }),
            None => Ok(BundleConfig {
                bundle_dir,
                modset: ModSet::vanilla(),
                manifest_present: false,
            }),
        }
    }

    /// Resolve the active bundle from configuration, precedence HIGHEST first:
    ///   1. `explicit` argument (a CLI `--bundle <dir>` value),
    ///   2. `REPLAY_BUNDLE` env var,
    ///   3. `default_bundle_dir` (e.g. the repo's `game_assets`).
    /// The chosen dir's `active_modset.json` is then loaded (or vanilla).
    pub fn resolve(
        explicit: Option<&str>,
        default_bundle_dir: impl Into<PathBuf>,
    ) -> Result<BundleConfig, ModError> {
        let dir = explicit
            .map(PathBuf::from)
            .or_else(|| std::env::var("REPLAY_BUNDLE").ok().map(PathBuf::from))
            .unwrap_or_else(|| default_bundle_dir.into());
        Self::from_bundle_dir(dir)
    }
}

// =============================================================================
// Auto-match + mismatch (deliverable 2).
// =============================================================================

/// One concrete difference between the replay's mod-set and the selected bundle's,
/// so the UI can EXPLAIN a mismatch (Codex review A: don't compare digests only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModDelta {
    /// A mod the REPLAY has that the selected bundle does not.
    MissingFromBundle { ident: String, label: String },
    /// A mod the BUNDLE has that the replay's mod-set does not.
    ExtraInBundle { ident: String, label: String },
    /// Same mod (by ident) but a different version.
    VersionDiffers {
        ident: String,
        replay: String,
        bundle: String,
    },
    /// Same set of mods, but a different LOAD ORDER (which can change UI resolution).
    OrderDiffers,
}

impl ModDelta {
    /// A one-line, path-free description for the warning surface.
    pub fn describe(&self) -> String {
        match self {
            ModDelta::MissingFromBundle { label, .. } => {
                format!("replay used '{label}', missing from the selected mod-set")
            }
            ModDelta::ExtraInBundle { label, .. } => {
                format!("selected mod-set adds '{label}', not in the replay")
            }
            ModDelta::VersionDiffers {
                ident,
                replay,
                bundle,
            } => format!(
                "'{ident}' version differs (replay {}, selected {})",
                vshow(replay),
                vshow(bundle)
            ),
            ModDelta::OrderDiffers => {
                "same mods but a different load order (UI resolution may differ)".to_string()
            }
        }
    }
}

fn vshow(v: &str) -> &str {
    if v.is_empty() {
        "<none>"
    } else {
        v
    }
}

/// The outcome of auto-matching a replay against the selected bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    /// The selected bundle's mod-set matches the replay's recorded one — render away.
    Match,
    /// They DIFFER — the UI must surface a clear warning. `deltas` explains how; an
    /// empty `deltas` with non-equal ids means a digest disagreement we could not
    /// itemise (still a mismatch — never suppressed).
    Mismatch { deltas: Vec<ModDelta> },
    /// The replay's `provenance.mod` was absent/null — its mod identity is UNKNOWN.
    /// NOT a match and NOT a mismatch: the UI shows an advisory (can't auto-select),
    /// never a false all-clear (no fabrication from absence).
    ReplayUnknown,
}

impl MatchOutcome {
    pub fn is_match(&self) -> bool {
        matches!(self, MatchOutcome::Match)
    }
    /// True iff the UI should raise a visible MISMATCH warning (the user's explicit
    /// concern: vanilla-UI-over-a-mod-replay). `ReplayUnknown` is an advisory, not a
    /// mismatch warning.
    pub fn is_mismatch(&self) -> bool {
        matches!(self, MatchOutcome::Mismatch { .. })
    }
    /// The user-facing warning text, or `None` when there is nothing to warn about.
    pub fn warning(&self) -> Option<String> {
        match self {
            MatchOutcome::Match => None,
            MatchOutcome::ReplayUnknown => Some(
                "This replay did not record its mod-set, so the correct mod skin \
                 cannot be auto-selected. Verify the active mod-set matches the \
                 game this replay was recorded from."
                    .to_string(),
            ),
            MatchOutcome::Mismatch { deltas } => {
                let mut s = String::from(
                    "MOD MISMATCH: the selected mod-set does not match the one this \
                     replay was recorded under — the UI may render the wrong skin. ",
                );
                if deltas.is_empty() {
                    s.push_str("(mod-set identities differ)");
                } else {
                    let lines: Vec<String> = deltas.iter().map(|d| d.describe()).collect();
                    s.push_str(&lines.join("; "));
                }
                Some(s)
            }
        }
    }
}

/// Auto-match a replay's recorded mod-set against the SELECTED bundle's mod-set.
///
/// `replay_mod` is the parsed `provenance.mod` (`None` => unknown, no fabrication).
/// `bundle` is the active bundle's mod-set. The comparison is on RESOLVED identity
/// (recomputed digest) AND a full structured diff so a mismatch is explainable.
///
/// Reliability (Codex review A, addressed): a version difference, a load-ORDER
/// difference, an added/removed mod, or any digest disagreement all yield
/// [`MatchOutcome::Mismatch`]. We compute the digest from CONTENT (never trust a
/// possibly-stale stored `modset_id`), and we itemise the diff rather than relying
/// on the digest alone.
pub fn match_replay(replay_mod: Option<&ModSet>, bundle: &ModSet) -> MatchOutcome {
    let replay = match replay_mod {
        None => return MatchOutcome::ReplayUnknown,
        Some(r) => r,
    };

    let same_id = replay.resolved_id() == bundle.resolved_id();
    let deltas = diff_modsets(replay, bundle);

    if same_id && deltas.is_empty() {
        MatchOutcome::Match
    } else {
        // If the ids disagree we MUST warn even if we somehow itemised no deltas
        // (defensive — never suppress a digest disagreement).
        MatchOutcome::Mismatch { deltas }
    }
}

/// Itemise the differences between two mod-sets (replay vs bundle). Compares the set
/// of mods by `ident()`, flags version diffs on shared mods, and — when the sets are
/// otherwise identical — flags a pure load-ORDER difference.
fn diff_modsets(replay: &ModSet, bundle: &ModSet) -> Vec<ModDelta> {
    use std::collections::BTreeMap;
    let mut out = Vec::new();

    let r_by: BTreeMap<String, &ModEntry> = replay.mods.iter().map(|m| (m.ident(), m)).collect();
    let b_by: BTreeMap<String, &ModEntry> = bundle.mods.iter().map(|m| (m.ident(), m)).collect();

    // Mods the replay had that the bundle lacks.
    for (id, m) in &r_by {
        if !b_by.contains_key(id) {
            out.push(ModDelta::MissingFromBundle {
                ident: id.clone(),
                label: m.label(),
            });
        }
    }
    // Mods the bundle adds that the replay lacks.
    for (id, m) in &b_by {
        if !r_by.contains_key(id) {
            out.push(ModDelta::ExtraInBundle {
                ident: id.clone(),
                label: m.label(),
            });
        }
    }
    // Version diffs on shared mods.
    for (id, rm) in &r_by {
        if let Some(bm) = b_by.get(id) {
            let rv = rm.norm_version();
            let bv = bm.norm_version();
            if rv != bv {
                out.push(ModDelta::VersionDiffers {
                    ident: id.clone(),
                    replay: rv,
                    bundle: bv,
                });
            }
        }
    }
    // Same SET of mods (no add/remove) but a different ORDER -> flag order. Order
    // can change which mod's file wins under Clausewitz overlay/replace_path rules.
    if out.is_empty() {
        let r_order: Vec<String> = replay.mods.iter().map(|m| m.ident()).collect();
        let b_order: Vec<String> = bundle.mods.iter().map(|m| m.ident()).collect();
        if r_order != b_order {
            out.push(ModDelta::OrderDiffers);
        }
    }
    out
}

// =============================================================================
// A tiny, dependency-free SHA-1 (so the digest matches Python's hashlib.sha1
// without pulling in a crypto crate). SHA-1 here is a content FINGERPRINT for
// identity comparison, not a security primitive.
// =============================================================================

fn sha1_hex16(data: &[u8]) -> String {
    let digest = sha1(data);
    let mut s = String::with_capacity(16);
    // first 8 bytes -> 16 hex chars (matches Python's [:16] of the full hexdigest).
    for b in &digest[..8] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let ml = (data.len() as u64).wrapping_mul(8);

    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mod_entry(name: &str, wid: Option<&str>, ver: &str) -> ModEntry {
        ModEntry {
            name: Some(name.to_string()),
            workshop_id: wid.map(|s| s.to_string()),
            version: Some(ver.to_string()),
            supported_version: None,
            descriptor: None,
        }
    }
    fn modset(mods: Vec<ModEntry>) -> ModSet {
        ModSet {
            modset_id: String::new(), // force resolved_id recompute
            source: Some("dlc_load.json".into()),
            load_order: mods
                .iter()
                .map(|m| m.descriptor.clone().unwrap_or_else(|| m.ident()))
                .collect(),
            mods,
        }
    }

    // --- SHA-1 / digest parity with Python ---------------------------------

    #[test]
    fn sha1_matches_known_vector() {
        // SHA1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
        assert_eq!(&sha1_hex16(b"abc"), "a9993e364706816a");
    }

    #[test]
    fn vanilla_digest_is_sentinel() {
        assert_eq!(ModSet::vanilla().resolved_id(), "vanilla");
        assert_eq!(modset(vec![]).resolved_id(), "vanilla");
    }

    /// The BUNDLE manifest shape (`active_modset.json`): `load_order` is an array of
    /// LAYER OBJECTS with `kind`. Identity MUST come only from the `kind == "mod"`
    /// layers — the vanilla base layer is skipped (Codex-flagged: including it poisons
    /// the digest). This is the exact shape the offline extractor writes.
    #[test]
    fn deserializes_bundle_manifest_shape_and_skips_vanilla() {
        let manifest = json!({
            "schema": 1,
            "selection_source": "launcher dlc_load.json",
            "load_order": [
                {"load_index":0,"label":"vanilla","kind":"vanilla","name":null,
                 "workshop_id":null,"version":null,"descriptor":null,
                 "replace_path":[],"dependencies":[]},
                {"load_index":1,"label":"AMSB - DEV","kind":"mod","name":"AMSB - DEV",
                 "workshop_id":null,"version":"1.18","supported_version":"1.19.*",
                 "descriptor":"mod/test.mod","replace_path":["common/countries"],
                 "dependencies":[]}
            ],
            "mod_count": 1,
            "warnings": [],
            "note": "..."
        });
        let ms: ModSet = serde_json::from_value(manifest).expect("bundle manifest must parse");
        // exactly ONE identity-bearing mod (vanilla layer excluded).
        assert_eq!(ms.mods.len(), 1, "the vanilla base layer must NOT be a mod");
        assert_eq!(ms.mods[0].name.as_deref(), Some("AMSB - DEV"));
        assert_eq!(ms.mods[0].version.as_deref(), Some("1.18"));
        // display load order keeps BOTH labels.
        assert_eq!(
            ms.load_order,
            vec!["vanilla".to_string(), "AMSB - DEV".to_string()]
        );
        // it is NOT vanilla (a real mod is present).
        assert_ne!(ms.resolved_id(), "vanilla");
        assert!(!ms.is_vanilla());
    }

    /// The REPLAY shape (`provenance.mod`): an explicit `mods` + string `load_order`.
    /// Both shapes must still parse (the custom deserialize handles both).
    #[test]
    fn deserializes_replay_provenance_shape() {
        let prov = json!({
            "modset_id": "deadbeef",
            "load_order": ["amsb"],
            "mods": [{"name":"AMSB","workshop_id":"123","version":"1.18"}]
        });
        let ms: ModSet = serde_json::from_value(prov).expect("replay shape must parse");
        assert_eq!(ms.mods.len(), 1);
        assert_eq!(ms.mods[0].workshop_id.as_deref(), Some("123"));
        assert_eq!(ms.modset_id, "deadbeef");
    }

    /// Identity namespacing trap (Codex §2): a bundle mod `name:AMSB - DEV` and a
    /// replay mod `workshop:123` are DIFFERENT identities even at the same version, so
    /// they must NOT match (a vanilla-name mod must never alias a workshop mod).
    #[test]
    fn name_and_workshop_identity_never_alias() {
        let bundle = modset(vec![mod_entry("AMSB - DEV", None, "1.18")]);
        let replay = modset(vec![mod_entry("AMSB", Some("123"), "1.18")]);
        assert_ne!(bundle.resolved_id(), replay.resolved_id());
        assert!(match_replay(Some(&replay), &bundle).is_mismatch());
    }

    #[test]
    fn resolved_id_matches_python_real_test_mod() {
        // CROSS-LANGUAGE PARITY: the Python `schema.modset_id` digests the REAL host
        // `test.mod` (name "AMSB - DEV", no workshop id, version "1.18") with the
        // NAMESPACED `name:` ident to "e23fc38a4e83b612". The Rust digest of the same
        // mod-set MUST equal that exact value — proving the capture-side id and the
        // native matcher agree byte-for-byte.
        let ms = modset(vec![mod_entry("AMSB - DEV", None, "1.18")]);
        assert_eq!(ms.resolved_id(), "e23fc38a4e83b612");
    }

    #[test]
    fn resolved_id_matches_python_canonical_json() {
        // The Python side digests json.dumps([[ident,ver],...], separators=(",",":")).
        // For one HMM workshop mod the NAMESPACED canonical string is
        // [["workshop:3365264825","1.18"]].
        let ms = modset(vec![mod_entry("HMM", Some("3365264825"), "1.18")]);
        let canonical = r#"[["workshop:3365264825","1.18"]]"#;
        assert_eq!(ms.resolved_id(), sha1_hex16(canonical.as_bytes()));
        assert_eq!(ms.resolved_id(), "0b2726d1c4f45cff");
    }

    // --- the MATCH path ----------------------------------------------------

    #[test]
    fn local_name_does_not_collide_with_workshop_id() {
        // Codex review A: a LOCAL mod named "12345" must NOT be identified the same
        // as a WORKSHOP mod whose id is "12345" (namespaced idents prevent it).
        let local = ModEntry {
            name: Some("12345".into()),
            workshop_id: None,
            version: Some("1.0".into()),
            ..Default::default()
        };
        let workshop = ModEntry {
            name: Some("Some Name".into()),
            workshop_id: Some("12345".into()),
            version: Some("1.0".into()),
            ..Default::default()
        };
        assert_ne!(local.ident(), workshop.ident());
        let replay = modset(vec![local]);
        let bundle = modset(vec![workshop]);
        // Different identity => MISMATCH, never a false match.
        assert!(match_replay(Some(&replay), &bundle).is_mismatch());
    }

    #[test]
    fn exact_match_is_match() {
        let a = modset(vec![mod_entry("AMSB", None, "1.18")]);
        let b = modset(vec![mod_entry("AMSB", None, "1.18")]);
        let out = match_replay(Some(&a), &b);
        assert_eq!(out, MatchOutcome::Match);
        assert!(out.is_match());
        assert!(out.warning().is_none());
    }

    #[test]
    fn vanilla_replay_vs_vanilla_bundle_is_match() {
        let out = match_replay(Some(&ModSet::vanilla()), &ModSet::vanilla());
        assert!(out.is_match());
    }

    // --- the MISMATCH paths (the user's explicit concern) ------------------

    #[test]
    fn vanilla_bundle_over_mod_replay_warns() {
        // THE failure the user wants prevented: a mod replay, vanilla UI selected.
        let replay = modset(vec![mod_entry("AMSB", None, "1.18")]);
        let bundle = ModSet::vanilla();
        let out = match_replay(Some(&replay), &bundle);
        assert!(out.is_mismatch(), "vanilla-over-mod MUST warn");
        let w = out.warning().unwrap();
        assert!(w.contains("MOD MISMATCH"));
        assert!(w.contains("AMSB"));
    }

    #[test]
    fn version_difference_warns() {
        let replay = modset(vec![mod_entry("AMSB", Some("123"), "1.18")]);
        let bundle = modset(vec![mod_entry("AMSB", Some("123"), "1.19")]);
        let out = match_replay(Some(&replay), &bundle);
        assert!(out.is_mismatch());
        match &out {
            MatchOutcome::Mismatch { deltas } => {
                assert!(deltas.iter().any(|d| matches!(
                    d,
                    ModDelta::VersionDiffers { ident, replay, bundle }
                        if ident == "workshop:123" && replay == "1.18" && bundle == "1.19"
                )));
            }
            _ => panic!("expected mismatch"),
        }
        assert!(out.warning().unwrap().contains("version differs"));
    }

    #[test]
    fn load_order_difference_warns() {
        let m1 = mod_entry("A", Some("1"), "1.0");
        let m2 = mod_entry("B", Some("2"), "1.0");
        let replay = modset(vec![m1.clone(), m2.clone()]);
        let bundle = modset(vec![m2, m1]); // same mods, swapped order
        let out = match_replay(Some(&replay), &bundle);
        assert!(out.is_mismatch(), "a load-order difference must warn");
        match &out {
            MatchOutcome::Mismatch { deltas } => {
                assert_eq!(deltas, &vec![ModDelta::OrderDiffers]);
            }
            _ => panic!("expected mismatch"),
        }
    }

    #[test]
    fn added_and_removed_mods_are_itemised() {
        let replay = modset(vec![mod_entry("A", Some("1"), "1.0")]);
        let bundle = modset(vec![mod_entry("B", Some("2"), "1.0")]);
        let out = match_replay(Some(&replay), &bundle);
        match &out {
            MatchOutcome::Mismatch { deltas } => {
                assert!(deltas
                    .iter()
                    .any(|d| matches!(d, ModDelta::MissingFromBundle { ident, .. } if ident == "workshop:1")));
                assert!(deltas.iter().any(
                    |d| matches!(d, ModDelta::ExtraInBundle { ident, .. } if ident == "workshop:2")
                ));
            }
            _ => panic!("expected mismatch"),
        }
    }

    // --- the UNKNOWN path (no fabrication from absence) ---------------------

    #[test]
    fn absent_provenance_is_unknown_not_match() {
        let bundle = modset(vec![mod_entry("AMSB", None, "1.18")]);
        let out = match_replay(None, &bundle);
        assert_eq!(out, MatchOutcome::ReplayUnknown);
        assert!(!out.is_match(), "unknown must NOT be a false all-clear");
        assert!(!out.is_mismatch(), "unknown must NOT be a false alarm");
        assert!(out
            .warning()
            .unwrap()
            .contains("did not record its mod-set"));
    }

    #[test]
    fn from_provenance_null_is_none() {
        assert!(ModSet::from_provenance(None).is_none());
        assert!(ModSet::from_provenance(Some(&serde_json::Value::Null)).is_none());
    }

    #[test]
    fn from_provenance_parses_python_shape() {
        // exactly the dict the Python mod_identity.read_modset emits.
        let v = json!({
            "modset_id": "e3f78c919cdba31d",
            "source": "dlc_load.json",
            "load_order": ["test.mod"],
            "mods": [{
                "name": "AMSB - DEV", "version": "1.18",
                "supported_version": "1.19.*", "workshop_id": null,
                "descriptor": "test.mod"
            }]
        });
        let ms = ModSet::from_provenance(Some(&v)).expect("parse");
        assert_eq!(ms.mods.len(), 1);
        assert_eq!(ms.mods[0].name.as_deref(), Some("AMSB - DEV"));
        // a bundle extracted from the SAME mod-set matches this replay.
        let bundle = ms.clone();
        assert!(match_replay(Some(&ms), &bundle).is_match());
    }

    // --- bundle config (deliverable 1) -------------------------------------

    #[test]
    fn bundle_without_manifest_is_vanilla() {
        let dir = std::env::temp_dir().join(format!("modset_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = BundleConfig::from_bundle_dir(&dir).unwrap();
        assert!(!cfg.manifest_present);
        assert!(cfg.modset.is_vanilla());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundle_loads_active_modset_manifest() {
        let dir = std::env::temp_dir().join(format!("modset_test_m_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({
            "modset_id": "abc", "source": "dlc_load.json",
            "load_order": ["test.mod"],
            "mods": [{"name":"AMSB - DEV","version":"1.18","workshop_id":null,
                      "descriptor":"test.mod"}]
        });
        std::fs::write(
            dir.join(BundleConfig::MANIFEST_NAME),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let cfg = BundleConfig::from_bundle_dir(&dir).unwrap();
        assert!(cfg.manifest_present);
        assert_eq!(cfg.modset.mods.len(), 1);
        assert!(!cfg.modset.is_vanilla());

        // a replay recorded under this same mod-set auto-matches the bundle.
        let replay = cfg.modset.clone();
        assert!(match_replay(Some(&replay), &cfg.modset).is_match());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_prefers_explicit_over_default() {
        let dir = std::env::temp_dir().join(format!("modset_test_r_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = BundleConfig::resolve(Some(dir.to_str().unwrap()), "F:/does/not/exist").unwrap();
        assert_eq!(cfg.bundle_dir, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
