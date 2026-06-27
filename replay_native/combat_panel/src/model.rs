//! The COMBAT roster-aggregation view-model (plan §C.2 + §G).
//!
//! `combats[]` entry shape (empirical, from the inspected snapshot):
//! ```json
//! { "combat_id":"5457:62", "kind":"land", "location_province":12740,
//!   "attackers":[[7989,4713]], "defenders":[[2528,4713], …],
//!   "combat_width":null, "terrain":"hills", "progress":14.55877,
//!   "modifiers":{"day":44,"duration":44} }
//! ```
//! The `[unit_leader_id, 4713]` pairs are unit-LEADER / command refs, **not**
//! division ids — there is NO division roster in `combats[]`.
//!
//! ## No-fabrication contract (Codex-refuted 2026-06-26)
//! Every field here is one of three honesty grades:
//! * **Exact**  — read verbatim from the snapshot (`combat_id`, `kind`,
//!   `location_province` id, side ref COUNTS, which refs are attackers/defenders).
//! * **Derived** — computed from a related fact, labelled as such (`duration` from
//!   `modifiers.duration`; the DEFENDER *holding tag* from the contested province's
//!   controller; the province NAME from static meta).
//! * **Unavailable** — `null`, M\*-uncracked, or genuinely absent from this data →
//!   carried as [`Field::Unavailable`] and rendered "—", NEVER as `0`/invented.
//!
//! Codex sharpenings folded in:
//!   1. The province controller is the DEFENDER's *holding* tag only — it is NOT a
//!      full unit-side-country fact, and the ATTACKER tag stays **Unavailable**
//!      (we must not infer "the enemy of the controller").
//!   2. Side ref arrays are counted as "attacker/defender refs", NEVER "divisions".
//!   3. `progress` (a raw positive float) is **Unavailable** as the −1..1 GUI bar.

use serde_json::Value as Json;

/// A measured-or-absent scalar, mirroring the snapshot Absent/Null/Measured
/// trichotomy at the view-model layer. `Unavailable` carries a short reason for
/// the badge tooltip; it renders "—" everywhere, never a fabricated `0`.
#[derive(Debug, Clone, PartialEq)]
pub enum Field<T> {
    /// A real measured value (including a measured `0`).
    Value(T),
    /// Not available from this data. `reason` names the missing field / hard limit.
    Unavailable { reason: &'static str },
}

impl<T> Field<T> {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Field::Unavailable { .. })
    }
    pub fn value(&self) -> Option<&T> {
        match self {
            Field::Value(v) => Some(v),
            Field::Unavailable { .. } => None,
        }
    }
}

impl<T: std::fmt::Display> Field<T> {
    /// Render for display: the value, or the no-fabrication dash for unavailable.
    pub fn display(&self) -> String {
        match self {
            Field::Value(v) => v.to_string(),
            Field::Unavailable { .. } => "\u{2014}".to_string(), // —
        }
    }
}

/// The honesty grade attached to a side's resolved owning TAG.
#[derive(Debug, Clone, PartialEq)]
pub enum SideTag {
    /// Derived from the contested province's CONTROLLER. Honest label: "province
    /// controller (derived)". (Codex: do NOT call this the "defender's country" —
    /// the controller is who holds the contested province, which is normally but
    /// not provably the defending side. The provenance — province controller — is
    /// the exact claim we can stand behind.)
    HoldingTagDerived(String),
    /// Genuinely not resolvable from this data (e.g. the ATTACKER tag — no
    /// leader-ref→country map, and we must NOT infer it from the defender).
    Unavailable,
}

impl SideTag {
    pub fn tag(&self) -> Option<&str> {
        match self {
            SideTag::HoldingTagDerived(t) => Some(t.as_str()),
            SideTag::Unavailable => None,
        }
    }
    pub fn display(&self) -> String {
        match self {
            SideTag::HoldingTagDerived(t) => t.clone(),
            SideTag::Unavailable => "\u{2014}".to_string(),
        }
    }
}

/// Which side of a combat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Attacker,
    Defender,
}

/// One side's roster aggregation (plan §C.2). EXACT: the ref count + raw refs.
/// DERIVED/UNAVAILABLE: the owning TAG (see [`SideTag`]). Per-division stat rows
/// and committed/reserve counts are **not present** in `combats[]` → omitted by
/// construction (the panel badges them Unavailable as a block).
#[derive(Debug, Clone, PartialEq)]
pub struct SideRoster {
    pub side: Side,
    /// Resolved owning TAG — derived (defender) or unavailable (attacker).
    pub tag: SideTag,
    /// The raw `[leader_id, type_token]` ref pairs, verbatim. EXACT.
    pub refs: Vec<[i64; 2]>,
    /// Whether the side's ref ARRAY was present in the JSON. `false` = the key was
    /// absent/null/non-array → the count is UNAVAILABLE ("—"), NOT a measured `0`
    /// (Codex guardrail). `true` with empty `refs` = a measured zero refs.
    pub refs_present: bool,
    /// Committed/reserve DIVISION roster — never present in `combats[]`.
    pub division_roster: Field<()>,
}

