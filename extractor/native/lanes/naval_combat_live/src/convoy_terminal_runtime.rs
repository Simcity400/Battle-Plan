use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::convoy_route_source::{
    ExactA729RouteRead, CONVOY_ROUTE_REFUSAL_UNCLASSIFIED_INDEX,
};
use crate::convoy_terminal_attribution::{
    route_resolution_is_well_formed, CombatEpoch, ConvoyCombatRefusal, ConvoyCombatRefusalCause,
    ConvoyRouteRefusalCause, EntityHandle,
    ExactCombatEpochResolver, ExactLiveRouteResolver, LiveRouteResolution,
    CONVOY_ROUTE_REFUSAL_CAUSE_COUNT, D90_STATIC_AUTHORITY_MANIFEST_SHA256,
    GAME_ENTITY_TYPE_TAG,
};
use crate::extractor::ValidatedCombatSnapshot;
use crate::member_epoch_runtime::{build_member_epoch_map, MemberEpochBinding};

const MAX_ACTIVE_ROUTE_BINDINGS: usize = 65_536;

static NEXT_PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static RESOLVER_STATE: OnceLock<RwLock<Arc<ResolverSnapshot>>> = OnceLock::new();

/// Hour of the most recent resolver publication (`i32::MIN` before the first).
static LAST_PUBLICATION_HOUR: AtomicI32 = AtomicI32::new(i32::MIN);
/// Running ceiling of observed inter-publication intervals. The staleness
/// bound events are judged against: a sink between two publications can lag
/// the receipt by at most one cadence interval, so anything older than the
/// WORST interval this session has produced is too stale to attribute.
static MAX_PUBLICATION_INTERVAL_HOURS: AtomicU32 = AtomicU32::new(0);

static RESOLVER_EVER_PARTIAL: AtomicBool = AtomicBool::new(false);

