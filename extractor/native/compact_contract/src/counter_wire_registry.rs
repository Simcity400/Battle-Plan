//! Which diagnostic counter a domain computes, and the wire name it arrives
//! under.
//!
//! A counter that names a gap is worth its claim only if a reader can find its
//! value in the capture. [`REGISTRY`] is that mapping and
//! `compact_contract/tests/compact_contract_suite/counter_wire_census.rs`
//! refuses a producer field that is missing from it.

/// One counter, from where it is computed to who reads it.
pub struct CounterWireRow {
    /// Where the value is computed, relative to `extractor/native`.
    pub source_file: &'static str,
    pub source_struct: &'static str,
    /// Field name. Identical on the source struct and on the producer struct,
    /// so a rename cannot leave the two silently describing different values.
    pub field: &'static str,
    /// The field's type on `source_struct`. Pinned so the widening the
    /// collector's copy performs is a stated conversion rather than a guess,
    /// and so the arrival harness knows which fields are boolean state.
    pub source_type: &'static str,
    /// Every expression the lane's lift function assigns to `field`,
    /// whitespace insignificant, `..` standing for a struct literal that omits
    /// the field and takes it from a `..rest`. The census requires the set to
    /// match the source exactly, so a second return path cannot lift a
    /// neighbouring value while the first still reads right.
    pub lifts: &'static [&'static str],
    /// The [`wire_counters!`] struct that publishes it.
    pub producer_struct: &'static str,
    /// The DOMAIN_PERF diagnostic-unit name, `concat!(prefix, field)`.
    pub unit: &'static str,
    /// The role this counter is published under, which the census pins against
    /// the `=> Role` in the producer's own declaration and the arrival harness
    /// pins against the bytes. Stated HERE rather than read off the generated
    /// table, so a declaration that quietly stopped saying what a unit is for
    /// is a failure rather than a change nobody sees.
    pub role: crate::diagnostic_channel::UnitRole,
    pub consumer: &'static str,
}

/// One domain's counter producer.
pub struct CounterWireProducer {
    pub producer_struct: &'static str,
    /// The file holding the [`wire_counters!`] invocation.
    pub producer_file: &'static str,
    pub source_file: &'static str,
    /// The struct the collector reads: the lane's PUBLISHED diagnostics.
    pub source_struct: &'static str,
    /// The lane function that builds `source_struct` out of `state_struct`.
    pub lift_fn: &'static str,
    /// How many ways `lift_fn` can return: every `source_struct` literal in it,
    /// plus every whole-struct `default()` return. Pinned because the lifts
    /// below are a SET -- a second degraded path returning `default()` would
    /// otherwise add nothing to that set and publish zeros unremarked.
    pub lift_return_paths: usize,
    /// The struct the lane counts INTO. A counter kept here and never lifted
    /// into `source_struct` is the original d102 defect, so the census holds
    /// its integer fields to the registry as well.
    pub state_struct: &'static str,
    /// The [`wire_counters!`] struct `state_struct` holds its counters in. The
    /// stored counters are fields of THIS struct, so a field exists only
    /// because the declaration lists it and no spelling of "increment" can
    /// create an unregistered one.
    pub state_counters_struct: &'static str,
    /// Counter-typed declarations in `source_file` that are not counters, each
    /// with the reason. The declaration backstop refuses an entry that does not
    /// exist and one that is also registered, so the list cannot go stale and
    /// cannot excuse a registered counter twice.
    pub declarations_that_are_not_counters: &'static [(&'static str, &'static str)],
    /// Integer fields of `state_struct` that are not counters, each with the
    /// reason. The census refuses an entry that does not exist, that is also
    /// registered, or whose NAME is counter-shaped, so the list cannot go stale
    /// and cannot be used to excuse a counter.
    pub state_fields_off_the_channel: &'static [(&'static str, &'static str)],
    /// Locals the lane counts up that are not diagnostic counters, each with
    /// the reason. A local declares nothing the backstop can read, so this is
    /// where one is accounted; a local the registry's own lifts NAME needs no
    /// entry, because that names how it reaches the wire.
    pub locals_that_are_not_counters: &'static [(&'static str, &'static str)],
    /// Modules the lane's CRATE declares that this domain's own module closure
    /// does not reach, each with the reason. The declaration backstop reads the
    /// closure of `source_file`; this list is what stops a counter being moved
    /// one `mod` sideways, out of the closure and into the same crate. It is
    /// asserted equal to the crate's module tree in both directions.
    pub crate_modules_outside_the_domain: &'static [(&'static str, &'static str)],
    /// The collector type whose construction can put HAND-WRITTEN units in this
    /// domain's trailer, or `""` when the emission publishes only the counters.
    ///
    /// A registered domain's trailer is the units argument at the emission
    /// site, and for d102 that argument copies a measured slice before it
    /// appends the counters. Those units carry no registry row, so this is
    /// where they are accounted (Codex VB2 round 6, finding 1).
    pub measured_units_carrier: &'static str,
    /// Every hand-written unit `measured_units_carrier` can publish, with what
    /// it measures. Asserted equal to the units built in the functions that
    /// construct that type, in both directions.
    pub measured_units: &'static [(&'static str, &'static str)],
    /// Measured units whose NAME is not a literal, keyed by the expression the
    /// name is taken from and saying what that expression can publish. A name a
    /// source scan cannot resolve is accounted here or the census refuses it.
    pub measured_unit_name_sources: &'static [(&'static str, &'static str)],
    /// The domain constant spelled at the emission site.
    pub domain_const: &'static str,
    /// What `domain_const` evaluates to. The collector's arrival harness
    /// asserts the two agree, so this number cannot drift from the constant.
    pub domain_id: u16,
    /// The collector function that turns the producer into units.
    pub emission_fn: &'static str,
    /// The whole units argument at the emission site, whitespace removed. It is
    /// pinned in full because `&fn(&Default::default())` also names the
    /// function and would publish zeros.
    pub emission_argument: &'static str,
}

/// The manifest every DOMAIN_PERF diagnostic unit lands in.
///
/// `recorder_host`'s `registered_counter_wire_units_reach_the_manifest` pins
/// itself to this string, so the citation names the function that test drives.
pub const PERF_UNIT_CONSUMER: &str =
    "recorder_host/src/main.rs::decode_compact_domain_perf_diagnostic_units";