impl SideRoster {
    /// Count of command/leader refs on this side, as a no-fabrication [`Field`]:
    /// `Value(n)` when the array was present (a measured count, incl. a real `0`),
    /// `Unavailable` when the array key was absent/null. NOTE: this counts
    /// leader/command refs (duplicates included — Codex: do NOT dedupe), NOT
    /// divisions.
    pub fn ref_count_field(&self) -> Field<usize> {
        if self.refs_present {
            Field::Value(self.refs.len())
        } else {
            Field::Unavailable { reason: "side ref array absent/null in combats[]" }
        }
    }

    /// Raw count of parsed refs (array length). Use [`ref_count_field`](Self::ref_count_field)
    /// for display so an absent array reads "—", not "0". This is for sizing/layout
    /// (a bubble radius), where 0 and absent are equivalent.
    pub fn ref_count(&self) -> usize {
        self.refs.len()
    }
}

/// One active combat, fully aggregated into the panel view-model.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatVM {
    /// `combat_id` (e.g. "5457:62"). EXACT — the lifecycle key.
    pub combat_id: String,
    /// `kind` ("land" / "naval" / "air"). EXACT.
    pub kind: String,
    /// `location_province` id. EXACT.
    pub location_province: i64,
    /// Province display name. DERIVED from static meta (None until resolved).
    pub location_name: Field<String>,
    /// Battle duration in days, from `modifiers.duration`. DERIVED (§G permits).
    pub duration_days: Field<i64>,

    pub attacker: SideRoster,
    pub defender: SideRoster,

    // ---- battle-DETAIL — §G HARD LIMIT, all Unavailable ----
    /// `combat_width` — `null` in the snapshot.
    pub combat_width: Field<i64>,
    /// `terrain` — present ("hills") but M\*-uncracked battle-detail (§G).
    pub terrain: Field<String>,
    /// progress as the −1..1 GUI bar — `progress` is a raw positive float, NOT the
    /// bar (Codex). Always Unavailable; never bound to a progress bar.
    pub progress_bar: Field<f64>,
    /// Tactics (current/countered/list) — not in `combats[]` (§G).
    pub tactics: Field<String>,
    /// Combat-modifier vector — only `{day,duration}` present, no real vector (§G).
    pub modifiers_vector: Field<String>,
    /// Air support to the battle — not in `combats[]` (§G).
    pub air_support: Field<String>,
    /// Combat leaders (name/skill/portrait) — needs the `commanders` domain (absent).
    pub leaders: Field<String>,
    /// Per-division stat rows (org/str/soft/hard/defense) — no division join (§G).
    pub division_stat_rows: Field<()>,
}

