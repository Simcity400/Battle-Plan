//! Pinned a729 power writers and their native observation points.
//! P: hoi4re fn/asm at the VAs beside each site; image base 140000000.
//! Writer closure is checked live by collector interval conservation, not
//! claimed from an offset sweep. Constructors/load parsers establish baselines.
//! Native notifications (140B9DFA0, 141142D40) do not store a government cause.
use crate::{
    Memory, ccountry, cideology_group, cpolitical_party, cpolitical_status, msvc_string, read,
};

#[derive(Debug, Clone, Copy)]
pub struct HookSite {
    pub name: &'static str,
    pub function_rva: usize,
    pub rva: usize,
    pub sentinel: [u8; 16],
    pub commit: bool,
}

/// P: 140B98130, 140B98130; entry: RCX status, RDX signed fixed5; signal follows clamp.
pub const ADD_POLITICAL_POWER: HookSite = HookSite {
    name: "AddPoliticalPower",
    function_rva: 0xb98130,
    rva: 0xb98130,
    sentinel: [
        0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x8c, 0x14, 0x8a, 0x02, 0x00, 0x48, 0x8b,
        0xf9,
    ],
    commit: false,
};

/// P: 140B98220, 140B98220; entry: RCX status, RDX signed fixed5; no-deficit floor.
pub const ADD_POLITICAL_POWER_NO_DEFICIT: HookSite = HookSite {
    name: "AddPoliticalPowerNoDeficit",
    function_rva: 0xb98220,
    rva: 0xb98220,
    sentinel: [
        0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x9c, 0x13, 0x8a, 0x02, 0x00, 0x48, 0x8b,
        0xf9,
    ],
    commit: false,
};

/// P: 1403638E0, 140363927; commit: RSI status, RCX evaluated target; negative target then floors to zero.
pub const SET_POLITICAL_POWER: HookSite = HookSite {
    name: "SetPoliticalPower",
    function_rva: 0x3638e0,
    rva: 0x363927,
    sentinel: [
        0x48, 0x89, 0x8e, 0xe0, 0x00, 0x00, 0x00, 0x48, 0x83, 0x7c, 0x24, 0x40, 0x00, 0x7d, 0x0b,
        0x48,
    ],
    commit: true,
};

/// P: 140B98E20, 140B98E20; entry: RCX status; saves old government before party destruction.
pub const CLEAR_POLITICS: HookSite = HookSite {
    name: "ClearPolitics",
    function_rva: 0xb98e20,
    rva: 0xb98e20,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x54, 0x48, 0x83, 0xec, 0x20, 0x65, 0x48,
        0x8b,
    ],
    commit: false,
};

/// P: 1406D61D0, 1406D61D0; entry: RCX country; CP reset at 1406D64A1, nested clear/setup observed separately.
pub const RESET_COUNTRY: HookSite = HookSite {
    name: "ResetCountry",
    function_rva: 0x6d61d0,
    rva: 0x6d61d0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
    commit: false,
};

/// P: 1406C3320, 1406C3320; entry: RCX country, RDX pointer to signed fixed5, return void.
pub const ADD_COMMAND_POWER: HookSite = HookSite {
    name: "AddCommandPower",
    function_rva: 0x6c3320,
    rva: 0x6c3320,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x20,
        0x48,
    ],
    commit: false,
};

/// P: 140707F90, 140707F90; entry: RCX country, RDX pointer to charge; AL success, preserve full RAX.
pub const SPEND_COMMAND_POWER: HookSite = HookSite {
    name: "SpendCommandPower",
    function_rva: 0x707f90,
    rva: 0x707f90,
    sentinel: [
        0x48, 0x8b, 0x81, 0xf0, 0x01, 0x00, 0x00, 0x4c, 0x8b, 0x02, 0x49, 0x3b, 0xc0, 0x7c, 0x0d,
        0x49,
    ],
    commit: false,
};

/// P: 140707C00, 140707C00; entry: RCX country, RDX allocator, R8B charge; AL success, preserve full RAX.
pub const ALLOCATE_COMMAND_POWER: HookSite = HookSite {
    name: "AllocateCommandPower",
    function_rva: 0x707c00,
    rva: 0x707c00,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x54, 0x41,
        0x56,
    ],
    commit: false,
};

/// P: 1406DE290, 1406DE290; entry: RCX country, RDX allocator; nested AddCP refund, own cap.
pub const RELEASE_COMMAND_POWER: HookSite = HookSite {
    name: "ReleaseCommandPower",
    function_rva: 0x6de290,
    rva: 0x6de290,
    sentinel: [
        0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x56, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x4f,
        0xa9,
    ],
    commit: false,
};