/// Declare a counter struct and the wire name of every field in ONE place.
///
/// The field exists only because this declaration lists it, and the same list
/// builds `wire_counters()`, so a counter cannot be computed without also being
/// published. The names are `concat!(prefix, field)`: a counter cannot take a
/// neighbour's name, and no second list can drift from this one.
///
/// A field may carry `=> Role`, which is the [`UnitRole`] the value is published
/// under. Omitted, it is `Undeclared`, which is what the untyped diagnostic
/// funnel wraps a bare `(name, value)` pair as.
///
/// [`UnitRole`]: crate::diagnostic_channel::UnitRole
#[macro_export]
macro_rules! wire_counters {
    (@role) => {
        $crate::diagnostic_channel::UnitRole::Undeclared
    };
    (@role $role:ident) => {
        $crate::diagnostic_channel::UnitRole::$role
    };
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident, prefix = $prefix:literal;
        $( $(#[$field_meta:meta])* $field:ident $(=> $role:ident)?, )+
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $( $(#[$field_meta])* $vis $field: u64, )+
        }

        #[allow(dead_code)]
        impl $name {
            /// Field names in declaration order.
            pub const COUNTER_FIELDS: [&'static str; [$(stringify!($field),)+].len()] =
                [$(stringify!($field),)+];

            /// Published unit names, in `COUNTER_FIELDS` order.
            pub const COUNTER_UNITS: [&'static str; [$(stringify!($field),)+].len()] =
                [$(concat!($prefix, stringify!($field)),)+];

            /// Published roles, in `COUNTER_FIELDS` order.
            pub const COUNTER_ROLES: [$crate::diagnostic_channel::UnitRole;
                [$(stringify!($field),)+].len()] =
                [$($crate::wire_counters!(@role $($role)?),)+];

            pub const COUNTER_UNIT_PREFIX: &'static str = $prefix;

            /// Every counter with the name it is published under.
            pub fn wire_counters(&self) -> [(&'static str, u64); [$(stringify!($field),)+].len()] {
                [$((concat!($prefix, stringify!($field)), self.$field),)+]
            }

            /// The same, each with the role its declaration gives it.
            pub fn declared_counters(
                &self,
            ) -> [(&'static str, u64, $crate::diagnostic_channel::UnitRole);
                    [$(stringify!($field),)+].len()] {
                [$((
                    concat!($prefix, stringify!($field)),
                    self.$field,
                    $crate::wire_counters!(@role $($role)?),
                ),)+]
            }

            /// Add every counter of `other` to this one, saturating.
            ///
            /// Field for field off the SAME declaration, so a counter added to
            /// the list is folded by this too and cannot be left behind at
            /// zero by a second list nobody updated.
            pub fn accumulate(&mut self, other: &Self) {
                $( self.$field = self.$field.saturating_add(other.$field); )+
            }

            /// Add one to the counter the selector names, saturating.
            ///
            /// The selector can only name a field this declaration made, which
            /// is what stops an increment from creating an unpublished counter.
            pub fn bump(&mut self, select: impl FnOnce(&mut Self) -> &mut u64) -> u64 {
                let slot = select(self);
                *slot = slot.saturating_add(1);
                *slot
            }
        }

        // No distinctness assert: a unit is `concat!(one prefix, one field
        // identifier)` and Rust already refuses a struct with two fields of one
        // name, so it could not fire (Codex round 3, finding 9).
        const _: () = {
            let mut index = 0;
            while index < $name::COUNTER_UNITS.len() {
                assert!(
                    $crate::diagnostic_channel::declares_explicit_unit_accumulation(
                        $name::COUNTER_UNITS[index],
                    ),
                    "a counter's published name has no accumulation class in \
                     compact_contract::diagnostic_channel, so a fold would guess it",
                );
                index += 1;
            }
        };
    };
}

const D79_FILE: &str = "lanes/hud_role_observer_live/src/a729_hook.rs";
const D87_FILE: &str = "lanes/decision_panel_live/src/a729_hook.rs";
const D72_FILE: &str = "lanes/resolved_tooltip_instances_live/src/a729_hook.rs";
const D78_FILE: &str = "lanes/design_catalog_context_live/src/a729_hook.rs";
const D102_FILE: &str = "lanes/fog_derivation_inputs_live/src/lib.rs";
const D103_FILE: &str = "lanes/naval_combat_results_live/src/lib.rs";
const D95_FILE: &str = "lanes/spotted_units_live/src/a729.rs";

/// Every registered producer.
pub const PRODUCERS: &[CounterWireProducer] = &[
    // d95 keeps its census for the process lifetime, in ONE declaration whose
    // two halves are the two surfaces. Its emission was a name array beside an
    // index list, and the emission site's own comment warned the two could
    // disagree; there is no second list now.
    CounterWireProducer {
        producer_struct: "D95Counters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        lift_fn: "diagnostics",
        lift_return_paths: 1,
        state_struct: "LatestFrames",
        state_counters_struct: "D95StateCounters",
        state_fields_off_the_channel: &[],
        declarations_that_are_not_counters: &[
            (
                "BuildingFrame.pov_country_id",
                "the engine country id the frame is being built for, compared for equality \
                 so a frame is never attributed to another POV",
            ),
            (
                "BuildingFrame.rows",
                "the frame's admitted rows, keyed by engine handle and valued by the raw \n                 intel byte the engine reported. A row SET, and the rows themselves are \n                 what the domain's data record carries; the counts taken over them are \n                 the registered census arms",
            ),
            (
                "CompleteFrame.pov_country_id",
                "the engine country id the finished frame belongs to",
            ),
            (
                "CompleteFrame.sequence",
                "the id next_sequence handed this frame; a copy of that counter's value, \
                 compared for equality by the served check",
            ),
            (
                "SnapshotHeader.pov_country_id",
                "the engine country id the snapshot is published under",
            ),
            (
                "SnapshotHeader.capture_sequence",
                "the id this DRAIN was stamped with, taken from the registered next_sequence \
                 allocator like a frame's; an identity, never added to",
            ),
            (
                "SnapshotHeader.land_frame_sequence",
                "the land frame's id, carried so a drain can mark exactly that frame served",
            ),
            (
                "SnapshotHeader.taskforce_frame_sequence",
                "the task-force frame's id, carried for the same reason",
            ),
        ],
        locals_that_are_not_counters: &[],
        crate_modules_outside_the_domain: &[],
        measured_units_carrier: "",
        measured_units: &[],
        measured_unit_name_sources: &[],
        domain_const: "COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE",
        domain_id: 95,
        emission_fn: "d95_declared_diagnostic_units",
        emission_argument:
            "&d95_declared_diagnostic_units(&spotted_units_live_lane::diagnostics())",
    },
    // d102 is the domain whose orphaned counter started this: it counted
    // `units_with_undispatched_class` in the walk and published nothing, so an
    // "excluded and accounted" claim was true of the code and false of the
    // capture. Its drops are declared by the macro now, so a walk site can only
    // add to a field that already has a wire name.
    CounterWireProducer {
        producer_struct: "D102FogInputCounters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D102_FILE,
        source_struct: "FogInputWalk",
        lift_fn: "read_census",
        lift_return_paths: 1,
        state_struct: "Sink",
        state_counters_struct: "D102WalkCounters",
        state_fields_off_the_channel: &[],
        declarations_that_are_not_counters: &[
            (
                "FogInputGuards.country_vtable",
                "the engine vtable address a walked country object must match; compared for                  equality, never added to",
            ),
            (
                "FogInputGuards.province_vtable",
                "the engine vtable address a walked province object must match",
            ),
            (
                "FogInputGuards.division_vtable",
                "the engine vtable address a walked division object must match",
            ),
            (
                "FogInputGuards.fleet_vtable",
                "the engine vtable address a walked fleet object must match",
            ),
            (
                "FogInputGuards.taskforce_vtable",
                "the engine vtable address a walked task force object must match",
            ),
            (
                "FogInputGuards.theatre_vtable",
                "the engine vtable address a walked theatre object must match",
            ),
            (
                "FogInputGuards.controller_area_vtable",
                "the engine vtable address a walked controller area object must match",
            ),
            (
                "FogInputGuards.strategic_air_manager_vtable",
                "the engine vtable address the strategic air manager must match",
            ),
            (
                "FogInputGuards.air_base_vtable",
                "the engine vtable address a walked air base must match",
            ),
            (
                "FogInputGuards.air_container_vtable",
                "the engine vtable address a walked per-country air container must match",
            ),
            (
                "FogInputGuards.air_pool_vtable",
                "the engine vtable address a walked air-wing pool must match",
            ),
            (
                "FogInputGuards.air_wing_object_vtable",
                "the engine vtable address a walked air-wing object must match",
            ),
            (
                "FogInputGuards.air_wing_vtable",
                "the engine vtable address a walked air-wing body must match",
            ),
            (
                "EngineVector.data_address",
                "the engine address the vector's data pointer was read from, held for the                  post-bracket recycle check",
            ),
            (
                "EngineVector.count_address",
                "the engine address the vector's length was read from",
            ),
            (
                "EngineVector.data",
                "the engine's own data pointer for the vector, the base every element read                  is offset from",
            ),
            (
                "EngineVector.count",
                "the engine's own length for the vector, copied per read and bounded against                  the walk's maximum; nothing adds to it",
            ),
            (
                "Sink.volunteer_host_ids",
                "every volunteer-host id the walk copied and the row index that carries it,                  held for the resolution that runs after the country enumeration",
            ),
            (
                "Sink.country_tag_round_trips",
                "the (ordinal, tag database id) pairs the walk authenticated, checked for the                  ordinal table's round trip after the enumeration",
            ),
            (
                "Sink.stationed_unit_owners",
                "the distinct CUnit+0x1E0 owner ids the walk filed stationed rows under,                  resolved against the ordinal set after the enumeration",
            ),
            (
                "Sink.stationed_key_countries",
                "the distinct container+0x110 keys the walk filed rows under, resolved                  against the ordinal set after the enumeration",
            ),
            (
                "Sink.air_container_round_trips",
                "the (slot, container+0xB0 id) pairs the walk published air-base pool rows                  under, checked for the ordinal table's round trip after the enumeration",
            ),
        ],
        locals_that_are_not_counters: &[],
        crate_modules_outside_the_domain: &[],
        measured_units_carrier: "CompactFogDerivationInputsRead",
        measured_units: &[
            (
                "fog_derivation_inputs_worker_panic",
                "the census worker panicked; the hour carries no walk at all",
            ),
            (
                "fog_derivation_inputs_bracket_refused",
                "the bracket could not vouch for the hour it observed, or held steady on a different hour, so no census is attributable to this one. The WALK may still have run and its counters are carried",
            ),
            (
                "fog_derivation_inputs_walk_incoherent",
                "the census the walk read does not hold together: two reads of a bulk array disagreed, or a key, a unit identity or an address read as two things",
            ),
            (
                "fog_derivation_inputs_full_census",
                "the census this hour computed was sent WHOLE rather than as a delta. Retained when the payload then refuses to encode, where the refusal unit says the record carries neither",
            ),
            (
                "fog_derivation_inputs_delta",
                "the census this hour computed was sent as a DELTA against the tracker's baseline. Retained when the payload then refuses to encode, where the refusal unit says the record carries neither",
            ),
            (
                "fog_derivation_inputs_keyframe_by_cadence",
                "the keyframe cadence was due AND the whole census was sent. Not exclusive: no \n                 baseline, or a delta no smaller than the census, sends the whole census \n                 too, and this unit is 1 whenever both hold",
            ),
            (
                "fog_derivation_inputs_compute_us",
                "microseconds the worker spent on the sort, delta and digest",
            ),
            (
                "fog_derivation_inputs_units_walked",
                "units the walk observed; a measurement of the hour, published in full",
            ),
            (
                "fog_derivation_inputs_radar_rows",
                "rows the CENSUS this hour computed holds in the radar-coverage table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_stationed_province_rows",
                "rows the CENSUS this hour computed holds in the stationed-province table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_stationed_unit_rows",
                "rows the CENSUS this hour computed holds in the stationed-unit table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_viewer_level_rows",
                "rows the CENSUS this hour computed holds in the unit viewer-level table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_unit_class_rows",
                "rows the CENSUS this hour computed holds in the unit-class table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_country_ordinal_rows",
                "rows the CENSUS this hour computed holds in the country-ordinal table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_volunteer_host_rows",
                "rows the CENSUS this hour computed holds in the volunteer-host table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_air_base_pool_wing_rows",
                "rows the CENSUS this hour computed holds in the air-base pool membership table. It is retained when the payload then refuses to encode, and fog_derivation_inputs_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "fog_derivation_inputs_payload_encode_refused",
                "the payload would not encode. THIS record carries the refusal, and the delta tracker is reset so the NEXT record that encodes carries a full census rather than a delta against a baseline nothing received",
            ),
            (
                "fog_derivation_inputs_encode_us",
                "microseconds the MAIN THREAD spent encoding the payload, kept apart from compute_us",
            ),
        ],
        measured_unit_name_sources: &[(
            "name",
            "the refusal's own name: FogInputRefusal::name() when the walk refused, or              fog_derivation_inputs_frame_status_failure when the frame had already failed.              A marker for WHY the hour carries no census, valued 1; the lane's refusal              names are an enum, and none of them is a count",
        )],
        domain_const: "COMPACT_DOMAIN_FOG_DERIVATION_INPUTS_LIVE",
        domain_id: 102,
        emission_fn: "d102_fog_derivation_inputs_units",
        emission_argument:
            "&d102_fog_derivation_inputs_units(read.walk_counters, &read.diagnostic_units)",
    },
    // d103 is d102's shape without the delta: one full census per hour. Its
    // record walk fills one sink in its own bracket; each active combat's rows
    // are read in domain 49's visit of the object into a sink of their own,
    // and `assemble_census` lifts the record sink's tally and the visits'
    // count and drops into the walk the counters are read from.
    CounterWireProducer {
        producer_struct: "D103NavalResultCounters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D103_FILE,
        source_struct: "NavalResultWalk",
        lift_fn: "assemble_census",
        lift_return_paths: 1,
        state_struct: "Sink",
        state_counters_struct: "D103WalkCounters",
        state_fields_off_the_channel: &[],
        declarations_that_are_not_counters: &[
            (
                "NavalResultGuards.module_base",
                "the attached hoi4.exe image base every registry RVA is resolved against",
            ),
            (
                "NavalResultGuards.record_vtable",
                "the engine vtable address a walked naval-result record must match",
            ),
            (
                "NavalResultGuards.naval_combat_vtable",
                "the engine vtable address a walked naval combat must match",
            ),
            (
                "NavalResultGuards.naval_combat_side_vtable",
                "the engine vtable address a walked naval combat side must match",
            ),
            (
                "NavalResultGuards.transfer_vtable",
                "the engine vtable address a resolved unit transfer must match",
            ),
            (
                "NavalResultGuards.division_vtable",
                "one of the three engine vtable addresses a resolved unit may carry",
            ),
            (
                "NavalResultGuards.fleet_vtable",
                "one of the three engine vtable addresses a resolved unit may carry",
            ),
            (
                "NavalResultGuards.taskforce_vtable",
                "one of the three engine vtable addresses a resolved unit may carry",
            ),
            (
                "NavalResultGuards.province_vtable",
                "the engine vtable address a walked province must match",
            ),
            (
                "EngineVector.data_address",
                "the engine address the vector's data pointer was read from, held for the                  post-walk coherence check",
            ),
            (
                "EngineVector.count_address",
                "the engine address the vector's length was read from",
            ),
            (
                "EngineVector.data",
                "the engine's own data pointer for the vector, the base every element read                  is offset from",
            ),
            (
                "EngineVector.count",
                "the engine's own length for the vector, copied per read and bounded against                  the walk's maximum; nothing adds to it",
            ),
            (
                "VisitBudget.unit_visits_left",
                "the transfer-unit visits one hour may still spend, set from                  MAX_UNIT_VISITS_PER_HOUR and only ever decremented; exhaustion refuses the                  visit by name (naval_results_visit_budget_exhausted), so nothing is dropped                  silently",
            ),
            (
                "CombatVisit.generation",
                "the domain-49 epoch generation the collector minted for the visit, copied                  into the active-combat row's k3/k4 as its join key; nothing adds to it",
            ),
            (
                "Sink.country_ids",
                "every side-list and admitted-list id the walk copied and the row index that                  carries it, held for the canonical-entry read that runs after the walk",
            ),
        ],
        locals_that_are_not_counters: &[
            (
                "lanes/naval_combat_results_live/src/lib.rs::present",
            ),
            (
                "lanes/naval_combat_results_live/src/lib.rs::probe",
                "the robin-hood probe distance the resolver compares against a slot's own                  distance; it walks the registry the engine's way and counts nothing dropped",
            ),
            (
                "lanes/naval_combat_results_live/src/lib.rs::probed",
                "how many slots the resolver has visited, bounded by the table's bucket count                  so an unterminated chain refuses rather than spins; not a diagnostic",
            ),
        ],
        crate_modules_outside_the_domain: &[],
        measured_units_carrier: "CompactNavalCombatResultsRead",
        measured_units: &[
            (
                "naval_combat_results_worker_panic",
                "the census worker panicked; the hour carries no walk at all",
            ),
            (
                "naval_combat_results_bracket_refused",
                "the bracket could not vouch for the hour it observed, or held steady on a different hour, so no census is attributable to this one. The WALK may still have run and its counters are carried",
            ),
            (
                "naval_combat_results_walk_incoherent",
                "the census the walk read does not hold together: one key was read with two values",
            ),
            (
                "naval_combat_results_occurrence_visits_refused",
                "active combats whose d49 visit refused by name this hour: their rows are not in the census, every other combat's are, and the refusal's own name is the unit beside this one that says why",
            ),
            (
                "naval_combat_results_occurrence_visit_unavailable",
                "domain 49 did not complete a walk this frame, or did not run, so no active combat object was visited and its d103 rows were never read; the hour carries no census",
            ),
            (
                "naval_combat_results_full_census",
                "the census this hour computed was sent WHOLE; this domain never sends a delta. Retained when the payload then refuses to encode, where the refusal unit says the record carries none",
            ),
            (
                "naval_combat_results_compute_us",
                "microseconds the MAIN THREAD spent assembling domain 49's visits onto the record walk, the sort and the digest, kept apart from encode_us",
            ),
            (
                "naval_combat_results_record_rows",
                "rows the CENSUS this hour computed holds in the record table. It is retained when the payload then refuses to encode, and naval_combat_results_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "naval_combat_results_side_country_rows",
                "rows the CENSUS this hour computed holds in the side country-list table. It is retained when the payload then refuses to encode, and naval_combat_results_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "naval_combat_results_side_member_rows",
                "rows the CENSUS this hour computed holds in the side member-count table. It is retained when the payload then refuses to encode, and naval_combat_results_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "naval_combat_results_admitted_rows",
                "rows the CENSUS this hour computed holds in the admitted-participant table. It is retained when the payload then refuses to encode, and naval_combat_results_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "naval_combat_results_active_combat_rows",
                "rows the CENSUS this hour computed holds in the active naval-combat table. It is retained when the payload then refuses to encode, and naval_combat_results_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "naval_combat_results_spill_over_rows",
                "rows the CENSUS this hour computed holds in the spill-over unit-location table. It is retained when the payload then refuses to encode, and naval_combat_results_payload_encode_refused is the unit that says the record carries none of them",
            ),
            (
                "naval_combat_results_payload_encode_refused",
                "the payload would not encode. THIS record carries the refusal and no census",
            ),
            (
                "naval_combat_results_encode_us",
                "microseconds the MAIN THREAD spent encoding the payload, kept apart from compute_us",
            ),
        ],
        measured_unit_name_sources: &[(
            "name",
            "the refusal's own name: NavalResultRefusal::name() when the walk refused, or              naval_combat_results_frame_status_failure when the frame had already failed.              A marker for WHY the hour carries no census, valued 1; the lane's refusal              names are an enum, and none of them is a count",
        )],
        domain_const: "COMPACT_DOMAIN_NAVAL_COMBAT_RESULTS_LIVE",
        domain_id: 103,
        emission_fn: "d103_naval_combat_results_units",
        emission_argument:
            "&d103_naval_combat_results_units(read.walk_counters, &read.diagnostic_units)",
    },
    CounterWireProducer {
        producer_struct: "D79HookCounters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D79_FILE,
        source_struct: "HookDiagnostics",
        lift_fn: "diagnostics",
        lift_return_paths: 2,
        state_struct: "HookState",
        state_counters_struct: "D79StateCounters",
        state_fields_off_the_channel: &[],
        declarations_that_are_not_counters: &[
            (
                "ActiveAlertRow.alert_type",
                "the engine's own alert-type code for the row",
            ),
            (
                "ActiveAlertRow.source_ordinal",
                "the row's position in the engine's alert list, read per capture",
            ),
            (
                "ActiveAlertRow.template_class",
                "the engine's template-class code for the row",
            ),
            (
                "ActiveAlertRow.country_target_db_id",
                "the engine database id of the country the alert points at",
            ),
            (
                "ActiveAlertRow.icon_frame",
                "which frame of the alert's icon sprite the engine selected",
            ),
            (
                "RawHudControllerRoot.sequence",
                "one id taken from the registered NEXT_SEQUENCE allocator when the root is \
                 observed, so two reads of one surface can be ordered; the allocator is the \
                 counter and is published, this is a single draw from it",
            ),
            (
                "RawHudControllerRoot.setup_rva",
                "the module-relative address of the setup callback the root was seen through",
            ),
            (
                "TopbarAlertCounts.country_tag_db_id",
                "the engine database id of the country the topbar row belongs to",
            ),(
            "ACTIVE_CALLBACKS",
            "a per-thread bitmask of the four HUD setup callbacks currently on this \
             thread's stack; bits are set and restored, never accumulated",
        )],
        locals_that_are_not_counters: &[
            (
                "lanes/hud_role_observer_live/src/lib.rs::designer_count",
                "how many designer surfaces the activation frame lists, counted inside validate() to refuse a frame carrying more than one. It never leaves that function and nothing reads it afterwards: a validation tally, not a reading of the hour",
            ),
            (
                "lanes/hud_role_observer_live/src/lib.rs::gui_roots",
                "how many of a surface's bindings are GUI roots, recounted inside validate() to check the closure has exactly one",
            ),
        ],
        crate_modules_outside_the_domain: &[],
        measured_units_carrier: "",
        measured_units: &[],
        measured_unit_name_sources: &[],
        domain_const: "COMPACT_DOMAIN_HUD_SURFACE_ACTIVATION_LIVE",
        domain_id: 79,
        emission_fn: "d79_hook_counter_units",
        emission_argument:
            "&d79_hook_counter_units(&hud_role_observer_live_lane::a729_hook::diagnostics())",
    },
    CounterWireProducer {
        producer_struct: "D87HookCounters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        lift_fn: "diagnostics",
        lift_return_paths: 2,
        state_struct: "HookState",
        state_counters_struct: "D87StateCounters",
        state_fields_off_the_channel: &[
            (
                "latest_frame_hour",
                "the game hour of the snapshot the hook is holding, replaced each capture",
            ),
            (
                "outstanding_receipt_nonce",
                "the nonce of the receipt awaiting collection, compared for equality",
            ),
        ],
        declarations_that_are_not_counters: &[
            (
                "DecisionPanelChild.presentation_index",
                "the child's position in the panel's presentation order, read per capture",
            ),
            (
                "DecisionPanelChild.decision_row_index",
                "which decision row this child belongs to, an index into the same snapshot",
            ),
            (
                "DecisionPanelFrame.elapsed_wall_ms",
                "the wall clock the frame was published at, replaced every frame",
            ),
            (
                "DecisionPanelFrame.frame_hour",
                "the game hour the frame was published under",
            ),
            (
                "DecisionPanelFrame.status_flags",
                "the frame's status word, a bitfield the consumer reads by mask",
            ),
            (
                "DecisionPanelSnapshot.source_sequence",
                "the id of the capture this snapshot came from, compared for equality",
            ),
            (
                "DecisionPanelSnapshot.materialization_qpc_ticks",
                "a QPC timestamp: when the engine materialised the panel",
            ),
            (
                "DecisionPanelSnapshot.available_alert_count",
                "the engine's own count of available-decision alerts FOR THIS SNAPSHOT, carried  in the domain's data record and replaced each capture, never added to",
            ),
            (
                "DecisionPanelSnapshot.timeout_alert_count",
                "the engine's own count of timing-out decision alerts FOR THIS SNAPSHOT, carried in the domain's data record beside available_alert_count and replaced each capture, never added to",
            ),
            (
                "DecisionRow.presentation_index",
                "the row's position in the panel's presentation order",
            ),
            (
                "DecisionRow.interface_state",
                "the engine's interface-state code for the row",
            ),
            (
                "DecisionRow.on_map_entry_ordinal",
                "which on-map entry the row's target resolves to, an engine ordinal",
            ),
            (
                "SelectedPresentStamp.frame_hour",
                "the game hour the selected present was stamped in",
            ),
            (
                "SelectedPresentStamp.selected_present_ordinal",
                "which present call the stamp was taken in, compared for equality",
            ),
            (
                "SelectedPresentStamp.selected_present_qpc_ticks",
                "a QPC timestamp: when that present call ran",
            ),
            (
                "SelectedPresentStamp.qpc_frequency_hz",
                "the machine's QPC frequency, so the ticks above can be read as time",
            ),
            (
                "Target::State.0",
                "the engine state id a decision targets, carried in the enum's payload",
            ),
            (
                "CaptureEpoch.bound_thread_id",
                "the OS thread the epoch's first callback bound to, compared for equality to \
                 refuse a materializer that crossed threads",
            ),
            (
                "CaptureEpoch.frame_hour",
                "the game hour the epoch was opened for, carried so a receipt can be matched \
                 to its hour",
            ),
            (
                "CaptureEpoch.generation",
                "the epoch id handed out by next_generation; a copy of that counter's value, \
                 not a second counter",
            ),
            (
                "CompletedAlert.count",
                "the alert accumulator's own argument -- the engine's row count for THIS \
                 call -- replaced per callback and never added to",
            ),
            (
                "HookRowWitness.interface_state",
                "the engine's interface-state code for the row",
            ),
            (
                "HookRowWitness.presentation_index",
                "the row's position in the panel's presentation order",
            ),
            (
                "MaterializationReceipt.active_country_tag_db_id",
                "the engine's database id for the POV country",
            ),
            (
                "MaterializationReceipt.frame_hour",
                "the game hour the receipt was materialized for",
            ),
            (
                "MaterializationReceipt.materialization_qpc_ticks",
                "a QPC timestamp of the materialization, not a count of anything",
            ),
            (
                "MaterializationReceipt.nonce",
                "the receipt's own nonce, matched against the outstanding one",
            ),
            (
                "MaterializationReceipt.source_sequence",
                "the snapshot's source sequence, a stamp copied in from the read",
            ),
            (
                "ProductionDecisionSemanticQuery.interface_state",
                "the engine's interface-state code the query carries",
            ),
            (
                "ResolvedDecisionPanelSupplement.country_tag_db_id",
                "the engine's database id for the country the panel belongs to",
            ),
            (
                "WorkingPanel.active_country_tag_db_id",
                "the engine's database id for the POV country, on the in-flight panel",
            ),
            (
                "WorkingPanel.materialization_qpc_ticks",
                "a QPC timestamp of the in-flight materialization",
            ),
            (
                "WorkingPanel.qpc_frequency_hz",
                "the QPC tick rate the OS reports, a rate rather than a count",
            ),
            (
                "WorkingPresenter.interface_state",
                "the engine's interface-state code for the presenter",
            ),
        ],
        locals_that_are_not_counters: &[
            (
                "lanes/decision_panel_live/src/lib.rs::index",
                "a loop index over the snapshot's rows, walking positions rather than counting anything",
            ),
            (
                "lanes/decision_panel_live/src/lib.rs::take",
                "a recount of the snapshot's own available_alert_count, compared against it inside validate()",
            ),
            (
                "lanes/decision_panel_live/src/lib.rs::timeout",
                "a recount of the snapshot's own timeout_alert_count, compared against it inside validate()",
            ),
        ],
        crate_modules_outside_the_domain: &[],
        measured_units_carrier: "",
        measured_units: &[],
        measured_unit_name_sources: &[],
        domain_const: "decision_panel_live_lane::DOMAIN_ID",
        domain_id: 87,
        emission_fn: "d87_hook_counter_units",
        emission_argument:
            "&d87_hook_counter_units(&decision_panel_live_lane::a729_hook::diagnostics())",
    },
    CounterWireProducer {
        producer_struct: "D72HookCounters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        lift_fn: "diagnostics",
        lift_return_paths: 1,
        state_struct: "HookState",
        state_counters_struct: "D72StateCounters",
        declarations_that_are_not_counters: &[
            (
                "SelectedPresentStamp.frame_hour",
                "the game hour the selected present was stamped in",
            ),
            (
                "SelectedPresentStamp.selected_present_candidate_id",
                "which present candidate the stamp was taken for, compared for equality",
            ),
            (
                "SelectedPresentStamp.selected_present_ordinal",
                "which present call the stamp was taken in",
            ),
            (
                "SelectedPresentStamp.selected_present_qpc_ticks",
                "a QPC timestamp: when that present call ran",
            ),
            (
                "ResolvedTooltipInstance.frame_hour",
                "the game hour the instance was resolved in",
            ),
            (
                "ResolvedTooltipInstance.repeat_index",
                "which repeat of an identical tooltip this row is, so two are distinguishable",
            ),
            (
                "TooltipTraceFrame.elapsed_wall_ms",
                "the wall clock the frame was published at, replaced every frame",
            ),
            (
                "TooltipTraceFrame.frame_hour",
                "the game hour the frame was published under",
            ),
            (
                "TooltipTraceFrame.status_flags",
                "the frame's status word, a bitfield the consumer reads by mask",
            ),
            (
                "TooltipTraceFrame.selected_present_candidate_id",
                "the candidate id the frame's stamp was taken for",
            ),
            (
                "TooltipTraceFrame.selected_present_ordinal",
                "the present call the frame's stamp was taken in",
            ),
            (
                "TooltipTraceFrame.selected_present_qpc_ticks",
                "a QPC timestamp: when that present call ran",
            ),
            (
                "TooltipTraceFrame.producer_manifest_sha256",
                "the digest of the producer manifest the frame was built against",
            ),
            (
                "TooltipTraceFrame.expected_instance_count",
                "how many instances the producer said this frame would carry, carried in the  domain's data record and compared against what arrived, never added to",
            ),
            (
                "A729ResolvedTooltipHookOwner.epoch",
                "the INSTALL_EPOCH value this owner installed under, compared for equality \
                 when it uninstalls",
            ),
            (
                "A729ResolvedTooltipHookSource.install_epoch",
                "the same install stamp on the read side, compared for equality to refuse a \
                 source outliving its install",
            ),
            (
                "ControllerObservation.qpc_ticks",
                "a QPC timestamp of the hidden observation, not a count of anything",
            ),
            (
                "ControllerObservation.sequence",
                "a copy of observation_sequence stamped on the queued observation",
            ),
            (
                "SnapshotResult.qpc_ticks",
                "a QPC timestamp read inside the snapshot, not a count of anything",
            ),
            (
                "VisibleObservation.owner_generation",
                "a copy of the registered owner_generation stamped on the observation",
            ),
            (
                "VisibleObservation.qpc_ticks",
                "a QPC timestamp of the visible observation, not a count of anything",
            ),
            (
                "VisibleObservation.sequence",
                "a copy of observation_sequence stamped on the queued observation",
            ),
        ],
        state_fields_off_the_channel: &[
            (
                "last_sealed_present_qpc_ticks",
                "a QPC timestamp of the last sealed present, not a count of anything",
            ),
            (
                "last_owner_address",
                "the owner object's address, compared to detect a replaced owner",
            ),
        ],
        locals_that_are_not_counters: &[
            (
                "lanes/resolved_tooltip_instances_live/src/a729_hook.rs::index",
                "a loop index over the 32 bytes of a hex digest, walking positions",
            ),
        ],
        crate_modules_outside_the_domain: &[],
        measured_units_carrier: "",
        measured_units: &[],
        measured_unit_name_sources: &[],
        domain_const: "COMPACT_DOMAIN_RESOLVED_TOOLTIP_INSTANCES_LIVE",
        domain_id: 72,
        emission_fn: "d72_hook_counter_units",
        emission_argument:
            "&d72_hook_counter_units(&resolved_tooltip_instances_live_lane::a729_hook::diagnostics())",
    },
    CounterWireProducer {
        producer_struct: "D78HookCounters",
        producer_file: "collector/src/attach_log.rs",
        source_file: D78_FILE,
        source_struct: "HookDiagnostics",
        lift_fn: "diagnostics",
        lift_return_paths: 2,
        state_struct: "HookState",
        state_counters_struct: "D78StateCounters",
        declarations_that_are_not_counters: &[
            (
                "DesignCatalogContextFrame.elapsed_wall_ms",
                "the wall clock the frame was published at, replaced every frame",
            ),
            (
                "DesignCatalogContextFrame.frame_hour",
                "the game hour the frame was published under",
            ),
            (
                "DesignCatalogContextFrame.status_flags",
                "the frame's status word, a bitfield the consumer reads by mask",
            ),
            (
                "DesignerViewContext.transition_sequence",
                "the id the registered next_transition_sequence minted for this transition: bump increments that counter and returns the new value, which is stamped here and never added to",
            ),
            (
                "DesignerViewContext.current_country_tag_id",
                "the engine database id of the country the designer view is open for",
            ),
            (
                "DesignerViewContext.ordered_design_ids",
                "the design ids the view lists, in the engine's own order; a row set, not a count",
            ),
            (
                "DesignerViewContext.selected_country_tag_ids",
                "the country ids selected in the view's filter; a row set, not a count",
            ),(
            "TEARDOWN_PHASE",
            "a four-value state machine for what an interrupted teardown still owes; stored \
             and compared for equality, never accumulated",
        )],
        state_fields_off_the_channel: &[
            (
                "latest_view_ptr",
                "the designer view object's address, compared to detect a replaced view",
            ),
            (
                "latest_runtime_root_ptr",
                "the runtime root's address, read back by name resolution",
            ),
        ],
        locals_that_are_not_counters: &[
            (
                "lanes/design_catalog_context_live/src/lib.rs::i",
                "a loop index over a byte string, walking positions",
            ),
        ],
        crate_modules_outside_the_domain: &[
            (
                "lanes/design_catalog_context_live/src/a40_peace_evidence.rs",
                "the peace-conference evidence producer's own module: a different domain that  shares this crate, whose counters are that domain's to publish",
            ),
            (
                "lanes/design_catalog_context_live/src/peace_conference_live.rs",
                "the peace-conference domain's reader, sharing this crate",
            ),
            (
                "lanes/design_catalog_context_live/src/peace_conference_presence.rs",
                "the peace-conference domain's presence probe, sharing this crate",
            ),
            (
                "lanes/design_catalog_context_live/src/selected_designer_identity.rs",
                "the selected-designer-identity domain, sharing this crate",
            ),
            (
                "lanes/design_catalog_context_live/src/selected_designer_identity/a729_hook.rs",
                "that domain's own hook, with its own diagnostics struct; it publishes a  hand-written trailer and is named in DOMAINS_WITH_HAND_WRITTEN_DIAGNOSTICS",
            ),
        ],
        measured_units_carrier: "",
        measured_units: &[],
        measured_unit_name_sources: &[],
        domain_const: "COMPACT_DOMAIN_DESIGN_CATALOG_CONTEXT_LIVE",
        domain_id: 78,
        emission_fn: "d78_hook_counter_units",
        emission_argument:
            "&d78_hook_counter_units(&design_catalog_context_live_lane::a729_hook::diagnostics())",
    },
];

/// Every registered counter.
pub const REGISTRY: &[CounterWireRow] = &[
    // ---- d95 spotted units ----------------------------------------------
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "hooks_installed",
        source_type: "bool",
        lifts: &["installed()"],
        producer_struct: "D95Counters",
        unit: "d95_hooks_installed",
        role: crate::diagnostic_channel::UnitRole::Deciding,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "site_entries_intel_getter",
        source_type: "u64",
        lifts: &["SITE_ENTRIES[SITE_INTEL_GETTER].load(Ordering::Relaxed)"],
        producer_struct: "D95Counters",
        unit: "d95_site_entries_intel_getter",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "site_entries_land_frame",
        source_type: "u64",
        lifts: &["SITE_ENTRIES[SITE_LAND_FRAME].load(Ordering::Relaxed)"],
        producer_struct: "D95Counters",
        unit: "d95_site_entries_land_frame",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "site_entries_land_eligibility",
        source_type: "u64",
        lifts: &["SITE_ENTRIES[SITE_LAND_ELIGIBILITY].load(Ordering::Relaxed)"],
        producer_struct: "D95Counters",
        unit: "d95_site_entries_land_eligibility",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "site_entries_taskforce_frame",
        source_type: "u64",
        lifts: &["SITE_ENTRIES[SITE_TASKFORCE_FRAME].load(Ordering::Relaxed)"],
        producer_struct: "D95Counters",
        unit: "d95_site_entries_taskforce_frame",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_candidates_seen",
        source_type: "u64",
        lifts: &["counters.land_candidates_seen"],
        producer_struct: "D95Counters",
        unit: "d95_land_candidates_seen",
        role: crate::diagnostic_channel::UnitRole::Deciding,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_rejected_unreadable_handle",
        source_type: "u64",
        lifts: &["counters.land_rejected_unreadable_handle"],
        producer_struct: "D95Counters",
        unit: "d95_land_rejected_unreadable_handle",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_rejected_below_intel_threshold",
        source_type: "u64",
        lifts: &["counters.land_rejected_below_intel_threshold"],
        producer_struct: "D95Counters",
        unit: "d95_land_rejected_below_intel_threshold",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_rejected_by_eligibility",
        source_type: "u64",
        lifts: &["counters.land_rejected_by_eligibility"],
        producer_struct: "D95Counters",
        unit: "d95_land_rejected_by_eligibility",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_rejected_taskforce_state",
        source_type: "u64",
        lifts: &["counters.land_rejected_taskforce_state"],
        producer_struct: "D95Counters",
        unit: "d95_land_rejected_taskforce_state",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_abandoned_pending",
        source_type: "u64",
        lifts: &["counters.land_abandoned_pending"],
        producer_struct: "D95Counters",
        unit: "d95_land_abandoned_pending",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_admitted",
        source_type: "u64",
        lifts: &["counters.land_admitted"],
        producer_struct: "D95Counters",
        unit: "d95_land_admitted",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_frames_without_enumeration",
        source_type: "u64",
        lifts: &["counters.land_frames_without_enumeration"],
        producer_struct: "D95Counters",
        unit: "d95_land_frames_without_enumeration",
        role: crate::diagnostic_channel::UnitRole::Locator,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_frames_producer_gate_closed",
        source_type: "u64",
        lifts: &["counters.land_frames_producer_gate_closed"],
        producer_struct: "D95Counters",
        unit: "d95_land_frames_producer_gate_closed",
        role: crate::diagnostic_channel::UnitRole::Locator,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "land_admitted_rows_lost_to_replacement",
        source_type: "u64",
        lifts: &["counters.land_admitted_rows_lost_to_replacement"],
        producer_struct: "D95Counters",
        unit: "d95_land_admitted_rows_lost_to_replacement",
        role: crate::diagnostic_channel::UnitRole::Deciding,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_candidates_seen",
        source_type: "u64",
        lifts: &["counters.taskforce_candidates_seen"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_candidates_seen",
        role: crate::diagnostic_channel::UnitRole::Deciding,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_rejected_unreadable_handle",
        source_type: "u64",
        lifts: &["counters.taskforce_rejected_unreadable_handle"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_rejected_unreadable_handle",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_rejected_below_intel_threshold",
        source_type: "u64",
        lifts: &["counters.taskforce_rejected_below_intel_threshold"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_rejected_below_intel_threshold",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_rejected_by_eligibility",
        source_type: "u64",
        lifts: &["counters.taskforce_rejected_by_eligibility"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_rejected_by_eligibility",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_rejected_taskforce_state",
        source_type: "u64",
        lifts: &["counters.taskforce_rejected_taskforce_state"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_rejected_taskforce_state",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_abandoned_pending",
        source_type: "u64",
        lifts: &["counters.taskforce_abandoned_pending"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_abandoned_pending",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_admitted",
        source_type: "u64",
        lifts: &["counters.taskforce_admitted"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_admitted",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_frames_without_enumeration",
        source_type: "u64",
        lifts: &["counters.taskforce_frames_without_enumeration"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_frames_without_enumeration",
        role: crate::diagnostic_channel::UnitRole::Locator,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_frames_producer_gate_closed",
        source_type: "u64",
        lifts: &["counters.taskforce_frames_producer_gate_closed"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_frames_producer_gate_closed",
        role: crate::diagnostic_channel::UnitRole::Locator,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_admitted_rows_lost_to_replacement",
        source_type: "u64",
        lifts: &["counters.taskforce_admitted_rows_lost_to_replacement"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_admitted_rows_lost_to_replacement",
        role: crate::diagnostic_channel::UnitRole::Deciding,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "taskforce_frames_source_collection_empty",
        source_type: "u64",
        lifts: &["counters.taskforce_frames_source_collection_empty"],
        producer_struct: "D95Counters",
        unit: "d95_taskforce_frames_source_collection_empty",
        role: crate::diagnostic_channel::UnitRole::Locator,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D95_FILE,
        source_struct: "CensusDiagnostics",
        field: "next_sequence",
        source_type: "u64",
        lifts: &["NEXT_SEQUENCE.load(Ordering::Acquire)"],
        producer_struct: "D95Counters",
        unit: "d95_next_sequence",
        role: crate::diagnostic_channel::UnitRole::Counter,
        consumer: PERF_UNIT_CONSUMER,
    },
    // ---- d102 fog derivation inputs -------------------------------------
    CounterWireRow {
        source_file: D102_FILE,
        source_struct: "FogInputWalk",
        field: "countries_walked",
        source_type: "u32",
        lifts: &["countries_walked"],
        producer_struct: "D102FogInputCounters",
        unit: "fog_derivation_inputs_countries_walked",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D102_FILE,
        source_struct: "FogInputWalk",
        field: "units_without_handle",
        source_type: "u64",
        lifts: &["sink.counters.units_without_handle"],
        producer_struct: "D102FogInputCounters",
        unit: "fog_derivation_inputs_units_without_handle",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D102_FILE,
        source_struct: "FogInputWalk",
        field: "viewer_levels_non_positive",
        source_type: "u64",
        lifts: &["sink.counters.viewer_levels_non_positive"],
        producer_struct: "D102FogInputCounters",
        unit: "fog_derivation_inputs_viewer_levels_non_positive",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D102_FILE,
        source_struct: "FogInputWalk",
        field: "units_undispatched_class",
        source_type: "u64",
        lifts: &["sink.counters.units_undispatched_class"],
        producer_struct: "D102FogInputCounters",
        unit: "fog_derivation_inputs_units_undispatched_class",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D102_FILE,
        source_struct: "FogInputWalk",
        field: "air_containers_at_reserved_slot",
        source_type: "u64",
        lifts: &["sink.counters.air_containers_at_reserved_slot"],
        producer_struct: "D102FogInputCounters",
        unit: "fog_derivation_inputs_air_containers_at_reserved_slot",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    // ---- d103 naval combat results --------------------------------------
    CounterWireRow {
        source_file: D103_FILE,
        source_struct: "NavalResultWalk",
        field: "records_walked",
        source_type: "u32",
        lifts: &["sink.records_walked"],
        producer_struct: "D103NavalResultCounters",
        unit: "naval_combat_results_records_walked",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D103_FILE,
        source_struct: "NavalResultWalk",
        field: "combats_walked",
        source_type: "u32",
        lifts: &["visits.len().min(u32::MAX as usize) as u32"],
        producer_struct: "D103NavalResultCounters",
        unit: "naval_combat_results_combats_walked",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D103_FILE,
        source_struct: "NavalResultWalk",
        field: "units_without_location",
        source_type: "u64",
        lifts: &["visits.iter().map(|visit| visit.occurrence.counters.units_without_location).sum()"],
        producer_struct: "D103NavalResultCounters",
        unit: "naval_combat_results_units_without_location",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    // ---- d79 hud surface activation ------------------------------------
    CounterWireRow {
        source_file: D79_FILE,
        source_struct: "HookDiagnostics",
        field: "callback_count",
        source_type: "u64",
        lifts: &["state.counters.callback_count", "default"],
        producer_struct: "D79HookCounters",
        unit: "d79_hook_callback_count",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D79_FILE,
        source_struct: "HookDiagnostics",
        field: "rejected_observation_count",
        source_type: "u64",
        lifts: &["state.counters.rejected_observation_count", "default"],
        producer_struct: "D79HookCounters",
        unit: "d79_hook_rejected_observation_count",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D79_FILE,
        source_struct: "HookDiagnostics",
        field: "authenticated_surface_count",
        source_type: "usize",
        lifts: &["state.latest.len()", "default"],
        producer_struct: "D79HookCounters",
        unit: "d79_hook_authenticated_surface_count",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D79_FILE,
        source_struct: "HookDiagnostics",
        field: "in_flight_callbacks",
        source_type: "usize",
        lifts: &["IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)", "default"],
        producer_struct: "D79HookCounters",
        unit: "d79_hook_in_flight_callbacks",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D79_FILE,
        source_struct: "HookDiagnostics",
        field: "next_sequence",
        source_type: "u64",
        lifts: &["NEXT_SEQUENCE.load(Ordering::Acquire)", "default"],
        producer_struct: "D79HookCounters",
        unit: "d79_hook_next_sequence",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    // ---- d87 decision panel ---------------------------------------------
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "installed",
        source_type: "bool",
        lifts: &["installed()"],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_installed",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "epoch_open",
        source_type: "bool",
        lifts: &["state.epoch.is_some()", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_epoch_open",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "pending_receipt",
        source_type: "bool",
        lifts: &["state.pending_receipt.is_some()", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_pending_receipt",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "authenticated_snapshot",
        source_type: "bool",
        lifts: &["state.latest_snapshot.is_some()", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_authenticated_snapshot",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "materializer_calls",
        source_type: "u64",
        lifts: &["state.counters.materializer_calls", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_materializer_calls",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "insertion_calls",
        source_type: "u64",
        lifts: &["state.counters.insertion_calls", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_insertion_calls",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "text_events",
        source_type: "u64",
        lifts: &["state.counters.text_events", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_text_events",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "can_take_calls",
        source_type: "u64",
        lifts: &["state.counters.can_take_calls", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_can_take_calls",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "timeout_calls",
        source_type: "u64",
        lifts: &["state.counters.timeout_calls", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_timeout_calls",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "in_flight_callbacks",
        source_type: "usize",
        lifts: &["IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)"],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_in_flight_callbacks",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "next_generation",
        source_type: "u64",
        lifts: &["state.counters.next_generation", ".."],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_next_generation",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D87_FILE,
        source_struct: "HookDiagnostics",
        field: "current_error",
        source_type: "bool",
        lifts: &["state.current_error.is_some()", "true"],
        producer_struct: "D87HookCounters",
        unit: "d87_hook_current_error",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    // ---- d72 resolved tooltip instances ---------------------------------
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "installed",
        source_type: "bool",
        lifts: &["INSTALLED.load(Ordering::Acquire)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_installed",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "install_epoch",
        source_type: "u64",
        lifts: &["INSTALL_EPOCH.load(Ordering::Acquire) as u64"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_install_epoch",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "callbacks_open",
        source_type: "bool",
        lifts: &["CALLBACKS_OPEN.load(Ordering::Acquire)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_callbacks_open",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "in_flight_callbacks",
        source_type: "usize",
        lifts: &["IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_in_flight_callbacks",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "controller_calls",
        source_type: "u64",
        lifts: &["state.as_ref().map_or(0, |state| state.counters.controller_calls)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_controller_calls",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "observation_sequence",
        source_type: "u64",
        lifts: &["state.as_ref().map_or(0, |state| state.counters.observation_sequence)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_observation_sequence",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "owner_generation",
        source_type: "u64",
        lifts: &["state.as_ref().map_or(0, |state| state.counters.owner_generation)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_owner_generation",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "has_observed_baseline",
        source_type: "bool",
        lifts: &["state.as_ref().is_some_and(|state| state.counters.controller_calls > 0)"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_has_observed_baseline",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D72_FILE,
        source_struct: "HookDiagnostics",
        field: "poisoned",
        source_type: "bool",
        lifts: &["state.as_ref().is_some_and(|state| state.poison.is_some())"],
        producer_struct: "D72HookCounters",
        unit: "d72_hook_poisoned",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    // ---- d78 design catalog context -------------------------------------
    CounterWireRow {
        source_file: D78_FILE,
        source_struct: "HookDiagnostics",
        field: "callback_count",
        source_type: "u64",
        lifts: &["state.counters.callback_count", "default"],
        producer_struct: "D78HookCounters",
        unit: "d78_hook_callback_count",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D78_FILE,
        source_struct: "HookDiagnostics",
        field: "latest_transition_sequence",
        source_type: "u64",
        lifts: &[
            "state.latest.as_ref().map(|row| row.transition_sequence).unwrap_or(0)",
            "default",
        ],
        producer_struct: "D78HookCounters",
        unit: "d78_hook_latest_transition_sequence",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D78_FILE,
        source_struct: "HookDiagnostics",
        field: "next_transition_sequence",
        source_type: "u64",
        lifts: &["state.counters.next_transition_sequence", "default"],
        producer_struct: "D78HookCounters",
        unit: "d78_hook_next_transition_sequence",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D78_FILE,
        source_struct: "HookDiagnostics",
        field: "in_flight_callbacks",
        source_type: "usize",
        lifts: &["IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)", "default"],
        producer_struct: "D78HookCounters",
        unit: "d78_hook_in_flight_callbacks",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
    CounterWireRow {
        source_file: D78_FILE,
        source_struct: "HookDiagnostics",
        field: "has_complete_snapshot",
        source_type: "bool",
        lifts: &["state.latest.is_some()", "default"],
        producer_struct: "D78HookCounters",
        unit: "d78_hook_has_complete_snapshot",
        role: crate::diagnostic_channel::UnitRole::Undeclared,
        consumer: PERF_UNIT_CONSUMER,
    },
];

/// The registry rows of one producer, in declaration order.
pub fn rows_for(producer_struct: &str) -> Vec<&'static CounterWireRow> {
    REGISTRY
        .iter()
        .filter(|row| row.producer_struct == producer_struct)
        .collect()
}
