//! flying-aircraft map, for hash-pinned a729. Observing them tells the
//! collector which countries to re-read; recording never calls them.
//!
//! - Enemies (`CDiplomacyStatus+0x98`) and wartime allies (`+0xB0`):
//!   `RebuildWartimeAllies@140D31E20` rebuilds both. Its callers are war
//!   creation (`CreatePairWarRelation@140D20680`), the join-war helper
//!   `140D26940` (which also rebuilds every target's enemies),
//!   `EndWarRelation@140D08520` (peace conference `140E2D670`, civil war
//!   `1410CB7E0`, subject apply `140D133E0`, `CRelationStatus::Reset`), the
//!   daily capitulation helper `140D261C0`, `140D27A10`,
//!   `CCountry::RefreshDerivedStateByFlags@1406F2680` bit 7 (faction
//!   `AddMember@140D75210`, hot join, release and 26 others) and
//!   `CCountry::construct_operation_manager@1406C73C0`.
//!   `CDiplomacyStatus::Reset@140D25970` clears both, from the constructor
//!   path, `ResetCountryAppearance@1406D61D0`, `140D133E0` and the
//!   destructor `140D1EE00`. No other runtime writer exists.
//! - Reciprocal donors (`CCountry+0x1410`):
//!   `CCountry::AddReciprocalCountryLink@1406C48D0` and
//!   `RemoveReciprocalCountryLink@140701C60`. The other users
//!   (`1406CA310`, `1406F0BA0` from `PreHourlyUpdate`,
//!   `ResetCountryAppearance`) remove through the latter; `1406F0440`,
//!   `140C3C560` and `140C43B30` only read. `ReadMember@1406F9170` fills it at
//!   load, which the collector treats as a discontinuity.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Receivers: a CDiplomacyStatus for the first two, a CCountry for the rest.
pub const MUTATIONS: [MutationSite; 4] = [
    MutationSite {
        name: "rebuild_wartime_allies",
        rva: 0xd31e20,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
            0x56, 0x41,
        ],
    },
    MutationSite {
        name: "diplomacy_reset",
        rva: 0xd25970,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x55,
        ],
    },
    MutationSite {
        name: "add_reciprocal_link",
        rva: 0x6c48d0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
            0x20, 0x55,
        ],
    },
    MutationSite {
        name: "remove_reciprocal_link",
        rva: 0x701c60,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
            0x18, 0x55,
        ],
    },
];
