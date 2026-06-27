//! Domain-scoped, READ-ONLY accessors over a cached [`Snapshot`].
//!
//! Each open surface re-binds only from the domain(s) it actually needs, so a
//! per-tick re-bind never walks the unused domains (notably the ~3 MB `divisions`
//! array). The in-process domain split is M6's job; the on-disk per-domain split is
//! a separate Python store-format change explicitly OUT of this milestone's lane.
//!
//! Every accessor returns a `&` reference into the shared snapshot — there is no
//! `&mut` anywhere in this module, so a surface CANNOT mutate the cached payload
//! through a view. That is half of the no-fabrication enforcement (the other half
//! is that the bind layer writes into a throwaway scene-graph, not the snapshot).

use serde_json::{Map, Value};
use snapshot::Snapshot;

/// The root domain-scoped view: hand out per-surface sub-views, each scoped to the
/// single domain (or single entity) that surface reads. Borrows the snapshot
/// immutably for the view's lifetime.
#[derive(Clone, Copy)]
pub struct DomainView<'a> {
    snap: &'a Snapshot,
}

impl<'a> DomainView<'a> {
    pub fn new(snap: &'a Snapshot) -> DomainView<'a> {
        DomainView { snap }
    }

    /// The hour this view's snapshot was captured at (for surface titles / asserts).
    pub fn hour_index(&self) -> Option<i64> {
        self.snap.hour_index
    }
    /// The top-level snapshot date (the topbar clock reads this).
    pub fn date(&self) -> Option<&'a str> {
        self.snap.date.as_deref()
    }

    /// The MAP surface's view: `states` (owner/controller per state) + the `map`
    /// domain. It deliberately exposes NO access to `countries` or `divisions`.
    pub fn map(&self) -> MapView<'a> {
        MapView { snap: self.snap }
    }

    /// The TOPBAR surface's view for one observed country (`countries.<TAG>` only).
    pub fn country(&self, tag: &str) -> CountryView<'a> {
        CountryView {
            obj: self.snap.country(tag),
            tag: tag.to_string(),
        }
    }

    /// The STATE-PANEL surface's view for one selected state (`states.<sid>` only).
    pub fn state(&self, sid: &str) -> StateView<'a> {
        StateView {
            obj: self.snap.state(sid),
            sid: sid.to_string(),
        }
    }

    /// Escape hatch for code (like `bind::apply_topbar` / `bind::bind_state`) that
    /// is already written against `&Snapshot`. Still `&` — read-only, no mutation.
    /// The whole snapshot is borrowed but the bind functions only read the one
    /// domain they need; the typed sub-views above are how NEW code should scope.
    pub fn snapshot(&self) -> &'a Snapshot {
        self.snap
    }
}

/// Map-surface view: the `states` map (for the owner/controller LUT) and the `map`
/// domain. Reads nothing else — a map recolour never touches `countries`/`divisions`.
#[derive(Clone, Copy)]
pub struct MapView<'a> {
    snap: &'a Snapshot,
}

impl<'a> MapView<'a> {
    /// The `states` object, used to build the political-colour LUT (owner per
    /// state). `None` if the `states` domain is ABSENT **or** a captured JSON null
    /// (`Snapshot::states` `as_object`s it). For the map LUT both mean "no states to
    /// colour" — the same outcome — so the collapse is intentional and safe here. A
    /// caller that must distinguish a captured `states: null` from an absent domain
    /// reads [`DomainView::snapshot`]`().domain("states")` directly (Some(Null) vs
    /// None), which preserves the trichotomy.
    pub fn states(&self) -> Option<&'a Map<String, Value>> {
        self.snap.states()
    }
    /// The raw `states` domain WITHOUT the `as_object` collapse: `None` = absent,
    /// `Some(Null)` = captured-null, `Some(obj)` = present. Use this when the
    /// Absent/Null distinction matters; [`states`](Self::states) is the convenience
    /// form for the (collapse-safe) LUT build.
    pub fn states_raw(&self) -> Option<&'a Value> {
        self.snap.domain("states")
    }
    /// The raw `map` domain (projection / metadata): `None` = absent,
    /// `Some(Null)` = captured-null, `Some(v)` = present.
    pub fn map_domain(&self) -> Option<&'a Value> {
        self.snap.domain("map")
    }
    /// Owner TAG of one state for the political-colour lookup. Returns `None` for
    /// absent / null / non-string — all of which correctly mean "no owner colour".
    /// This is a deliberate trichotomy COLLAPSE: a coloured map has the same output
    /// (no political colour) whether the owner is absent or a captured null, so the
    /// no-fabrication guarantee is preserved at the RENDER boundary (no fabricated
    /// tag is ever invented). To distinguish absent vs null, use
    /// [`owner_cell`](Self::owner_cell).
    pub fn owner_of(&self, sid: &str) -> Option<&'a str> {
        self.snap.state_path(sid, "owner").and_then(Value::as_str)
    }
    /// Controller tag of one state (occupation colouring). Same collapse contract
    /// as [`owner_of`](Self::owner_of); use [`controller_cell`](Self::controller_cell)
    /// for the Absent/Null distinction.
    pub fn controller_of(&self, sid: &str) -> Option<&'a str> {
        self.snap
            .state_path(sid, "controller")
            .and_then(Value::as_str)
    }
    /// The raw `owner` cell, preserving the no-fabrication trichotomy: `None` =
    /// absent (state/owner never captured), `Some(Null)` = captured-null,
    /// `Some(String)` = a measured tag.
    pub fn owner_cell(&self, sid: &str) -> Option<&'a Value> {
        self.snap.state_path(sid, "owner")
    }
    /// The raw `controller` cell (trichotomy-preserving; see [`owner_cell`](Self::owner_cell)).
    pub fn controller_cell(&self, sid: &str) -> Option<&'a Value> {
        self.snap.state_path(sid, "controller")
    }
}