impl CombatVM {
    /// Build the view-model for one `combats[]` entry. `controller_of` resolves a
    /// province id → its CONTROLLER tag (used only for the DEFENDER holding tag,
    /// per Codex — the attacker tag stays Unavailable). `province_name` resolves
    /// the location province id → its display name (DERIVED), `None` when unknown.
    pub fn from_combat(
        entry: &Json,
        controller_of: &dyn Fn(i64) -> Option<String>,
        province_name: &dyn Fn(i64) -> Option<String>,
    ) -> Option<CombatVM> {
        let combat_id = entry.get("combat_id")?.as_str()?.to_string();
        let kind = entry
            .get("kind")
            .and_then(Json::as_str)
            .unwrap_or("unknown")
            .to_string();
        let location_province = entry.get("location_province")?.as_i64()?;

        let (attackers, atk_present) = parse_refs(entry.get("attackers"));
        let (defenders, def_present) = parse_refs(entry.get("defenders"));

        // DEFENDER holding tag = the contested province's controller (the defender
        // holds it). DERIVED. The ATTACKER tag is genuinely Unavailable — we must
        // NOT infer it as the controller's enemy (Codex honesty trap).
        let defender_tag = match controller_of(location_province) {
            Some(t) => SideTag::HoldingTagDerived(t),
            None => SideTag::Unavailable,
        };

        let location_name = match province_name(location_province) {
            Some(n) => Field::Value(n),
            None => Field::Unavailable {
                reason: "province name not in static meta",
            },
        };

        // duration — modifiers.duration (days). DERIVED (§G permits duration).
        let duration_days = entry
            .get("modifiers")
            .and_then(|m| m.get("duration"))
            .and_then(Json::as_i64)
            .map(Field::Value)
            .unwrap_or(Field::Unavailable {
                reason: "modifiers.duration absent",
            });

        // combat_width — null in the snapshot → Unavailable (not 0). The badge
        // REASON keeps the Absent vs Null distinction honest (Codex): a missing key
        // is "absent", an explicit JSON null is "null" — both render "—".
        let combat_width = match entry.get("combat_width") {
            Some(Json::Number(n)) => n
                .as_i64()
                .map(Field::Value)
                .unwrap_or(Field::Unavailable { reason: "combat_width not integer" }),
            Some(Json::Null) => Field::Unavailable { reason: "combat_width null" },
            None => Field::Unavailable { reason: "combat_width absent" },
            Some(_) => Field::Unavailable { reason: "combat_width non-numeric" },
        };

        Some(CombatVM {
            combat_id,
            kind,
            location_province,
            location_name,
            duration_days,
            attacker: SideRoster {
                side: Side::Attacker,
                tag: SideTag::Unavailable, // Codex: never inferred from the defender
                refs: attackers,
                refs_present: atk_present,
                division_roster: Field::Unavailable {
                    reason: "no division roster in combats[] (leader refs only)",
                },
            },
            defender: SideRoster {
                side: Side::Defender,
                tag: defender_tag,
                refs: defenders,
                refs_present: def_present,
                division_roster: Field::Unavailable {
                    reason: "no division roster in combats[] (leader refs only)",
                },
            },
            combat_width,
            terrain: Field::Unavailable { reason: "terrain M*-uncracked battle-detail (§G)" },
            progress_bar: Field::Unavailable {
                reason: "progress is a raw positive float, NOT the -1..1 GUI bar",
            },
            tactics: Field::Unavailable { reason: "tactics not in combats[] (§G)" },
            modifiers_vector: Field::Unavailable {
                reason: "only {day,duration} present, no combat-modifier vector (§G)",
            },
            air_support: Field::Unavailable { reason: "air support not in combats[] (§G)" },
            leaders: Field::Unavailable { reason: "commanders domain absent" },
            division_stat_rows: Field::Unavailable {
                reason: "no division join in combats[]; per-division stats runtime-computed (§G)",
            },
        })
    }

    /// Window title "Battle of <province>" — DERIVED. Falls back to the province id
    /// when the name is unavailable (still honest: shows the exact id).
    pub fn window_title(&self) -> String {
        match self.location_name.value() {
            Some(name) => format!("Battle of {name}"),
            None => format!("Battle at province {}", self.location_province),
        }
    }
}

/// Parse a `[[leader_id, token], …]` JSON array into `[i64;2]` ref pairs, plus a
/// `present` flag: `false` when the key was absent/null/non-array (the count is
/// then UNAVAILABLE, not a measured `0`). A malformed inner element is skipped
/// (never fabricated into a zero pair).
fn parse_refs(v: Option<&Json>) -> (Vec<[i64; 2]>, bool) {
    let arr = match v.and_then(Json::as_array) {
        Some(a) => a,
        None => return (Vec::new(), false),
    };
    let mut out = Vec::with_capacity(arr.len());
    for pair in arr {
        if let Some(p) = pair.as_array() {
            if p.len() == 2 {
                if let (Some(a), Some(b)) = (p[0].as_i64(), p[1].as_i64()) {
                    out.push([a, b]);
                }
            }
        }
    }
    (out, true)
}