/// P: 1406DD930, 1406DD930; entry: RCX country, RDX definition, R8 scope, R9B charge; direct upfront subtraction then nested allocation.
pub const ACTIVATE_ABILITY: HookSite = HookSite {
    name: "ActivateAbility",
    function_rva: 0x6dd930,
    rva: 0x6dd930,
    sentinel: [
        0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d,
        0x6c,
    ],
    commit: false,
};

/// P: 1406EF9A0, 1406EFC6D; commit: R13 country, RCX new balance, RDX evaluated charge, R15+RBX record; one commit per paid anniversary.
pub const ABILITY_UPKEEP: HookSite = HookSite {
    name: "AbilityUpkeep",
    function_rva: 0x6ef9a0,
    rva: 0x6efc6d,
    sentinel: [
        0x49, 0x89, 0x8d, 0xf0, 0x01, 0x00, 0x00, 0xeb, 0x03, 0x40, 0x32, 0xff, 0x49, 0x8b, 0x54,
        0x1f,
    ],
    commit: true,
};

/// P: 1406C6D30, 1406C6D30; entry: RCX country, RDX region, R8 allocator, R9 adjustment, fifth byte charge.
pub const GROUND_CREW: HookSite = HookSite {
    name: "GroundCrew",
    function_rva: 0x6c6d30,
    rva: 0x6c6d30,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
    commit: false,
};

/// P: 140B9F440, 140B9F5B6; commit: RBX status, RAX new party; before refresh/on-action.
pub const SET_RULING_PARTY: HookSite = HookSite {
    name: "SetRulingParty",
    function_rva: 0xb9f440,
    rva: 0xb9f5b6,
    sentinel: [
        0x48, 0x89, 0x83, 0xd0, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcb, 0x48, 0x8b, 0x5c, 0x24, 0x78,
        0x48,
    ],
    commit: true,
};

/// P: 140B9E500, 140B9E582; commit: RBP status, RSI new party; before refresh/on-action.
pub const FORCE_RULING_PARTY: HookSite = HookSite {
    name: "ForceRulingParty",
    function_rva: 0xb9e500,
    rva: 0xb9e582,
    sentinel: [
        0x48, 0x89, 0xb5, 0xd0, 0x00, 0x00, 0x00, 0xe8, 0x32, 0xfd, 0xff, 0xff, 0xba, 0x80, 0x8d,
        0x5b,
    ],
    commit: true,
};

/// P: 140B9F5E0, 140B9F770; commit: RBP status, RCX new party; setup/dynamic reuse.
pub const SETUP_POLITICS: HookSite = HookSite {
    name: "SetupPolitics",
    function_rva: 0xb9f5e0,
    rva: 0xb9f770,
    sentinel: [
        0x48, 0x89, 0x8d, 0xd0, 0x00, 0x00, 0x00, 0x4c, 0x8b, 0xbd, 0xc8, 0x00, 0x00, 0x00, 0x48,
        0x8d,
    ],
    commit: true,
};

/// P: 140B9D920, 140B9DA73; loop exit: R14 status, stored final party +D0.
/// ZF is set on loop exhaustion/empty roster; the pre-existing-party branch
/// at 140B9DA3B reaches this site with ZF clear. No intermediate maxima emit.
pub const FALLBACK_RULING_PARTY: HookSite = HookSite {
    name: "FallbackRulingParty",
    function_rva: 0xb9d920,
    rva: 0xb9da73,
    sentinel: [
        0x49, 0x8b, 0x86, 0xc8, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xa8, 0x68, 0x0f, 0x00, 0x00, 0x49,
        0x8b,
    ],
    commit: true,
};

/// P: 140B9A1D0, 140B9AE45; observation: R13 status, RDI gross, [RBP-0x38] reduction, RAX net, R15 blockade count; before output/tooltip adjustment.
pub const DAILY_POLITICAL_RATE: HookSite = HookSite {
    name: "DailyPoliticalRate",
    function_rva: 0xb9a1d0,
    rva: 0xb9ae45,
    sentinel: [
        0x4c, 0x8b, 0xbd, 0x68, 0x03, 0x00, 0x00, 0x49, 0x89, 0x07, 0x4d, 0x85, 0xf6, 0x0f, 0x84,
        0x55,
    ],
    commit: true,
};

/// Native daily AddPP call returns 140B993AC; separate timed charges return
/// 140B99633. Daily CP applies inline-computed fixed5 at 1406DADC2.
pub const DAILY_PP_CALLER: usize = 0xb993ac;
pub const TIMED_PP_CALLER: usize = 0xb99633;
pub const DAILY_CP_CALLER: usize = 0x6dadc7;
/// 140B9A1D0: evaluated reduction remains at [RBP-0x38] when net is stored.
pub const DAILY_COST_STACK_OFFSET: isize = -0x38;

