//! BATCH 0 deliverable 3 — the BIND / RENDER CONTEXTS + the no-fabrication value
//! types every view builder is handed.
//!
//! A view BUILDER is PURE: it is given a [`BindContext`] (the immutable snapshot for
//! the current hour + the static [`Stage`] data + loc + assets-resolver + a fidelity
//! sink) and returns a fresh bound output. It does NOT reach app globals, never
//! mutates the snapshot, and — crucially — physically CANNOT emit a fabricated `0`
//! for a blocked field: the only way to turn a captured value into display text is
//! through [`Captured::display_with`] / [`DisplayAtom`], which renders the
//! not-captured glyph ("—") for an Absent/Null/blocked field (Codex (e): the
//! no-fabrication invariant lives in the type, not in discipline).
//!
//! The RENDER context ([`RenderContext`]) carries the per-frame screen size + the
//! map-pass handles a `Drawable` needs to composite. Draw is replayable: the builder
//! produced the scene-graph once per scrub tick; the draw re-runs only the GPU work.

use std::path::PathBuf;

use assets::resolver::GameResolver;
use assets::SpriteManifest;
use bind::format::NOT_CAPTURED;
use bind::Fidelity;
use gui_runtime::{GuiFile, Loc};
use snapshot::Snapshot;

/// The static (hour-independent) stage assets, loaded ONCE at startup and then shared
/// (immutably) by every view builder + the compose pass. This is the SAME data the
/// old single-`Stage` carried, kept here so the host owns it and the views borrow it.
///
/// `gui_files` is a small registry of parsed `.gui` files keyed by their *file* name
/// (e.g. `"province_state.json"`), so a view descriptor that names a ROOT/TEMPLATE in
/// a file resolves it through here without each view re-parsing the file.
pub struct Stage {
    pub resolver: GameResolver,
    pub manifest: SpriteManifest,
    pub loc: Loc,
    /// Parsed `.gui` files, keyed by file name. The compose helpers (gridbox/template
    /// instantiation) need the *raw* `GuiFile` for template lookups, so we retain them.
    pub gui_files: std::collections::HashMap<String, GuiFile>,
    pub map_dir: PathBuf,
    /// The assets root (`game_assets/`), so a view can resolve its `.gui` file lazily
    /// if it was not pre-parsed at startup.
    pub assets_dir: PathBuf,
}

impl Stage {
    /// Look up a pre-parsed `.gui` file by name (e.g. `"topbar.json"`).
    pub fn gui_file(&self, file: &str) -> Option<&GuiFile> {
        self.gui_files.get(file)
    }

    /// Load a SET of parsed `.gui` files by name from a `gui/` directory into a map,
    /// keyed by file name. The binary calls this with [`super::registry::ViewRegistry::
    /// required_gui_files`] so EXACTLY the `.gui` files the registered views need are
    /// pre-loaded into [`Stage::gui_files`] — then a cluster builder fetches its real
    /// layout via [`BindContext::gui_symbol`] without ever loading a `.gui` itself.
    ///
    /// A file that fails to read/parse is SKIPPED with a warning (never fabricated) and
    /// recorded in the returned `failed` list, so a cluster's `gui_symbol` honestly
    /// returns `None` for it (the view badges unavailable, never invents a layout). This
    /// is READ-ONLY and retry-tolerant of the concurrent asset extractor.
    pub fn load_gui_set(
        gui_dir: &std::path::Path,
        files: &[&str],
    ) -> (std::collections::HashMap<String, GuiFile>, Vec<String>) {
        let mut map = std::collections::HashMap::new();
        let mut failed = Vec::new();
        for f in files {
            match GuiFile::load(&gui_dir.join(f)) {
                Ok(g) => {
                    map.insert(f.to_string(), g);
                }
                Err(e) => {
                    eprintln!("[stage] WARNING: gui set: parse {f}: {e}");
                    failed.push(f.to_string());
                }
            }
        }
        (map, failed)
    }
}

/// The structural trichotomy of a CAPTURED value, lifted to a typed wrapper so a
/// builder works on `Captured<T>` rather than raw JSON. This is the in-app mirror of
/// `bind::ValueState` (Absent / Null / Measured) — the no-fabrication contract made
/// un-bypassable: there is no `T` to format until you have `Measured(T)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Captured<T> {
    /// A key along the path was absent — never captured.
    Absent,
    /// Present but JSON null — captured-but-unobtained.
    Null,
    /// A real measured value (a number incl. a measured `0`, or a string).
    Measured(T),
}

