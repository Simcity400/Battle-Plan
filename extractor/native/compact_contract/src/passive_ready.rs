//! Canonical digest preimages for passive CaptureReadyV2 evidence.
//!
//! Collector and recorder must use these exact encoders. Local JSON strings,
//! debug formatting, pointer values, and platform-native struct layouts are
//! never digest authority.

use crate::calendar::hour_index_from_raw;
use sha2::{Digest, Sha256};

pub const PASSIVE_READY_DIGEST_VERSION: u16 = 1;
pub const HOOK_IDENTITY_DIGEST_LABEL: &[u8] = b"battle-plan.capture_ready.hook_identity.v1";
pub const HOOK_TARGET_NAME_DIGEST_LABEL: &[u8] = b"battle-plan.capture_ready.hook_target_name.v1";
pub const HOOK_PRIMITIVE_ID_DIGEST_LABEL: &[u8] =
    b"battle-plan.capture_ready.hook_primitive_id.v1";
pub const HOOK_ALLOWLIST_DIGEST_LABEL: &[u8] = b"battle-plan.capture_ready.hook_allowlist.v1";
pub const HOOK_OUTCOMES_DIGEST_LABEL: &[u8] = b"battle-plan.capture_ready.hook_outcomes.v1";
pub const SCHEDULER_CONFIG_DIGEST_LABEL: &[u8] = b"battle-plan.capture_ready.scheduler_config.v1";
pub const READ_ONLY_CLOCK_WITNESS_DIGEST_LABEL: &[u8] =
    b"battle-plan.capture_ready.read_only_clock_witness.v1";
pub const OUTPUT_PIPE_IDENTITY_DIGEST_LABEL: &[u8] =
    b"battle-plan.capture_ready.output_pipe_identity.v1";
pub const ZERO_MUTATION_POLICY_LABEL: &[u8] =
    b"battle-plan.capture_ready.zero_mutation_policy.v1";
pub const ZERO_MUTATION_POLICY_VERSION: u16 = 1;
pub const PINNED_A729_TARGET_EXECUTABLE_SHA256: [u8; 32] =
    digest_from_hex("870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");

