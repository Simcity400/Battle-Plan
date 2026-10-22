//! The runtime writers of every field the raid state tables record from
//! a state, and of a country's building-resource overrides. Observing them
//! tells the collector which states and countries to read within the hour;
//! recording never calls them.
//!
//! StateResourceInputs, StateResources, StateDynamicModifiers and StateCores
//! (cores, claims and strategic locations) record each state's additive gate
//! (`cstate::RESOURCE_MODIFIER_GATE`); effective resource accumulator values
//! (`cstate::BASE_MODIFIERS`, in modifier block `cstate::RESOURCE_MODIFIER`;
//! global factor key 0x223 and each resource's state additive, state factor
//! and modifier keys, `cstrategic_resource::STATE_ADDITIVE_KEY`,
//! `STATE_FACTOR_KEY` and `MODIFIER_KEY`); the controller's accumulator in the
//! country-specific hash (`cstate::COUNTRY_MODIFIERS`, key 0x292); resource
//! pool (`cstate::RESOURCE_POOL`); dynamic modifier list
//! (`cstate::DYNAMIC_MODIFIERS`); core list (`cstate::CORES`); claim list
//! (`cstate::CLAIMS`); infrastructure level through the building status
//! (`cstate::BUILDING_STATUS`); controller resource-origin presence; and a
//! positive maximum province-node value on the controller's supply routes.
//! CountryBuildingResources records country resources' building overrides
//! (`ccountry_resources::BUILDING_RESOURCE_OVERRIDES`).
//!
//! Writer coverage:
//! - Gate, resource accumulator and country hash: resource-gate refresh is
//!   the sole runtime writer. It clears and rebuilds the accumulator from the
//!   state's block list, including resistance and occupation blocks, and the
//!   hash from a second state field. Derived-modifier recalculation calls it
//!   unconditionally from state modifier effects, dynamic modifier effects,
//!   state category effects, occupation laws and garrisons. Other callers are
//!   controller and owner changes, controller recomputation from provinces,
//!   daily state update, building-level refresh, core insertion and
//!   construction. The two country-modifier lookup paths only read the hash.
//!   State reset clears these fields only during load, game runtime reset or
//!   four map-editor paths: an invalidating hour or nudge mode, forwarded by
//!   the building family.
//! - Dynamic modifier list: add and remove effects refresh the state through
//!   the shared effect path. Daily state update refreshes after modifier
//!   expiry erases an entry. Building modifier add and remove change the list
//!   without refresh. Dynamic value recalculation changes entry values,
//!   which no row records.
//! - Resource pool: only the resource-add effect calls state resource addition,
//!   which delegates to pool addition. The pool subtraction method removes
//!   expired timed amounts, called only by daily state update with the state
//!   pool as receiver.
//! - Cores: insertion follows two effects, country save loading and four
//!   other paths. Erasure follows two effects, released-state transfer,
//!   removing all country cores and another country path. The shared array
//!   erase reaches state cores only there; the claims eraser instead writes
//!   the claim list.
//! - Claims (`cstate::CLAIMS`): the state's claim insertion is the only appender of its
//!   list. The two claim effects and country save loading call it, and so
//!   does the country's own claim insertion, which first records the state
//!   in the country's claimed-state list and then calls it for its tag
//!   unless the state already lists it. The state's claim eraser is the only
//!   remover, called by the two claim-removal effects and the country's own
//!   claim eraser. Both take (state, &tag) and leave the list unchanged for a
//!   nonpositive tag.
//! - Strategic locations (`cstate::STRATEGIC_LOCATIONS` and each province's
//!   `cprovince::STRATEGIC_LOCATIONS`): the state's and the province's
//!   appenders are the only run-time writers. Each is called only by its
//!   state-scoped effect and takes the state or province in RCX; neither
//!   list is ever shortened. Loading reads both lists.
//! - Infrastructure: [`crate::building_mutations`] owns and forwards the
//!   building-status writers.
//! - Resource origin: controller resources' origin list
//!   (`ccountry_resources::RESOURCE_ORIGINS`) holds one origin per state,
//!   named by the origin's `cresource_origin::STATE`. Only origin
//!   construction creates it, with the state in R8. Trade-origin refresh in
//!   the hourly trade task and three other resource paths call construction.
//!   An origin leaves only through destruction, including its virtual
//!   deleting wrapper; swap-removal changes no state's presence.
//! - Supply-route presence is not observed. Hourly supply update rebuilds
//!   node values, so this flag is read with the state and certifies nothing.
//! - Country building overrides: addition follows the building-modifier
//!   effect and loader. Clear follows a resource helper, destruction, reset
//!   and subject-relation application through its resource helper.
//!   Construction and member loading are load-only.
//! - RaidAirSites records country air lists `cstrategic_air::FIRST_AIR_SITES`
//!   (air bases and carriers) and `SECOND_AIR_SITES` (rocket sites and
//!   missile ships), and each site's container capacity
//!   (`ccountry_air_container::CAPACITY_RAW`; base containers
//!   `cair_base::COUNTRY_CONTAINERS`, by canonical country slot). Their
//!   appenders follow ship lifecycle and transfer, daily state repair, access
//!   refresh, the building listener and controller changes. Air-base access
//!   refresh also appends its own first list from the diplomatic access
//!   wrapper; initial population and base-container repair run only at
//!   initialization and load completion. Withdrawal erases the first list,
//!   also from the daily pass when capacity is not positive; a separate
//!   eraser handles the second. Host transfer erases the giving country's
//!   list inline and uses appenders for the receiving country. The two clears
//!   follow dynamic reuse and destruction cleanup; construction and
//!   destruction bound list lifetime. Container creation sets default
//!   capacity through appenders and those load paths. Otherwise only the
//!   capacity setter writes it, from the daily pass and capacity
//!   recalculation. Most daily writes keep the stored value, so the site
//!   marks the country only on a difference. The third list has its own
//!   append and withdrawal paths for gun emplacements, which no row records.
//! - TradeRoutes: the state-to-state updater owns the land and general naval
//!   builders. Supply clients instead use the port-to-port updater, which writes
//!   kind, state and port endpoints and sea regions itself. Province control,
//!   ports, access and supply changes reach these two updaters. Origin refresh
//!   can also clear kind and sea-region count inline when its capital is missing;
//!   otherwise it delegates to the state-to-state updater. Constructors and the
//!   save reader bound route lifetime. Destruction removes a census reference;
//!   exchange, lend-lease and market membership changes select their owners via
//!   the freshly read route census, as do supply-client creation and removal.
//!
//! Every site takes at most four integer or pointer arguments and returns
//! RAX unchanged from its original.