impl<T> Captured<T> {
    /// True iff there is a real measured value behind this.
    pub fn is_measured(&self) -> bool {
        matches!(self, Captured::Measured(_))
    }

    /// Map the measured payload, preserving Absent/Null.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Captured<U> {
        match self {
            Captured::Measured(v) => Captured::Measured(f(v)),
            Captured::Absent => Captured::Absent,
            Captured::Null => Captured::Null,
        }
    }

    /// THE no-fabrication entry point: turn a captured value into a [`DisplayAtom`].
    /// A measured value is formatted (and badged `Exact` unless overridden via
    /// [`display_with_fidelity`](Self::display_with_fidelity)); an Absent/Null value
    /// becomes the not-captured glyph — there is NO path that yields a fabricated `0`.
    pub fn display_with(self, fmt: impl FnOnce(T) -> String) -> DisplayAtom {
        self.display_with_fidelity(Fidelity::Exact, fmt)
    }

    /// As [`display_with`](Self::display_with) but stamping a chosen fidelity on the
    /// MEASURED case (e.g. `Derived`). Absent/Null always render not-captured.
    pub fn display_with_fidelity(
        self,
        fidelity: Fidelity,
        fmt: impl FnOnce(T) -> String,
    ) -> DisplayAtom {
        match self {
            Captured::Measured(v) => DisplayAtom::text(fmt(v), fidelity),
            Captured::Absent => DisplayAtom::unavailable_because(UnavailableReason::Absent),
            Captured::Null => DisplayAtom::unavailable_because(UnavailableReason::Null),
        }
    }
}

/// Why a field is not captured (kept distinct so a UI can badge missing/null/blocked
/// differently — Codex: `Unavailable` should carry a reason, not just the glyph).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// A key along the path was absent — never captured.
    Absent,
    /// Present but JSON null — captured-but-unobtained.
    Null,
    /// Schema-present-but-unfed / runtime-computed value we do not store (the binding
    /// is badged `Unavailable` regardless of the snapshot).
    Blocked,
}

/// A value READY FOR THE SCREEN: either real measured text (with its fidelity badge)
/// or the not-captured glyph. A view binds an element to a `DisplayAtom`; it cannot
/// bind a bare `0`/`i32`/`String`, so a fabricated value can never reach a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayAtom {
    /// Real, measured text + its honesty badge.
    Text { text: String, fidelity: Fidelity },
    /// A blocked / absent / null field: render the not-captured glyph ("—"). This is
    /// `Fidelity::Unavailable` and carries NO numeric payload by construction; the
    /// `reason` preserves the Absent/Null/Blocked distinction for a richer badge.
    Unavailable {
        text: &'static str,
        reason: UnavailableReason,
    },
}

impl DisplayAtom {
    pub fn text(text: impl Into<String>, fidelity: Fidelity) -> DisplayAtom {
        DisplayAtom::Text {
            text: text.into(),
            fidelity,
        }
    }

    /// The badged not-captured atom (Absent reason by default) — the ONLY thing a
    /// blocked field can be (never 0).
    pub fn unavailable() -> DisplayAtom {
        DisplayAtom::Unavailable {
            text: NOT_CAPTURED,
            reason: UnavailableReason::Absent,
        }
    }

    /// The not-captured atom with an explicit reason.
    pub fn unavailable_because(reason: UnavailableReason) -> DisplayAtom {
        DisplayAtom::Unavailable {
            text: NOT_CAPTURED,
            reason,
        }
    }

    /// The string that should be drawn for this atom (measured text or "—").
    pub fn as_str(&self) -> &str {
        match self {
            DisplayAtom::Text { text, .. } => text,
            DisplayAtom::Unavailable { text, .. } => text,
        }
    }

    /// The not-captured reason (Absent/Null/Blocked), if this atom is unavailable.
    pub fn reason(&self) -> Option<UnavailableReason> {
        match self {
            DisplayAtom::Unavailable { reason, .. } => Some(*reason),
            DisplayAtom::Text { .. } => None,
        }
    }

    /// The honesty badge for this atom.
    pub fn fidelity(&self) -> Fidelity {
        match self {
            DisplayAtom::Text { fidelity, .. } => *fidelity,
            DisplayAtom::Unavailable { .. } => Fidelity::Unavailable,
        }
    }