/// P: CALL 140B9A2CA returns 140B9A2CF, writing key-0x26 cost at [RBP-38].
/// Tap 140B9A2D2 skips XORPS: MOVUPS [RBP+C0],XMM0 relocates seven bytes,
/// with no RIP operand/interior target/return PC. R13 status, [RBP-38] cost
/// and [RBP+358] daily caller 140B99362 remain unchanged (140B9A1D0 asm).
pub const DAILY_PP_MODIFIER: HookSite = HookSite {
    name: "DailyPpModifier",
    function_rva: 0xb9a1d0,
    rva: 0xb9a2d2,
    sentinel: [
        0x0f, 0x11, 0x85, 0xc0, 0x00, 0x00, 0x00, 0x66, 0x0f, 0x6f, 0x35, 0x9f, 0x76, 0xb6, 0x01,
        0xf3,
    ],
    commit: true,
};
/// P: 140B98130/140B98220 asm: assert CALLs return 140B98204/140B98307.
/// Leave their MOV RCX,[RDI+100] intact; tap subsequent ADD RSP,30 + POP RDI
/// (five bytes), no RIP operands/interior targets/return PCs. RSP/RDI are
/// unchanged before this span; RCX now holds the signal, unused by TLS flush.
/// Native EmitVoidSignal@14032F1D0 and its unrelated callers stay untouched.
pub const PP_SIGNAL: HookSite = HookSite {
    name: "PpSignal",
    function_rva: 0xb98130,
    rva: 0xb9820b,
    sentinel: [
        0x48, 0x83, 0xc4, 0x30, 0x5f, 0xe9, 0xbb, 0x6f, 0x79, 0xff, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
        0xcc,
    ],
    commit: true,
};
pub const PP_SIGNAL_NO_DEFICIT: HookSite = HookSite {
    name: "PpSignalNoDeficit",
    function_rva: 0xb98220,
    rva: 0xb9830e,
    sentinel: [
        0x48, 0x83, 0xc4, 0x30, 0x5f, 0xe9, 0xb8, 0x6e, 0x79, 0xff, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
        0xcc,
    ],
    commit: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub pointer: usize,
    pub country: u32,
    /// P: 1406BD180 initializes the native +0x0C identity word and
    /// 1406D61D0 can reset it; 1401DA850 publishes a new raw +08 link.
    /// Preserve both words, without assuming the instance is monotonic.
    pub instance: u32,
    pub politics: Option<usize>,
}

/// Read only native ownership, never player/human/AI status. P: CCountry
/// identity +08/+0C and politics backlink +F90; AddPP 140B98130 uses owner
/// +C8. P: 1406BD180 assigns +F90 from its 0x110 allocation, retaining
/// null on allocation failure. A native null status is absence, not zero PP.
pub fn country_owner(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Owner, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("politics country address overflow")
    };
    let mut identity = [0; 16];
    let mut status = [0; 8];
    if pointer == 0
        || memory.copy_scattered(
            [pointer, at(ccountry::POLITICAL_STATUS)?],
            &mut [&mut identity, &mut status],
        ) != 2
    {
        return Err("politics country identity unreadable");
    }
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) as usize
        != base
            .checked_add(ccountry::VTABLE_RVA)
            .ok_or("politics module overflow")?
    {
        return Err("politics country vtable mismatch");
    }
    let country = u32::from_le_bytes(identity[8..12].try_into().unwrap());
    if country == 0 {
        return Err("politics country has null identity");
    }
    let status = usize::try_from(u64::from_le_bytes(status))
        .map_err(|_| "politics status pointer overflow")?;
    if status != 0 {
        if read::<u64>(memory, status).map(|p| p as usize)
            != base.checked_add(cpolitical_status::VTABLE_RVA)
        {
            return Err("politics status vtable mismatch");
        }
        if status
            .checked_add(cpolitical_status::COUNTRY)
            .and_then(|p| read::<u64>(memory, p))
            != Some(pointer as u64)
        {
            return Err("politics status owner mismatch");
        }
    }
    Ok(Owner {
        pointer,
        country,
        instance: u32::from_le_bytes(identity[12..].try_into().unwrap()),
        politics: (status != 0).then_some(status),
    })
}

