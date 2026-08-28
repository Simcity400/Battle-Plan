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
    /// Its terminal custody, release accounting, and zero-observation
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

pub const PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS: &str = "pe64.import-address-slot.atomic-cas.v1";

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
    // d13's change-signal gate (owner ruling 2026-08-25): the five runtime
    // writers of [CTechnology]+0x170. Proof digests are SHA-256 over each
    // site's 16-byte sentinel, exactly as for every roster above; the bytes
    // come from the hash-pinned indexed instruction artifacts re-verified with
    // ghidra_verify by lane RO (runs/ro_d13_change_gate/RE_EVIDENCE.md).
    pinned_rva!(
        "gameplay.d13.tech_research_completed",
        0xECECE0,
        5,
        MINHOOK_NORMAL_REL5,
        "ae793f49175312a6780acc9002f7a3066007a56b8dd90e2d9b11946be9fcf067"
    ),
    pinned_rva!(
        "gameplay.d13.tech_status_classify_sweep",
        0xECB170,
        5,
        MINHOOK_NORMAL_REL5,
        "d490289b3d7782b9f0c50b8b3acb55acca85dead49b4087ffca8e2630c72fee6"
    ),
    pinned_rva!(
        "gameplay.d13.tech_status_fresh_init",
        0xECE9E0,
        5,
        MINHOOK_NORMAL_REL5,
        "c8b38b9201635de7981835cfc2d591d521cd9d6a2c326be08f4d735d7bd35896"
    ),
    pinned_rva!(
        "gameplay.d13.tech_status_init_post_read",
        0xECB270,
        5,
        MINHOOK_NORMAL_REL5,
        "0445026d2a77a511d0a8e49aadbdf846f8d609063aaeac4b0bf5b38a6cfff50c"
    ),
    pinned_rva!(
        "gameplay.d13.technology_propagate",
        0xEC19C0,
        5,
        MINHOOK_NORMAL_REL5,
        "e3e2b7d0933663f2cbf4cd46600126c4cf6ee0db690e91e4d78adacafd4b2bc2"
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
    // The three X12 speed/pause transition sites were FORBIDDEN for passive
    // captures from 2026-08-13 until the owner ruling of 2026-08-24 admitted
    // them into the production install set: without them d57 can never close
    // its speed/pause transition evidence path, and every capture marked the
    // domain source_unavailable|partial_data. Same RVAs, primitive, patch
    // length, and proof digests the forbidden entries carried; only the roster
    // changed (and with it this list's digest).
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
    // d81's rail write-tap (owner rulings 2026-08-24 minimise-hooks and
    // 2026-08-27 structural perf): the three IDENTIFIED writers of the
    // `CProvinceRailwayInfo` pair vector. Lane TD's dossier
    // (runs/td_rail_change_signal/DOSSIER.md) is a whole-image search that
    // found no usable passive signal - the level is written in place, the
    // engine's own consumer re-compares the vector's content, and the one
    // dirty bit the engine keeps is cleared by its own producer inside a
    // single update - and its own section 6.6 states that its writer census
    // is NOT closed. Proof digests are SHA-256 over
    // each site's 16-byte sentinel, exactly as for every roster above; the
    // bytes come from the hash-pinned indexed instruction artifacts re-verified
    // with ghidra_verify by lane SN (runs/sn_rail_write_tap/RE_EVIDENCE.md,
    // 3/3 agrees). Table-slot writers are NOT here: d81 already copies the rail
    // table extent every frame and compares the list it derives from it, so a
    // hook on those sites would violate the minimise-hooks ruling.
    pinned_rva!(
        "gameplay.d81.rail_info_destructor",
        0xE7D5C0,
        5,
        MINHOOK_NORMAL_REL5,
        "cc42c0bc351fedeea108f928801f827376f07b4839781d742e1666b043fdbed9"
    ),
    pinned_rva!(
        "gameplay.d81.rail_info_load_token",
        0xE80070,
        5,
        MINHOOK_NORMAL_REL5,
        "e60bc0f195cdfb5bdceee7a57cba744caf60efcdbff835911fb66eb15946246c"
    ),
    pinned_rva!(
        "gameplay.d81.rail_set_railway",
        0xE806E0,
        5,
        MINHOOK_NORMAL_REL5,
        "e016815dc536a92c93eb16963f84b5e84d5dc07739b9829d45333c757e6d1cbe"
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
    pinned_rva!(
        "gameplay.d94.pov_province_visibility_producer",
        0x1606EA0,
        5,
        MINHOOK_NORMAL_REL5,
        "77e4a1c964f868a7109cead30bc7df64860f2ff5097ff4849fede855b5d9e261"
    ),
    pinned_rva!(
        "gameplay.d95.intel_getter",
        0x16068C0,
        5,
        MINHOOK_NORMAL_REL5,
        "dae3cf2b2319946cd88aa78458f1e58ebb1f3181ea6356ebd028951bfb64ad85"
    ),
    pinned_rva!(
        "gameplay.d95.land_eligibility",
        0xBE2AB0,
        5,
        MINHOOK_NORMAL_REL5,
        "1c054f129d99f74e20b479c69d0fb641c70ff7c6d51c77daf7aab7de4a7b5304"
    ),
    pinned_rva!(
        "gameplay.d95.land_frame",
        0x16585E0,
        5,
        MINHOOK_NORMAL_REL5,
        "d274148e15e6292f1fff0c943c1b3ddd904dbe8eda6ac831d5ff2a8306e0814f"
    ),
    pinned_rva!(
        "gameplay.d95.taskforce_frame",
        0x18DDF50,
        5,
        MINHOOK_NORMAL_REL5,
        "0a8b346ff1b2343b6f2d79ab7b3f771fae1428718991c57aa453aa1d12a32d41"
    ),
    pinned_rva!(
        "gameplay.d97.claim_breakdown",
        0x19CE290,
        5,
        MINHOOK_NORMAL_REL5,
        "56ed55bbc110f4e2da4446e35fd39241fe4c7ed8a06c8723201b609038011694"
    ),
    pinned_rva!(
        "gameplay.d97.conference_refresh",
        0x184EB30,
        5,
        MINHOOK_NORMAL_REL5,
        "6c8733ecde9a394222cb216c51504fcf6a98b1047f1721ef99b624e55bc4a84d"
    ),
    pinned_rva!(
        "gameplay.d97.contest_breakdown",
        0x19CD540,
        5,
        MINHOOK_NORMAL_REL5,
        "e7f71427c3057c7ade226615453c6858a15c7ba1b80737aebe47b7a64e6cba2a"
    ),
    pinned_rva!(
        "gameplay.d97.terminal_summary",
        0xB6FF40,
        5,
        MINHOOK_NORMAL_REL5,
        "f386f95925e3467ca37d8bb7591a753ec82ef9b065342bb810744b866bf0a4a1"
    ),
];

const _: () = assert!(PINNED_PRODUCTION_HOOK_SPECS_V1.len() <= u16::MAX as usize);
pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;

pub const PINNED_PRODUCTION_IMPORT_HOOK_COUNT_V1: u16 = {
    let mut count = 0u16;
    let mut index = 0usize;
    while index < PINNED_PRODUCTION_HOOK_SPECS_V1.len() {
        if matches!(
            PINNED_PRODUCTION_HOOK_SPECS_V1[index].target,
            PinnedHookTargetSpecV1::ImportExport { .. }
        ) {
            count += 1;
        }
        index += 1;
    }
    count
};
pub const PINNED_PRODUCTION_HOOK_ALLOWLIST_SHA256_V1: [u8; 32] =
    digest_from_hex("850961e91b1e89ce931e32784bcbe11aa748448dcb8fcc90c6f12ab9259d579e");

pub fn pinned_production_hook_count_v1() -> Result<u16, PassiveReadyDigestError> {
    u16::try_from(PINNED_PRODUCTION_HOOK_SPECS_V1.len())
        .map_err(|_| PassiveReadyDigestError::LengthOverflow)
}

pub const FORBIDDEN_PASSIVE_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &[
    pinned_rva!(
        "gameplay.d85.division_order",
        0x1756C40,
        5,
        MINHOOK_NORMAL_REL5,
        "089ffa5d397c6a462479bce55968fba0a98270de5cd7d95982d14ab0e0922405"
    ),
    pinned_rva!(
        "gameplay.d85.division_selection",
        0x167AEE0,
        5,
        MINHOOK_NORMAL_REL5,
        "59101a8527cdb6c743d7c610e473c3406c261a153855ebcefddd019b21c769df"
    ),
    // named, which is what stopped a selection being read out of released heap.
    pinned_rva!(
        "gameplay.d85.division_view_destroy",
        0x1748D60,
        5,
        MINHOOK_NORMAL_REL5,
        "760b6625f4b632020d9068aed3b6681465e0ae57103c97cf90bf99b38d01ec7f"
    ),
    pinned_rva!(
        "gameplay.d85.division_view_handler_replace",
        0x174F670,
        5,
        MINHOOK_NORMAL_REL5,
        "dfa5fe476ac9ece5aa188548209e527153af0c176747d13e1525a014484398d2"
    ),
    pinned_rva!(
        "gameplay.d85.equipment_autodesign_success",
        0x147DDD0,
        5,
        MINHOOK_NORMAL_REL5,
        "4826719cd89c3fe2b41f2f55063aac041807738e3a2ae872f85b9870fd218fc9"
    ),
    pinned_rva!(
        "gameplay.d85.equipment_selection",
        0x17715A0,
        5,
        MINHOOK_NORMAL_REL5,
        "7096f6ab419c3d183111586bfb033bcc421b70141d60618dba728788735af757"
    ),
    pinned_rva!(
        "gameplay.d85.variant_upgrade_selection",
        0x1F5D420,
        5,
        MINHOOK_NORMAL_REL5,
        "77fc7a5ef65dc0f4739157a8a698f10a46bb4d58b0663e7583c9721504753a7f"
    ),
    // The d57 speed/pause sites left this list on 2026-08-24: the owner ruled
    // them into `PINNED_PRODUCTION_HOOK_SPECS_V1` (and the production install
    // registry) because a passive capture without them can never close d57's
    // transition evidence path. A forbidden entry that the registry itself
    // installs would be a capture-killing contradiction.
    pinned_rva!(
        "gameplay.d78.designer_refresh",
        0x17797A0,
        5,
        MINHOOK_NORMAL_REL5,
        "5e5e3d93a65ddd686cb3828e88035ec2401c4ba870cb5300e762e138bc08e25e"
    ),
    pinned_rva!(
        "gameplay.d99.land_combat_round_entry",
        0x12A3220,
        5,
        MINHOOK_NORMAL_REL5,
        "e7f71427c3057c7ade226615453c6858a15c7ba1b80737aebe47b7a64e6cba2a"
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinnedHookProducer {
    Bootstrap,
    Domain(crate::CompactDomainId),
}

pub fn pinned_hook_producer(logical_name: &str) -> Result<PinnedHookProducer, String> {
    if logical_name.starts_with("bootstrap.") {
        return Ok(PinnedHookProducer::Bootstrap);
    }
    let rest = logical_name.strip_prefix("gameplay.d").ok_or_else(|| {
        format!("pinned hook {logical_name} is neither bootstrap.* nor gameplay.d<NN>.<site>")
    })?;
    let digits = rest.find('.').ok_or_else(|| {
        format!("pinned hook {logical_name} has no site segment after its domain")
    })?;
    if digits == 0 || rest.len() == digits + 1 {
        return Err(format!(
            "pinned hook {logical_name} has an empty domain or site segment"
        ));
    }
    let id = rest[..digits]
        .parse::<crate::CompactDomainId>()
        .map_err(|_| format!("pinned hook {logical_name} has a non-numeric domain segment"))?;
    if crate::assigned_descriptor_by_id(id).is_none() {
        return Err(format!(
            "pinned hook {logical_name} names domain {id}, which this contract does not assign"
        ));
    }
    Ok(PinnedHookProducer::Domain(id))
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

pub const DIRECT_CAPTURE_IDENTITY_READ_BUDGET_MS: u64 = 5_000;

pub const PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1: u32 = 1;
pub const PRODUCTION_OUTPUT_QUEUE_CAPACITY_V1: u32 = 2;
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

pub fn direct_capture_identity_refusal_file_name(session_id: &str) -> Option<String> {
    canonical_session_uuid_bytes(session_id).ok()?;
    Some(format!("collector_campaign_identity_{session_id}.json"))
}

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

    /// patch — which is "a failed install must still release" enforced at the
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

    #[test]
    fn bootstrap_import_specs_name_the_import_slot_primitive() {
        assert_eq!(PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH, 8);
        let bootstrap: Vec<_> = PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .filter(|spec| matches!(spec.target, PinnedHookTargetSpecV1::ImportExport { .. }))
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
                primitive_id,
                super::MINHOOK_NORMAL_REL5,
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

    #[test]
    fn pinned_allowlist_literal_matches_the_encoder() {
        let entries = pinned_production_hook_allowlist_v1().unwrap();
        assert_eq!(
            super::hook_allowlist_sha256(&entries).unwrap(),
            super::PINNED_PRODUCTION_HOOK_ALLOWLIST_SHA256_V1
        );
        assert!(super::pinned_production_hook_allowlist_sha256_v1().is_ok());
    }

    #[test]
    fn no_descheduled_domain_keeps_a_pinned_production_hook_site() {
        // Reads production_hook_site_installs, so it stays meaningful for every
        // roster change: a pinned site passes only when its producer domain is
        // genuinely scheduled (or held by a live lane's prerequisite). d57's
        // three sites pass since the 2026-08-24 owner ruling readmitted the
        // domain; every descheduled domain's sites must stay out.
        use super::{PinnedHookProducer, PINNED_PRODUCTION_HOOK_SPECS_V1};

        let mut domain_sites: Vec<(crate::CompactDomainId, &'static str)> = Vec::new();
        let mut bootstrap = 0usize;
        for spec in PINNED_PRODUCTION_HOOK_SPECS_V1 {
            match super::pinned_hook_producer(spec.logical_name) {
                Ok(PinnedHookProducer::Bootstrap) => bootstrap += 1,
                Ok(PinnedHookProducer::Domain(id)) => domain_sites.push((id, spec.logical_name)),
                Err(error) => panic!(
                    "the deschedule law cannot judge this roster: {error}. Every pinned \
                     entry must name its producer, or a site can outlive a ruling by \
                     being unreadable"
                ),
            }
        }
        assert_eq!(
            bootstrap + domain_sites.len(),
            PINNED_PRODUCTION_HOOK_SPECS_V1.len(),
            "every pinned spec must be accounted for exactly once"
        );
        assert_eq!(
            bootstrap, 2,
            "the two bootstrap import hooks are the only non-domain entries"
        );

        let stale: Vec<&str> = domain_sites
            .iter()
            .filter(|(id, name)| !crate::production_hook_site_installs(name, *id))
            .map(|(_, name)| *name)
            .collect();
        assert!(
            stale.is_empty(),
            "these sites belong to descheduled domains, yet the pinned roster still \
             names them -- every capture would demand a receipt no install unit \
             mints: {stale:?}"
        );

        let pinned_names: Vec<&str> = domain_sites.iter().map(|(_, name)| *name).collect();
        for site in [
            "gameplay.d75.common_fire",
            "gameplay.d75.option_command",
            "gameplay.d75.timeout_resolution",
        ] {
            assert!(
                pinned_names.contains(&site),
                "{site} must stay pinned: it is d75's own producer now that d75 is \
                 scheduled, and it is still the dispatch observer d64 installs with"
            );
            assert_eq!(
                super::pinned_hook_producer(site),
                Ok(PinnedHookProducer::Domain(75))
            );
            assert!(crate::production_hook_site_installs(site, 75));
        }

        assert_eq!(
            super::pinned_hook_producer("gameplay.d78.designer_refresh"),
            Ok(PinnedHookProducer::Domain(78))
        );
        assert_eq!(
            super::pinned_hook_producer("bootstrap.d3d11_create_device"),
            Ok(PinnedHookProducer::Bootstrap)
        );
        for malformed in [
            "gameplay.d78",
            "gameplay.d.designer_refresh",
            "gameplay.d78.",
            "gameplay.d4242.designer_refresh",
            "designer_refresh",
        ] {
            assert!(
                super::pinned_hook_producer(malformed).is_err(),
                "{malformed} must be refused rather than silently skipped"
            );
        }
    }

    #[test]
    fn the_optional_companion_ruling_is_applied_except_where_it_is_named() {
        use super::{PinnedHookProducer, PINNED_PRODUCTION_HOOK_SPECS_V1};

        let pinned: Vec<&str> = PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .map(|spec| spec.logical_name)
            .collect();

        for deferred in crate::OPTIONAL_COMPANION_SITES_DEFERRED_TO_ACTIVE_LANES {
            let producer =
                super::pinned_hook_producer(deferred).expect("a deferred site names its producer");
            let PinnedHookProducer::Domain(id) = producer else {
                panic!("{deferred} is a bootstrap hook and no companion ruling reaches it");
            };
            assert!(
                crate::is_optional_fail_closed_companion(id),
                "{deferred} is deferred from the optional-companion ruling but d{id} \
                 is not an optional companion, so no ruling covers it"
            );
            assert!(
                !crate::is_descheduled_from_capture(id),
                "d{id} has been descheduled, so the deschedule authority already \
                 removes {deferred} and this deferral no longer defers anything -- \
                 delete the entry rather than leaving a carve-out that describes \
                 nothing"
            );
            assert!(
                pinned.contains(deferred),
                "{deferred} is deferred, which means STILL INSTALLED -- but it is not \
                 in the pinned roster, so the install rule answers `true` for a site \
                 the capture would then refuse as unpinned"
            );
        }

        let still_pinned: Vec<&str> = pinned
            .iter()
            .copied()
            .filter(|name| {
                if crate::OPTIONAL_COMPANION_SITES_DEFERRED_TO_ACTIVE_LANES.contains(name) {
                    return false;
                }
                match super::pinned_hook_producer(name) {
                    Ok(PinnedHookProducer::Domain(id)) => {
                        crate::is_optional_fail_closed_companion(id)
                    }
                    Ok(PinnedHookProducer::Bootstrap) => false,
                    Err(error) => panic!("the companion ruling cannot judge {name}: {error}"),
                }
            })
            .collect();
        assert!(
            still_pinned.is_empty(),
            "the owner ruled every optional fail-closed companion's detour out of a \
             passive capture on 2026-08-13, and these are still pinned without \
             being named as deferred. The one named exception so far is d57: the \
             owner ruling of 2026-08-24 readmitted its three speed/pause sites \
             to the production install set (registry group D57), so d57 left \
             OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS instead of joining this \
             deferral list -- every other companion stays out unless it is \
             named in OPTIONAL_COMPANION_SITES_DEFERRED_TO_ACTIVE_LANES: \
             {still_pinned:?}"
        );
    }

    #[test]
    fn the_forbidden_roster_covers_every_site_the_deschedule_removed() {
        use super::{
            PinnedHookProducer, FORBIDDEN_PASSIVE_PRODUCTION_HOOK_SPECS_V1,
            PINNED_PRODUCTION_HOOK_SPECS_V1,
        };

        for forbidden in FORBIDDEN_PASSIVE_PRODUCTION_HOOK_SPECS_V1 {
            assert!(
                !PINNED_PRODUCTION_HOOK_SPECS_V1
                    .iter()
                    .any(|spec| spec.logical_name == forbidden.logical_name),
                "{} is both required and forbidden",
                forbidden.logical_name
            );
            let producer = super::pinned_hook_producer(forbidden.logical_name)
                .expect("a forbidden site names its producer");
            let PinnedHookProducer::Domain(id) = producer else {
                panic!(
                    "{} is a bootstrap hook and can never be forbidden: the two \
                     import hooks are mandatory",
                    forbidden.logical_name
                );
            };
            assert!(
                !crate::production_hook_site_installs(forbidden.logical_name, id),
                "{} is forbidden but the install rule says a shipped capture \
                 patches it; the two authorities disagree",
                forbidden.logical_name
            );
            assert!(
                !forbidden.mandatory,
                "{} cannot be mandatory and forbidden at once",
                forbidden.logical_name
            );
        }

        for id in crate::DESCHEDULED_CAPTURE_DOMAIN_IDS {
            let still_pinned: Vec<&str> = PINNED_PRODUCTION_HOOK_SPECS_V1
                .iter()
                .map(|spec| spec.logical_name)
                .filter(|name| {
                    super::pinned_hook_producer(name) == Ok(PinnedHookProducer::Domain(*id))
                })
                .collect();
            assert!(
                still_pinned.is_empty(),
                "d{id} is descheduled, so these must not be pinned as installable: \
                 {still_pinned:?}"
            );
        }
    }
}