pub fn convoy_terminal_resolver_ever_partial() -> bool {
    RESOLVER_EVER_PARTIAL.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedLiveRouteBinding {
    pub client: EntityHandle,
    pub resolution: LiveRouteResolution,
    pub reference_resolver: usize,
    pub reference_registry: usize,
    pub reference_slot: usize,
    pub route_object: usize,
    pub route_vtable: usize,
    pub source_endpoint_object: usize,
    pub destination_endpoint_object: usize,
    pub observation_game_hour_raw: i32,
    pub raw_observation_sha256: [u8; 32],
    pub static_authority_manifest_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ResolverPublicationReceipt {
    pub publication_sequence: u64,
    pub game_hour_raw: i32,
    /// Game hours between THIS publication and the previous one, as observed
    /// by the collector's own cadence (0 for the session's first publication).
    /// This is the staleness bound events are judged against at capture time:
    /// a sink firing between publications can lag the receipt by at most the
    /// current inter-publication interval before its combat-epoch join is
    /// considered too stale to attribute. Derived, never hardcoded.
    pub publication_interval_hours: u32,
    pub member_bindings: u32,
    pub route_bindings: u32,
    pub unbound_convoy_clients: u32,
}

#[derive(Clone, Debug)]
pub struct ResolverSnapshot {
    receipt: ResolverPublicationReceipt,
    member_epochs: BTreeMap<usize, crate::convoy_terminal_attribution::CombatEpochMembership>,
    routes: BTreeMap<EntityHandle, AuthenticatedLiveRouteBinding>,
    /// Why each client the sweep REFUSED has no binding, by index into
    /// `CONVOY_ROUTE_REFUSAL_UNIT_NAMES`.
    ///
    /// Carried alongside the bindings because the sweep is the only place the
    /// typed `RouteSourceError` still exists: capture time sees a handle that
    /// is simply missing from `routes`, and "missing" is not a cause. Without
    /// this map every refused sink would be recorded under one indistinct
    /// bucket, which is the shape of the defect the refusal leaves were added
    /// to end.
    route_refusals: BTreeMap<EntityHandle, RouteRefusalRecord>,
}

/// What the sweep recorded about one refused client: the named cause, and the
/// producer vtable RVA when the cause was an unrecognised class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RouteRefusalRecord {
    pub cause_index: u8,
    pub unrecognised_vtable_rva: u32,
}

impl RouteRefusalRecord {
    /// A refusal whose cause names no vtable.
    pub fn cause(cause_index: u8) -> Self {
        Self {
            cause_index,
            unrecognised_vtable_rva: 0,
        }
    }
}

impl Default for ResolverSnapshot {
    fn default() -> Self {
        Self {
            receipt: ResolverPublicationReceipt::default(),
            member_epochs: BTreeMap::new(),
            routes: BTreeMap::new(),
            route_refusals: BTreeMap::new(),
        }
    }
}

impl ResolverSnapshot {
    pub fn receipt(&self) -> ResolverPublicationReceipt {
        self.receipt
    }

    pub fn route_evidence(&self, client: EntityHandle) -> Option<&AuthenticatedLiveRouteBinding> {
        self.routes.get(&client)
    }

    pub fn evidence_member_epochs(
        &self,
    ) -> impl Iterator<Item = (usize, crate::convoy_terminal_attribution::CombatEpochMembership)> + '_
    {
        self.member_epochs.iter().map(|(address, membership)| (*address, *membership))
    }
}

impl ExactCombatEpochResolver for ResolverSnapshot {
    fn resolve_member_epoch(
        &self,
        member_address: usize,
    ) -> Result<crate::convoy_terminal_attribution::CombatEpochMembership, ConvoyCombatRefusal> {
        self.member_epochs
            .get(&member_address)
            .copied()
            .ok_or_else(|| ConvoyCombatRefusal {
                cause: ConvoyCombatRefusalCause::MemberAbsentFromEpochSnapshot,
                detail: "CFEXMember is absent from the same-hour d49/d50 epoch snapshot".into(),
            })
    }
}

impl ExactLiveRouteResolver for ResolverSnapshot {
    fn resolve_client_route(
        &self,
        client: EntityHandle,
    ) -> Result<LiveRouteResolution, ConvoyRouteRefusalCause> {
        if let Some(binding) = self.routes.get(&client) {
            return Ok(binding.resolution);
        }
        // The cause is the one the SWEEP recorded for this client when it built
        // the publication, carried per client rather than re-derived: at this
        // point the typed `RouteSourceError` is long gone, and a message
        // composed here would say only "absent", which is what the sink already
        // knows. A client the sweep never even saw is a different fact and gets
        // its own cause rather than borrowing another's.
        let record = self.route_refusals.get(&client).copied().unwrap_or(
            RouteRefusalRecord {
                cause_index: CONVOY_ROUTE_REFUSAL_UNCLASSIFIED_INDEX as u8,
                unrecognised_vtable_rva: 0,
            },
        );
        let cause_index = record.cause_index;
        Err(ConvoyRouteRefusalCause {
            cause_index,
            unrecognised_vtable_rva: record.unrecognised_vtable_rva,
            detail: format!(
                "client is absent from the authenticated live route publication (cause {cause_index})"
            ),
        })
    }
}

fn state() -> &'static RwLock<Arc<ResolverSnapshot>> {
    RESOLVER_STATE.get_or_init(|| RwLock::new(Arc::new(ResolverSnapshot::default())))
}

fn accepted_authority() -> Result<[u8; 32], String> {
    D90_STATIC_AUTHORITY_MANIFEST_SHA256
        .ok_or_else(|| "D90 exact-a729 static authority manifest is not pinned".to_string())
}

fn authenticate_a729_route_read(
    read: ExactA729RouteRead,
    game_hour_raw: i32,
    static_authority_manifest_sha256: [u8; 32],
) -> AuthenticatedLiveRouteBinding {
    AuthenticatedLiveRouteBinding {
        client: read.client,
        resolution: read.resolution,
        reference_resolver: read.reference_resolver,
        reference_registry: read.reference_registry,
        reference_slot: read.reference_slot,
        route_object: read.route_object,
        route_vtable: read.route_vtable,
        source_endpoint_object: read.source_endpoint_object,
        destination_endpoint_object: read.destination_endpoint_object,
        observation_game_hour_raw: game_hour_raw,
        raw_observation_sha256: read.raw_observation_sha256(game_hour_raw),
        static_authority_manifest_sha256,
    }
}