/// Extract every `combats[]` entry from a snapshot's top-level `combats` array and
/// aggregate into view-models, optionally filtered by `kind`. EXACT enumeration:
/// one VM per active combat, in array order.
pub fn aggregate_combats(
    combats: &Json,
    kind_filter: Option<&str>,
    controller_of: &dyn Fn(i64) -> Option<String>,
    province_name: &dyn Fn(i64) -> Option<String>,
) -> Vec<CombatVM> {
    let arr = match combats.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter(|e| match kind_filter {
            Some(k) => e.get("kind").and_then(Json::as_str) == Some(k),
            None => true,
        })
        .filter_map(|e| CombatVM::from_combat(e, controller_of, province_name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Json {
        json!({
            "combat_id": "5457:62",
            "kind": "land",
            "location_province": 12740,
            "attackers": [[7989, 4713]],
            "defenders": [[2528,4713],[4030,4713],[2730,4713],[8274,4713],
                          [11503,4713],[3569,4713],[3955,4713]],
            "combat_width": null,
            "terrain": "hills",
            "progress": 14.55877,
            "modifiers": { "day": 44, "duration": 44 }
        })
    }

    fn ctrl(p: i64) -> Option<String> {
        if p == 12740 { Some("ENG".to_string()) } else { None }
    }
    fn pname(p: i64) -> Option<String> {
        if p == 12740 { Some("Tobruk".to_string()) } else { None }
    }

    #[test]
    fn aggregates_side_identity_exact() {
        let vm = CombatVM::from_combat(&sample(), &ctrl, &pname).unwrap();
        assert_eq!(vm.combat_id, "5457:62");
        assert_eq!(vm.kind, "land");
        assert_eq!(vm.location_province, 12740);
        // ref COUNTS are exact array lengths — NOT "divisions".
        assert_eq!(vm.attacker.ref_count(), 1);
        assert_eq!(vm.defender.ref_count(), 7);
        // the raw refs are verbatim.
        assert_eq!(vm.attacker.refs[0], [7989, 4713]);
        assert_eq!(vm.defender.refs[6], [3955, 4713]);
    }

    #[test]
    fn defender_tag_derived_attacker_unavailable() {
        let vm = CombatVM::from_combat(&sample(), &ctrl, &pname).unwrap();
        // DEFENDER holds the contested province → derived holding tag = ENG.
        assert_eq!(vm.defender.tag, SideTag::HoldingTagDerived("ENG".into()));
        assert_eq!(vm.defender.tag.tag(), Some("ENG"));
        // ATTACKER tag must stay Unavailable — never inferred from the defender.
        assert_eq!(vm.attacker.tag, SideTag::Unavailable);
        assert_eq!(vm.attacker.tag.display(), "\u{2014}");
    }

    #[test]
    fn duration_derived_location_derived() {
        let vm = CombatVM::from_combat(&sample(), &ctrl, &pname).unwrap();
        assert_eq!(vm.duration_days, Field::Value(44));
        assert_eq!(vm.location_name, Field::Value("Tobruk".into()));
        assert_eq!(vm.window_title(), "Battle of Tobruk");
    }

    #[test]
    fn battle_detail_all_unavailable() {
        let vm = CombatVM::from_combat(&sample(), &ctrl, &pname).unwrap();
        assert!(vm.combat_width.is_unavailable(), "combat_width null -> unavailable");
        assert!(vm.terrain.is_unavailable(), "terrain M*-uncracked");
        assert!(vm.progress_bar.is_unavailable(), "progress is not the -1..1 bar");
        assert!(vm.tactics.is_unavailable());
        assert!(vm.modifiers_vector.is_unavailable());
        assert!(vm.air_support.is_unavailable());
        assert!(vm.leaders.is_unavailable());
        assert!(vm.division_stat_rows.is_unavailable());
        assert!(vm.attacker.division_roster.is_unavailable());
        assert!(vm.defender.division_roster.is_unavailable());
        // EVERY unavailable field renders the dash, never 0.
        assert_eq!(vm.combat_width.display(), "\u{2014}");
        assert_eq!(vm.terrain.display(), "\u{2014}");
    }

    #[test]
    fn unknown_controller_makes_defender_tag_unavailable_not_fabricated() {
        // a combat whose province has no known controller -> defender tag unavailable
        let mut e = sample();
        e["location_province"] = json!(99999);
        let vm = CombatVM::from_combat(&e, &ctrl, &pname).unwrap();
        assert_eq!(vm.defender.tag, SideTag::Unavailable);
        assert!(vm.location_name.is_unavailable());
        // title still honest — shows the exact province id.
        assert_eq!(vm.window_title(), "Battle at province 99999");
    }

    #[test]
    fn aggregate_filters_by_kind_and_enumerates_in_order() {
        let world = json!([
            sample(),
            { "combat_id":"9:9", "kind":"naval", "location_province":1,
              "attackers":[], "defenders":[], "combat_width":null,
              "modifiers":{"duration":3} },
            { "combat_id":"2:2", "kind":"land", "location_province":12740,
              "attackers":[[1,4713]], "defenders":[[2,4713]], "combat_width":null,
              "modifiers":{"duration":5} }
        ]);
        let land = aggregate_combats(&world, Some("land"), &ctrl, &pname);
        assert_eq!(land.len(), 2);
        assert_eq!(land[0].combat_id, "5457:62");
        assert_eq!(land[1].combat_id, "2:2");
        let all = aggregate_combats(&world, None, &ctrl, &pname);
        assert_eq!(all.len(), 3);
        // naval entry binds nothing extra but is enumerated exactly.
        assert_eq!(all[1].kind, "naval");
        assert_eq!(all[1].duration_days, Field::Value(3));
    }

    #[test]
    fn progress_raw_float_never_becomes_a_value_or_bar() {
        // the raw `progress` float (14.55877) must NEVER surface as a value, and the
        // -1..1 bar field is hardcoded Unavailable (Codex regression check).
        let vm = CombatVM::from_combat(&sample(), &ctrl, &pname).unwrap();
        assert!(vm.progress_bar.is_unavailable());
        assert_eq!(vm.progress_bar.display(), "\u{2014}");
        // there is NO field on the VM that carries the raw progress number; assert
        // the type has no such accessor by confirming the only progress-ish field is
        // the unavailable bar. (Structural: progress is intentionally not stored.)
    }

    #[test]
    fn absent_ref_array_is_unavailable_not_zero() {
        // a combat MISSING the `attackers` key -> attacker ref count is "—", not "0".
        let e = json!({
            "combat_id":"x","kind":"land","location_province":12740,
            "defenders":[[2,4713]], "combat_width":null, "modifiers":{"duration":1}
        });
        let vm = CombatVM::from_combat(&e, &ctrl, &pname).unwrap();
        assert!(!vm.attacker.refs_present, "absent attackers key");
        assert!(vm.attacker.ref_count_field().is_unavailable());
        assert_eq!(vm.attacker.ref_count_field().display(), "\u{2014}");
        // a PRESENT empty array IS a measured zero -> shows "0", not "—".
        let e2 = json!({
            "combat_id":"y","kind":"land","location_province":12740,
            "attackers":[], "defenders":[[2,4713]], "combat_width":null, "modifiers":{}
        });
        let vm2 = CombatVM::from_combat(&e2, &ctrl, &pname).unwrap();
        assert!(vm2.attacker.refs_present, "present empty attackers array");
        assert_eq!(vm2.attacker.ref_count_field(), Field::Value(0));
        assert_eq!(vm2.attacker.ref_count_field().display(), "0");
    }

    #[test]
    fn combat_width_absent_vs_null_reason_distinct() {
        // Both render "—", but the badge REASON must keep Absent vs Null distinct
        // (no-fabrication provenance honesty; Codex).
        let null_e = json!({
            "combat_id":"a","kind":"land","location_province":12740,
            "attackers":[],"defenders":[], "combat_width": null, "modifiers":{}
        });
        let absent_e = json!({
            "combat_id":"b","kind":"land","location_province":12740,
            "attackers":[],"defenders":[], "modifiers":{}
        });
        let vm_null = CombatVM::from_combat(&null_e, &ctrl, &pname).unwrap();
        let vm_absent = CombatVM::from_combat(&absent_e, &ctrl, &pname).unwrap();
        assert!(vm_null.combat_width.is_unavailable());
        assert!(vm_absent.combat_width.is_unavailable());
        // both dash on screen, but their reasons differ (absent != null).
        assert_eq!(vm_null.combat_width.display(), "\u{2014}");
        assert_eq!(vm_absent.combat_width.display(), "\u{2014}");
        let reason = |f: &Field<i64>| match f {
            Field::Unavailable { reason } => *reason,
            _ => "VALUE",
        };
        assert_eq!(reason(&vm_null.combat_width), "combat_width null");
        assert_eq!(reason(&vm_absent.combat_width), "combat_width absent");
    }

    #[test]
    fn ref_count_does_not_dedupe() {
        // Codex: duplicate leader ids are counted (refs listed, not unique leaders).
        let e = json!({
            "combat_id":"z","kind":"land","location_province":12740,
            "attackers":[[7,4713],[7,4713],[7,4713]], "defenders":[[2,4713]],
            "combat_width":null, "modifiers":{"duration":1}
        });
        let vm = CombatVM::from_combat(&e, &ctrl, &pname).unwrap();
        assert_eq!(vm.attacker.ref_count_field(), Field::Value(3));
    }

    #[test]
    fn malformed_refs_skipped_never_fabricated() {
        let e = json!({
            "combat_id":"x", "kind":"land", "location_province":12740,
            "attackers":[[7989,4713],[1],"bad",[2,3,4]], "defenders":[],
            "combat_width":null, "modifiers":{}
        });
        let vm = CombatVM::from_combat(&e, &ctrl, &pname).unwrap();
        // only the one well-formed pair survives; no [0,0] fabricated for the bad ones.
        assert_eq!(vm.attacker.ref_count(), 1);
        assert_eq!(vm.attacker.refs[0], [7989, 4713]);
        // duration absent -> unavailable (not 0).
        assert!(vm.duration_days.is_unavailable());
    }
}
