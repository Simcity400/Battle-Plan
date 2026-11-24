use engine_native_memory::{Memory, msvc_string, read};
use engine_native::ccountry;
use engine_native_parts::cideology_group;
use engine_sites::Rva;

#[derive(Debug, Clone, Copy)]
pub struct HookSite {
    pub name: &'static str,
    pub rva: Rva,
    pub commit: bool,
}

pub const ADD_POLITICAL_POWER: HookSite = HookSite {
    name: "AddPoliticalPower",
    rva: Rva(0xbfa720),
    commit: false,
};

pub const ADD_POLITICAL_POWER_NO_DEFICIT: HookSite = HookSite {
    name: "AddPoliticalPowerNoDeficit",
    rva: Rva(0xbfa810),
    commit: false,
};

pub const SET_POLITICAL_POWER: HookSite = HookSite {
    name: "SetPoliticalPower",
    rva: Rva(0x37c597),
    commit: true,
};

pub const SET_POLITICAL_POWER_EFFECT: HookSite = HookSite {
    name: "SetPoliticalPowerEffect",
    rva: Rva(0x37c550),
    commit: false,
};

pub const CLEAR_POLITICS: HookSite = HookSite {
    name: "ClearPolitics",
    rva: Rva(0xbfb410),
    commit: false,
};

pub const RESET_COUNTRY: HookSite = HookSite {
    name: "ResetCountry",
    rva: Rva(0x72a990),
    commit: false,
};

pub const ADD_COMMAND_POWER: HookSite = HookSite {
    name: "AddCommandPower",
    rva: Rva(0x7179b0),
    commit: false,
};

pub const SPEND_COMMAND_POWER: HookSite = HookSite {
    name: "SpendCommandPower",
    rva: Rva(0x75cd40),
    commit: false,
};

pub const ALLOCATE_COMMAND_POWER: HookSite = HookSite {
    name: "AllocateCommandPower",
    rva: Rva(0x75c9b0),
    commit: false,
};

pub const RELEASE_COMMAND_POWER: HookSite = HookSite {
    name: "ReleaseCommandPower",
    rva: Rva(0x732a10),
    commit: false,
};

pub const ACTIVATE_ABILITY: HookSite = HookSite {
    name: "ActivateAbility",
    rva: Rva(0x7320b0),
    commit: false,
};

pub const ABILITY_UPKEEP: HookSite = HookSite {
    name: "AbilityUpkeep",
    rva: Rva(0x7445bd),
    commit: true,
};

pub const GROUND_CREW: HookSite = HookSite {
    name: "GroundCrew",
    rva: Rva(0x71b3c0),
    commit: false,
};

pub const SET_RULING_PARTY: HookSite = HookSite {
    name: "SetRulingParty",
    rva: Rva(0xc01ba6),
    commit: true,
};

pub const FORCE_RULING_PARTY: HookSite = HookSite {
    name: "ForceRulingParty",
    rva: Rva(0xc00b72),
    commit: true,
};

pub const SETUP_POLITICS: HookSite = HookSite {
    name: "SetupPolitics",
    rva: Rva(0xc01d60),
    commit: true,
};

pub const FALLBACK_RULING_PARTY: HookSite = HookSite {
    name: "FallbackRulingParty",
    rva: Rva(0xc00063),
    commit: true,
};

pub const DAILY_POLITICAL_RATE: HookSite = HookSite {
    name: "DailyPoliticalRate",
    rva: Rva(0xbfd435),
    commit: true,
};

pub const DAILY_PP_CALLER: Rva = Rva(0xbfb99c);
pub const TIMED_PP_CALLER: Rva = Rva(0xbfbc23);
pub const DAILY_CP_CALLER: Rva = Rva(0x72f4f7);

pub const DAILY_PP_MODIFIER: HookSite = HookSite {
    name: "DailyPpModifier",
    rva: Rva(0xbfc8c2),
    commit: true,
};
pub const PP_SIGNAL: HookSite = HookSite {
    name: "PpSignal",
    rva: Rva(0xbfa7fb),
    commit: true,
};
pub const PP_SIGNAL_NO_DEFICIT: HookSite = HookSite {
    name: "PpSignalNoDeficit",
    rva: Rva(0xbfa8fe),
    commit: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub pointer: usize,
    pub country: u32,
    pub instance: u32,
    pub politics: Option<usize>,
}