pub fn status_owner(
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> Result<Owner, &'static str> {
    let pointer = status
        .checked_add(cpolitical_status::COUNTRY)
        .and_then(|p| read::<u64>(memory, p))
        .filter(|p| *p != 0)
        .ok_or("politics owner pointer unreadable or null")? as usize;
    let owner = country_owner(memory, base, pointer)?;
    if owner.politics != Some(status) {
        return Err("politics country backlink mismatch");
    }
    Ok(owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    const BASE: usize = 0x140000000;
    const COUNTRY: usize = 0x100;
    const STATUS: usize = 0x1800;
    const PARTY: usize = 0x2000;
    const IDEOLOGY: usize = 0x2400;
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x2800];
        for (at, value) in [
            (COUNTRY, BASE + ccountry::VTABLE_RVA),
            (COUNTRY + ccountry::ID, 5001),
            (COUNTRY + ccountry::POLITICAL_STATUS, STATUS),
            (STATUS, BASE + cpolitical_status::VTABLE_RVA),
            (STATUS + cpolitical_status::COUNTRY, COUNTRY),
            (PARTY, BASE + cpolitical_party::VTABLE_RVA),
            (PARTY + cpolitical_party::ID, 37),
            (PARTY + cpolitical_party::IDEOLOGY_GROUP, IDEOLOGY),
            (IDEOLOGY, BASE + cideology_group::VTABLE_RVA),
            (IDEOLOGY + cideology_group::ID, 37),
        ] {
            put(&mut bytes, at, value as u64);
        }
        let text = IDEOLOGY + cideology_group::TOKEN;
        bytes[text..text + 9].copy_from_slice(b"mod_party");
        put(&mut bytes, text + msvc_string::LENGTH, 9);
        put(&mut bytes, text + msvc_string::CAPACITY, 15);
        bytes
    }
    #[test]
    fn every_country_owner_and_native_null_status_are_preserved() {
        let mut bytes = fixture();
        let expected = Owner {
            pointer: COUNTRY,
            country: 5001,
            instance: 0,
            politics: Some(STATUS),
        };
        assert_eq!(
            country_owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                COUNTRY
            ),
            Ok(expected)
        );
        assert_eq!(
            status_owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                STATUS
            ),
            Ok(expected)
        );
        put(&mut bytes, COUNTRY + ccountry::POLITICAL_STATUS, 0);
        assert_eq!(
            country_owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                COUNTRY
            ),
            Ok(Owner {
                politics: None,
                ..expected
            })
        );
        assert!(
            status_owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                STATUS
            )
            .is_err()
        );
    }
    #[test]
    fn broken_ownership_short_reads_and_address_overflow_fail() {
        for at in [
            COUNTRY,
            COUNTRY + ccountry::ID,
            STATUS,
            STATUS + cpolitical_status::COUNTRY,
        ] {
            let mut bytes = fixture();
            put(&mut bytes, at, 0);
            assert!(
                country_owner(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    BASE,
                    COUNTRY
                )
                .is_err()
            );
        }
        let bytes = fixture();
        for end in [
            COUNTRY + 15,
            COUNTRY + ccountry::POLITICAL_STATUS + 7,
            STATUS + 7,
            STATUS + cpolitical_status::COUNTRY + 7,
        ] {
            assert!(
                country_owner(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes[..end]
                    },
                    BASE,
                    COUNTRY
                )
                .is_err()
            );
        }
        assert!(
            country_owner(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                usize::MAX - 8
            )
            .is_err()
        );
    }
    #[test]
    fn party_identity_is_owned_before_deletion_and_never_truncated() {
        let mut bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let (id, literal) = party_literal(&memory, BASE, PARTY).unwrap().unwrap();
        assert_eq!(id, 37);
        let mut token = [0; 9];
        literal.copy(&memory, &mut token).unwrap();
        assert_eq!(&token, b"mod_party");
        assert!(literal.copy(&memory, &mut [0; 8]).is_err());
        put(&mut bytes, PARTY + cpolitical_party::ID, 38);
        assert!(
            party_literal(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                PARTY
            )
            .is_err()
        );
        bytes[PARTY..].fill(0);
        assert_eq!(&token, b"mod_party");
        assert_eq!(
            party_literal(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                0
            ),
            Ok(None)
        );
    }
    #[test]
    fn sites_have_one_detour_per_native_function() {
        let sites = [
            ADD_POLITICAL_POWER,
            ADD_POLITICAL_POWER_NO_DEFICIT,
            SET_POLITICAL_POWER,
            CLEAR_POLITICS,
            RESET_COUNTRY,
            ADD_COMMAND_POWER,
            SPEND_COMMAND_POWER,
            ALLOCATE_COMMAND_POWER,
            RELEASE_COMMAND_POWER,
            ACTIVATE_ABILITY,
            ABILITY_UPKEEP,
            GROUND_CREW,
            SET_RULING_PARTY,
            FORCE_RULING_PARTY,
            SETUP_POLITICS,
            FALLBACK_RULING_PARTY,
            DAILY_POLITICAL_RATE,
        ];
        for (index, site) in sites.iter().enumerate() {
            assert!(
                !sites[..index]
                    .iter()
                    .any(|other| other.function_rva == site.function_rva)
            );
            assert!(!site.sentinel.iter().all(|byte| *byte == 0));
            assert_eq!(site.commit, site.rva != site.function_rva);
        }
    }
}