pub use crate::advisor_mutations::MutationSite;

/// How a site's arguments name what it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is a state.
    State,
    /// RCX is a state's resource pool, `state + RESOURCE_POOL`.
    StatePool,
    /// R8 is the state the new resource origin names.
    OriginState,
    /// RCX is a resource origin; its `cresource_origin::STATE` names the state.
    Origin,
    /// RCX is a country's resources object; its `ccountry_resources::OWNER` raw link
    /// names the country.
    CountryResources,
    /// RCX is a country air object; its `cstrategic_air::COUNTRY` raw link names the
    /// country.
    StrategicAir,
    /// RDX points to the raw link of the country whose lists change: the
    /// constructor's country and the transfer's giving one.
    CountryTag,
    /// The lists of countries no argument names: initialization and load.
    EveryCountry,
    /// Capacity assignment (base, capacity, tag): R8 points to the raw country
    /// link whose container at RCX takes RDX.
    PoolCapacity,
    /// RCX is a province; its `cprovince::STATE` names the state.
    Province,
    /// RCX is a trade route; its sender link (`resource_map::ROUTE_LINKS[0]`) names the
    /// country holding it.
    Route,
    /// RCX is an origin; its client country names its own or rights-holder list owner.
    OriginRoute,
    /// RDX points to the raw link of the country that sends every route being built: a
    /// route's sender, or a new country resources object's country, whose delivery routes
    /// it builds.
    RouteSender,
    /// Every country's routes: the save loader.
    EveryRoute,
}