const MAX_HOOK_NAME_LEN: usize = 96;
const MAX_PIPE_NAME_LEN: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookTargetV1 {
    ImportExport {
        module_name_sha256: [u8; 32],
        symbol_name_sha256: [u8; 32],
        patch_len: u16,
        primitive_id_sha256: [u8; 32],
    },
    Hoi4Rva {
        module_name_sha256: [u8; 32],
        rva: u64,
        patch_len: u16,
        primitive_id_sha256: [u8; 32],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookAllowlistEntryV1 {
    pub identity_sha256: [u8; 32],
    pub target: HookTargetV1,
    pub proof_sha256: [u8; 32],
    /// Whether this hook failing to install voids the entire capture.
    ///
    /// This is the ONE place that question is answered, and it is answered as
    /// a declared property of the roster rather than as a consequence of which
    /// code path happened to run. [`hook_install_outcomes_preimage`] enforces
    /// it: an outcome roster in which a mandatory entry is anything other than
    /// `Installed` is refused, so a mandatory hook can never be reported
    /// unavailable and have the capture stand.
    ///
    /// The two bootstrap import hooks are mandatory. They are installed before
    /// the registry owns a single gameplay hook, so there is nothing yet to
    /// isolate them from, and the registry's own shape checks require exactly
    /// two of them.
    ///
    /// Every gameplay-domain producer detour is NOT mandatory. One domain's
    /// producer failing to patch costs that domain's observations and nothing
    /// else, and a run in which 72 of 73 domains recorded is worth
    /// incomparably more than no run at all. It is also the only way the
    /// remaining install defects are found in one pass instead of one per game
    /// launch — three separate validators each took the whole 73-domain
    /// registry down on a single day, each found only by launching the game.
    ///
    /// What this does NOT relax is anything about a domain that DID install.
    /// Its terminal custody, release accounting, and zero-observation
    /// admission are untouched. And [`HookInstallStatus::Failed`] is
    /// admissible only with [`HookInstallBindingV1::None`], so no lane can
    /// report itself unavailable while still claiming an observed patch.
    pub mandatory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinnedHookTargetSpecV1 {
    ImportExport {
        module_name: &'static str,
        symbol_name: &'static str,
        patch_len: u16,
        primitive_id: &'static str,
    },
    Hoi4Rva {
        module_name: &'static str,
        rva: u64,
        patch_len: u16,
        primitive_id: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PinnedProductionHookSpecV1 {
    pub logical_name: &'static str,
    pub target: PinnedHookTargetSpecV1,
    pub proof_sha256: [u8; 32],
    pub mandatory: bool,
}

const MINHOOK_NORMAL_REL5: &str = "minhook-0.9.0.normal-jmp-rel5";
const D75_PRODUCTION_INLINE: &str = "fired_event_popups.a729_inline_hook.production_semantic.v1";
const D90_CONVOY_REL32_CALLSITE: &str = "naval_combat.convoy_terminal.rel32_callsite.v1";

/// Primitive the bootstrap provider actually installs at the two pinned
/// import sites. It writes no code bytes: the whole admitted mutation is one
/// aligned pointer-width store into hoi4.exe's own import address table, so
/// the imported export's prologue stays pristine and no thread is suspended.
///
/// This literal must stay equal to
/// `production_render_submit_live_lane::a729_hook::PRODUCTION_BOOTSTRAP_HOOK_PRIMITIVE_ID`;
/// `compact_contract` sits below that lane in the dependency graph and cannot
/// import it, so the equality is asserted by the collector-side cross-check
/// `pinned_bootstrap_import_specs_match_installed_provider_primitive`.
pub const PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS: &str = "pe64.import-address-slot.atomic-cas.v1";

/// Width of that single admitted mutation: one 64-bit import slot.
pub const PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH: u16 = 8;

macro_rules! pinned_import {
    ($name:literal, $module:literal, $symbol:literal, $proof:literal) => {
        PinnedProductionHookSpecV1 {
            logical_name: $name,
            target: PinnedHookTargetSpecV1::ImportExport {
                module_name: $module,
                symbol_name: $symbol,
                patch_len: PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH,
                primitive_id: PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS,
            },
            proof_sha256: digest_from_hex($proof),
            mandatory: true,
        }
    };
}

/// Every entry built by this macro is a single gameplay domain's producer
/// detour, and every one of them is `mandatory: false`. See
/// [`HookAllowlistEntryV1::mandatory`] for why, and for what that does not
/// relax.
macro_rules! pinned_rva {
    ($name:literal, $rva:literal, $len:literal, $primitive:expr, $proof:literal) => {
        PinnedProductionHookSpecV1 {
            logical_name: $name,
            target: PinnedHookTargetSpecV1::Hoi4Rva {
                module_name: "hoi4.exe",
                rva: $rva,
                patch_len: $len,
                primitive_id: $primitive,
            },
            proof_sha256: digest_from_hex($proof),
            mandatory: false,
        }
    };
}

/// Exact a729 full-production hook roster. Entries are stable-name sorted.
/// Canonical digest order is identity-digest order and is produced by
/// [`pinned_production_hook_allowlist_v1`].
pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &[
    pinned_import!(
        "bootstrap.d3d11_create_device",
        "d3d11.dll",
        "D3D11CreateDevice",
        "152168d97a6afbbf938a4658444f961b3eb734fdc0654f9400dc38cf0ca5dece"
    ),
    pinned_import!(
        "bootstrap.d3d_compile",
        "d3dcompiler_47.dll",
        "D3DCompile",
        "152168d97a6afbbf938a4658444f961b3eb734fdc0654f9400dc38cf0ca5dece"
    ),
    pinned_rva!(
        "gameplay.d45.decision_cancel",
        0x71B350,
        5,
        MINHOOK_NORMAL_REL5,
        "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
    ),
    pinned_rva!(
        "gameplay.d45.decision_remove",
        0x71B590,
        5,
        MINHOOK_NORMAL_REL5,
        "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
    ),
    pinned_rva!(
        "gameplay.d45.decision_timeout",
        0x71C490,
        5,
        MINHOOK_NORMAL_REL5,
        "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
    ),
    pinned_rva!(
        "gameplay.d45.focus_completed",
        0x2CEF60,
        5,
        MINHOOK_NORMAL_REL5,
        "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3"
    ),
    pinned_rva!(
        "gameplay.d45.on_action_invoked",
        0xA6AAE0,
        5,
        MINHOOK_NORMAL_REL5,
        "8012d37ac03a47187bc2744e509acac0dbec1e6a7a91ffd20d45d58903e0e3da"
    ),
    pinned_rva!(
        "gameplay.d45.ordinary_decision_taken",
        0x728BC0,
        5,
        MINHOOK_NORMAL_REL5,
        "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3"
    ),
    pinned_rva!(
        "gameplay.d45.research_completed",
        0xECE360,
        5,
        MINHOOK_NORMAL_REL5,
        "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3"
    ),
    pinned_rva!(
        "gameplay.d45.targeted_decision_cancel",
        0x71B7D0,
        5,
        MINHOOK_NORMAL_REL5,
        "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
    ),
    pinned_rva!(
        "gameplay.d45.targeted_decision_remove",
        0x71BE10,
        5,
        MINHOOK_NORMAL_REL5,
        "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
    ),
    pinned_rva!(
        "gameplay.d45.targeted_decision_taken",
        0x728CE0,
        5,
        MINHOOK_NORMAL_REL5,
        "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3"
    ),
    pinned_rva!(
        "gameplay.d45.targeted_decision_timeout",
        0x71C130,
        5,
        MINHOOK_NORMAL_REL5,
        "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
    ),
    pinned_rva!(
        "gameplay.d45.timed_decision_activated",
        0x728FB0,
        5,
        MINHOOK_NORMAL_REL5,
        "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3"
    ),
    pinned_rva!(
        "gameplay.d55.active_damage_loss_writer",
        0xC7BFC0,
        5,
        MINHOOK_NORMAL_REL5,
        "d98faa826db8fa83dfa98618a308fcae68fbb08a446dba1fe69806b0ef7d6a4f"
    ),
    pinned_rva!(
        "gameplay.d55.division_width_leaf",
        0xC6E270,
        5,
        MINHOOK_NORMAL_REL5,
        "b17fdbf47dfb9f150b9dd55018b81765b8e3777c42cf64b96bf04a7f3e6cd1fb"
    ),
    pinned_rva!(
        "gameplay.d55.panel_leader_resolver",
        0x129E190,
        5,
        MINHOOK_NORMAL_REL5,
        "cbbed7de3e640c8cf0f5e941b35be61b6c2f25dc113b4a10d4a7016929103d95"
    ),
    pinned_rva!(
        "gameplay.d55.panel_modifier_append",
        0x15BBF00,
        5,
        MINHOOK_NORMAL_REL5,
        "bbc4acfda31f32b5bdc627bd8e530bcda20726e5871b934879bc61024b110051"
    ),
    // The `StandardHookReceiptGroup::D55Panel` trio. Found by a live run, not by
    // reading: with D99 pinned, capture 20260731-d99fix-01 got one step further
    // and the refusal named these three. The prefix-based guard could not see
    // them -- `gameplay.d55.` was already covered by the other seven D55 specs,
    // so a whole receipt group hid behind a satisfied prefix. That is why the
    // guard below now compares exact per-group name sets.
    pinned_rva!(
        "gameplay.d55.panel_side_materializer",
        0x15C9860,
        5,
        MINHOOK_NORMAL_REL5,
        "a37bc314eac46e705d27cbdf932dafe32b3fc38289cc001d8dc72310aee19c71"
    ),
    pinned_rva!(
        "gameplay.d55.participant_lifecycle_loss_writer",
        0x1299850,
        5,
        MINHOOK_NORMAL_REL5,
        "d98faa826db8fa83dfa98618a308fcae68fbb08a446dba1fe69806b0ef7d6a4f"
    ),
    pinned_rva!(
        "gameplay.d55.terminal_normal_writer",
        0xCCC7F0,
        5,
        MINHOOK_NORMAL_REL5,
        "3e5c698b7e151edc704b32f90f663b29de85e52d06f0563229885f8849637726"
    ),
    pinned_rva!(
        "gameplay.d55.terminal_overrun_writer",
        0xCCC100,
        5,
        MINHOOK_NORMAL_REL5,
        "3e5c698b7e151edc704b32f90f663b29de85e52d06f0563229885f8849637726"
    ),
    pinned_rva!(
        "gameplay.d55.terminal_record_allocator",
        0xCC9F10,
        5,
        MINHOOK_NORMAL_REL5,
        "3e5c698b7e151edc704b32f90f663b29de85e52d06f0563229885f8849637726"
    ),
    pinned_rva!(
        "gameplay.d55.width_producer",
        0x1299610,
        5,
        MINHOOK_NORMAL_REL5,
        "b17fdbf47dfb9f150b9dd55018b81765b8e3777c42cf64b96bf04a7f3e6cd1fb"
    ),
    pinned_rva!(
        "gameplay.d57.pause_forced",
        0xDBE260,
        5,
        MINHOOK_NORMAL_REL5,
        "a4c743e2f6eb067019e9f2cc1162b21466a2b133635c9730cd896021eb15ca51"
    ),
    pinned_rva!(
        "gameplay.d57.pause_toggle",
        0xDC6520,
        5,
        MINHOOK_NORMAL_REL5,
        "a4c743e2f6eb067019e9f2cc1162b21466a2b133635c9730cd896021eb15ca51"
    ),
    pinned_rva!(
        "gameplay.d57.speed_setter",
        0x1EDAD0,
        5,
        MINHOOK_NORMAL_REL5,
        "a4c743e2f6eb067019e9f2cc1162b21466a2b133635c9730cd896021eb15ca51"
    ),
    pinned_rva!(
        "gameplay.d64.definition_cancel",
        0x6DD7F0,
        5,
        MINHOOK_NORMAL_REL5,
        "a79f58a290af99a967f9f8febdb0e5142926cc62f68b008b0a3452c0981d0efc"
    ),
    pinned_rva!(
        "gameplay.d64.hourly_delayed_event_update",
        0x6EFE10,
        5,
        MINHOOK_NORMAL_REL5,
        "a79f58a290af99a967f9f8febdb0e5142926cc62f68b008b0a3452c0981d0efc"
    ),
    pinned_rva!(
        "gameplay.d67.daily_modifier_update",
        0x54BEB0,
        5,
        MINHOOK_NORMAL_REL5,
        "9281a49e3f046984a699c3c42fd1ad172361d670aed9586631329255b038cd25"
    ),
    pinned_rva!(
        "gameplay.d67.modifier_entry_erase",
        0x604D30,
        5,
        MINHOOK_NORMAL_REL5,
        "9281a49e3f046984a699c3c42fd1ad172361d670aed9586631329255b038cd25"
    ),
    pinned_rva!(
        "gameplay.d75.common_fire",
        0xA028A0,
        15,
        D75_PRODUCTION_INLINE,
        "240d7500bcc91514f59d62739df026a5a76cce41f824923422cbe253e05d1cec"
    ),
    pinned_rva!(
        "gameplay.d75.option_command",
        0x1527B30,
        17,
        D75_PRODUCTION_INLINE,
        "240d7500bcc91514f59d62739df026a5a76cce41f824923422cbe253e05d1cec"
    ),
    pinned_rva!(
        "gameplay.d75.timeout_resolution",
        0x1227360,
        16,
        D75_PRODUCTION_INLINE,
        "240d7500bcc91514f59d62739df026a5a76cce41f824923422cbe253e05d1cec"
    ),
    pinned_rva!(
        "gameplay.d86.attributed_record_writer",
        0x1D2C90,
        5,
        MINHOOK_NORMAL_REL5,
        "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0"
    ),
    pinned_rva!(
        "gameplay.d86.cause_record_writer",
        0x1D2DA0,
        5,
        MINHOOK_NORMAL_REL5,
        "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0"
    ),
    pinned_rva!(
        "gameplay.d86.combat_source_record_writer",
        0x1D2D10,
        5,
        MINHOOK_NORMAL_REL5,
        "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0"
    ),
    pinned_rva!(
        "gameplay.d86.outer_cfex_member_writer",
        0x1961DD0,
        5,
        MINHOOK_NORMAL_REL5,
        "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0"
    ),
    pinned_rva!(
        "gameplay.d90.convoy_terminal_route_callsite",
        0x1962378,
        5,
        D90_CONVOY_REL32_CALLSITE,
        "afeccd1f65e76fb2b10c56bb28c56c4ee84e4d10d36b0f6e74c6e6c8d4c80e99"
    ),
    // D94's producer detour. The proof digest is the FOW1 evidence receipt the
    // recorder host publishes in the authority block and the replay core
    // refuses any other value for.
    pinned_rva!(
        "gameplay.d94.pov_province_visibility_producer",
        0x1606EA0,
        5,
        MINHOOK_NORMAL_REL5,
        "77e4a1c964f868a7109cead30bc7df64860f2ff5097ff4849fede855b5d9e261"
    ),
    // D99's combat-resolution round detour. The production hook registry
    // installs this site unconditionally (it is `StandardHookReceiptGroup::D99`)
    // and `collect_installed_standard_receipts` collects its receipt, so a
    // roster that omits it is not describing production truth: every capture
    // then dies in `outcomes()` with "installed production hook receipt roster
    // has unpinned entries", which is exactly what killed runs
    // 20260730-postmerge-04 and 20260731-postmerge-06.
    //
    // The RVA is spelled as a NUMERIC LITERAL on purpose, and must stay one.
    // Two independent out-of-tree parsers re-derive this roster by regex --
    // `scripts/PassiveCapture.Lifecycle.ps1` and
    // `scripts/verify_sink_reconciliation.py` -- and both accept only
    // `0x[0-9A-Fa-f]+|\d+` in this position. Writing it as
    // `crate::division_combat_stats::COMBAT_ROUND_RVA as u64` reads better, and
    // was this entry's first form, but it made both parsers silently skip the
    // entry and see 37 of 38 specs, failing the harness's post-capture READY
    // authority with `canonical_hook_authority_roster_count_mismatch`.
    //
    // The value is held equal to that constant -- and to the lane's whole hook
    // authority -- by the collector-side
    // `pinned_d99_spec_matches_the_lane_hook_authority`, which is the crate
    // that can see this roster and the lane at once, and is where the equality
    // belongs anyway. The proof digest is SHA-256 over the lane's
    // the five-byte REL5 patch lands on; the same test binds it.
    pinned_rva!(
        "gameplay.d99.land_combat_round_entry",
        0x12A3220,
        5,
        MINHOOK_NORMAL_REL5,
        "e7f71427c3057c7ade226615453c6858a15c7ba1b80737aebe47b7a64e6cba2a"
    ),
];

/// Derived from the sole explicit roster authority above. Never hardcode a
/// parallel 36/41/etc. count in READY producers, verifiers, or package gates.
const _: () = assert!(PINNED_PRODUCTION_HOOK_SPECS_V1.len() <= u16::MAX as usize);
pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;
/// SHA-256 of the canonical identity-digest-sorted allowlist encoding.
///
/// Package and verifier tooling strict-extracts this literal rather than
/// reimplementing the Rust encoding. Any roster edit must update the literal
/// and pass the equality test below.
pub const PINNED_PRODUCTION_HOOK_ALLOWLIST_SHA256_V1: [u8; 32] =
    // Recomputed by this crate's own `hook_allowlist_sha256` encoder after the
    // roster took BOTH of the 2026-07-31 changes, so NEITHER parent branch's
    // digest describes the merged allowlist:
    //   * the four missing gameplay sites were pinned --
    //     `gameplay.d99.land_combat_round_entry` plus the three
    //     `gameplay.d55.panel_*` sites. The production hook registry installs
    //     all four, so a roster without them described a process that does not
    //     exist.
    //   * the gameplay producer detours became non-mandatory, so one domain's
    //     install failure marks that domain unavailable instead of voiding the
    //     capture. The mandatory bit is part of the allowlist preimage, so the
    //     digest moves with it and no consumer can be handed a roster whose
    //     isolation policy differs from the one it verified.
    // This literal was recomputed by this crate's own `hook_allowlist_sha256`
    // encoder over the merged spec list. It is deliberately neither parent's
    // value; taking either would have described a roster nobody runs.
    // Prior values for this roster, newest first:
    //   4f04dec77336c7991950910aea92bc8eb1f47b55dcf8335076ac81dd9c5acc90
    //     (producers non-mandatory, D99 and the D55 panel sites not yet pinned)
    //   b100b582f946788d45d66a4ee9192ac506a5498148582815476642c2010ce2dd
    //     (D99 and the D55 panel sites pinned, producers still mandatory)
    //   c4030f53c1bbe0127ccc8f0201f96f467f0b003711696d65758019562280383c
    //     (D99 pinned, D55 panel still missing)
    //   a260ad650825273f860d7e181627411e6004d480ef19ccfaafde77d54daaeb91
    //     (before D99)
    //   1eed3cf79a86d69cb79f3d55ca95e5d8817cb3846100e96fcb103e0c21d2a3a6
    //     (before the two bootstrap import entries were corrected from the
    //     retired inline primitive, patch_len 5
    //     minhook-0.9.0.normal-jmp-rel5, to the primitive the provider
    //     actually installs, patch_len 8
    //     pe64.import-address-slot.atomic-cas.v1)
    digest_from_hex("2e16bec7755fc97a40076b06e131622417281e5e321ceb80418dbbcbe389748a");

pub fn pinned_production_hook_count_v1() -> Result<u16, PassiveReadyDigestError> {
    u16::try_from(PINNED_PRODUCTION_HOOK_SPECS_V1.len())
        .map_err(|_| PassiveReadyDigestError::LengthOverflow)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum HookInstallStatus {
    Installed = 1,
    NotRequired = 2,
    Unavailable = 3,
    Failed = 4,
}

impl HookInstallStatus {
    pub const fn code(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookInstallBindingV1 {
    None,
    ImportExport {
        observed_module_name_sha256: [u8; 32],
        observed_symbol_name_sha256: [u8; 32],
        observed_module_sha256: [u8; 32],
        observed_export_rva: u64,
        observed_patch_len: u16,
        observed_primitive_id_sha256: [u8; 32],
        observed_placement: HookPatchPlacementV1,
    },
    Hoi4Rva {
        observed_module_sha256: [u8; 32],
        observed_rva: u64,
        observed_patch_len: u16,
        observed_primitive_id_sha256: [u8; 32],
        observed_placement: HookPatchPlacementV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum HookPatchPlacementV1 {
    NormalTarget = 1,
    PatchAboveSplit = 2,
}

impl HookPatchPlacementV1 {
    pub const fn code(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookInstallOutcomeV1 {
    pub identity_sha256: [u8; 32],
    pub status: HookInstallStatus,
    pub binding: HookInstallBindingV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassiveSchedulerConfigV1 {
    pub clock_poll_interval_us: u32,
    pub domain_queue_capacity: u32,
    pub output_queue_capacity: u32,
    pub worker_count: u16,
    pub domain_timeout_us: u32,
    pub proposal_pool_capacity_bytes: u64,
}

pub const PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1: u32 = 1;
pub const PRODUCTION_OUTPUT_QUEUE_CAPACITY_V1: u32 = 2;
/// Zero means no client-blocking deadline. Stable closing-clock brackets, not
/// wall-clock waiting or client mutation, decide accepted versus typed gap.
pub const PRODUCTION_DOMAIN_TIMEOUT_US_V1: u32 = 0;
pub const PRODUCTION_PROPOSAL_POOL_CAPACITY_BYTES_V1: u64 = 256 * 1024 * 1024;

pub fn production_scheduler_config_v1(
    clock_poll_interval_us: u32,
    worker_count: u16,
) -> PassiveSchedulerConfigV1 {
    PassiveSchedulerConfigV1 {
        clock_poll_interval_us,
        domain_queue_capacity: PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1,
        output_queue_capacity: PRODUCTION_OUTPUT_QUEUE_CAPACITY_V1,
        worker_count,
        domain_timeout_us: PRODUCTION_DOMAIN_TIMEOUT_US_V1,
        proposal_pool_capacity_bytes: PRODUCTION_PROPOSAL_POOL_CAPACITY_BYTES_V1,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadOnlyClockWitnessV1 {
    pub hour_read_a: i32,
    pub hour_read_b: i32,
    /// Collector-owned ordinal of the stable read attempt. This is not an
    /// engine tick and carries no pointer-derived value.
    pub observation_ordinal: u64,
    pub target_executable_sha256: [u8; 32],
    pub build_profile_sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputPipeIdentityV1 {
    pub canonical_pipe_name: String,
    pub target_pid: u32,
    pub target_creation_filetime_utc: u64,
    pub session_id: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PassiveReadyDigestError {
    InvalidHookName,
    EmptyAllowlist,
    AllowlistNotStrictlySorted,
    InvalidAllowlistEntry,
    OutcomeRosterMismatch,
    OutcomesNotStrictlySorted,
    InvalidHookOutcome,
    InvalidSchedulerConfig,
    ClockReadsDiffer { first: i32, second: i32 },
    InvalidGameHour(i32),
    InvalidClockWitness,
    InvalidPipeName,
    InvalidTargetEpoch,
    InvalidSessionId,
    PinnedAllowlistDigestMismatch,
    LengthOverflow,
}

impl core::fmt::Display for PassiveReadyDigestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid passive READY digest preimage: {self:?}")
    }
}

impl std::error::Error for PassiveReadyDigestError {}

pub fn hook_identity_preimage(name: &str) -> Result<Vec<u8>, PassiveReadyDigestError> {
    if name.is_empty()
        || name.len() > MAX_HOOK_NAME_LEN
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        return Err(PassiveReadyDigestError::InvalidHookName);
    }
    let mut out = labelled_preimage(HOOK_IDENTITY_DIGEST_LABEL)?;
    push_len_prefixed(&mut out, name.as_bytes())?;
    Ok(out)
}

pub fn hook_identity_sha256(name: &str) -> Result<[u8; 32], PassiveReadyDigestError> {
    Ok(digest(&hook_identity_preimage(name)?))
}

pub fn hook_target_name_sha256(name: &str) -> Result<[u8; 32], PassiveReadyDigestError> {
    if name.is_empty()
        || name.len() > MAX_HOOK_NAME_LEN
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(PassiveReadyDigestError::InvalidHookName);
    }
    let mut out = labelled_preimage(HOOK_TARGET_NAME_DIGEST_LABEL)?;
    push_len_prefixed(&mut out, name.as_bytes())?;
    Ok(digest(&out))
}

pub fn hook_primitive_id_sha256(name: &str) -> Result<[u8; 32], PassiveReadyDigestError> {
    if name.is_empty()
        || name.len() > MAX_HOOK_NAME_LEN
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        return Err(PassiveReadyDigestError::InvalidHookName);
    }
    let mut out = labelled_preimage(HOOK_PRIMITIVE_ID_DIGEST_LABEL)?;
    push_len_prefixed(&mut out, name.as_bytes())?;
    Ok(digest(&out))
}

pub fn pinned_production_hook_allowlist_v1(
) -> Result<Vec<HookAllowlistEntryV1>, PassiveReadyDigestError> {
    if PINNED_PRODUCTION_HOOK_SPECS_V1
        .windows(2)
        .any(|pair| pair[0].logical_name >= pair[1].logical_name)
    {
        return Err(PassiveReadyDigestError::AllowlistNotStrictlySorted);
    }
    let mut entries = Vec::with_capacity(PINNED_PRODUCTION_HOOK_SPECS_V1.len());
    for spec in PINNED_PRODUCTION_HOOK_SPECS_V1 {
        let target = match spec.target {
            PinnedHookTargetSpecV1::ImportExport {
                module_name,
                symbol_name,
                patch_len,
                primitive_id,
            } => HookTargetV1::ImportExport {
                module_name_sha256: hook_target_name_sha256(module_name)?,
                symbol_name_sha256: hook_target_name_sha256(symbol_name)?,
                patch_len,
                primitive_id_sha256: hook_primitive_id_sha256(primitive_id)?,
            },
            PinnedHookTargetSpecV1::Hoi4Rva {
                module_name,
                rva,
                patch_len,
                primitive_id,
            } => HookTargetV1::Hoi4Rva {
                module_name_sha256: hook_target_name_sha256(module_name)?,
                rva,
                patch_len,
                primitive_id_sha256: hook_primitive_id_sha256(primitive_id)?,
            },
        };
        entries.push(HookAllowlistEntryV1 {
            identity_sha256: hook_identity_sha256(spec.logical_name)?,
            target,
            proof_sha256: spec.proof_sha256,
            mandatory: spec.mandatory,
        });
    }
    entries.sort_unstable_by_key(|entry| entry.identity_sha256);
    hook_allowlist_preimage(&entries)?;
    Ok(entries)
}

pub fn pinned_production_hook_allowlist_sha256_v1() -> Result<[u8; 32], PassiveReadyDigestError> {
    let canonical = hook_allowlist_sha256(&pinned_production_hook_allowlist_v1()?)?;
    if canonical != PINNED_PRODUCTION_HOOK_ALLOWLIST_SHA256_V1 {
        return Err(PassiveReadyDigestError::PinnedAllowlistDigestMismatch);
    }
    Ok(canonical)
}

pub fn hook_allowlist_preimage(
    entries: &[HookAllowlistEntryV1],
) -> Result<Vec<u8>, PassiveReadyDigestError> {
    if entries.is_empty() {
        return Err(PassiveReadyDigestError::EmptyAllowlist);
    }
    if entries
        .windows(2)
        .any(|pair| pair[0].identity_sha256 >= pair[1].identity_sha256)
    {
        return Err(PassiveReadyDigestError::AllowlistNotStrictlySorted);
    }
    let mut out = labelled_preimage(HOOK_ALLOWLIST_DIGEST_LABEL)?;
    push_count(&mut out, entries.len())?;
    for entry in entries {
        if is_zero_digest(&entry.identity_sha256) || is_zero_digest(&entry.proof_sha256) {
            return Err(PassiveReadyDigestError::InvalidAllowlistEntry);
        }
        out.extend_from_slice(&entry.identity_sha256);
        match entry.target {
            HookTargetV1::ImportExport {
                module_name_sha256,
                symbol_name_sha256,
                patch_len,
                primitive_id_sha256,
            } if !is_zero_digest(&module_name_sha256)
                && !is_zero_digest(&symbol_name_sha256)
                && patch_len != 0
                && !is_zero_digest(&primitive_id_sha256) =>
            {
                out.extend_from_slice(&1u16.to_le_bytes());
                out.extend_from_slice(&patch_len.to_le_bytes());
                out.extend_from_slice(&module_name_sha256);
                out.extend_from_slice(&symbol_name_sha256);
                out.extend_from_slice(&primitive_id_sha256);
            }
            HookTargetV1::Hoi4Rva {
                module_name_sha256,
                rva,
                patch_len,
                primitive_id_sha256,
            } if !is_zero_digest(&module_name_sha256)
                && rva != 0
                && patch_len != 0
                && !is_zero_digest(&primitive_id_sha256) =>
            {
                out.extend_from_slice(&2u16.to_le_bytes());
                out.extend_from_slice(&patch_len.to_le_bytes());
                out.extend_from_slice(&module_name_sha256);
                out.extend_from_slice(&rva.to_le_bytes());
                out.extend_from_slice(&primitive_id_sha256);
                out.extend_from_slice(&[0; 24]);
            }
            _ => return Err(PassiveReadyDigestError::InvalidAllowlistEntry),
        }
        out.extend_from_slice(&entry.proof_sha256);
        out.push(u8::from(entry.mandatory));
        out.extend_from_slice(&[0; 7]);
    }
    Ok(out)
}

pub fn hook_allowlist_sha256(
    entries: &[HookAllowlistEntryV1],
) -> Result<[u8; 32], PassiveReadyDigestError> {
    Ok(digest(&hook_allowlist_preimage(entries)?))
}

pub fn hook_install_outcomes_preimage(
    allowlist: &[HookAllowlistEntryV1],
    outcomes: &[HookInstallOutcomeV1],
) -> Result<Vec<u8>, PassiveReadyDigestError> {
    let allowlist_digest = hook_allowlist_sha256(allowlist)?;
    if allowlist.len() != outcomes.len()
        || allowlist
            .iter()
            .zip(outcomes)
            .any(|(entry, outcome)| entry.identity_sha256 != outcome.identity_sha256)
    {
        return Err(PassiveReadyDigestError::OutcomeRosterMismatch);
    }
    if outcomes.windows(2).any(|pair| {
        (pair[0].identity_sha256, pair[0].status.code())
            >= (pair[1].identity_sha256, pair[1].status.code())
    }) {
        return Err(PassiveReadyDigestError::OutcomesNotStrictlySorted);
    }
    let mut out = labelled_preimage(HOOK_OUTCOMES_DIGEST_LABEL)?;
    out.extend_from_slice(&allowlist_digest);
    push_count(&mut out, outcomes.len())?;
    for (entry, outcome) in allowlist.iter().zip(outcomes) {
        let installed = outcome.status == HookInstallStatus::Installed;
        if is_zero_digest(&outcome.identity_sha256) || (entry.mandatory && !installed) {
            return Err(PassiveReadyDigestError::InvalidHookOutcome);
        }
        let binding_valid = match (installed, entry.target, outcome.binding) {
            (false, _, HookInstallBindingV1::None) => true,
            (
                true,
                HookTargetV1::ImportExport {
                    module_name_sha256,
                    symbol_name_sha256,
                    patch_len,
                    primitive_id_sha256,
                },
                HookInstallBindingV1::ImportExport {
                    observed_module_name_sha256,
                    observed_symbol_name_sha256,
                    observed_module_sha256,
                    observed_export_rva,
                    observed_patch_len,
                    observed_primitive_id_sha256,
                    observed_placement,
                },
            ) => {
                observed_module_name_sha256 == module_name_sha256
                    && observed_symbol_name_sha256 == symbol_name_sha256
                    && !is_zero_digest(&observed_module_sha256)
                    && observed_export_rva != 0
                    && observed_patch_len == patch_len
                    && observed_primitive_id_sha256 == primitive_id_sha256
                    && observed_placement == HookPatchPlacementV1::NormalTarget
            }
            (
                true,
                HookTargetV1::Hoi4Rva {
                    // The Hoi4Rva *binding* carries no observed module-name digest, so there is
                    // nothing to compare the allowlisted name digest against here. The module is
                    // pinned below by exact executable content hash
                    // (`observed_module_sha256 == PINNED_A729_TARGET_EXECUTABLE_SHA256`), which is
                    // strictly stronger than a name match. The name digest is still covered by the
                    // allowlist preimage in `hook_allowlist_preimage`, which rejects a zero digest.
                    module_name_sha256: _,
                    rva,
                    patch_len,
                    primitive_id_sha256,
                },
                HookInstallBindingV1::Hoi4Rva {
                    observed_module_sha256,
                    observed_rva,
                    observed_patch_len,
                    observed_primitive_id_sha256,
                    observed_placement,
                },
            ) => {
                !is_zero_digest(&observed_module_sha256)
                    && observed_module_sha256 == PINNED_A729_TARGET_EXECUTABLE_SHA256
                    && observed_rva == rva
                    && observed_patch_len == patch_len
                    && observed_primitive_id_sha256 == primitive_id_sha256
                    && observed_placement == HookPatchPlacementV1::NormalTarget
            }
            _ => false,
        };
        if !binding_valid {
            return Err(PassiveReadyDigestError::InvalidHookOutcome);
        }
        out.extend_from_slice(&outcome.identity_sha256);
        out.extend_from_slice(&outcome.status.code().to_le_bytes());
        match outcome.binding {
            HookInstallBindingV1::None => {
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&[0; 134]);
            }
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256,
                observed_symbol_name_sha256,
                observed_module_sha256,
                observed_export_rva,
                observed_patch_len,
                observed_primitive_id_sha256,
                observed_placement,
            } => {
                out.extend_from_slice(&1u16.to_le_bytes());
                out.extend_from_slice(&observed_patch_len.to_le_bytes());
                out.extend_from_slice(&observed_module_name_sha256);
                out.extend_from_slice(&observed_symbol_name_sha256);
                out.extend_from_slice(&observed_module_sha256);
                out.extend_from_slice(&observed_export_rva.to_le_bytes());
                out.extend_from_slice(&observed_primitive_id_sha256);
                out.extend_from_slice(&observed_placement.code().to_le_bytes());
                out.extend_from_slice(&[0; 58]);
            }
            HookInstallBindingV1::Hoi4Rva {
                observed_module_sha256,
                observed_rva,
                observed_patch_len,
                observed_primitive_id_sha256,
                observed_placement,
            } => {
                out.extend_from_slice(&2u16.to_le_bytes());
                out.extend_from_slice(&observed_patch_len.to_le_bytes());
                out.extend_from_slice(&observed_module_sha256);
                out.extend_from_slice(&observed_rva.to_le_bytes());
                out.extend_from_slice(&observed_primitive_id_sha256);
                out.extend_from_slice(&observed_placement.code().to_le_bytes());
                out.extend_from_slice(&[0; 58]);
            }
        }
    }
    Ok(out)
}

pub fn hook_install_outcomes_sha256(
    allowlist: &[HookAllowlistEntryV1],
    outcomes: &[HookInstallOutcomeV1],
) -> Result<[u8; 32], PassiveReadyDigestError> {
    Ok(digest(&hook_install_outcomes_preimage(
        allowlist, outcomes,
    )?))
}

pub fn scheduler_config_preimage(
    config: PassiveSchedulerConfigV1,
) -> Result<Vec<u8>, PassiveReadyDigestError> {
    if config.clock_poll_interval_us == 0
        || config.domain_queue_capacity == 0
        || config.output_queue_capacity == 0
        || config.worker_count == 0
        || config.proposal_pool_capacity_bytes == 0
    {
        return Err(PassiveReadyDigestError::InvalidSchedulerConfig);
    }
    let mut out = labelled_preimage(SCHEDULER_CONFIG_DIGEST_LABEL)?;
    out.extend_from_slice(&config.clock_poll_interval_us.to_le_bytes());
    out.extend_from_slice(&config.domain_queue_capacity.to_le_bytes());
    out.extend_from_slice(&config.output_queue_capacity.to_le_bytes());
    out.extend_from_slice(&config.worker_count.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&config.domain_timeout_us.to_le_bytes());
    out.extend_from_slice(&config.proposal_pool_capacity_bytes.to_le_bytes());
    Ok(out)
}

pub fn scheduler_config_sha256(
    config: PassiveSchedulerConfigV1,
) -> Result<[u8; 32], PassiveReadyDigestError> {
    Ok(digest(&scheduler_config_preimage(config)?))
}

pub fn read_only_clock_witness_preimage(
    witness: ReadOnlyClockWitnessV1,
) -> Result<Vec<u8>, PassiveReadyDigestError> {
    if witness.hour_read_a != witness.hour_read_b {
        return Err(PassiveReadyDigestError::ClockReadsDiffer {
            first: witness.hour_read_a,
            second: witness.hour_read_b,
        });
    }
    if hour_index_from_raw(witness.hour_read_a).is_none() {
        return Err(PassiveReadyDigestError::InvalidGameHour(
            witness.hour_read_a,
        ));
    }
    if witness.observation_ordinal == u64::MAX
        || is_zero_digest(&witness.target_executable_sha256)
        || is_zero_digest(&witness.build_profile_sha256)
    {
        return Err(PassiveReadyDigestError::InvalidClockWitness);
    }
    let mut out = labelled_preimage(READ_ONLY_CLOCK_WITNESS_DIGEST_LABEL)?;
    out.extend_from_slice(&witness.hour_read_a.to_le_bytes());
    out.extend_from_slice(&witness.hour_read_b.to_le_bytes());
    out.extend_from_slice(&witness.observation_ordinal.to_le_bytes());
    out.extend_from_slice(&witness.target_executable_sha256);
    out.extend_from_slice(&witness.build_profile_sha256);
    Ok(out)
}

pub fn read_only_clock_witness_sha256(
    witness: ReadOnlyClockWitnessV1,
) -> Result<[u8; 32], PassiveReadyDigestError> {
    Ok(digest(&read_only_clock_witness_preimage(witness)?))
}

pub fn output_pipe_identity_preimage(
    identity: &OutputPipeIdentityV1,
) -> Result<Vec<u8>, PassiveReadyDigestError> {
    let name = identity.canonical_pipe_name.as_bytes();
    if name.len() > MAX_PIPE_NAME_LEN
        || !identity.canonical_pipe_name.starts_with(r"\\.\pipe\")
        || !name.iter().all(|byte| byte.is_ascii_graphic())
        || identity.canonical_pipe_name[9..].is_empty()
        || identity.canonical_pipe_name[9..].contains('\\')
        || identity.canonical_pipe_name[9..].contains('/')
    {
        return Err(PassiveReadyDigestError::InvalidPipeName);
    }
    if identity.target_pid == 0 || identity.target_creation_filetime_utc == 0 {
        return Err(PassiveReadyDigestError::InvalidTargetEpoch);
    }
    if identity.session_id == [0; 16] {
        return Err(PassiveReadyDigestError::InvalidSessionId);
    }
    let mut out = labelled_preimage(OUTPUT_PIPE_IDENTITY_DIGEST_LABEL)?;
    push_len_prefixed(&mut out, name)?;
    out.extend_from_slice(&identity.target_pid.to_le_bytes());
    out.extend_from_slice(&identity.target_creation_filetime_utc.to_le_bytes());
    out.extend_from_slice(&identity.session_id);
    Ok(out)
}

pub fn output_pipe_identity_sha256(
    identity: &OutputPipeIdentityV1,
) -> Result<[u8; 32], PassiveReadyDigestError> {
    Ok(digest(&output_pipe_identity_preimage(identity)?))
}

/// Parse the only session-id spelling admitted by full-production passive
/// READY: canonical lowercase UUID text. Hashing an arbitrary session label
/// into 16 bytes is deliberately not an identity conversion.
pub fn canonical_session_uuid_bytes(value: &str) -> Result<[u8; 16], PassiveReadyDigestError> {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || [8, 13, 18, 23]
            .into_iter()
            .any(|offset| bytes[offset] != b'-')
    {
        return Err(PassiveReadyDigestError::InvalidSessionId);
    }
    let mut decoded = [0u8; 16];
    let mut input = 0usize;
    let mut output = 0usize;
    while input < bytes.len() {
        if matches!(input, 8 | 13 | 18 | 23) {
            input += 1;
            continue;
        }
        let high =
            lowercase_hex_nibble(bytes[input]).ok_or(PassiveReadyDigestError::InvalidSessionId)?;
        let low = lowercase_hex_nibble(bytes[input + 1])
            .ok_or(PassiveReadyDigestError::InvalidSessionId)?;
        decoded[output] = (high << 4) | low;
        input += 2;
        output += 1;
    }
    if decoded == [0; 16] {
        return Err(PassiveReadyDigestError::InvalidSessionId);
    }
    Ok(decoded)
}

pub fn zero_mutation_policy_preimage() -> Result<Vec<u8>, PassiveReadyDigestError> {
    let mut out = labelled_preimage(ZERO_MUTATION_POLICY_LABEL)?;
    out.extend_from_slice(&ZERO_MUTATION_POLICY_VERSION.to_le_bytes());
    Ok(out)
}

pub fn zero_mutation_policy_sha256() -> [u8; 32] {
    digest(
        &zero_mutation_policy_preimage()
            .expect("fixed zero-mutation policy label is within protocol bounds"),
    )
}

fn labelled_preimage(label: &[u8]) -> Result<Vec<u8>, PassiveReadyDigestError> {
    let mut out = Vec::with_capacity(label.len() + 4);
    push_len_prefixed(&mut out, label)?;
    out.extend_from_slice(&PASSIVE_READY_DIGEST_VERSION.to_le_bytes());
    Ok(out)
}

fn push_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PassiveReadyDigestError> {
    let len = u16::try_from(bytes.len()).map_err(|_| PassiveReadyDigestError::LengthOverflow)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn push_count(out: &mut Vec<u8>, count: usize) -> Result<(), PassiveReadyDigestError> {
    let count = u16::try_from(count).map_err(|_| PassiveReadyDigestError::LengthOverflow)?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn is_zero_digest(value: &[u8; 32]) -> bool {
    value == &[0; 32]
}

fn lowercase_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

const fn digest_from_hex(value: &str) -> [u8; 32] {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        panic!("pinned SHA-256 text must contain exactly 64 lowercase hex digits");
    }
    let mut decoded = [0u8; 32];
    let mut index = 0usize;
    while index < decoded.len() {
        decoded[index] = (const_lowercase_hex_nibble(bytes[index * 2]) << 4)
            | const_lowercase_hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    decoded
}

const fn const_lowercase_hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("pinned SHA-256 text must be lowercase hexadecimal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn hooks() -> ([HookAllowlistEntryV1; 2], [HookInstallOutcomeV1; 2]) {
        let first = hook_identity_sha256("bootstrap.clock").unwrap();
        let second = hook_identity_sha256("bootstrap.world").unwrap();
        let primitive = hook_primitive_id_sha256("minhook-0.9").unwrap();
        let (first, second) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        (
            [
                HookAllowlistEntryV1 {
                    identity_sha256: first,
                    target: HookTargetV1::Hoi4Rva {
                        module_name_sha256: hook_target_name_sha256("hoi4.exe").unwrap(),
                        rva: 0x1010,
                        patch_len: 5,
                        primitive_id_sha256: primitive,
                    },
                    proof_sha256: [1; 32],
                    mandatory: true,
                },
                HookAllowlistEntryV1 {
                    identity_sha256: second,
                    target: HookTargetV1::ImportExport {
                        module_name_sha256: hook_target_name_sha256("d3d11.dll").unwrap(),
                        symbol_name_sha256: hook_target_name_sha256("D3D11CreateDevice").unwrap(),
                        patch_len: 5,
                        primitive_id_sha256: primitive,
                    },
                    proof_sha256: [2; 32],
                    mandatory: false,
                },
            ],
            [
                HookInstallOutcomeV1 {
                    identity_sha256: first,
                    status: HookInstallStatus::Installed,
                    binding: HookInstallBindingV1::Hoi4Rva {
                        observed_module_sha256: PINNED_A729_TARGET_EXECUTABLE_SHA256,
                        observed_rva: 0x1010,
                        observed_patch_len: 5,
                        observed_primitive_id_sha256: primitive,
                        observed_placement: HookPatchPlacementV1::NormalTarget,
                    },
                },
                HookInstallOutcomeV1 {
                    identity_sha256: second,
                    status: HookInstallStatus::Unavailable,
                    binding: HookInstallBindingV1::None,
                },
            ],
        )
    }

    #[test]
    fn hook_preimages_are_sorted_closed_and_allowlist_bound() {
        let (entries, outcomes) = hooks();
        let allowlist = hook_allowlist_sha256(&entries).unwrap();
        let outcome = hook_install_outcomes_sha256(&entries, &outcomes).unwrap();
        assert_ne!(allowlist, outcome);
        assert_eq!(allowlist, hook_allowlist_sha256(&entries).unwrap());

        let mut reversed = entries;
        reversed.reverse();
        assert_eq!(
            hook_allowlist_sha256(&reversed),
            Err(PassiveReadyDigestError::AllowlistNotStrictlySorted)
        );
        let mut wrong = outcomes;
        wrong[0].identity_sha256 = [9; 32];
        assert_eq!(
            hook_install_outcomes_sha256(&entries, &wrong),
            Err(PassiveReadyDigestError::OutcomeRosterMismatch)
        );
        let mut off_allowlist = outcomes;
        off_allowlist[0].binding = HookInstallBindingV1::Hoi4Rva {
            observed_module_sha256: PINNED_A729_TARGET_EXECUTABLE_SHA256,
            observed_rva: 0x1011,
            observed_patch_len: 5,
            observed_primitive_id_sha256: hook_primitive_id_sha256("minhook-0.9").unwrap(),
            observed_placement: HookPatchPlacementV1::NormalTarget,
        };
        assert_eq!(
            hook_install_outcomes_sha256(&entries, &off_allowlist),
            Err(PassiveReadyDigestError::InvalidHookOutcome)
        );

        let mut wrong_width = outcomes;
        wrong_width[0].binding = HookInstallBindingV1::Hoi4Rva {
            observed_module_sha256: PINNED_A729_TARGET_EXECUTABLE_SHA256,
            observed_rva: 0x1010,
            observed_patch_len: 6,
            observed_primitive_id_sha256: hook_primitive_id_sha256("minhook-0.9").unwrap(),
            observed_placement: HookPatchPlacementV1::NormalTarget,
        };
        assert_eq!(
            hook_install_outcomes_sha256(&entries, &wrong_width),
            Err(PassiveReadyDigestError::InvalidHookOutcome)
        );

        let mut wrong_module = outcomes;
        wrong_module[0].binding = HookInstallBindingV1::Hoi4Rva {
            observed_module_sha256: [8; 32],
            observed_rva: 0x1010,
            observed_patch_len: 5,
            observed_primitive_id_sha256: hook_primitive_id_sha256("minhook-0.9").unwrap(),
            observed_placement: HookPatchPlacementV1::NormalTarget,
        };
        assert_eq!(
            hook_install_outcomes_sha256(&entries, &wrong_module),
            Err(PassiveReadyDigestError::InvalidHookOutcome)
        );

        let mut patch_above = outcomes;
        patch_above[0].binding = HookInstallBindingV1::Hoi4Rva {
            observed_module_sha256: PINNED_A729_TARGET_EXECUTABLE_SHA256,
            observed_rva: 0x1010,
            observed_patch_len: 5,
            observed_primitive_id_sha256: hook_primitive_id_sha256("minhook-0.9").unwrap(),
            observed_placement: HookPatchPlacementV1::PatchAboveSplit,
        };
        assert_eq!(
            hook_install_outcomes_sha256(&entries, &patch_above),
            Err(PassiveReadyDigestError::InvalidHookOutcome)
        );

        let mut missing_mandatory = outcomes;
        missing_mandatory[0] = HookInstallOutcomeV1 {
            identity_sha256: entries[0].identity_sha256,
            status: HookInstallStatus::Unavailable,
            binding: HookInstallBindingV1::None,
        };
        assert_eq!(
            hook_install_outcomes_sha256(&entries, &missing_mandatory),
            Err(PassiveReadyDigestError::InvalidHookOutcome)
        );
    }

    #[test]
    fn installed_import_export_binds_module_export_width_primitive_and_placement() {
        let primitive = hook_primitive_id_sha256("minhook-0.9").unwrap();
        let identity = hook_identity_sha256("bootstrap.d3d11_create_device").unwrap();
        let module_name = hook_target_name_sha256("d3d11.dll").unwrap();
        let symbol_name = hook_target_name_sha256("D3D11CreateDevice").unwrap();
        let allowlist = [HookAllowlistEntryV1 {
            identity_sha256: identity,
            target: HookTargetV1::ImportExport {
                module_name_sha256: module_name,
                symbol_name_sha256: symbol_name,
                patch_len: 5,
                primitive_id_sha256: primitive,
            },
            proof_sha256: [7; 32],
            mandatory: true,
        }];
        let installed = HookInstallOutcomeV1 {
            identity_sha256: identity,
            status: HookInstallStatus::Installed,
            binding: HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [8; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 5,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
        };
        assert!(hook_install_outcomes_sha256(&allowlist, &[installed]).is_ok());

        let invalid_bindings = [
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [0; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 5,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [8; 32],
                observed_export_rva: 0,
                observed_patch_len: 5,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [8; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 6,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [8; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 5,
                observed_primitive_id_sha256: hook_primitive_id_sha256("custom-inline-v1").unwrap(),
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [8; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 5,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::PatchAboveSplit,
            },
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: hook_target_name_sha256("wrong.dll").unwrap(),
                observed_symbol_name_sha256: symbol_name,
                observed_module_sha256: [8; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 5,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
            HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name,
                observed_symbol_name_sha256: hook_target_name_sha256("WrongExport").unwrap(),
                observed_module_sha256: [8; 32],
                observed_export_rva: 0x1234,
                observed_patch_len: 5,
                observed_primitive_id_sha256: primitive,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
        ];
        for binding in invalid_bindings {
            let outcome = HookInstallOutcomeV1 {
                binding,
                ..installed
            };
            assert_eq!(
                hook_install_outcomes_sha256(&allowlist, &[outcome]),
                Err(PassiveReadyDigestError::InvalidHookOutcome)
            );
        }
    }

    #[test]
    fn pinned_full_production_roster_is_exact_closed_and_canonical() {
        assert_eq!(
            pinned_production_hook_count_v1().unwrap(),
            PINNED_PRODUCTION_HOOK_COUNT_V1
        );
        assert_eq!(
            usize::from(PINNED_PRODUCTION_HOOK_COUNT_V1),
            PINNED_PRODUCTION_HOOK_SPECS_V1.len()
        );
        assert!(PINNED_PRODUCTION_HOOK_SPECS_V1
            .windows(2)
            .all(|pair| pair[0].logical_name < pair[1].logical_name));
        // Which entries are mandatory is the install-isolation policy and is
        // asserted on its own in `only_the_bootstrap_hooks_void_the_capture`.
        // What every entry must carry, whatever that policy says, is a proof
        // digest.
        assert!(PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .all(|spec| spec.proof_sha256 != [0; 32]));

        let common_fire = PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .find(|spec| spec.logical_name == "gameplay.d75.common_fire")
            .unwrap();
        assert!(matches!(
            common_fire.target,
            PinnedHookTargetSpecV1::Hoi4Rva {
                module_name: "hoi4.exe",
                rva: 0xA028A0,
                patch_len: 15,
                primitive_id: D75_PRODUCTION_INLINE,
            }
        ));
        let convoy = PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .find(|spec| spec.logical_name == "gameplay.d90.convoy_terminal_route_callsite")
            .unwrap();
        assert!(matches!(
            convoy.target,
            PinnedHookTargetSpecV1::Hoi4Rva {
                module_name: "hoi4.exe",
                rva: 0x1962378,
                patch_len: 5,
                primitive_id: D90_CONVOY_REL32_CALLSITE,
            }
        ));

        let allowlist = pinned_production_hook_allowlist_v1().unwrap();
        assert_eq!(allowlist.len(), PINNED_PRODUCTION_HOOK_SPECS_V1.len());
        assert!(allowlist
            .windows(2)
            .all(|pair| pair[0].identity_sha256 < pair[1].identity_sha256));
        // The mandatory bit must survive the spec -> allowlist encoding, since
        // it is the install-isolation policy and part of the digest preimage.
        for spec in PINNED_PRODUCTION_HOOK_SPECS_V1 {
            let identity = hook_identity_sha256(spec.logical_name).unwrap();
            let entry = allowlist
                .iter()
                .find(|entry| entry.identity_sha256 == identity)
                .unwrap_or_else(|| panic!("{} is absent from the allowlist", spec.logical_name));
            assert_eq!(
                entry.mandatory, spec.mandatory,
                "{} lost its mandatory bit in the allowlist encoding",
                spec.logical_name
            );
        }
        assert_eq!(
            pinned_production_hook_allowlist_sha256_v1().unwrap(),
            hook_allowlist_sha256(&allowlist).unwrap()
        );
    }

    #[test]
    fn scheduler_clock_pipe_and_policy_preimages_are_exact_and_pointer_free() {
        let scheduler = PassiveSchedulerConfigV1 {
            clock_poll_interval_us: 2_000,
            domain_queue_capacity: 128,
            output_queue_capacity: 256,
            worker_count: 8,
            domain_timeout_us: 28_000,
            proposal_pool_capacity_bytes: 1 << 20,
        };
        assert_eq!(
            scheduler_config_sha256(scheduler).unwrap(),
            digest(&scheduler_config_preimage(scheduler).unwrap())
        );
        let production = production_scheduler_config_v1(2_000, 8);
        assert_eq!(production.domain_queue_capacity, 1);
        assert_eq!(production.output_queue_capacity, 2);
        assert_eq!(production.domain_timeout_us, 0);
        assert_eq!(production.proposal_pool_capacity_bytes, 256 * 1024 * 1024);
        assert!(scheduler_config_sha256(production).is_ok());

        let hour = DATE_EPOCH_CONST + 1944 * 365 * 24;
        let clock = ReadOnlyClockWitnessV1 {
            hour_read_a: hour,
            hour_read_b: hour,
            observation_ordinal: 7,
            target_executable_sha256: [3; 32],
            build_profile_sha256: [4; 32],
        };
        assert_eq!(
            read_only_clock_witness_sha256(clock).unwrap(),
            digest(&read_only_clock_witness_preimage(clock).unwrap())
        );
        let mut torn = clock;
        torn.hour_read_b += 1;
        assert!(matches!(
            read_only_clock_witness_sha256(torn),
            Err(PassiveReadyDigestError::ClockReadsDiffer { .. })
        ));

        let pipe = OutputPipeIdentityV1 {
            canonical_pipe_name: r"\\.\pipe\hoi4-passive-42".to_owned(),
            target_pid: 42,
            target_creation_filetime_utc: 99,
            session_id: [5; 16],
        };
        assert_eq!(
            output_pipe_identity_sha256(&pipe).unwrap(),
            digest(&output_pipe_identity_preimage(&pipe).unwrap())
        );
        assert_eq!(
            canonical_session_uuid_bytes("a4b17fed-ed12-4361-9064-fb6576638110").unwrap(),
            [
                0xa4, 0xb1, 0x7f, 0xed, 0xed, 0x12, 0x43, 0x61, 0x90, 0x64, 0xfb, 0x65, 0x76, 0x63,
                0x81, 0x10,
            ]
        );
        for invalid in [
            "",
            "manual-session",
            "A4b17fed-ed12-4361-9064-fb6576638110",
            "00000000-0000-0000-0000-000000000000",
        ] {
            assert_eq!(
                canonical_session_uuid_bytes(invalid),
                Err(PassiveReadyDigestError::InvalidSessionId)
            );
        }
        assert_ne!(zero_mutation_policy_sha256(), [0; 32]);
    }
}

#[cfg(test)]
mod pinned_bootstrap_import_primitive {
    use super::{
        hook_install_outcomes_preimage, pinned_production_hook_allowlist_v1, HookAllowlistEntryV1,
        HookInstallBindingV1, HookInstallOutcomeV1, HookInstallStatus, HookPatchPlacementV1,
        HookTargetV1, PinnedHookTargetSpecV1, PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS,
        PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH, PINNED_PRODUCTION_HOOK_SPECS_V1,
    };

    /// The install-isolation policy, declared once and pinned here.
    ///
    /// Bootstrap is installed before the registry owns any gameplay hook, so
    /// there is nothing to isolate it from. Every gameplay-domain producer
    /// detour is isolatable: losing it costs that domain and nothing else.
    ///
    /// Goes red if a gameplay hook is made mandatory again — restoring the
    /// one-defect-per-game-launch behaviour this exists to end — or if
    /// bootstrap is made optional.
    #[test]
    fn only_the_bootstrap_hooks_void_the_capture() {
        let mut bootstrap = 0;
        let mut gameplay = 0;
        for spec in PINNED_PRODUCTION_HOOK_SPECS_V1 {
            if spec.logical_name.starts_with("bootstrap.") {
                bootstrap += 1;
                assert!(
                    spec.mandatory,
                    "{} is installed before the registry owns a gameplay hook, so there is \
                     nothing to isolate it from",
                    spec.logical_name
                );
            } else {
                gameplay += 1;
                assert!(
                    spec.logical_name.starts_with("gameplay."),
                    "{} is neither a bootstrap nor a gameplay hook, so this policy does not \
                     classify it",
                    spec.logical_name
                );
                assert!(
                    !spec.mandatory,
                    "{} is one domain's producer; its failure must mark that domain \
                     unavailable, not void the other 72",
                    spec.logical_name
                );
            }
        }
        assert_eq!(bootstrap, 2);
        assert!(gameplay > 0);
        assert_eq!(bootstrap + gameplay, PINNED_PRODUCTION_HOOK_SPECS_V1.len());
    }

    /// The enforcement half of that policy, exercised on the real roster.
    ///
    /// A mandatory hook may not be reported anything but `Installed`. A
    /// non-mandatory hook reported `Failed` must carry NO binding, so a lane
    /// can never claim to be unavailable while still claiming an observed
    /// patch — which is "a failed install must still release" enforced at the
    /// digest rather than by convention.
    #[test]
    fn the_outcome_digest_decides_which_hooks_may_be_reported_unavailable() {
        let allowlist = pinned_production_hook_allowlist_v1().unwrap();
        let installed_all = allowlist
            .iter()
            .map(|entry| HookInstallOutcomeV1 {
                identity_sha256: entry.identity_sha256,
                status: HookInstallStatus::Installed,
                binding: installed_binding_for(entry),
            })
            .collect::<Vec<_>>();
        assert!(hook_install_outcomes_preimage(&allowlist, &installed_all).is_ok());

        let mut mandatory_seen = 0;
        let mut optional_seen = 0;
        for (index, entry) in allowlist.iter().enumerate() {
            let mut failed = installed_all.clone();
            failed[index] = HookInstallOutcomeV1 {
                identity_sha256: entry.identity_sha256,
                status: HookInstallStatus::Failed,
                binding: HookInstallBindingV1::None,
            };
            let admitted = hook_install_outcomes_preimage(&allowlist, &failed).is_ok();
            assert_eq!(
                admitted,
                !entry.mandatory,
                "entry {index} (mandatory={}) was {}admitted as Failed",
                entry.mandatory,
                if admitted { "" } else { "not " }
            );
            if entry.mandatory {
                mandatory_seen += 1;
            } else {
                optional_seen += 1;
            }

            // Failed-but-still-bound is refused whatever the mandatory bit says.
            let mut still_bound = installed_all.clone();
            still_bound[index] = HookInstallOutcomeV1 {
                identity_sha256: entry.identity_sha256,
                status: HookInstallStatus::Failed,
                binding: installed_binding_for(entry),
            };
            assert!(
                hook_install_outcomes_preimage(&allowlist, &still_bound).is_err(),
                "entry {index} was admitted as Failed while still carrying a binding"
            );
        }
        // Both arms were actually exercised, so this cannot pass by covering
        // only one kind of entry.
        assert_eq!(mandatory_seen, 2);
        assert!(optional_seen > 0);
    }

    fn installed_binding_for(entry: &HookAllowlistEntryV1) -> HookInstallBindingV1 {
        match entry.target {
            HookTargetV1::ImportExport {
                module_name_sha256,
                symbol_name_sha256,
                patch_len,
                primitive_id_sha256,
            } => HookInstallBindingV1::ImportExport {
                observed_module_name_sha256: module_name_sha256,
                observed_symbol_name_sha256: symbol_name_sha256,
                observed_module_sha256: [7; 32],
                observed_export_rva: 0x1000,
                observed_patch_len: patch_len,
                observed_primitive_id_sha256: primitive_id_sha256,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
            HookTargetV1::Hoi4Rva {
                rva,
                patch_len,
                primitive_id_sha256,
                ..
            } => HookInstallBindingV1::Hoi4Rva {
                observed_module_sha256: super::PINNED_A729_TARGET_EXECUTABLE_SHA256,
                observed_rva: rva,
                observed_patch_len: patch_len,
                observed_primitive_id_sha256: primitive_id_sha256,
                observed_placement: HookPatchPlacementV1::NormalTarget,
            },
        }
    }

    /// The two pinned bootstrap sites are import-address-slot stores, not
    /// inline code patches. The retired inline primitive id and its 5-byte
    /// patch width must not reappear on either of them: the collector compares
    /// these two fields against the provider's live receipt and fails closed on
    /// any difference.
    ///
    /// Fails if either bootstrap entry is moved back to
    /// `minhook-0.9.0.normal-jmp-rel5`/`patch_len: 5`, or if the width constant
    /// stops being pointer-width.
    #[test]
    fn bootstrap_import_specs_name_the_import_slot_primitive() {
        assert_eq!(PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH, 8);
        let bootstrap: Vec<_> = PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .filter(|spec| {
                matches!(spec.target, PinnedHookTargetSpecV1::ImportExport { .. })
            })
            .collect();
        assert_eq!(bootstrap.len(), 2);
        for spec in bootstrap {
            let PinnedHookTargetSpecV1::ImportExport {
                patch_len,
                primitive_id,
                ..
            } = spec.target
            else {
                unreachable!("filtered to ImportExport targets");
            };
            assert_eq!(
                primitive_id, PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS,
                "{} must name the installed import-slot primitive",
                spec.logical_name
            );
            assert_ne!(
                primitive_id, super::MINHOOK_NORMAL_REL5,
                "{} must not name the retired suspending inline primitive",
                spec.logical_name
            );
            assert_eq!(
                patch_len, PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH,
                "{} admits exactly one pointer-width slot store",
                spec.logical_name
            );
        }
    }

    /// The pinned literal must be the digest this crate's own encoder produces
    /// for the current roster. Fails if any roster field changes without the
    /// literal being recomputed, and vice versa.
    #[test]
    fn pinned_allowlist_literal_matches_the_encoder() {
        let entries = pinned_production_hook_allowlist_v1().unwrap();
        assert_eq!(
            super::hook_allowlist_sha256(&entries).unwrap(),
            super::PINNED_PRODUCTION_HOOK_ALLOWLIST_SHA256_V1
        );
        assert!(super::pinned_production_hook_allowlist_sha256_v1().is_ok());
    }
}