    /// True iff this atom is the not-captured glyph (a blocked field).
    pub fn is_unavailable(&self) -> bool {
        matches!(self, DisplayAtom::Unavailable { .. })
    }
}

/// A per-view fidelity report: how many fields a builder bound at each honesty level,
/// so the host can surface a per-view honesty badge + the accounting tests can assert
/// "a blocked field badged `unavailable`, not a fabricated 0".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FidelityReport {
    pub exact: u32,
    pub derived: u32,
    pub unavailable: u32,
}

impl FidelityReport {
    /// Tally one bound atom into the report.
    pub fn record(&mut self, atom: &DisplayAtom) {
        match atom.fidelity() {
            Fidelity::Exact => self.exact += 1,
            Fidelity::Derived => self.derived += 1,
            Fidelity::Unavailable => self.unavailable += 1,
        }
    }

    /// The worst (least faithful) badge present, for a one-word per-view summary.
    pub fn worst_badge(&self) -> &'static str {
        if self.unavailable > 0 {
            "unavailable"
        } else if self.derived > 0 {
            "derived"
        } else {
            "exact"
        }
    }

    pub fn total(&self) -> u32 {
        self.exact + self.derived + self.unavailable
    }
}

/// The immutable CONTEXT handed to every view builder's `bind`. It carries the current
/// hour's snapshot (shared, immutable `&Snapshot`), the static [`Stage`], the observed
/// country, the screen rect, and the date — everything a pure builder needs.
///
/// A `BindContext` exposes the snapshot ONLY through the [`Captured`]-returning
/// accessors below, so a builder reads `country_field` / `state_field` and gets the
/// trichotomy directly. The raw `snapshot()` escape hatch exists for the migrated
/// surfaces that drive the existing `bind::apply_*` layer (themselves no-fabrication),
/// and is documented as the audited seam.
pub struct BindContext<'a> {
    pub stage: &'a Stage,
    pub snapshot: &'a Snapshot,
    /// The observed country tag (whose topbar/focus tree we show).
    pub country: &'a str,
    /// The full screen rect this frame (for layout).
    pub screen: gui_runtime::Rect,
    /// The current in-game date string (top-level snapshot field).
    pub date: &'a str,
    /// The cursor position this frame (screen px) — overlays use it for hover-on-bind
    /// only when their persistent instance state already captured it via `input`.
    pub cursor: (f32, f32),
    /// The live MAP CAMERA (pan/zoom), so an overlay that places things on the map
    /// (combat bubbles, on-map icons) can project to screen space. `None` before the
    /// map view is seeded.
    pub map_camera: Option<crate::camera::MapCamera>,
}

impl<'a> BindContext<'a> {
    /// Read a dotted path under the observed country as a [`Captured`] string.
    /// (Numeric callers map afterwards; strings stay strings.)
    pub fn country_field(&self, tag: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        classify_value(self.snapshot.country_path(tag, dotted))
    }

    /// Read a dotted path under a state id as a [`Captured`] value.
    pub fn state_field(&self, sid: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        classify_value(self.snapshot.state_path(sid, dotted))
    }

    // =====================================================================================
    // TOP-LEVEL / FOLD-IN DOMAIN ACCESSORS (Codex (a): the no-fabrication seam for the
    // domains that are NOT under `countries.<TAG>` / `states.<id>`).
    //
    // Cluster builders can reach per-country/per-state data through `country_field` /
    // `state_field`, but the military / combat / logistics clusters bind TOP-LEVEL domains
    // (`divisions[]`, `combats[]`, `order_of_battle.…`, `strategic_air.…`, top-level
    // `intel.…`, `world.…`, `world_events.…`, `raids.…`). The no-fabrication guard
    // (`views::digest::no_fabricated_default_in_cluster_views`) BANS the raw `ctx.snapshot(`
    // seam in cluster files, so a cluster cannot reach these domains itself. These FIRST-
    // CLASS accessors are the audited replacement for the per-cluster `designs_domain` shim:
    // each returns the SAME `Captured` Absent/Null/Measured trichotomy a `country_field`
    // does — **Absent** when the domain key is missing from this hour's snapshot (never
    // captured), **Null** when present-but-JSON-null (captured-but-unobtained), **Measured**
    // when present — so a blocked/absent domain renders the not-captured glyph, never a
    // fabricated default. They do NOT expose the raw `snapshot()` value: a builder still
    // walks the returned `&Value` with the no-fabrication `display_with` / `classify_value`
    // path, exactly as it does for a country field. The guard keeps passing because a
    // cluster calls e.g. `ctx.divisions()` — there is no `.snapshot(` substring in the
    // cluster source. (The internal `self.snapshot.domain(..)` lives HERE, in
    // `app/src/host`, which the cluster-only source scan does not walk.)
    // =====================================================================================