/// The raid state sites, site list sites, claim sites, strategic-location sites, then route sites.
pub const MUTATIONS: [(MutationSite, Receiver); 35] = [
    // Refresh resource gates (state).
    (
        MutationSite {
            name: "refresh_resource_modifier_gates",
            rva: 0x9d_3470,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::State,
    ),
    // Building dynamic modifier addition (state, ...): select the dynamic modifier list,
    // set R9D to -1 and tail-jump to the list writer.
    (
        MutationSite {
            name: "add_building_dynamic_modifier",
            rva: 0x9c_49c0,
            sentinel: [
                0x48, 0x81, 0xc1, 0x88, 0x07, 0x00, 0x00, 0x41, 0xb9, 0xff, 0xff, 0xff, 0xff, 0xe9,
                0x9e, 0x61,
            ],
        },
        Receiver::State,
    ),
    // Building dynamic modifier removal (state, ...): select the dynamic modifier list
    // and tail-jump to the list writer.
    (
        MutationSite {
            name: "remove_building_dynamic_modifier",
            rva: 0x9d_0690,
            sentinel: [
                0x48, 0x81, 0xc1, 0x88, 0x07, 0x00, 0x00, 0xe9, 0xc4, 0x45, 0xc3, 0xff, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::State,
    ),
    // Resource addition (state, amount, days).
    (
        MutationSite {
            name: "add_resource",
            rva: 0x9c_4300,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x41, 0x56, 0x48, 0x83,
                0xec, 0x20,
            ],
        },
        Receiver::State,
    ),
    // Pool subtraction (state resource pool, amount) removes an expired timed amount.
    (
        MutationSite {
            name: "expire_timed_resource",
            rva: 0xbb_c100,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::StatePool,
    ),
    // State destruction (state).
    (
        MutationSite {
            name: "destroy_state",
            rva: 0x9c_3460,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::State,
    ),
    // Core insertion (state, tag): save RBX and reserve 0x30 stack bytes.
    (
        MutationSite {
            name: "add_core",
            rva: 0x9c_4810,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x83, 0x3a, 0x00, 0x48, 0x8b, 0xd9, 0x48, 0x89,
                0x7c, 0x24,
            ],
        },
        Receiver::State,
    ),
    // Core erasure (state, tag).
    (
        MutationSite {
            name: "remove_core",
            rva: 0x9d_0600,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x83, 0x3a, 0x00, 0x48,
                0x8b, 0xfa,
            ],
        },
        Receiver::State,
    ),
    // Origin construction (origin, tag, state).
    (
        MutationSite {
            name: "construct_resource_origin",
            rva: 0xc9_5a80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x57,
            ],
        },
        Receiver::OriginState,
    ),
    // Origin destruction (origin).
    (
        MutationSite {
            name: "destroy_resource_origin",
            rva: 0xc9_6490,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xef,
                0xf6, 0xca,
            ],
        },
        Receiver::Origin,
    ),
    // Building-resource modifier addition (resources, building, resource, amount).
    (
        MutationSite {
            name: "add_building_resource_modifier",
            rva: 0xc9_6de0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::CountryResources,
    ),
    // Clear country resource overrides (resources).
    (
        MutationSite {
            name: "clear_country_resources",
            rva: 0xc9_b830,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::CountryResources,
    ),
    // Country air construction (air, tag, manager).
    (
        MutationSite {
            name: "construct_strategic_air",
            rva: 0xc3_8740,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::CountryTag,
    ),
    // Ensure an air-base country container (air, base): the first site list.
    (
        MutationSite {
            name: "append_air_site",
            rva: 0xc3_b940,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
                0x10, 0x57,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Ensure a rocket-site country container (air, base): the second site list.
    (
        MutationSite {
            name: "append_missile_site",
            rva: 0xc3_c720,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Air-base access refresh (air, countries): its own first site list.
    (
        MutationSite {
            name: "refresh_air_base_access",
            rva: 0xc5_55a0,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x53, 0x10, 0x53, 0x56, 0x48, 0x83, 0xec, 0x58, 0x48,
                0x63, 0x42,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Initial site population (manager).
    (
        MutationSite {
            name: "populate_air_sites",
            rva: 0xc4_b6a0,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x41, 0x57, 0x48, 0x83, 0xec, 0x60, 0x48, 0x63, 0x41, 0x3c, 0x4c,
                0x8b, 0xf9,
            ],
        },
        Receiver::EveryCountry,
    ),
    // Base-container repair (base), at load completion.
    (
        MutationSite {
            name: "repair_air_base_containers",
            rva: 0xc4_9c40,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83,
                0xec, 0x40,
            ],
        },
        Receiver::EveryCountry,
    ),
    // Air-base withdrawal (air, base): the first site list.
    (
        MutationSite {
            name: "withdraw_air_site",
            rva: 0xc5_3620,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x53, 0x10, 0x49, 0x89, 0x4b, 0x08, 0x53, 0x55, 0x48,
                0x83, 0xec,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Rocket-site withdrawal (air, base): the second site list.
    (
        MutationSite {
            name: "withdraw_missile_site",
            rva: 0xc5_4130,
            sentinel: [
                0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Host transfer (manager, from, to, base): the giving country.
    (
        MutationSite {
            name: "transfer_air_base",
            rva: 0xc5_4cc0,
            sentinel: [
                0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
                0x8b, 0xec,
            ],
        },
        Receiver::CountryTag,
    ),
    // Clear air-base sites (air): the first site list.
    (
        MutationSite {
            name: "clear_air_sites",
            rva: 0xc3_fa60,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x08, 0x49, 0x89, 0x6b, 0x10, 0x49, 0x89, 0x73,
                0x18, 0x49,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Clear rocket sites (air): the second site list.
    (
        MutationSite {
            name: "clear_missile_sites",
            rva: 0xc3_fcc0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Country air destruction (air).
    (
        MutationSite {
            name: "destroy_strategic_air",
            rva: 0xc3_98b0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::StrategicAir,
    ),
    // Country-pool capacity assignment (base, capacity, tag).
    (
        MutationSite {
            name: "set_country_pool_capacity",
            rva: 0xc5_4860,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41,
                0x8b, 0x00,
            ],
        },
        Receiver::PoolCapacity,
    ),
    // Claim insertion (state, tag).
    (
        MutationSite {
            name: "add_claim",
            rva: 0x9c_46c0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x83, 0x3a, 0x00, 0x48,
                0x8b, 0xfa,
            ],
        },
        Receiver::State,
    ),
    // Claim erasure (state, tag).
    (
        MutationSite {
            name: "remove_claim",
            rva: 0x9d_0460,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x83, 0x3a, 0x00, 0x48,
                0x8b, 0xfa,
            ],
        },
        Receiver::State,
    ),
    // State strategic-location append (state, location, anchor province).
    (
        MutationSite {
            name: "add_state_strategic_location",
            rva: 0x9c_4a60,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30,
                0x8b, 0x5a,
            ],
        },
        Receiver::State,
    ),
    // Province strategic-location append (province, location).
    (
        MutationSite {
            name: "add_province_strategic_location",
            rva: 0xe6_5050,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x41, 0x56, 0x48, 0x83,
                0xec, 0x30,
            ],
        },
        Receiver::Province,
    ),
    // State-to-state route update (route, from, to, force).
    (
        MutationSite {
            name: "update_trade_route",
            rva: 0xca_d170,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x55, 0x56, 0x57,
                0x41, 0x54,
            ],
        },
        Receiver::Route,
    ),
    // Route construction (route, &sender, &receiver): an empty route of no kind.
    (
        MutationSite {
            name: "construct_trade_route",
            rva: 0xc9_58c0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x33, 0xff, 0xc6, 0x41,
                0x08, 0x00,
            ],
        },
        Receiver::RouteSender,
    ),
    // Country resources construction (resources, &country): builds the country's delivery
    // routes in place, one per importer slot.
    (
        MutationSite {
            name: "construct_country_resources",
            rva: 0xc9_5280,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
                0x54, 0x41,
            ],
        },
        Receiver::RouteSender,
    ),
    // Route save loading (route, reader, token), once per saved field.
    (
        MutationSite {
            name: "load_trade_route",
            rva: 0xca_8b50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::EveryRoute,
    ),
    // Port-to-port supply route update (route, from province, to province).
    (
        MutationSite {
            name: "update_supply_trade_route",
            rva: 0xca_d6b0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Route,
    ),
    // Origin refresh (origin, force): a missing capital clears kind inline.
    (
        MutationSite {
            name: "refresh_origin_trade_route",
            rva: 0xc9_a950,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::OriginRoute,
    ),
];

/// The [`MUTATIONS`] index of the first trade-route writer, after the claim writers.
pub const FIRST_ROUTE_SITE: usize = 29;
/// Origin refresh requires a before/after kind check around its original.
pub const ORIGIN_ROUTE_SITE: usize = 34;
/// An origin's client country owns its census rows, independently of its route's sender.
pub const ORIGIN_OWNER: usize = 0x18;

/// The origin refresh's only inline change visible to TradeRoutes is its kind.
/// Its other inline store clears sea-region count at kind zero, which reads the
/// land path. Every path rebuild delegates to the hooked route updater.
pub fn origin_route_kind(memory: &impl crate::Memory, origin: usize) -> Option<u8> {
    crate::read(
        memory,
        origin
            .checked_add(crate::cresource_origin::DELIVERY_ROUTE)?
            .checked_add(crate::resource_map::ROUTE_KIND)?,
    )
}

/// The raw link of the country holding the route a route writer names: the route's sender
/// for [`Receiver::Route`], the origin's client country for [`Receiver::OriginRoute`],
/// or the link RDX points to for [`Receiver::RouteSender`]. The observer forwards
/// an origin's nested route updates through its origin receiver too.
/// `Ok(None)` for link zero, which no country holds; `Err` when it does not read.
pub fn route_country(
    memory: &impl crate::Memory,
    receiver: Receiver,
    first: usize,
    second: usize,
) -> Result<Option<u32>, Unreadable> {
    let link = match receiver {
        Receiver::Route => first.checked_add(crate::resource_map::ROUTE_LINKS[0]),
        Receiver::OriginRoute => first.checked_add(ORIGIN_OWNER),
        Receiver::RouteSender => Some(second),
        _ => None,
    };
    link.and_then(|address| crate::read::<u32>(memory, address))
        .map(|raw| (raw != 0).then_some(raw))
        .ok_or(Unreadable)
}

/// Daily base-capacity pass (air, pairs), calling the capacity setter once
/// per base/country pair. Only strategic-air daily update calls it, in the
/// serial country loop after parallel stages join. The capacity setter keeps
/// its own patch for the other capacity-recalculation paths.
pub const DAILY_CAPACITIES: MutationSite = MutationSite {
    name: "daily_refresh_base_capacities",
    rva: 0xc3_ca10,
    sentinel: [
        0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x58,
        0x45,
    ],
};

const DAILY_BASES: usize = 0x10;
const DAILY_BASE_COUNT: usize = 0x1c;
const CAPACITY_PAIR_VECTOR_STRIDE: usize = 0x18;
const CAPACITY_PAIR_COUNT: usize = 0xc;
const CAPACITY_PAIR_VALUE: usize = 8;

/// Count the daily base-capacity pass's stores when each keeps the capacity
/// [`pool_capacity`] reads and no base is withdrawn. Return `None` otherwise
/// or when any required word does not read.
///
/// The pass walks the air object's base vector (`DAILY_BASES`), bounded by
/// its count (`DAILY_BASE_COUNT`), and the corresponding vector from the
/// pairs pointer. Each 0x18-byte vector has a data pointer and count
/// (`CAPACITY_PAIR_COUNT`), with 16-byte entries holding a raw link and
/// then the capacity (`CAPACITY_PAIR_VALUE`); the setter takes base,
/// capacity and entry. The game rereads these controls each round; this
/// reader takes them once, valid only when the pass changes none. Storing
/// an existing capacity neither evicts (which needs new capacity below one
/// and old above zero) nor writes anything else. After each base's stores,
/// the pass withdraws this air object from a state base if its own
/// container (at the canonical slot of the air object's
/// `cstrategic_air::COUNTRY`) is missing or below one. Withdrawal removes
/// the base from the base vector and destroys containers. Without
/// withdrawal and with unchanged stores, no country table, link, container
/// vector or capacity changes; these pre-pass reads therefore equal each
/// setter call's reads.
pub fn daily_unchanged(
    memory: &impl crate::Memory,
    module: usize,
    air: usize,
    pairs: usize,
) -> Option<u64> {
    use crate::{cpdx_array::Array, read};
    let game = read::<u64>(
        memory,
        module.checked_add(crate::cgame_state::SINGLETON_RVA)?,
    )? as usize;
    let links = Array::read(
        memory,
        game.checked_add(crate::cgame_state::COUNTRY_LINKS)?,
        4,
    )?;
    // The capacity [`pool_capacity`] reads for raw link `raw` in `containers`.
    let held = |containers: Array, raw: u32| -> Option<i64> {
        let slot = if raw == 0 {
            0
        } else {
            read::<u32>(memory, links.element(raw, 4)?)?
        };
        let container = read::<u64>(memory, containers.element(slot, 8)?)? as usize;
        if container == 0 {
            return None;
        }
        read::<i64>(
            memory,
            container.checked_add(crate::ccountry_air_container::CAPACITY_RAW)?,
        )
    };
    let own = read::<u32>(memory, air.checked_add(crate::cstrategic_air::COUNTRY)?)?;
    let bases = read::<u64>(memory, air.checked_add(DAILY_BASES)?)? as usize;
    let count = read::<i32>(memory, air.checked_add(DAILY_BASE_COUNT)?)?.max(0) as usize;
    let vectors = read::<u64>(memory, pairs)? as usize;
    let mut stores = 0;
    for index in 0..count {
        let base = read::<u64>(memory, bases.checked_add(index.checked_mul(8)?)?)? as usize;
        let containers = Array::read(
            memory,
            base.checked_add(crate::cair_base::COUNTRY_CONTAINERS)?,
            8,
        )?;
        let vector = vectors.checked_add(index.checked_mul(CAPACITY_PAIR_VECTOR_STRIDE)?)?;
        let data = read::<u64>(memory, vector)? as usize;
        let entries = usize::try_from(read::<i32>(
            memory,
            vector.checked_add(CAPACITY_PAIR_COUNT)?,
        )?)
        .ok()?;
        for entry in 0..entries {
            let pair = read::<[u8; 16]>(memory, data.checked_add(entry.checked_mul(16)?)?)?;
            let raw = u32::from_le_bytes(pair[..4].try_into().unwrap());
            let capacity = i64::from_le_bytes(pair[CAPACITY_PAIR_VALUE..].try_into().unwrap());
            if held(containers, raw)? != capacity {
                return None;
            }
            stores += 1;
        }
        if read::<u64>(memory, base.checked_add(crate::cair_base::STATE)?)? != 0
            && held(containers, own).is_none_or(|capacity| capacity < 1)
        {
            return None;
        }
    }
    Some(stores)
}

/// A state id or pointer that did not read, or read zero, which no state has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreadable;

/// The state `receiver` names, given a site's first and third arguments:
/// its guarded id `cstate::ID`. `Ok(None)` for an origin without a state,
/// which no row reads; `Err` when an id or pointer does not read, or reads
/// zero, which no state has.
pub fn state(
    memory: &impl crate::Memory,
    receiver: Receiver,
    first: usize,
    third: usize,
) -> Result<Option<u32>, Unreadable> {
    let id = |state: usize| {
        state
            .checked_add(crate::cstate::ID)
            .and_then(|address| crate::read::<u32>(memory, address))
            .filter(|&id| id != 0)
            .map(Some)
            .ok_or(Unreadable)
    };
    match receiver {
        Receiver::State => id(first),
        Receiver::StatePool => id(first
            .checked_sub(crate::cstate::RESOURCE_POOL)
            .ok_or(Unreadable)?),
        Receiver::OriginState if third == 0 => Ok(None),
        Receiver::OriginState => id(third),
        Receiver::Origin => {
            let state = first
                .checked_add(crate::cresource_origin::STATE)
                .and_then(|address| crate::read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or(Unreadable)?;
            if state == 0 { Ok(None) } else { id(state) }
        }
        Receiver::Province => {
            let state = first
                .checked_add(crate::cprovince::STATE)
                .and_then(|address| crate::read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or(Unreadable)?;
            if state == 0 { Ok(None) } else { id(state) }
        }
        _ => Err(Unreadable),
    }
}

/// The raw country link a country's resources receiver names
/// (`ccountry_resources::OWNER`);
/// `None` when it does not read or reads zero.
pub fn country(memory: &impl crate::Memory, resources: usize) -> Option<u32> {
    resources
        .checked_add(crate::ccountry_resources::OWNER)
        .and_then(|address| crate::read::<u32>(memory, address))
        .filter(|&raw| raw != 0)
}

/// The [`MUTATIONS`] index of the first site list writer.
#[cfg(test)]
pub const FIRST_SITE: usize = 12;
/// The [`MUTATIONS`] index of the first claim writer, after the site list
/// writers.
#[cfg(test)]
pub const FIRST_CLAIM_SITE: usize = 25;
/// The [`MUTATIONS`] index of the first strategic-location writer, after the
/// claim writers.
#[cfg(test)]
pub const FIRST_STRATEGIC_LOCATION_SITE: usize = 27;

/// The raw country link a site list receiver names, given a site's first
/// three arguments: a country air object's `cstrategic_air::COUNTRY` for
/// [`Receiver::StrategicAir`], the link RDX points to for
/// [`Receiver::CountryTag`] and the link R8 points to for
/// [`Receiver::PoolCapacity`]. `Ok(None)` for link zero, which no country
/// holds; `Err` when it does not read.
pub fn site_country(
    memory: &impl crate::Memory,
    receiver: Receiver,
    first: usize,
    second: usize,
    third: usize,
) -> Result<Option<u32>, Unreadable> {
    let link = match receiver {
        Receiver::StrategicAir => first.checked_add(crate::cstrategic_air::COUNTRY),
        Receiver::CountryTag => Some(second),
        Receiver::PoolCapacity => Some(third),
        _ => None,
    };
    link.and_then(|address| crate::read::<u32>(memory, address))
        .map(|raw| (raw != 0).then_some(raw))
        .ok_or(Unreadable)
}

/// The capacity overwritten by the country-pool setter: the base's container
/// at `slot` in `cair_base::COUNTRY_CONTAINERS`, its
/// `ccountry_air_container::CAPACITY_RAW`. The tag's raw link resolves through
/// the game state link table (`cgame_state::COUNTRY_LINKS`) to its canonical
/// slot; link zero uses slot zero.
/// Return `None` without a container or when a required word does not read.
pub fn pool_capacity(
    memory: &impl crate::Memory,
    module: usize,
    base: usize,
    tag: usize,
) -> Option<i64> {
    let raw = crate::read::<u32>(memory, tag)?;
    let slot = if raw == 0 {
        0
    } else {
        let game = crate::read::<u64>(
            memory,
            module.checked_add(crate::cgame_state::SINGLETON_RVA)?,
        )?;
        let links = crate::cpdx_array::Array::read(
            memory,
            usize::try_from(game)
                .ok()?
                .checked_add(crate::cgame_state::COUNTRY_LINKS)?,
            4,
        )?;
        crate::read::<u32>(memory, links.element(raw, 4)?)?
    };
    let containers = crate::cpdx_array::Array::read(
        memory,
        base.checked_add(crate::cair_base::COUNTRY_CONTAINERS)?,
        8,
    )?;
    let container = crate::read::<u64>(memory, containers.element(slot, 8)?)?;
    if container == 0 {
        return None;
    }
    crate::read::<i64>(
        memory,
        usize::try_from(container)
            .ok()?
            .checked_add(crate::ccountry_air_container::CAPACITY_RAW)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    /// A daily pass over two bases (a host base, then a state base) with two
    /// (country, capacity) pairs each, every capacity already stored.
    struct Daily(Vec<u8>);
    impl Daily {
        const AT: usize = 0x1000_0000;
        const GAME: usize = Self::AT + 0x100;
        const LINKS: usize = Self::AT + 0x800;
        const AIR: usize = Self::AT + 0x1000;
        const BASES: usize = Self::AT + 0x1100;
        const PAIRS: usize = Self::AT + 0x5000;
        const VECTORS: usize = Self::AT + 0x5100;
        fn base(index: usize) -> usize {
            Self::AT + 0x2000 + index * 0x1000
        }
        fn container(base: usize, slot: usize) -> usize {
            Self::AT + 0x8000 + base * 0x400 + slot * 0x100
        }
        fn entries(base: usize) -> usize {
            Self::AT + 0x6000 + base * 0x100
        }
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.0[address - Self::AT..][..bytes.len()].copy_from_slice(bytes);
        }
        fn word(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.word(address, data as u64);
            self.put(address + 8, &count.to_le_bytes());
            self.put(address + 12, &count.to_le_bytes());
        }
        fn capacity(&mut self, base: usize, slot: usize, value: i64) {
            self.put(
                Self::container(base, slot) + crate::ccountry_air_container::CAPACITY_RAW,
                &value.to_le_bytes(),
            );
        }
        fn new() -> Self {
            let mut f = Self(vec![0; 0xa000]);
            f.word(Self::AT, Self::GAME as u64);
            f.array(
                Self::GAME + crate::cgame_state::COUNTRY_LINKS,
                Self::LINKS,
                4,
            );
            for raw in 1..4_u32 {
                f.put(Self::LINKS + raw as usize * 4, &raw.to_le_bytes());
            }
            f.array(Self::AIR + 0x10, Self::BASES, 2);
            f.put(
                Self::AIR + crate::cstrategic_air::COUNTRY,
                &1_u32.to_le_bytes(),
            );
            f.word(Self::PAIRS, Self::VECTORS as u64);
            for index in 0..2 {
                let base = Self::base(index);
                f.word(Self::BASES + index * 8, base as u64);
                let containers = base + 0x100;
                f.array(base + crate::cair_base::COUNTRY_CONTAINERS, containers, 4);
                for slot in 1..4 {
                    f.word(containers + slot * 8, Self::container(index, slot) as u64);
                    f.capacity(index, slot, 100_000);
                }
                f.array(Self::VECTORS + index * 0x18, Self::entries(index), 2);
                for (entry, raw) in [2_u32, 3].into_iter().enumerate() {
                    let at = Self::entries(index) + entry * 16;
                    f.put(at, &raw.to_le_bytes());
                    f.put(at + 8, &100_000_i64.to_le_bytes());
                }
            }
            f.word(Self::base(1) + crate::cair_base::STATE, 0x77);
            f
        }
        fn unchanged(&self) -> Option<u64> {
            let memory = SliceMemory {
                base: Self::AT,
                bytes: &self.0,
            };
            daily_unchanged(
                &memory,
                Self::AT - crate::cgame_state::SINGLETON_RVA,
                Self::AIR,
                Self::PAIRS,
            )
        }
    }

    /// A pass whose every store keeps its capacity and that withdraws no
    /// base counts its stores; one changing store, a missing container, a
    /// state base about to be withdrawn or a word that does not read
    /// certifies nothing, so the setter's own path decides.
    #[test]
    fn a_daily_pass_is_certified_only_when_every_store_is_unchanged() {
        assert_eq!(Daily::new().unchanged(), Some(4));
        let mut changed = Daily::new();
        changed.capacity(1, 3, 99_999);
        assert_eq!(changed.unchanged(), None);
        let mut missing = Daily::new();
        missing.word(Daily::base(0) + 0x100 + 2 * 8, 0);
        assert_eq!(missing.unchanged(), None);
        // The state base's own container (raw 1) below one: withdrawn.
        let mut withdrawn = Daily::new();
        withdrawn.capacity(1, 1, 0);
        assert_eq!(withdrawn.unchanged(), None);
        // A host base keeps this air whatever its own capacity.
        let mut host = Daily::new();
        host.capacity(0, 1, 0);
        assert_eq!(host.unchanged(), Some(4));
        let mut torn = Daily::new();
        torn.word(Daily::VECTORS + 0x18, 0x7000_0000);
        assert_eq!(torn.unchanged(), None);
    }

    /// One patch owner per native address and name.
    #[test]
    fn sites_are_distinct_and_named_once() {
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            for (other, _) in &MUTATIONS[index + 1..] {
                assert_ne!(site.rva, other.rva, "{}", site.name);
                assert_ne!(site.name, other.name);
            }
        }
    }

    /// A route writer names its sender, an origin its list owner, and constructors
    /// the country link they receive; link zero names nothing.
    #[test]
    fn route_writers_name_the_country_holding_the_route() {
        let base = 0x1000;
        let mut bytes = vec![0_u8; 0x800];
        let route = 0x100;
        bytes[route + crate::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&9_u32.to_le_bytes());
        bytes[route + crate::resource_map::ROUTE_LINKS[1]..][..4]
            .copy_from_slice(&5_u32.to_le_bytes());
        let link = 0x300;
        bytes[link..][..4].copy_from_slice(&7_u32.to_le_bytes());
        let origin = 0x400;
        bytes[origin + ORIGIN_OWNER..][..4].copy_from_slice(&11_u32.to_le_bytes());
        bytes[origin
            + crate::cresource_origin::DELIVERY_ROUTE
            + crate::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&9_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        assert_eq!(
            route_country(&memory, Receiver::Route, base + route, 0),
            Ok(Some(9))
        );
        assert_eq!(
            route_country(&memory, Receiver::RouteSender, 0, base + link),
            Ok(Some(7))
        );
        assert_eq!(
            route_country(&memory, Receiver::RouteSender, 0, base + 0x380),
            Ok(None)
        );
        assert_eq!(
            route_country(&memory, Receiver::Route, 0x10, 0),
            Err(Unreadable)
        );
        assert_eq!(
            route_country(&memory, Receiver::EveryRoute, base + route, base + link),
            Err(Unreadable)
        );
        assert_eq!(
            route_country(&memory, Receiver::OriginRoute, base + origin, 0),
            Ok(Some(11))
        );
        assert_eq!(origin_route_kind(&memory, base + origin), Some(0));
        assert_eq!(origin_route_kind(&memory, usize::MAX), None);
        for (site, receiver) in &MUTATIONS[FIRST_ROUTE_SITE..] {
            assert!(
                matches!(
                    receiver,
                    Receiver::Route
                        | Receiver::RouteSender
                        | Receiver::OriginRoute
                        | Receiver::EveryRoute
                ),
                "{}",
                site.name
            );
        }
    }

    /// A state site names its state through `cstate::ID`, the pool method
    /// through the pool's offset in its state, the origin constructor through
    /// `r8` and the destructor through `cresource_origin::STATE`; a country site
    /// through `ccountry_resources::OWNER`.
    #[test]
    fn receivers_name_their_state_or_country() {
        let base = 0x1000;
        let mut bytes = vec![0_u8; 0x2000];
        let state = 0x400;
        bytes[state + crate::cstate::ID..][..4].copy_from_slice(&42_u32.to_le_bytes());
        let origin = 0x100;
        bytes[origin + crate::cresource_origin::STATE..][..8]
            .copy_from_slice(&((base + state) as u64).to_le_bytes());
        let resources = 0x1800;
        bytes[resources + crate::ccountry_resources::OWNER..][..4]
            .copy_from_slice(&7_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        let at = |offset: usize| base + offset;
        assert_eq!(
            super::state(&memory, Receiver::State, at(state), 0),
            Ok(Some(42))
        );
        assert_eq!(
            super::state(
                &memory,
                Receiver::StatePool,
                at(state) + crate::cstate::RESOURCE_POOL,
                0
            ),
            Ok(Some(42))
        );
        assert_eq!(
            super::state(&memory, Receiver::OriginState, 0, at(state)),
            Ok(Some(42))
        );
        assert_eq!(super::state(&memory, Receiver::OriginState, 0, 0), Ok(None));
        assert_eq!(
            super::state(&memory, Receiver::Origin, at(origin), 0),
            Ok(Some(42))
        );
        // An origin without a state names none; an unreadable one is an error.
        assert_eq!(
            super::state(&memory, Receiver::Origin, at(0x200), 0),
            Ok(None)
        );
        assert_eq!(
            super::state(&memory, Receiver::Origin, 0x10, 0),
            Err(Unreadable)
        );
        // A zero id names no state.
        assert_eq!(
            super::state(&memory, Receiver::State, at(0x600), 0),
            Err(Unreadable)
        );
        assert_eq!(super::country(&memory, at(resources)), Some(7));
        assert_eq!(super::country(&memory, at(0x600)), None);
    }

    /// A site list writer names its country through the strategic air object's
    /// `cstrategic_air::COUNTRY`, or the link RDX or R8 points to; link zero names
    /// none. The site list writers follow the state and override sites.
    #[test]
    fn site_list_receivers_name_their_country() {
        let base = 0x1000;
        let mut bytes = vec![0_u8; 0x400];
        bytes[0x100 + crate::cstrategic_air::COUNTRY..][..4].copy_from_slice(&9_u32.to_le_bytes());
        bytes[0x300..0x304].copy_from_slice(&5_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        let at = |offset: usize| base + offset;
        let name = |receiver, first, second, third| {
            super::site_country(&memory, receiver, first, second, third)
        };
        assert_eq!(name(Receiver::StrategicAir, at(0x100), 0, 0), Ok(Some(9)));
        assert_eq!(name(Receiver::CountryTag, 0, at(0x300), 0), Ok(Some(5)));
        assert_eq!(name(Receiver::PoolCapacity, 0, 0, at(0x300)), Ok(Some(5)));
        assert_eq!(name(Receiver::StrategicAir, at(0x200), 0, 0), Ok(None));
        assert_eq!(name(Receiver::CountryTag, 0, 0x10, 0), Err(Unreadable));
        assert!(MUTATIONS[..FIRST_SITE].iter().all(|(_, receiver)| matches!(
            receiver,
            Receiver::State
                | Receiver::StatePool
                | Receiver::OriginState
                | Receiver::Origin
                | Receiver::CountryResources
        )));
        assert!(
            MUTATIONS[FIRST_SITE..FIRST_CLAIM_SITE]
                .iter()
                .all(|(_, receiver)| matches!(
                    receiver,
                    Receiver::StrategicAir
                        | Receiver::CountryTag
                        | Receiver::EveryCountry
                        | Receiver::PoolCapacity
                ))
        );
        // The claim writers name their state, as the core writers do.
        assert!(
            MUTATIONS[FIRST_CLAIM_SITE..FIRST_STRATEGIC_LOCATION_SITE]
                .iter()
                .all(|(_, receiver)| *receiver == Receiver::State)
        );
        // The strategic-location writers name the state, directly or
        // through the province.
        assert_eq!(
            MUTATIONS[FIRST_STRATEGIC_LOCATION_SITE..FIRST_ROUTE_SITE]
                .iter()
                .map(|(_, receiver)| *receiver)
                .collect::<Vec<_>>(),
            [Receiver::State, Receiver::Province]
        );
    }

    /// The capacity the setter overwrites: the container at the tag's
    /// canonical slot, through the game state's link table; none without a
    /// container or a readable tag.
    #[test]
    fn the_pool_capacity_is_the_tag_slots_container_word() {
        let module = 0x1000;
        let base = module + crate::cgame_state::SINGLETON_RVA;
        let mut bytes = vec![0_u8; 0x2000];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        let array = |data: usize, count: u32| {
            let mut header = [0_u8; 16];
            header[..8].copy_from_slice(&((base + data) as u64).to_le_bytes());
            header[8..12].copy_from_slice(&count.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
            header
        };
        // The singleton names the game state at 0x100; its links map raw 7
        // to slot 2.
        put(0, &((base + 0x100) as u64).to_le_bytes());
        put(0x100 + crate::cgame_state::COUNTRY_LINKS, &array(0x800, 8));
        put(0x800 + 7 * 4, &2_u32.to_le_bytes());
        // The base's containers: slot 2 holds one with capacity 50000.
        put(
            0x1000 + crate::cair_base::COUNTRY_CONTAINERS,
            &array(0x1100, 4),
        );
        put(0x1100 + 2 * 8, &((base + 0x1200) as u64).to_le_bytes());
        put(
            0x1200 + crate::ccountry_air_container::CAPACITY_RAW,
            &50_000_i64.to_le_bytes(),
        );
        // Tags: raw 7, raw 3 (slot 0, no container), and an unreadable one.
        put(0x1800, &7_u32.to_le_bytes());
        put(0x1804, &3_u32.to_le_bytes());
        let memory = SliceMemory {
            base,
            bytes: &bytes,
        };
        let capacity = |tag| super::pool_capacity(&memory, module, base + 0x1000, tag);
        assert_eq!(capacity(base + 0x1800), Some(50_000));
        assert_eq!(capacity(base + 0x1804), None);
        assert_eq!(capacity(0x10), None);
    }
}
