use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::convoy_route_source::ExactA729RouteRead;
use crate::convoy_terminal_attribution::{
    CombatEpoch, CombatEpochMembership, EntityHandle, ExactCombatEpochResolver,
    ExactLiveRouteResolver, LiveRouteResolution, RouteEndpointKind, RouteProducerKind,
    D90_STATIC_AUTHORITY_MANIFEST_SHA256, GAME_ENTITY_TYPE_TAG,
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
}

impl Default for ResolverSnapshot {
    fn default() -> Self {
        Self {
            receipt: ResolverPublicationReceipt::default(),
            member_epochs: BTreeMap::new(),
            routes: BTreeMap::new(),
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
    ) -> Result<crate::convoy_terminal_attribution::CombatEpochMembership, String> {
        self.member_epochs.get(&member_address).copied().ok_or_else(|| {
            "CFEXMember is absent from the same-hour d49/d50 epoch snapshot".into()
        })
    }
}

impl ExactLiveRouteResolver for ResolverSnapshot {
    fn resolve_client_route(&self, client: EntityHandle) -> Result<LiveRouteResolution, String> {
        self.routes
            .get(&client)
            .map(|binding| binding.resolution)
            .ok_or_else(|| {
                "client handle is absent from the authenticated live route snapshot".into()
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
    if binding.resolution.source.id == 0 || binding.resolution.destination.id == 0 {
        return Err("live route binding has a zero endpoint".into());
    }
    match binding.resolution.producer_kind {
        RouteProducerKind::NavalTransport
        | RouteProducerKind::LendLease
        | RouteProducerKind::Export => {}
    }
    match binding.resolution.source.kind {
        RouteEndpointKind::State | RouteEndpointKind::Province => {}
    }
    match binding.resolution.destination.kind {
        RouteEndpointKind::State | RouteEndpointKind::Province => {}
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
    publish_convoy_terminal_resolver_bindings(&member_bindings, routes, game_hour_raw, 0)
}

pub fn publish_convoy_terminal_resolver_bindings(
    member_bindings: &[MemberEpochBinding],
    routes: &[AuthenticatedLiveRouteBinding],
    game_hour_raw: i32,
    unbound_convoy_clients: u32,
) -> Result<ResolverPublicationReceipt, String> {
    let authority = accepted_authority()?;
    publish_convoy_terminal_resolver_bindings_with_hashes(
        member_bindings,
        routes,
        game_hour_raw,
        authority,
        unbound_convoy_clients,
    )
}

pub fn publish_evidence_convoy_terminal_resolver_bindings(
    member_bindings: &[MemberEpochBinding],
    routes: &[AuthenticatedLiveRouteBinding],
    game_hour_raw: i32,
    unbound_convoy_clients: u32,
) -> Result<ResolverPublicationReceipt, String> {
    publish_convoy_terminal_resolver_bindings_with_hashes(
        member_bindings,
        routes,
        game_hour_raw,
        [0; 32],
        unbound_convoy_clients,
    )
}

fn publish_convoy_terminal_resolver_bindings_with_hashes(
    member_bindings: &[MemberEpochBinding],
    routes: &[AuthenticatedLiveRouteBinding],
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

    #[test]
    fn a_partial_publication_latches_partiality_for_the_rest_of_the_capture() {
        let publish = |unbound: u32| {
            publish_convoy_terminal_resolver_bindings_with_hashes(
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