/// P: direct CALL instructions in the pinned writer receipts. Return addresses
/// identify the native charging path, never an item or a coincident table diff.
pub const CHARGING_CALLERS: &[(usize, &str)] = &[
    (0x241982, "Console"),               // 140241380
    (0x26ecb2, "Console"),               // 14026E6E0
    (0x2cea1b, "FocusRefund"),           // 1402CE790
    (0x2daa9a, "Focus"),                 // 1402DA9F0
    (0x348c42, "ScriptEffect"),          // 140348BD0
    (0x34afc9, "ScriptEffect"),          // 14034AF40
    (0x34b992, "ScaledScriptEffect"),    // 14034B770
    (0x66b8e5, "Autonomy"),              // 14066B860
    (0x66d923, "Autonomy"),              // 14066D8B0
    (0x6c6d75, "AirAction"),             // 1406C6D30
    (0x6c7319, "AirAction"),             // 1406C6D30
    (0x6ddce7, "AbilityActivation"),     // 1406DD930
    (0x6de4cf, "AllocationRefund"),      // 1406DE290
    (0x71b05e, "Decision"),              // 14071AEB0
    (0x71c2a7, "DecisionTimeout"),       // 14071C130
    (0x728efb, "TargetedDecision"),      // 140728CE0
    (0xaa2836, "Diplomacy"),             // 140AA2750
    (0xc0bd82, "LeaderAssignment"),      // 140C0BD10
    (0xc10ae6, "LeaderAssignment"),      // 140C10A60
    (0xc137df, "LeaderAssignment"),      // 140C13780
    (0xc13811, "LeaderAssignment"),      // 140C13780
    (0xd13032, "Diplomacy"),             // 140D12DA0
    (0xda09b7, "MioPolicy"),             // 140DA08A0
    (0xf4ec52, "AirAction"),             // 140F4E970
    (0xf72155, "AirAction"),             // 140F72110
    (0xf7225c, "AirAction"),             // 140F72110
    (0xfb9155, "Idea"),                  // 140FB8FD0
    (0xfd4ecc, "Raid"),                  // 140FD4DE0
    (0xfd7966, "Raid"),                  // 140FD7600
    (0xfd797d, "Raid"),                  // 140FD7600
    (0x10d66b1, "AdvisorRemoval"),       // 1410D64E0
    (0x10eea93, "Diplomacy"),            // 1410EE2D0
    (0x10eeac3, "Diplomacy"),            // 1410EE2D0
    (0x10f1ca0, "WarJustification"),     // 1410F1C20
    (0x10f3a90, "JoinWar"),              // 1410F2B70
    (0x1128494, "JoinWar"),              // 1411271C0
    (0x113bbe6, "Advisor"),              // 14113BB40
    (0x113bc3d, "Advisor"),              // 14113BB40
    (0x113bf98, "Advisor"),              // 14113BF10
    (0x11415c1, "LeaderRecruitment"),    // 141141570
    (0x1144cc8, "Advisor"),              // 141144BB0
    (0x1144d21, "Advisor"),              // 141144BB0
    (0x1147ed7, "PreferredTactic"),      // 141147D20
    (0x114880f, "ScorchedEarth"),        // 1411487C0
    (0x114ad21, "OfficerUpgrade"),       // 14114ACB0
    (0x114aeff, "CaptainUpgrade"),       // 14114AE70
    (0x128eb87, "Advisor"),              // 14128EA50
    (0x128f77e, "Advisor"),              // 14128F730
    (0x128f7a6, "Advisor"),              // 14128F730
    (0x13417eb, "PrideOfFleet"),         // 141341790
    (0x135207c, "LeaderPromotion"),      // 141352010
    (0x1434c79, "Medal"),                // 141434AC0
    (0x1830522, "LeaderTrait"),          // 1418304D0
    (0x1837f5b, "LeaderAssignment"),     // 141837E00
    (0x1a13cd1, "SpyMaster"),            // 141A13BB0
    (0x1a8ed6c, "Blockade"),             // 141A8ECC0
    (0x1b948b5, "ScientistFaction"),     // 141B94830
    (0x1edef60, "ScientistRecruitment"), // 141EDEE70
];