impl Owner {
    pub fn same_owner(self, other: Self) -> bool {
        (self.pointer, self.country, self.politics)
            == (other.pointer, other.country, other.politics)
    }
}

pub fn command_owner(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Owner, &'static str> {
    let identity = read::<[u8; 16]>(memory, pointer)
        .filter(|_| pointer != 0)
        .ok_or("politics country identity unreadable")?;
    owner_identity(base, pointer, identity)
}
fn owner_identity(base: usize, pointer: usize, identity: [u8; 16]) -> Result<Owner, &'static str> {
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) as usize
        != engine_sites::at(base, ccountry::VTABLE_RVA).ok_or("politics module overflow")?
    {
        return Err("politics country vtable mismatch");
    }
    let country = u32::from_le_bytes(identity[8..12].try_into().unwrap());
    if country == 0 {
        return Err("politics country has null identity");
    }
    Ok(Owner {
        pointer,
        country,
        instance: u32::from_le_bytes(identity[12..].try_into().unwrap()),
        politics: None,
    })
}

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
    let mut owner = owner_identity(base, pointer, identity)?;
    let status = usize::try_from(u64::from_le_bytes(status))
        .map_err(|_| "politics status pointer overflow")?;
    if status != 0 {
        let vtable = engine_sites::at(base, engine_native_parts::cpolitical_status_layout::VTABLE_RVA)
            .ok_or("politics status vtable mismatch")?;
        if read::<u64>(memory, status) != Some(vtable as u64) {
            return Err("politics status vtable mismatch");
        }
        if status
            .checked_add(engine_native_parts::cpolitical_status_layout::COUNTRY)
            .and_then(|p| read::<u64>(memory, p))
            != Some(pointer as u64)
        {
            return Err("politics status owner mismatch");
        }
    }
    owner.politics = (status != 0).then_some(status);
    Ok(owner)
}