/// Country-surface view: one `countries.<TAG>` object. The topbar binds entirely
/// from here; it never needs the rest of the world.
#[derive(Clone)]
pub struct CountryView<'a> {
    obj: Option<&'a Value>,
    tag: String,
}

impl<'a> CountryView<'a> {
    pub fn tag(&self) -> &str {
        &self.tag
    }
    /// True if this country exists in the snapshot (never fabricated when absent).
    pub fn exists(&self) -> bool {
        self.obj.is_some()
    }
    /// Walk a dotted path under this country object. Preserves the no-fabrication
    /// trichotomy: `None` = absent, `Some(Null)` = captured-null, `Some(v)` = value.
    pub fn path(&self, dotted: &str) -> Option<&'a Value> {
        path_walk(self.obj?, dotted)
    }
    /// Convenience: a dotted path under `politics`.
    pub fn politics_path(&self, dotted: &str) -> Option<&'a Value> {
        self.path(&format!("politics.{dotted}"))
    }
}

/// State-surface view: one `states.<sid>` object. The state panel binds from here;
/// it never walks the full `states` map or any other domain.
#[derive(Clone)]
pub struct StateView<'a> {
    obj: Option<&'a Value>,
    sid: String,
}

impl<'a> StateView<'a> {
    pub fn sid(&self) -> &str {
        &self.sid
    }
    pub fn exists(&self) -> bool {
        self.obj.is_some()
    }
    /// Trichotomy-preserving dotted lookup: `None` = absent, `Some(Null)` =
    /// captured-null, `Some(v)` = measured. The state panel's scalar binding goes
    /// through this (via `bind::bind_state` on the snapshot) so the "—" vs measured
    /// decision keeps Absent and Null distinct.
    pub fn path(&self, dotted: &str) -> Option<&'a Value> {
        path_walk(self.obj?, dotted)
    }
    /// The owner TAG as a string for the flag lookup; `None` for absent/null/
    /// non-string (all "no owner flag"). For the Absent vs Null distinction use
    /// [`path`](Self::path)`("owner")` or [`owner_cell`](Self::owner_cell).
    pub fn owner(&self) -> Option<&'a str> {
        self.path("owner").and_then(Value::as_str)
    }
    pub fn controller(&self) -> Option<&'a str> {
        self.path("controller").and_then(Value::as_str)
    }
    /// The raw `owner` cell (trichotomy-preserving).
    pub fn owner_cell(&self) -> Option<&'a Value> {
        self.path("owner")
    }
    /// The raw `controller` cell (trichotomy-preserving).
    pub fn controller_cell(&self) -> Option<&'a Value> {
        self.path("controller")
    }
}

/// Walk a dotted path inside a JSON object (the same semantics as the snapshot
/// crate's internal `path_walk`, re-implemented here so the view is self-contained).
fn path_walk<'a>(mut cur: &'a Value, dotted: &str) -> Option<&'a Value> {
    for seg in dotted.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap() -> Snapshot {
        let j = json!({
            "date":"1941.4.13.11","hour_index":17005619,
            "countries":{"GER":{"politics":{"political_power":247.0,"stability":0.75}}},
            "states":{"64":{"owner":"GER","controller":"GER"},
                      "1":{"owner":"ITA","controller":"SOV"}},
            "map":{"projection":"flat"},
            "divisions":[{"id":1}]
        });
        serde_json::from_value(j).unwrap()
    }

    #[test]
    fn map_view_scoped_to_states_and_map() {
        let s = snap();
        let v = DomainView::new(&s);
        let m = v.map();
        assert!(m.states().is_some());
        assert!(m.map_domain().is_some());
        assert_eq!(m.owner_of("64"), Some("GER"));
        assert_eq!(m.controller_of("1"), Some("SOV"));
        assert_eq!(m.owner_of("999"), None); // absent, not fabricated
    }

    #[test]
    fn country_view_scoped_and_no_fabrication() {
        let s = snap();
        let v = DomainView::new(&s);
        let c = v.country("GER");
        assert!(c.exists());
        assert_eq!(c.politics_path("political_power").and_then(|x| x.as_f64()), Some(247.0));
        // an absent country yields None, never a fabricated object.
        let none = v.country("ZZZ");
        assert!(!none.exists());
        assert!(none.path("politics.political_power").is_none());
    }

    #[test]
    fn state_view_scoped() {
        let s = snap();
        let v = DomainView::new(&s);
        assert_eq!(v.state("1").owner(), Some("ITA"));
        assert_eq!(v.state("1").controller(), Some("SOV"));
        assert!(!v.state("404").exists());
    }
}