/// P: synchronous charging/provenance scopes. Their pinned entry ABIs use
/// integer/pointer arguments (at most five) and return through RAX.
pub const SCOPES: &[HookSite] = &[
    HookSite {
        name: "PoliticsDaily",
        function_rva: 0xb990b0,
        rva: 0xb990b0,
        sentinel: [
            0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
            0x8d, 0xac,
        ],
        commit: false,
    }, // P: 140B990B0
    HookSite {
        name: "ImproveRelationDaily",
        function_rva: 0xd0efa0,
        rva: 0xd0efa0,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xda, 0x4d, 0x8b, 0xc8, 0x4c, 0x8d,
            0x41, 0x14,
        ],
        commit: false,
    }, // P: 140D0EFA0
    HookSite {
        name: "BoostPartyDaily",
        function_rva: 0xd0e970,
        rva: 0xd0e970,
        sentinel: [
            0x83, 0x79, 0x10, 0x00, 0x7e, 0x14, 0x83, 0x79, 0x14, 0x00, 0x7e, 0x0e, 0x48, 0x8b,
            0x05, 0xfd,
        ],
        commit: false,
    }, // P: 140D0E970
    HookSite {
        name: "MioResearchDaily",
        function_rva: 0xda1380,
        rva: 0xda1380,
        sentinel: [
            0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x40, 0x49,
        ],
        commit: false,
    }, // P: 140DA1380
    HookSite {
        name: "MioProductionDaily",
        function_rva: 0xda12e0,
        rva: 0xda12e0,
        sentinel: [
            0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x40, 0x49,
        ],
        commit: false,
    }, // P: 140DA12E0
    HookSite {
        name: "AddIdea",
        function_rva: 0x113c9c0,
        rva: 0x113c9c0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x55,
            0x41, 0x56,
        ],
        commit: false,
    }, // P: 14113C9C0
    HookSite {
        name: "RemoveIdea",
        function_rva: 0x11445e0,
        rva: 0x11445e0,
        sentinel: [
            0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0xdc, 0x4f, 0x2f, 0x02, 0x00, 0x48,
            0x8b, 0xf9,
        ],
        commit: false,
    }, // P: 1411445E0
    HookSite {
        name: "ReplaceIdea",
        function_rva: 0x1144e10,
        rva: 0x1144e10,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
            0x48, 0x83,
        ],
        commit: false,
    }, // P: 141144E10
    HookSite {
        name: "HireAdvisor",
        function_rva: 0x113bb40,
        rva: 0x113bb40,
        sentinel: [
            0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
            0x8d, 0x6c,
        ],
        commit: false,
    }, // P: 14113BB40
    HookSite {
        name: "ReplaceAdvisor",
        function_rva: 0x1144bb0,
        rva: 0x1144bb0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
        commit: false,
    }, // P: 141144BB0
    HookSite {
        name: "Decision",
        function_rva: 0x71aeb0,
        rva: 0x71aeb0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
        commit: false,
    }, // P: 14071AEB0
    HookSite {
        name: "TargetedDecision",
        function_rva: 0x728ce0,
        rva: 0x728ce0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
        commit: false,
    }, // P: 140728CE0
    HookSite {
        name: "DecisionTimeout",
        function_rva: 0x71c130,
        rva: 0x71c130,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x55,
            0x41, 0x56,
        ],
        commit: false,
    }, // P: 14071C130
    HookSite {
        name: "Diplomacy",
        function_rva: 0x10ee2d0,
        rva: 0x10ee2d0,
        sentinel: [
            0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
            0x8d, 0xac,
        ],
        commit: false,
    }, // P: 1410EE2D0
    HookSite {
        name: "ScriptedDiplomacy",
        function_rva: 0xaa2750,
        rva: 0xaa2750,
        sentinel: [
            0x40, 0x53, 0x48, 0x81, 0xec, 0x90, 0x01, 0x00, 0x00, 0x48, 0x8b, 0xd9, 0x48, 0x8d,
            0x4c, 0x24,
        ],
        commit: false,
    }, // P: 140AA2750
    HookSite {
        name: "LeaderPromotion",
        function_rva: 0x1352010,
        rva: 0x1352010,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x40, 0x83, 0x79, 0x28, 0x00, 0x48, 0x8d, 0x59, 0x28,
            0x75, 0x0a,
        ],
        commit: false,
    }, // P: 141352010
    HookSite {
        name: "LeaderTrait",
        function_rva: 0x18304d0,
        rva: 0x18304d0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x33, 0xff, 0x48, 0x8b,
            0xd9, 0x40,
        ],
        commit: false,
    }, // P: 1418304D0
    HookSite {
        name: "AbilityCost",
        function_rva: 0xff4c30,
        rva: 0xff4c30,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x57,
        ],
        commit: false,
    }, // P: 140FF4C30
    HookSite {
        name: "CoupCompletion",
        function_rva: 0xacb850,
        rva: 0xacb850,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
            0x56, 0x41,
        ],
        commit: false,
    }, // P: 140ACB850
    HookSite {
        name: "CivilWar",
        function_rva: 0x10c8ca0,
        rva: 0x10c8ca0,
        sentinel: [
            0x48, 0x8b, 0xc4, 0x4c, 0x89, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48,
            0x08, 0x55,
        ],
        commit: false,
    }, // P: 1410C8CA0
    HookSite {
        name: "SetPoliticsEffect",
        function_rva: 0x363980,
        rva: 0x363980,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
            0x48, 0x83,
        ],
        commit: false,
    }, // P: 140363980
    HookSite {
        name: "PeaceForceGovernment",
        function_rva: 0x1442850,
        rva: 0x1442850,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x48,
        ],
        commit: false,
    }, // P: 141442850
    HookSite {
        name: "SubjectGovernment",
        function_rva: 0xd10630,
        rva: 0xd10630,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x89, 0x44, 0x24,
            0x18, 0x89,
        ],
        commit: false,
    }, // P: 140D10630
    HookSite {
        name: "ConsoleGovernment",
        function_rva: 0x285f60,
        rva: 0x285f60,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
            0x54, 0x41,
        ],
        commit: false,
    }, // P: 140285F60
    HookSite {
        name: "ConsoleSubject",
        function_rva: 0x2348d0,
        rva: 0x2348d0,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x55, 0x41, 0x56, 0x41,
            0x57, 0x48,
        ],
        commit: false,
    }, // P: 1402348D0
    HookSite {
        name: "OnAction",
        function_rva: 0xa6aae0,
        rva: 0xa6aae0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
            0x18, 0x55,
        ],
        commit: false,
    }, // P: 140A6AAE0
    HookSite {
        name: "Focus",
        function_rva: 0x2da9f0,
        rva: 0x2da9f0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
        commit: false,
    }, // P: 1402DA9F0
    HookSite {
        name: "FocusRefund",
        function_rva: 0x2ce790,
        rva: 0x2ce790,
        sentinel: [
            0x48, 0x8b, 0xc4, 0x88, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08, 0x48, 0x83, 0xec, 0x68,
            0x48, 0x89,
        ],
        commit: false,
    }, // P: 1402CE790
];