pub fn authenticate_exact_a729_route_read(
    read: ExactA729RouteRead,
    game_hour_raw: i32,
) -> Result<AuthenticatedLiveRouteBinding, String> {
    let static_authority_manifest_sha256 = accepted_authority()?;
    Ok(authenticate_a729_route_read(
        read,
        game_hour_raw,
        static_authority_manifest_sha256,
    ))
}

pub fn authenticate_evidence_a729_route_read(
    read: ExactA729RouteRead,
    game_hour_raw: i32,
) -> Result<AuthenticatedLiveRouteBinding, String> {
    Ok(authenticate_a729_route_read(read, game_hour_raw, [0; 32]))
}

fn validate_route_binding(
    binding: &AuthenticatedLiveRouteBinding,
    expected_hour: i32,
    authority: [u8; 32],
) -> Result<(), String> {
    if binding.client.id == 0
        || binding.client.type_tag != GAME_ENTITY_TYPE_TAG
        || binding.resolution.route_key != binding.client
    {
        return Err("live route binding has an invalid or nonmatching client key".into());
    }
    if binding.reference_resolver == 0
        || binding.reference_registry == 0
        || binding.reference_slot == 0
        || binding.route_object == 0
        || binding.route_vtable == 0
        || binding.source_endpoint_object == 0
        || binding.destination_endpoint_object == 0
    {
        return Err("live route binding lacks rooted process-local custody".into());
    }
    if binding.observation_game_hour_raw != expected_hour {
        return Err("live route binding hour differs from d49/d50 publication hour".into());
    }
    if binding.raw_observation_sha256 == [0; 32]
        || binding.static_authority_manifest_sha256 != authority
    {
        return Err("live route binding evidence hashes are absent or not accepted".into());
    }
    // The producer decides the endpoint kind and the range its ids may take,
    // so the pairing is checked here rather than each variant being merely
    // enumerated: a country tag published as a province id is a reading a
    // consumer cannot detect.
    if !route_resolution_is_well_formed(&binding.resolution) {
        return Err("live route binding endpoints do not match its producer kind".into());
    }
    Ok(())
}

pub fn publish_convoy_terminal_resolver_snapshot(
    snapshots: &[ValidatedCombatSnapshot],
    routes: &[AuthenticatedLiveRouteBinding],
    game_hour_raw: i32,
) -> Result<ResolverPublicationReceipt, String> {
    let mut member_bindings = Vec::new();
    for snapshot in snapshots {
        if snapshot.lifecycle.combat_id == 0 || snapshot.lifecycle.generation == 0 {
            return Err("d49/d50 snapshot has an invalid combat epoch".into());
        }
        let epoch = CombatEpoch {
            combat_id: snapshot.lifecycle.combat_id,
            generation: snapshot.lifecycle.generation,
        };
        for member in &snapshot.members {
            let member_ptr = usize::try_from(member.member_ptr)
                .map_err(|_| "CFEXMember pointer does not fit this process".to_string())?;
            if member.combat_id != epoch.combat_id || member.generation != epoch.generation {
                return Err("d49/d50 member pointer or epoch is inconsistent".into());
            }
            member_bindings.push(MemberEpochBinding {
                member_address: member_ptr,
                combat_id: epoch.combat_id,
                combat_generation: epoch.generation,
                battle_slot_index_raw: member.battle_slot_index_raw,
            });
        }
    }
    publish_convoy_terminal_resolver_bindings(&member_bindings, routes, &[], game_hour_raw, 0)
}

pub fn publish_convoy_terminal_resolver_bindings(
    member_bindings: &[MemberEpochBinding],
    routes: &[AuthenticatedLiveRouteBinding],
    route_refusals: &[(EntityHandle, RouteRefusalRecord)],
    game_hour_raw: i32,
    unbound_convoy_clients: u32,
) -> Result<ResolverPublicationReceipt, String> {
    let authority = accepted_authority()?;
    publish_convoy_terminal_resolver_bindings_with_hashes(
        member_bindings,
        routes,
        route_refusals,
        game_hour_raw,
        authority,
        unbound_convoy_clients,
    )
}