    /// The TOP-LEVEL `divisions` domain (`divisions[]`, §6) as a [`Captured`] value.
    /// Absent when this hour's snapshot has no `divisions` key, Measured when present.
    pub fn divisions(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("divisions")
    }

    /// The TOP-LEVEL `combats` domain (`combats[]`) as a [`Captured`] value (land/naval/
    /// air combat list). Absent when the snapshot has no `combats` key.
    pub fn combats(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("combats")
    }

    /// The TOP-LEVEL `order_of_battle` domain (armies / army_groups / fleets / ships /
    /// air_wings / battle_plan_orders / by_country) as a [`Captured`] value.
    pub fn order_of_battle(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("order_of_battle")
    }

    /// The TOP-LEVEL `intel` domain (agency / operatives / operations / decryption /
    /// cryptology_live / peace_conference, §3.8) as a [`Captured`] value. This is the
    /// GLOBAL intel domain — NOT `countries.<TAG>.intel`, which a builder reads via
    /// [`country_field`](Self::country_field).
    pub fn intel(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("intel")
    }

    /// The TOP-LEVEL `world` domain (global `variables` / `flags` / `power_balances` /
    /// `world_tension_inputs`, §18) as a [`Captured`] value. Absent when unfed (the demo
    /// snapshot has no `world` key) — so a VP/score banner badges, never fabricates `0`.
    pub fn world(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("world")
    }

    /// The TOP-LEVEL `world_events` domain (`threat[]` world-tension contributors, §17)
    /// as a [`Captured`] value. Absent when the domain is not folded into the snapshot.
    pub fn world_events(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("world_events")
    }

    /// The TOP-LEVEL `strategic_air` domain (`air_bases[]` capacity, §strategic-air) as a
    /// [`Captured`] value. Absent when the snapshot has no `strategic_air` key.
    pub fn strategic_air(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("strategic_air")
    }

    /// The TOP-LEVEL `raids` domain (`countries` / `targets`, §16) as a [`Captured`]
    /// value. Absent when the snapshot has no `raids` key.
    pub fn raids(&self) -> Captured<&'a serde_json::Value> {
        self.domain_field("raids")
    }

    /// The shared TOP-LEVEL DOMAIN accessor backing all the named domain methods above
    /// (and available for any other fold-in domain a future cluster references). Converts
    /// a `Snapshot::domain(key)` lookup into the [`Captured`] Absent/Null/Measured
    /// trichotomy, preserving the no-fabrication contract: it returns a borrowed `&Value`
    /// on Measured (so the builder walks it with the same honesty path as a country
    /// field), **Absent** when the key is missing, **Null** when present-but-null. It does
    /// NOT hand back the raw `&Snapshot` — there is no way to widen this into the banned
    /// seam. This supersedes the per-cluster `designs_domain` shim
    /// (`views::clusters::designers`); a cluster reads `ctx.divisions()` etc. instead.
    pub fn domain_field(&self, key: &str) -> Captured<&'a serde_json::Value> {
        classify_value(self.snapshot.domain(key))
    }

    /// The AUDITED raw-snapshot seam (Codex (e)): used only by the migrated surfaces
    /// that drive the already-no-fabrication `bind::apply_*` layer. New cluster views
    /// should prefer [`country_field`](Self::country_field) /
    /// [`state_field`](Self::state_field) + [`Captured::display_with`].
    pub fn snapshot(&self) -> &'a Snapshot {
        self.snapshot
    }

    /// Resolve a registered `.gui` SYMBOL (a window ROOT or a repeated row/card TEMPLATE)
    /// to a freshly-built, UN-LAID-OUT scene-graph [`Node`](gui_runtime::scene::Node) — the
    /// REAL parsed game layout. This is the BATCH-1 `.gui`-set seam: a cluster builder
    /// fetches its view's actual `.gui` root/template by symbol and binds its named
    /// elements to data, instead of hand-rolling synthetic chrome. The `.gui` files were
    /// pre-loaded into [`Stage::gui_files`] by the binary from
    /// [`ViewRegistry::required_gui_files`], so a cluster NEVER loads a `.gui` itself
    /// (the conflict-free seam). Returns `None` if the file was not pre-loaded or the
    /// symbol name is absent — never a fabricated node. The caller then runs
    /// `gui_runtime::layout` on the returned root with its target rect.
    pub fn gui_symbol(
        &self,
        sym: &crate::host::registry::GuiSymbolRef,
    ) -> Option<gui_runtime::scene::Node> {
        let file = self.stage.gui_file(sym.file)?;
        let raw = file
            .windows
            .iter()
            .find(|w| w.name.as_deref() == Some(sym.name))?;
        Some(gui_runtime::scene::Node::from_raw(
            raw,
            &self.stage.manifest,
            &self.stage.loc,
        ))
    }

    /// Convenience: resolve a window ROOT by `(file, name)` to a scene-graph node.
    pub fn view_root(
        &self,
        file: &'static str,
        name: &'static str,
    ) -> Option<gui_runtime::scene::Node> {
        self.gui_symbol(&crate::host::registry::GuiSymbolRef::root(file, name))
    }

    /// Convenience: resolve a repeated row/card TEMPLATE by `(file, name)`.
    pub fn view_template(
        &self,
        file: &'static str,
        name: &'static str,
    ) -> Option<gui_runtime::scene::Node> {
        self.gui_symbol(&crate::host::registry::GuiSymbolRef::template(file, name))
    }
}