/// A borrowed native literal, copied synchronously while its command/party is
/// alive. Length follows the native descriptor; callers never choose a cap.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Literal {
    pub address: usize,
    pub len: usize,
}
impl Literal {
    pub fn copy(self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        if output.len() < self.len {
            return Err("politics literal staging short");
        }
        let descriptor = msvc_string::Descriptor::read(memory, self.address)?;
        if descriptor.len != self.len {
            return Err("politics literal changed during observation");
        }
        descriptor.copy(memory, output)?;
        Ok(())
    }
}
/// P: MSVC string descriptor, shared with the existing content readers.
pub fn literal(memory: &impl Memory, address: usize) -> Result<Literal, &'static str> {
    let descriptor = msvc_string::Descriptor::read(memory, address)?;
    Ok(Literal {
        address,
        len: descriptor.len,
    })
}
/// P: 14118E330 / 140B9F440; ideology IDs are not lexical IDs.
pub fn party_literal(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Option<(u32, Literal)>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let (id, ideology) = cpolitical_party::identity(memory, base, pointer)?;
    Ok(Some((
        id,
        literal(
            memory,
            ideology
                .checked_add(cideology_group::TOKEN)
                .ok_or("politics ideology address overflow")?,
        )?,
    )))
}

/// Same-hour census inputs for conservation. Includes every authenticated live
/// country, independently of its diplomatic activity or controller status.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BalanceSnapshot {
    pub country: u32,
    pub instance: u32,
    pub pp: Option<i64>,
    pub cp: i64,
    pub party: Option<u32>,
}
pub fn balances(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<BalanceSnapshot, &'static str> {
    let owner = country_owner(memory, base, pointer)?;
    let cp = read::<i64>(
        memory,
        pointer
            .checked_add(ccountry::COMMAND_POWER)
            .ok_or("CP address overflow")?,
    )
    .ok_or("CP balance unreadable")?;
    let (pp, party) = if let Some(status) = owner.politics {
        let pp = read::<i64>(
            memory,
            status
                .checked_add(cpolitical_status::POLITICAL_POWER)
                .ok_or("PP address overflow")?,
        )
        .ok_or("PP balance unreadable")?;
        let pointer = read::<u64>(
            memory,
            status
                .checked_add(cpolitical_status::RULING_PARTY)
                .ok_or("party address overflow")?,
        )
        .ok_or("ruling pointer unreadable")? as usize;
        let party = if pointer == 0 {
            None
        } else {
            Some(cpolitical_party::identity(memory, base, pointer)?.0)
        };
        (Some(pp), party)
    } else {
        (None, None)
    };
    Ok(BalanceSnapshot {
        country: owner.country,
        instance: owner.instance,
        pp,
        cp,
        party,
    })
}