pub fn publish_evidence_convoy_terminal_resolver_bindings(
    member_bindings: &[MemberEpochBinding],
    routes: &[AuthenticatedLiveRouteBinding],
    route_refusals: &[(EntityHandle, RouteRefusalRecord)],
    game_hour_raw: i32,
    unbound_convoy_clients: u32,
) -> Result<ResolverPublicationReceipt, String> {
    publish_convoy_terminal_resolver_bindings_with_hashes(
        member_bindings,
        routes,
        route_refusals,
        game_hour_raw,
        [0; 32],
        unbound_convoy_clients,
    )
}

fn publish_convoy_terminal_resolver_bindings_with_hashes(
    member_bindings: &[MemberEpochBinding],
    routes: &[AuthenticatedLiveRouteBinding],
    route_refusals: &[(EntityHandle, RouteRefusalRecord)],
    game_hour_raw: i32,
    authority: [u8; 32],
    unbound_convoy_clients: u32,
) -> Result<ResolverPublicationReceipt, String> {
    let member_epochs: BTreeMap<usize, crate::convoy_terminal_attribution::CombatEpochMembership> =
        build_member_epoch_map(member_bindings)?
            .into_iter()
            .map(|(member_address, resolution)| {
                (
                    member_address,
                    crate::convoy_terminal_attribution::CombatEpochMembership {
                        epoch: resolution.epoch,
                        battle_slot_index_raw: resolution.battle_slot_index_raw,
                    },
                )
            })
            .collect();

    let mut route_map = BTreeMap::new();
    for binding in routes {
        validate_route_binding(binding, game_hour_raw, authority)?;
        if route_map.insert(binding.client, *binding).is_some() {
            return Err("duplicate client handle in live generic-reference publication".into());
        }
        if route_map.len() > MAX_ACTIVE_ROUTE_BINDINGS {
            return Err("live route binding cap exceeded".into());
        }
    }

    let mut refusal_map = BTreeMap::new();
    for (client, record) in route_refusals {
        if usize::from(record.cause_index) >= CONVOY_ROUTE_REFUSAL_CAUSE_COUNT {
            return Err("route refusal cause index is outside the named set".into());
        }
        // A client cannot be both bound and refused in one publication: the
        // sink would then carry a route AND a reason it has none, and which one
        // a reader believed would depend on lookup order.
        if route_map.contains_key(client) {
            return Err("client handle is both bound and refused in one publication".into());
        }
        if refusal_map.insert(*client, *record).is_some() {
            return Err("duplicate client handle in the route refusal set".into());
        }
        if refusal_map.len() > MAX_ACTIVE_ROUTE_BINDINGS {
            return Err("live route refusal cap exceeded".into());
        }
    }

    if unbound_convoy_clients != 0 {
        RESOLVER_EVER_PARTIAL.store(true, Ordering::Release);
    }
    let publication_sequence = NEXT_PUBLICATION_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if publication_sequence == 0 {
        return Err("resolver publication sequence exhausted".into());
    }
    // The collector's own cadence, measured between consecutive publications.
    // The receipt carries the running CEILING of observed intervals: a cadence
    // slowdown takes effect immediately (tighter intervals may refuse until a
    // new ceiling lands, which is the safe direction), and a speedup does not
    // silently loosen the bound mid-session. A regressed clock or the first
    // publication yields interval 0 = "same-hour only".
    let previous_hour = LAST_PUBLICATION_HOUR.swap(game_hour_raw, Ordering::AcqRel);
    let interval = if previous_hour != i32::MIN && game_hour_raw > previous_hour {
        u32::try_from(game_hour_raw - previous_hour).unwrap_or(u32::MAX)
    } else {
        0
    };
    MAX_PUBLICATION_INTERVAL_HOURS.fetch_max(interval, Ordering::AcqRel);
    let publication_interval_hours = MAX_PUBLICATION_INTERVAL_HOURS.load(Ordering::Acquire);
    let receipt = ResolverPublicationReceipt {
        publication_sequence,
        game_hour_raw,
        publication_interval_hours,
        member_bindings: u32::try_from(member_epochs.len())
            .map_err(|_| "member binding count exceeds u32".to_string())?,
        route_bindings: u32::try_from(route_map.len())
            .map_err(|_| "route binding count exceeds u32".to_string())?,
        unbound_convoy_clients,
    };
    let next = Arc::new(ResolverSnapshot {
        receipt,
        member_epochs,
        routes: route_map,
        route_refusals: refusal_map,
    });
    *state()
        .write()
        .map_err(|_| "convoy terminal resolver state poisoned".to_string())? = next;
    Ok(receipt)
}