pub fn status_owner(
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> Result<Owner, &'static str> {
    let pointer = status
        .checked_add(engine_native_parts::cpolitical_status_layout::COUNTRY)
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
    use engine_native_memory::memory::SliceMemory;
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
            (COUNTRY, BASE + ccountry::VTABLE_RVA.0),
            (COUNTRY + ccountry::ID, 5001),
            (COUNTRY + ccountry::POLITICAL_STATUS, STATUS),
            (STATUS, BASE + engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0),
            (STATUS + engine_native_parts::cpolitical_status_layout::COUNTRY, COUNTRY),
            (PARTY, BASE + engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0),
            (PARTY + engine_native_parts::cpolitical_party_layout::ID, 37),
            (PARTY + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, IDEOLOGY),
            (IDEOLOGY, BASE + cideology_group::VTABLE_RVA.0),
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
    fn command_owner_authenticates_only_the_original_country_identity() {
        struct IdentityOnly {
            header: [u8; 16],
            reads: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for IdentityOnly {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                if address != COUNTRY || output.len() != self.header.len() {
                    return false;
                }
                output.copy_from_slice(&self.header);
                true
            }
        }
        let memory = IdentityOnly {
            header: fixture()[COUNTRY..COUNTRY + 16].try_into().unwrap(),
            reads: Default::default(),
        };
        assert_eq!(
            command_owner(&memory, BASE, COUNTRY),
            Ok(Owner {
                pointer: COUNTRY,
                country: 5001,
                instance: 0,
                politics: None,
            })
        );
        assert_eq!(*memory.reads.borrow(), [(COUNTRY, 16)]);
        assert_eq!(
            country_owner(&memory, BASE, COUNTRY),
            Err("politics country identity unreadable")
        );
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
            STATUS + engine_native_parts::cpolitical_status_layout::COUNTRY,
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
            STATUS + engine_native_parts::cpolitical_status_layout::COUNTRY + 7,
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
        put(&mut bytes, PARTY + engine_native_parts::cpolitical_party_layout::ID, 38);
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
}

pub const CHARGING_CALLERS: &[(Rva, &str)] = &[
    (Rva(0x259452), "Console"),
    (Rva(0x2865c2), "Console"),
    (Rva(0x2e77bb), "FocusRefund"),
    (Rva(0x2f387a), "Focus"),
    (Rva(0x3618b2), "ScriptEffect"),
    (Rva(0x363c39), "ScriptEffect"),
    (Rva(0x3645f7), "ScaledScriptEffect"),
    (Rva(0x6bfa05), "Autonomy"),
    (Rva(0x6c1a43), "Autonomy"),
    (Rva(0x71b405), "AirAction"),
    (Rva(0x71b9a9), "AirAction"),
    (Rva(0x732467), "AbilityActivation"),
    (Rva(0x732c4f), "AllocationRefund"),
    (Rva(0x76fe2e), "Decision"),
    (Rva(0x771077), "DecisionTimeout"),
    (Rva(0x77dcd6), "TargetedDecision"),
    (Rva(0xb02d86), "Diplomacy"),
    (Rva(0xc71212), "LeaderAssignment"),
    (Rva(0xc76066), "LeaderAssignment"),
    (Rva(0xc78d1f), "LeaderAssignment"),
    (Rva(0xc78d51), "LeaderAssignment"),
    (Rva(0xd89162), "Diplomacy"),
    (Rva(0xe22d67), "MioPolicy"),
    (Rva(0xfe8302), "AirAction"),
    (Rva(0x100b8a5), "AirAction"),
    (Rva(0x100b9ac), "AirAction"),
    (Rva(0x1052da5), "Idea"),
    (Rva(0x106fe4c), "Raid"),
    (Rva(0x1072856), "Raid"),
    (Rva(0x107286d), "Raid"),
    (Rva(0x1172d61), "AdvisorRemoval"),
    (Rva(0x1189be3), "Diplomacy"),
    (Rva(0x1189c13), "Diplomacy"),
    (Rva(0x118d050), "WarJustification"),
    (Rva(0x118ee10), "JoinWar"),
    (Rva(0x11c6cea), "JoinWar"),
    (Rva(0x11da88c), "Advisor"),
    (Rva(0x11da8e3), "Advisor"),
    (Rva(0x11dac38), "Advisor"),
    (Rva(0x11e0141), "LeaderRecruitment"),
    (Rva(0x11e37dc), "Advisor"),
    (Rva(0x11e3835), "Advisor"),
    (Rva(0x11e69a7), "PreferredTactic"),
    (Rva(0x11e747f), "ScorchedEarth"),
    (Rva(0x11e9981), "OfficerUpgrade"),
    (Rva(0x11e9b5f), "CaptainUpgrade"),
    (Rva(0x13338a7), "Advisor"),
    (Rva(0x133447e), "Advisor"),
    (Rva(0x13344a6), "Advisor"),
    (Rva(0x13eb50f), "PrideOfFleet"),
    (Rva(0x13fd15c), "LeaderPromotion"),
    (Rva(0x14e3569), "Medal"),
    (Rva(0x18d36f2), "LeaderTrait"),
    (Rva(0x18db48b), "LeaderAssignment"),
    (Rva(0x1ab9bc1), "SpyMaster"),
    (Rva(0x1b365dc), "Blockade"),
    (Rva(0x1c36ca8), "ScientistFaction"),
    (Rva(0x1f6b6e0), "ScientistRecruitment"),
];


pub const SCOPES: &[HookSite] = &[
    HookSite {
        name: "PoliticsDaily",
        rva: Rva(0xbfb6a0),
        commit: false,
    },
    HookSite {
        name: "ImproveRelationDaily",
        rva: Rva(0xd85080),
        commit: false,
    },
    HookSite {
        name: "BoostPartyDaily",
        rva: Rva(0xd84a50),
        commit: false,
    },
    HookSite {
        name: "MioResearchDaily",
        rva: Rva(0xe23770),
        commit: false,
    },
    HookSite {
        name: "MioProductionDaily",
        rva: Rva(0xe236d0),
        commit: false,
    },
    HookSite {
        name: "AddIdea",
        rva: Rva(0x11db650),
        commit: false,
    },
    HookSite {
        name: "RemoveIdea",
        rva: Rva(0x11e30b0),
        commit: false,
    },
    HookSite {
        name: "ReplaceIdea",
        rva: Rva(0x11e3920),
        commit: false,
    },
    HookSite {
        name: "HireAdvisor",
        rva: Rva(0x11da7f0),
        commit: false,
    },
    HookSite {
        name: "ReplaceAdvisor",
        rva: Rva(0x11e36d0),
        commit: false,
    },
    HookSite {
        name: "Decision",
        rva: Rva(0x76fc80),
        commit: false,
    },
    HookSite {
        name: "TargetedDecision",
        rva: Rva(0x77dab0),
        commit: false,
    },
    HookSite {
        name: "DecisionTimeout",
        rva: Rva(0x770f00),
        commit: false,
    },
    HookSite {
        name: "Diplomacy",
        rva: Rva(0x1189420),
        commit: false,
    },
    HookSite {
        name: "ScriptedDiplomacy",
        rva: Rva(0xb02ca0),
        commit: false,
    },
    HookSite {
        name: "LeaderPromotion",
        rva: Rva(0x13fd0f0),
        commit: false,
    },
    HookSite {
        name: "LeaderTrait",
        rva: Rva(0x18d36a0),
        commit: false,
    },
    HookSite {
        name: "AbilityCost",
        rva: Rva(0x108fd70),
        commit: false,
    },
    HookSite {
        name: "CoupCompletion",
        rva: Rva(0xb2d600),
        commit: false,
    },
    HookSite {
        name: "CivilWar",
        rva: Rva(0x1165a00),
        commit: false,
    },
    HookSite {
        name: "SetPoliticsEffect",
        rva: Rva(0x37c5f0),
        commit: false,
    },
    HookSite {
        name: "PeaceForceGovernment",
        rva: Rva(0x14f1050),
        commit: false,
    },
    HookSite {
        name: "SubjectGovernment",
        rva: Rva(0xd86710),
        commit: false,
    },
    HookSite {
        name: "ConsoleGovernment",
        rva: Rva(0x29d970),
        commit: false,
    },
    HookSite {
        name: "ConsoleSubject",
        rva: Rva(0x24c3a0),
        commit: false,
    },
    HookSite {
        name: "OnAction",
        rva: Rva(0xacaf60),
        commit: false,
    },
    HookSite {
        name: "Focus",
        rva: Rva(0x2f37d0),
        commit: false,
    },
    HookSite {
        name: "FocusRefund",
        rva: Rva(0x2e7530),
        commit: false,
    },
];

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
pub fn literal(memory: &impl Memory, address: usize) -> Result<Literal, &'static str> {
    let descriptor = msvc_string::Descriptor::read(memory, address)?;
    Ok(Literal {
        address,
        len: descriptor.len,
    })
}
pub fn party_literal(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Option<(u32, Literal)>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let (id, ideology) = engine_native_parts::cpolitical_party_layout::identity(memory, base, pointer)?;
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
                .checked_add(engine_native_parts::cpolitical_status_layout::POLITICAL_POWER)
                .ok_or("PP address overflow")?,
        )
        .ok_or("PP balance unreadable")?;
        let pointer = read::<u64>(
            memory,
            status
                .checked_add(engine_native_parts::cpolitical_status_layout::RULING_PARTY)
                .ok_or("party address overflow")?,
        )
        .ok_or("ruling pointer unreadable")? as usize;
        let party = if pointer == 0 {
            None
        } else {
            Some(engine_native_parts::cpolitical_party_layout::identity(memory, base, pointer)?.0)
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

pub fn blockade_cost(count_register: usize, loaded_define: i64) -> i64 {
    (count_register as i32 as i64)
        .wrapping_mul(loaded_define)
        .wrapping_mul(100_000)
        / 100_000
}
pub const BLOCKADE_COST_DEFINE_RVA: Rva = Rva(0x33e3ab8);

pub const DAILY_RECORDS: usize = 5;

pub fn daily_record_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<usize, &'static str> {
    use engine_native_memory::cpdx_array::Array;
    use crate::nindustrial_organisation_corganisation as organisation;

    let countries = ccountry::Reader::new(memory, base, state)?;
    let headers = countries.arrays();
    let array = |object: usize, offset: usize, stride: usize| {
        let address = object
            .checked_add(offset)
            .ok_or("politics staging roster address overflow")?;
        let header =
            read::<[u8; 16]>(memory, address).ok_or("politics staging roster unreadable")?;
        let array = Array::from_header(header, stride).ok_or("politics staging roster invalid")?;
        if read::<[u8; 16]>(memory, address) != Some(header) {
            return Err("politics staging roster changed");
        }
        Ok(array)
    };
    let organisation_vtable = engine_sites::at(base, organisation::VTABLE_RVA)
        .ok_or("politics staging organisation identity mismatch")?
        as u64;
    let mut records = 0usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let owner = country_owner(memory, base, country.pointer)?;
        let mut count = DAILY_RECORDS;
        if let Some(status) = owner.politics {
            count = count
                .checked_add(array(status, 8, 8)?.count as usize)
                .ok_or("politics staging record demand overflow")?;
        }
        count = count
            .checked_add(
                array(country.pointer, ccountry::ACTIVE_COMMAND_ABILITIES, 0x58)?.count as usize,
            )
            .ok_or("politics staging record demand overflow")?;
        for (offset, vtable, backlink, roster) in [
            (
                ccountry::DIPLOMACY_STATUS,
                engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
                engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
            ),
            (
                ccountry::PRODUCTION_STATUS,
                engine_native::cproduction_status::VTABLE_RVA,
                engine_native::cproduction_status::COUNTRY,
                engine_native::cproduction_status::ORGANISATIONS,
            ),
        ] {
            let pointer = country
                .pointer
                .checked_add(offset)
                .and_then(|address| read::<u64>(memory, address))
                .filter(|pointer| *pointer != 0)
                .ok_or("politics staging status unreadable or null")?
                as usize;
            let vtable = engine_sites::at(base, vtable)
                .ok_or("politics staging status identity mismatch")?;
            if read::<u64>(memory, pointer) != Some(vtable as u64)
                || pointer
                    .checked_add(backlink)
                    .and_then(|address| read::<u64>(memory, address))
                    != Some(country.pointer as u64)
            {
                return Err("politics staging status identity mismatch");
            }
            let roster = array(pointer, roster, 8)?;
            let mut extra = roster.count as usize;
            if offset == ccountry::PRODUCTION_STATUS {
                extra = 0;
                engine_native_memory::cpdx_array::pointers(
                    memory,
                    roster,
                    "politics staging organisation unreadable",
                    |_, pointer| {
                        if pointer == 0
                            || read::<u64>(memory, pointer) != Some(organisation_vtable)
                            || pointer
                                .checked_add(organisation::OWNER)
                                .and_then(|address| read::<u32>(memory, address))
                                != Some(owner.country)
                        {
                            return Err("politics staging organisation identity mismatch");
                        }
                        extra = (array(
                            pointer,
                            organisation::PRODUCTION_LINES,
                            crate::industrial_organisation_line_entry::STRIDE,
                        )?
                        .count as usize)
                            .checked_mul(2)
                            .and_then(|demand| extra.checked_add(demand))
                            .ok_or("politics staging record demand overflow")?;
                        Ok(())
                    },
                )?;
            }
            count = count
                .checked_add(extra)
                .ok_or("politics staging record demand overflow")?;
        }
        records = records
            .checked_add(count)
            .ok_or("politics staging record demand overflow")?;
    }
    if ccountry::Reader::new(memory, base, state)?.arrays() != headers {
        return Err("politics staging country registry changed");
    }
    Ok(records)
}

