//! Production resolver state shared by the exact d49/d50 member walk and
//! the a729 convoy-terminal call-site observer.
//!
//! Static RE proves that there is no separate route-manager hop in this join:
//! `CFEXMember+0xD0` is resolved through the generic reference resolver and
//! vtable-gated as a concrete route producer. The exact live reader publishes
//! bindings whose raw observation is bound to the pinned exact-a729 static
//! authority. Save-derived endpoints cannot enter this registry. Runtime
//! misses remain explicit unavailable state and never synthesize a route.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use serde::Serialize;

use crate::convoy_terminal_attribution::{
    CombatEpoch, EntityHandle, ExactCombatEpochResolver, ExactLiveRouteResolver,
    LiveRouteResolution, RouteEndpointKind, RouteProducerKind, GAME_ENTITY_TYPE_TAG,
    D90_STATIC_AUTHORITY_MANIFEST_SHA256,
};
use crate::extractor::ValidatedCombatSnapshot;
use crate::member_epoch_runtime::{build_member_epoch_map, MemberEpochBinding};
use crate::convoy_route_source::ExactA729RouteRead;

const MAX_ACTIVE_ROUTE_BINDINGS: usize = 65_536;

static NEXT_PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static RESOLVER_STATE: OnceLock<RwLock<Arc<ResolverSnapshot>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedLiveRouteBinding {
    pub client: EntityHandle,
    pub resolution: LiveRouteResolution,
    /// Process-local evidence only. These values are never encoded in CTA1.
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
    pub member_bindings: u32,
    pub route_bindings: u32,
    /// Convoy members of the same clean d49/d50 walk that this publication
    /// could NOT bind: their `CFEXMember+0xD0` client handle was unreadable or
    /// structurally invalid, or their route read/authentication was refused.
    ///
    /// This is the denominator that makes a partial resolver provable. A
    /// publication with `unbound_convoy_clients != 0` still resolves every
    /// client it DID bind -- `resolve_client_route` fails closed per client and
    /// never synthesizes -- so events for bound clients are captured normally
    /// while events for unbound ones are refused and counted. It is carried
    /// onto the D90 batch witness, where it forces `exact_events_complete` to
    /// false, so a partial binding set can never be read as an exhaustive
    /// event set.
    pub unbound_convoy_clients: u32,
}

#[derive(Clone, Debug)]
pub struct ResolverSnapshot {
    receipt: ResolverPublicationReceipt,
    member_epochs: BTreeMap<usize, CombatEpoch>,
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

    pub fn route_evidence(
        &self,
        client: EntityHandle,
    ) -> Option<&AuthenticatedLiveRouteBinding> {
        self.routes.get(&client)
    }

    /// Process-local acquisition view.  The returned addresses are never
    /// encoded into CTA1/CTA2 and are exposed only to the explicitly armed
    /// H-CONVOY evidence owner while the immutable resolver epoch is live.
    pub fn evidence_member_epochs(&self) -> impl Iterator<Item = (usize, CombatEpoch)> + '_ {
        self.member_epochs.iter().map(|(address, epoch)| (*address, *epoch))
    }
}

impl ExactCombatEpochResolver for ResolverSnapshot {
    fn resolve_member_epoch(&self, member_address: usize) -> Result<CombatEpoch, String> {
        self.member_epochs
            .get(&member_address)
            .copied()
            .ok_or_else(|| "CFEXMember is absent from the same-hour d49/d50 epoch snapshot".into())
    }
}

impl ExactLiveRouteResolver for ResolverSnapshot {
    fn resolve_client_route(
        &self,
        client: EntityHandle,
    ) -> Result<LiveRouteResolution, String> {
        self.routes
            .get(&client)
            .map(|binding| binding.resolution)
            .ok_or_else(|| "client handle is absent from the authenticated live route snapshot".into())
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

/// Bind one exact route read for the isolated H-CONVOY acquisition session.
///
/// The all-zero authority field is an explicit unpromoted evidence marker;
/// it is accepted only by the evidence publication function below. Production
/// publication continues to require the immutable exact-a729 static authority.
pub fn authenticate_evidence_a729_route_read(
    read: ExactA729RouteRead,
    game_hour_raw: i32,
) -> Result<AuthenticatedLiveRouteBinding, String> {
    Ok(authenticate_a729_route_read(
        read,
        game_hour_raw,
        [0; 32],
    ))
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

/// Atomically publish the same validated active-member graph used to produce
/// d49/d50 together with an authenticated generic-reference route snapshot.
///
/// Route bindings are supplied by the exact concrete-producer reader. This
/// function deliberately accepts no saved-route fallback and no current
/// province. A callback clones one immutable `Arc`, so its member and route
/// lookups cannot straddle two publications.
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
            if member.combat_id != epoch.combat_id
                || member.generation != epoch.generation
            {
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

/// Collector adapter for the exact process-local bindings produced by the
/// clean d49/d50 walk. This avoids rebuilding lossy snapshot objects merely to
/// publish the member-pointer epoch graph.
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

/// Publish resolver custody for the isolated H-CONVOY acquisition session.
/// This does not authenticate or activate domain 90: its zero-authority
/// marker cannot pass the production publication entry point.
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
    let member_epochs: BTreeMap<usize, CombatEpoch> = build_member_epoch_map(member_bindings)?
        .into_iter()
        .map(|(member_address, resolution)| (member_address, resolution.epoch))
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

    let publication_sequence = NEXT_PUBLICATION_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if publication_sequence == 0 {
        return Err("resolver publication sequence exhausted".into());
    }
    let receipt = ResolverPublicationReceipt {
        publication_sequence,
        game_hour_raw,
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

/// Callback-safe acquisition: contention or poison is a dropped observation,
/// never a reason to block the game thread.
pub fn try_current_convoy_terminal_resolver_snapshot() -> Option<Arc<ResolverSnapshot>> {
    state()
        .try_read()
        .ok()
        .map(|snapshot| Arc::clone(&snapshot))
}

pub fn current_convoy_terminal_publication_receipt() -> Result<ResolverPublicationReceipt, String> {
    Ok(current_convoy_terminal_resolver_snapshot()?.receipt())
}

/// Teardown clears process pointers only after the call-site observer has been
/// disabled and quiesced. Queued pointer-free CTA1 events are owned separately.
pub fn clear_convoy_terminal_resolver_snapshot() -> Result<(), String> {
    *state()
        .write()
        .map_err(|_| "convoy terminal resolver state poisoned".to_string())? =
        Arc::new(ResolverSnapshot::default());
    Ok(())
}