pub fn current_convoy_terminal_resolver_snapshot() -> Result<Arc<ResolverSnapshot>, String> {
    state()
        .read()
        .map(|snapshot| Arc::clone(&snapshot))
        .map_err(|_| "convoy terminal resolver state poisoned".to_string())
}

pub fn try_current_convoy_terminal_resolver_snapshot() -> Option<Arc<ResolverSnapshot>> {
    state()
        .try_read()
        .ok()
        .map(|snapshot| Arc::clone(&snapshot))
}

pub fn current_convoy_terminal_publication_receipt() -> Result<ResolverPublicationReceipt, String> {
    Ok(current_convoy_terminal_resolver_snapshot()?.receipt())
}

pub fn clear_convoy_terminal_resolver_snapshot() -> Result<(), String> {
    *state()
        .write()
        .map_err(|_| "convoy terminal resolver state poisoned".to_string())? =
        Arc::new(ResolverSnapshot::default());
    // The next publication starts a NEW interval: without this reset it would
    // be measured across the clear gap and could swallow the whole gap into
    // the cadence ceiling. The ceiling itself persists -- it is a session
    // fact about how fast this collector has ever republished.
    LAST_PUBLICATION_HOUR.store(i32::MIN, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the fixtures build endpoints; production here carries whole
    // resolutions, so the import belongs to the test module or it is an
    // unused import in every real build (Codex round 10).
    use crate::convoy_terminal_attribution::{RouteEndpoint, RouteEndpointKind, RouteProducerKind};

    /// The resolver snapshot is PROCESS-GLOBAL, and cargo runs the tests in
    /// one binary concurrently. Every test that publishes takes this first, or
    /// one test's publication replaces the snapshot another is about to read
    /// and the failure looks like a logic bug in whichever lost the race.
    static PUBLICATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn publication_guard() -> std::sync::MutexGuard<'static, ()> {
        PUBLICATION.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn client(id: u32) -> EntityHandle {
        EntityHandle {
            id,
            type_tag: GAME_ENTITY_TYPE_TAG,
        }
    }

    /// A binding that satisfies `validate_route_binding` under the evidence
    /// authority, so a test can publish a REAL binding rather than an empty
    /// slice.
    fn binding(client: EntityHandle, game_hour_raw: i32) -> AuthenticatedLiveRouteBinding {
        AuthenticatedLiveRouteBinding {
            client,
            resolution: LiveRouteResolution {
                route_key: client,
                producer_kind: RouteProducerKind::LendLease,
                source: RouteEndpoint {
                    kind: RouteEndpointKind::State,
                    id: 282,
                },
                destination: RouteEndpoint {
                    kind: RouteEndpointKind::State,
                    id: 64,
                },
            },
            reference_resolver: 0x1000,
            reference_registry: 0x2000,
            reference_slot: 0x3000,
            route_object: 0x4000,
            route_vtable: 0x5000,
            source_endpoint_object: 0x6000,
            destination_endpoint_object: 0x7000,
            observation_game_hour_raw: game_hour_raw,
            raw_observation_sha256: [7; 32],
            static_authority_manifest_sha256: [0; 32],
        }
    }

    /// Law: the cause the SWEEP classified is the cause the SINK is told.
    ///
    /// The typed `RouteSourceError` exists only while the sweep is building the
    /// publication. At capture time a refused client is simply a handle that is
    /// not in `routes`, and "not present" is not a cause -- that is how
    /// rc_vanilla published a per-frame error hash that resolved to nothing.
    /// Carrying the classification per client is what lets a recorded sink name
    /// which proof declined its route.
    ///
    /// What production change makes this fail: dropping `route_refusals` from
    /// the snapshot or from the publisher's arguments (every refused sink is
    /// then reported under the catch-all), or letting a client be both bound
    /// and refused (the sink would carry a route and a reason it has none, and
    /// which one a reader believed would depend on lookup order).
    #[test]
    fn a_refused_client_carries_the_sweeps_own_cause_to_the_sink() {
        let _guard = publication_guard();
        let refused = client(4_242);
        let never_seen = client(4_243);
        publish_convoy_terminal_resolver_bindings_with_hashes(
            &[],
            &[],
            &[(refused, RouteRefusalRecord::cause(5))],
            61_000_100,
            [0; 32],
            1,
        )
        .expect("a publication with no bindings and one refusal is valid");
        let snapshot = current_convoy_terminal_resolver_snapshot().expect("snapshot");

        assert_eq!(
            snapshot.resolve_client_route(refused).unwrap_err().cause_index,
            5,
            "the sink is told the cause the sweep classified, not a fresh message"
        );
        assert_eq!(
            snapshot
                .resolve_client_route(never_seen)
                .unwrap_err()
                .cause_index,
            CONVOY_ROUTE_REFUSAL_UNCLASSIFIED_INDEX as u8,
            "a client the sweep never saw is a different fact and gets its own cause"
        );
    }

    /// Law: a publication that contradicts itself is refused, not published.
    ///
    /// What production change makes this fail: dropping either guard in the
    /// refusal loop, or widening the accepted cause range past the set the
    /// carrier's decoder will admit -- which would produce rows the replay
    /// layer refuses hours later, on a capture.
    #[test]
    fn a_contradictory_or_unnameable_refusal_set_is_refused_at_publication() {
        let _guard = publication_guard();
        let both = client(4_244);
        assert!(publish_convoy_terminal_resolver_bindings_with_hashes(
            &[],
            &[],
            &[(
                both,
                RouteRefusalRecord::cause(u8::try_from(CONVOY_ROUTE_REFUSAL_CAUSE_COUNT).unwrap()),
            )],
            61_000_200,
            [0; 32],
            1,
        )
        .is_err());

        assert!(
            publish_convoy_terminal_resolver_bindings_with_hashes(
                &[],
                &[],
                &[
                    (both, RouteRefusalRecord::cause(1)),
                    (both, RouteRefusalRecord::cause(2)),
                ],
                61_000_300,
                [0; 32],
                1,
            )
            .is_err(),
            "one client cannot be refused twice with two different causes"
        );

        // AND THE GUARD THAT NEEDS A REAL BINDING TO EXERCISE. A client that is
        // both bound and refused would leave a sink carrying a route AND a
        // reason it has none, and which one a reader believed would depend on
        // lookup order.
        let hour = 61_000_400;
        assert!(
            publish_convoy_terminal_resolver_bindings_with_hashes(
                &[],
                &[binding(both, hour)],
                &[(both, RouteRefusalRecord::cause(1))],
                hour,
                [0; 32],
                1,
            )
            .is_err(),
            "a client cannot be bound and refused in the same publication"
        );
        assert!(
            publish_convoy_terminal_resolver_bindings_with_hashes(
                &[],
                &[binding(both, hour)],
                &[(client(9_999), RouteRefusalRecord::cause(1))],
                hour,
                [0; 32],
                1,
            )
            .is_ok(),
            "while a binding and an UNRELATED refusal publish together fine, \
             so the guard above is about the collision and not about refusals"
        );
    }

    #[test]
    fn a_partial_publication_latches_partiality_for_the_rest_of_the_capture() {
        let _guard = publication_guard();
        let publish = |unbound: u32| {
            publish_convoy_terminal_resolver_bindings_with_hashes(
                &[],
                &[],
                &[],
                61_000_000,
                [0; 32],
                unbound,
            )
            .expect("empty publication is valid")
        };

        let receipt = publish(0);
        assert_eq!(receipt.unbound_convoy_clients, 0);

        let receipt = publish(4);
        assert_eq!(receipt.unbound_convoy_clients, 4);
        assert!(
            convoy_terminal_resolver_ever_partial(),
            "a partial publication must latch"
        );

        let receipt = publish(0);
        assert_eq!(
            receipt.unbound_convoy_clients, 0,
            "the per-frame count does reset"
        );
        assert!(
            convoy_terminal_resolver_ever_partial(),
            "but the latch does not"
        );

        clear_convoy_terminal_resolver_snapshot().expect("clear");
        assert!(
            convoy_terminal_resolver_ever_partial(),
            "clearing the resolver must not clear the latch"
        );
    }
}