/// Classify an `Option<&Value>` lookup into the [`Captured`] trichotomy (the in-app
/// mirror of `bind::classify`, but carrying the value through on Measured).
pub fn classify_value(opt: Option<&serde_json::Value>) -> Captured<&serde_json::Value> {
    match opt {
        None => Captured::Absent,
        Some(serde_json::Value::Null) => Captured::Null,
        Some(v) => Captured::Measured(v),
    }
}

#[cfg(test)]
mod gui_set_tests {
    use super::*;

    /// The real `game_assets/gui` directory (sibling of `replay_native`). The test is
    /// SKIPPED (not failed) if the extracted assets are not present, so CI without the
    /// (gitignored) game assets still passes.
    fn gui_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui")
    }

    /// BATCH-1 `.gui`-set loader: a sample view's `.gui` loads + resolves BY SYMBOL into a
    /// real, non-empty scene-graph root (the seam clusters bind against). No-fabrication:
    /// an absent file / unknown symbol returns `None`, never a fabricated node.
    #[test]
    fn sample_view_gui_loads_and_resolves_by_symbol() {
        let dir = gui_dir();
        if !dir.join("topbar.json").exists() {
            eprintln!("[gui_set_tests] skipped: game_assets/gui not extracted");
            return;
        }
        // load the SET (just the two stable migrated-surface files for the test).
        let (map, failed) = Stage::load_gui_set(&dir, &["topbar.json", "province_state.json"]);
        assert!(
            failed.is_empty(),
            "sample gui files must parse: failed={failed:?}"
        );
        assert_eq!(map.len(), 2);

        // build a minimal Stage carrying the loaded set + an empty resolver/manifest/loc.
        let stage = Stage {
            resolver: GameResolver::new(None, None),
            manifest: SpriteManifest {
                sprites: std::collections::HashMap::new(),
            },
            loc: Loc::new(),
            gui_files: map,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        };
        let snap: Snapshot =
            serde_json::from_value(serde_json::json!({ "date": "1936.1.1.12" })).unwrap();
        let ctx = BindContext {
            stage: &stage,
            snapshot: &snap,
            country: "GER",
            screen: gui_runtime::Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1936.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        };

        // resolve the topbar ROOT by symbol -> a real scene-graph node with children.
        let root = ctx
            .view_root("topbar.json", "topbar")
            .expect("topbar root resolves by symbol");
        assert_eq!(root.name, "topbar");
        assert!(
            root.count() > 1,
            "the real topbar root has a populated subtree"
        );

        // a province_state TEMPLATE resolves too (the row/card seam clusters use).
        let tmpl = ctx.view_template("province_state.json", "state_building_entry");
        assert!(
            tmpl.is_some(),
            "a known province_state template resolves by symbol"
        );

        // no-fabrication: an absent file / unknown symbol -> None, never a fake node.
        assert!(ctx.view_root("topbar.json", "no_such_window").is_none());
        assert!(ctx.view_root("not_loaded.json", "x").is_none());
    }
}