pub fn lexical_key_demand(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let address = engine_native_memory::cpdx_lexer::witness_address(base)?;
    let header = read::<[u8; 64]>(memory, address).ok_or("lexer witness unreadable")?;
    let witness = engine_native_memory::cpdx_lexer::PreparedWitness::from_header(header)?;
    let mut longest = 0;
    for token in 0..=witness.maximum() {
        let address = witness
            .table()
            .checked_add(
                (token as usize)
                    .checked_mul(engine_native_memory::cpdx_lexer::TOKEN_NAME_STRIDE)
                    .ok_or("lexer key index overflow")?,
            )
            .ok_or("lexer key address overflow")?;
        longest = longest.max(literal(memory, address)?.len);
    }
    if read::<[u8; 64]>(memory, address) != Some(header) {
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
    fn politics_staging_demand_counts_daily_rosters() {
        use engine_native_memory::cpdx_array;
        use crate::nindustrial_organisation_corganisation as organisation;

        for countries in [1, 2] {
            let mut world = ccountry::synthetic::World::new(0x4000_0000, countries, 0x1700);
            for slot in 1..=countries {
                let country = world.object(slot);
                let status = world.alloc(0x110);
                world.u64(country + ccountry::POLITICAL_STATUS, status as u64);
                world.u64(
                    status,
                    (world.module + engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0) as u64,
                );
                world.u64(status + engine_native_parts::cpolitical_status_layout::COUNTRY, country as u64);
                world.array(status + 8, country, 2);
                world.array(country + ccountry::ACTIVE_COMMAND_ABILITIES, country, 3);
                for (offset, vtable, backlink, roster, count) in [
                    (
                        ccountry::DIPLOMACY_STATUS,
                        engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
                        engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                        engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
                        4,
                    ),
                    (
                        ccountry::PRODUCTION_STATUS,
                        engine_native::cproduction_status::VTABLE_RVA,
                        engine_native::cproduction_status::COUNTRY,
                        engine_native::cproduction_status::ORGANISATIONS,
                        1,
                    ),
                ] {
                    let pointer = world.alloc(0x300);
                    world.u64(country + offset, pointer as u64);
                    world.u64(pointer, (world.module + vtable.0) as u64);
                    world.u64(pointer + backlink, country as u64);
                    let data = if offset == ccountry::PRODUCTION_STATUS {
                        let object = world.alloc(0x130);
                        world.u64(object, (world.module + organisation::VTABLE_RVA.0) as u64);
                        world.u32(object + organisation::OWNER, slot);
                        world.array(object + organisation::PRODUCTION_LINES, country, 7);
                        let data = world.alloc(8);
                        world.u64(data, object as u64);
                        data
                    } else {
                        country
                    };
                    world.array(pointer + roster, data, count);
                    world.u32(pointer + roster + cpdx_array::CAPACITY, 10_000);
                }
            }
            assert_eq!(
                daily_record_demand(&world.memory(), world.module, world.game_state()),
                Ok(countries as usize * 28)
            );
            world.u64(world.object(countries) + ccountry::PRODUCTION_STATUS, 0);
            assert_eq!(
                daily_record_demand(&world.memory(), world.module, world.game_state()),
                Err("politics staging status unreadable or null")
            );
        }
    }

    #[test]
    fn every_site_is_unique_including_scoped_native_commands() {
        let mut sites = std::collections::BTreeSet::new();
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
                sites.insert(site.rva),
                "duplicate native hook {}",
                site.name
            );
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

pub struct NativeOperands(engine_native_memory::memory::LiveMemory);
impl NativeOperands {
    /// # Safety
    /// As `LiveMemory::new`: the reads are of foreign game memory under the caller's consistency
    /// protocol, or of immutable synthetic storage. SEH guards only against invalid addresses.
    #[inline(always)]
    pub unsafe fn new() -> Self {
        Self(unsafe { engine_native_memory::memory::LiveMemory::new() })
    }
}
impl Memory for NativeOperands {
    #[inline(always)]
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }
    #[inline(always)]
    fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
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
            read::<i64>(&memory, (&raw const value) as usize),
            Some(value)
        );
        assert_eq!(read::<i64>(&memory, 1), None);
        assert!(!memory.copy(1, &mut [0; 16]));
    }
}