/// P: 140B9AC84..140B9ACB4. The loaded blockade define is an integer which
/// native code multiplies into fixed5 and divides back. Retain its overflow
/// behaviour, rather than substituting a price from current vanilla content.
pub fn blockade_cost(count_register: usize, loaded_define: i64) -> i64 {
    (count_register as i32 as i64)
        .wrapping_mul(loaded_define)
        .wrapping_mul(100_000)
        / 100_000
}
pub const BLOCKADE_COST_DEFINE_RVA: usize = 0x3319e30;
/// Off-thread presizing from the live lexical dictionary. Newly authored or
/// dynamic keys can grow beyond this witness: the collector counts a named
/// identity shortage, preserves the known cause and grows its next staging.
pub fn lexical_key_demand(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let address = base
        .checked_add(crate::cpdx_lexer::INITIALIZED_RVA)
        .ok_or("lexer witness address overflow")?;
    let header = read::<[u8; 64]>(memory, address).ok_or("lexer witness unreadable")?;
    let witness = crate::cpdx_lexer::PreparedWitness::from_header(header)?;
    let mut longest = 0;
    for token in 0..=witness.maximum() {
        let address = witness
            .table()
            .checked_add(
                (token as usize)
                    .checked_mul(crate::cpdx_lexer::TOKEN_NAME_STRIDE)
                    .ok_or("lexer key index overflow")?,
            )
            .ok_or("lexer key address overflow")?;
        longest = longest.max(literal(memory, address)?.len);
    }
    if read::<[u8; 64]>(memory, base + crate::cpdx_lexer::INITIALIZED_RVA) != Some(header) {
        return Err("lexer changed during politics presizing");
    }
    longest
        .checked_mul(3)
        .ok_or("politics key staging extent overflow")
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    #[test]
    fn every_site_is_unique_including_scoped_native_commands() {
        let mut functions = std::collections::BTreeSet::new();
        for site in [
            ADD_POLITICAL_POWER,
            ADD_POLITICAL_POWER_NO_DEFICIT,
            SET_POLITICAL_POWER,
            CLEAR_POLITICS,
            RESET_COUNTRY,
            ADD_COMMAND_POWER,
            SPEND_COMMAND_POWER,
            ALLOCATE_COMMAND_POWER,
            RELEASE_COMMAND_POWER,
            ACTIVATE_ABILITY,
            ABILITY_UPKEEP,
            GROUND_CREW,
            SET_RULING_PARTY,
            FORCE_RULING_PARTY,
            SETUP_POLITICS,
            FALLBACK_RULING_PARTY,
            DAILY_POLITICAL_RATE,
        ]
        .into_iter()
        .chain(SCOPES.iter().copied())
        {
            assert!(
                functions.insert(site.function_rva),
                "duplicate native hook {}",
                site.name
            );
            assert!(!site.sentinel.iter().all(|byte| *byte == 0));
        }
        assert!(
            CHARGING_CALLERS
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
    }
    #[test]
    fn blockade_integer_to_fixed5_conversion_keeps_native_wrapping() {
        assert_eq!(blockade_cost(3, 2), 6);
        assert_eq!(blockade_cost(2, -3), -6);
        assert_eq!(blockade_cost(0, i64::MAX), 0);
        assert_eq!(blockade_cost(1, i64::MAX), -1);
        assert_eq!(blockade_cost(usize::MAX, 2), -2);
    }
}

/// Synchronous native operands, with SEH-guarded reads. Daily country/status
/// lifetime (1406DACC1/1406DACC8), upkeep records (1406EFBB8..1406EFC6D) and
/// PP stores (140B98130/140B98220) do not prove every authentication field or
/// pointee readable. Keep the guard for all three paths and their literal copies.
/// No pointer survives the observer or reaches a worker.
pub struct NativeOperands(crate::memory::LiveMemory);
impl NativeOperands {
    /// # Safety
    /// The caller runs inside the observed game process; spans still use SEH.
    #[inline(always)]
    pub unsafe fn new() -> Self {
        Self(unsafe { crate::memory::LiveMemory::new() })
    }
}
impl Memory for NativeOperands {
    #[inline(always)]
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }
    #[inline(always)]
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        self.0.read_scalar(address)
    }
}

#[cfg(test)]
mod operand_guard_tests {
    use super::*;
    #[test]
    fn politics_native_operands_keep_seh_guard_for_invalid_spans() {
        let memory = unsafe { NativeOperands::new() };
        let value = 123_i64;
        assert_eq!(
            read::<i64>(&memory, &value as *const i64 as usize),
            Some(value)
        );
        assert_eq!(read::<i64>(&memory, 1), None);
        assert!(!memory.copy(1, &mut [0; 16]));
    }
}