#[cfg(test)]
mod domain_accessor_tests {
    //! FIX 1 — the TOP-LEVEL / fold-in domain accessors on [`BindContext`]. Each accessor
    //! must return **Absent** on a snapshot lacking the domain (never captured) and
    //! **Measured** when the domain is present, preserving the no-fabrication trichotomy —
    //! WITHOUT exposing the banned raw `snapshot()` seam.
    use super::*;

    fn empty_stage() -> Stage {
        Stage {
            resolver: GameResolver::new(None, None),
            manifest: SpriteManifest {
                sprites: std::collections::HashMap::new(),
            },
            loc: Loc::new(),
            gui_files: std::collections::HashMap::new(),
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn snap_from(v: serde_json::Value) -> Snapshot {
        serde_json::from_value(v).expect("snapshot from json")
    }

    /// Every named fold-in domain accessor is **Absent** when the snapshot carries only
    /// the stable header (no top-level domain keys) — never a fabricated Measured/Null.
    #[test]
    fn fold_in_domains_absent_when_snapshot_lacks_them() {
        let stage = empty_stage();
        let snap = snap_from(serde_json::json!({ "date": "1936.1.1.12" }));
        let ctx = BindContext {
            stage: &stage,
            snapshot: &snap,
            country: "GER",
            screen: gui_runtime::Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1936.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        };
        assert_eq!(ctx.divisions(), Captured::Absent);
        assert_eq!(ctx.combats(), Captured::Absent);
        assert_eq!(ctx.order_of_battle(), Captured::Absent);
        assert_eq!(ctx.intel(), Captured::Absent);
        assert_eq!(ctx.world(), Captured::Absent);
        assert_eq!(ctx.world_events(), Captured::Absent);
        assert_eq!(ctx.strategic_air(), Captured::Absent);
        assert_eq!(ctx.raids(), Captured::Absent);
        // the generic backing accessor agrees + is Absent for any unknown domain too.
        assert_eq!(ctx.domain_field("divisions"), Captured::Absent);
        assert_eq!(ctx.domain_field("no_such_domain"), Captured::Absent);
    }

    /// Every named fold-in domain accessor is **Measured** (and carries the real `&Value`)
    /// when the domain IS present in the snapshot — mirroring the demo top-level set
    /// (`combats`/`divisions` present) plus the rest, so a builder binds real data.
    #[test]
    fn fold_in_domains_measured_when_present() {
        let stage = empty_stage();
        let snap = snap_from(serde_json::json!({
            "date": "1936.1.1.12",
            "divisions": [ { "div_id": 1, "owner": "GER" } ],
            "combats": [ { "id": 7 } ],
            "order_of_battle": { "armies": [] },
            "intel": { "agency": {} },
            "world": { "variables": { "allies_score": 3 } },
            "world_events": { "threat": [] },
            "strategic_air": { "air_bases": [] },
            "raids": { "targets": [] },
        }));
        let ctx = BindContext {
            stage: &stage,
            snapshot: &snap,
            country: "GER",
            screen: gui_runtime::Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1936.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        };
        assert!(ctx.divisions().is_measured());
        assert!(ctx.combats().is_measured());
        assert!(ctx.order_of_battle().is_measured());
        assert!(ctx.intel().is_measured());
        assert!(ctx.world().is_measured());
        assert!(ctx.world_events().is_measured());
        assert!(ctx.strategic_air().is_measured());
        assert!(ctx.raids().is_measured());

        // the returned value is the REAL borrowed payload (no fabrication): a builder can
        // walk it exactly as it walks a country field.
        match ctx.world() {
            Captured::Measured(v) => {
                assert_eq!(
                    v.pointer("/variables/allies_score"),
                    Some(&serde_json::json!(3))
                );
            }
            other => panic!("world must be Measured, got {other:?}"),
        }
    }

    /// The trichotomy's middle rung: a present-but-JSON-null domain is **Null**, not
    /// Absent and not a fabricated Measured — the captured-but-unobtained case.
    #[test]
    fn fold_in_domain_present_but_null_is_null() {
        let stage = empty_stage();
        let snap = snap_from(serde_json::json!({ "date": "1936.1.1.12", "raids": null }));
        let ctx = BindContext {
            stage: &stage,
            snapshot: &snap,
            country: "GER",
            screen: gui_runtime::Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1936.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        };
        assert_eq!(ctx.raids(), Captured::Null);
        assert_eq!(ctx.domain_field("raids"), Captured::Null);
    }
}
