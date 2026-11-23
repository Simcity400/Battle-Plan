
#[cfg(test)]
use super::pointer;
use super::{CountryIdentity, identity, state, value, verify_vtable};
use engine_native_memory::Memory;
use engine_native::{ccountry, cgame_state, crelation, cstate, cwar_relation};
use crate::peace_conference;
use engine_sites::Rva;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub return_rva: Rva,
    pub route: &'static str,
}

const fn caller(call_rva: Rva, route: &'static str) -> Caller {
    Caller {
        return_rva: call_rva.after(5),
        route,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    pub rva: Rva,
    pub callers: &'static [Caller],
}

pub const JOIN_WAR: Entry = Entry {
    name: "JoinWar",
    rva: Rva(0x00da_4e70),
    callers: &[
        caller(Rva(0x0118_c33d), "DeclarationJoin"),
        caller(Rva(0x0036_500f), "AddToWarEffect"),
        caller(Rva(0x00d8_9b5b), "SubjectJoinsOverlord"),
        caller(Rva(0x00d8_9e1b), "OverlordJoinsSubject"),
        caller(Rva(0x0118_e40a), "JoinAlly"),
        caller(Rva(0x011c_61b2), "CallAlly"),
    ],
};




pub const END_WAR: Entry = Entry {
    name: "EndWar",
    rva: Rva(0x00d7_e5c0),
    callers: &[
        caller(Rva(0x0116_88a8), "CivilWarTransferEnd"),
        caller(Rva(0x0071_d06a), "AnnexEnd"),
        caller(Rva(0x00d8_973e), "SubjectOverlordEnd"),
        caller(Rva(0x00d8_98d1), "SubjectWarsEnd"),
        caller(Rva(0x00d8_00fb), "StatusResetEnd"),
        caller(Rva(0x00eb_14d2), "PeaceEnd"),
    ],
};




pub const CREATE_CONFERENCE: Entry = Entry {
    name: "CreateConference",
    rva: Rva(0x00c1_2630),
    callers: &[
        caller(Rva(0x002b_0244), "ConsolePeace"),
        caller(Rva(0x0037_f045), "StartPeaceConferenceEffect"),
        caller(Rva(0x0038_182f), "WhitePeaceEffect"),
        caller(Rva(0x0072_a906), "Surrender"),
        caller(Rva(0x0118_f1fd), "PeaceProposal"),
    ],
};



pub const START_CONFERENCE: Entry = Entry {
    name: "StartConference",
    rva: Rva(0x00eb_e3e0),
    callers: &[],
};

pub const APPLY_CONFERENCE: Entry = Entry {
    name: "ApplyConference",
    rva: Rva(0x00eb_0ce0),
    callers: &[],
};

pub const ANNEX: Entry = Entry {
    name: "Annex",
    rva: Rva(0x0071_ba50),
    callers: &[
        caller(Rva(0x0116_70c7), "CivilWarAnnex"),
        caller(Rva(0x0025_6319), "ConsoleAnnex"),
        caller(Rva(0x0025_6548), "ConsoleAnnexSecond"),
        caller(Rva(0x006b_fe63), "Integration"),
        caller(Rva(0x0148_a9aa), "BorderWarAnnex"),
        caller(Rva(0x0148_ab0c), "BorderWarAnnexSecond"),
        caller(Rva(0x014f_1ae6), "PeaceAnnex"),
        caller(Rva(0x0036_5fee), "AnnexEffect"),
        caller(Rva(0x0032_b195), "TransferStateToAnnex"),
        caller(Rva(0x0038_0137), "TransferStateAnnex"),
    ],
};



pub const RELEASE: Entry = Entry {
    name: "Release",
    rva: Rva(0x0075_4b10),
    callers: &[
        caller(Rva(0x0035_e88f), "ReleasePuppetEffect"),
        caller(Rva(0x0035_e5ef), "ReleasePuppetOnControlledEffect"),
        caller(Rva(0x0035_e568), "ReleaseEffect"),
        caller(Rva(0x0035_e518), "ReleaseOnControlledEffect"),
        caller(Rva(0x0037_4d1a), "ReleaseAutonomyEffect"),
    ],
};



pub const RELEASE_COMMAND: Entry = Entry {
    name: "ReleaseCommand",
    rva: Rva(0x0075_4c30),
    callers: &[caller(Rva(0x011e_22c9), "ReleaseCommand")],
};

pub const ENTRIES: [Entry; 8] = [
    JOIN_WAR,
    END_WAR,
    CREATE_CONFERENCE,
    START_CONFERENCE,
    APPLY_CONFERENCE,
    ANNEX,
    RELEASE,
    RELEASE_COMMAND,
];

pub const DIPLOMACY_CALLERS: [Caller; 9] = [
    caller(Rva(0x0118_acb7), "DiplomaticCommand"),
    caller(Rva(0x0036_c949), "DeclareWarEffect"),
    caller(Rva(0x0116_76e4), "CivilWarDeclaration"),
    caller(Rva(0x0037_0950), "GiveGuaranteeEffect"),
    caller(Rva(0x0037_0ba3), "GiveMilitaryAccessEffect"),
    caller(Rva(0x0036_e543), "EmbargoEffect"),
    caller(Rva(0x0031_d743), "GiveMarketAccessEffect"),
    caller(Rva(0x0169_c8f0), "OfferJoinFaction"),
    caller(Rva(0x0025_b8d3), "ConsoleDiplomacy"),
];

pub const CIVIL_WAR_CALLERS: [Caller; 4] = [
    caller(Rva(0x0037_eb96), "StartCivilWarEffect"),
    caller(Rva(0x00b2_d936), "Coup"),
    caller(Rva(0x0025_a483), "ConsoleCivilWar"),
    caller(Rva(0x0025_a53b), "ConsoleCivilWarSecond"),
];

pub const STATE_OWNER_CALLERS: [Caller; 20] = [
    caller(Rva(0x0020_7cbc), "StateOwnerFix"),
    caller(Rva(0x0071_c8bd), "StateAnnex"),
    caller(Rva(0x0037_cc9b), "SetStateOwnerEffect"),
    caller(Rva(0x014f_204d), "StatePeaceWinner"),
    caller(Rva(0x00a2_2362), "StateDaily"),
    caller(Rva(0x00a2_6c24), "StateRead"),
    caller(Rva(0x0116_7972), "StateCivilWar"),
    caller(Rva(0x0116_7ba8), "StateCivilWarSecond"),
    caller(Rva(0x0029_ca2a), "ConsoleStateOwner"),
    caller(Rva(0x014f_5d1e), "StatePeaceLiberated"),
    caller(Rva(0x0032_b0fc), "TransferStateToEffect"),
    caller(Rva(0x0075_4e01), "StateRelease"),
    caller(Rva(0x014f_146f), "StatePeaceBeneficiary"),
    caller(Rva(0x0148_a953), "StateBorderWar"),
    caller(Rva(0x0148_aab5), "StateBorderWarSecond"),
    caller(Rva(0x0038_00b6), "TransferStateEffect"),
    caller(Rva(0x0116_7ce3), "StateCivilWarTransfer"),
    caller(Rva(0x0032_9b80), "SetStateOwnerToEffect"),
    caller(Rva(0x00a2_8741), "StateImpassable"),
    caller(Rva(0x0163_d5ec), "StateHistory"),
];

pub fn route(base: usize, callers: &[Caller], return_address: usize) -> Option<&'static str> {
    let Some(key) = engine_sites::reference(base, return_address) else {
        for caller in callers {
            if engine_sites::at(base, caller.return_rva).is_none() {
                break;
            }
        }
        return None;
    };
    callers
        .iter()
        .find(|caller| caller.return_rva == key)
        .map(|caller| caller.route)
}

pub struct World<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    countries: ccountry::Reader<'a, M>,
}

impl<'a, M: Memory> World<'a, M> {
    pub fn state_pointer(&self) -> usize {
        self.state
    }
    pub fn new(memory: &'a M, base: usize) -> Result<Self, &'static str> {
        let state = state(memory, base)?;
        Ok(Self {
            memory,
            base,
            state,
            countries: ccountry::Reader::new(memory, base, state)?,
        })
    }

    pub fn hour(&self) -> Result<i32, &'static str> {
        value(self.memory, self.state, cgame_state::CURRENT_HOUR)
    }

    pub fn country(&self, reference: u32) -> Result<CountryIdentity, &'static str> {
        Ok(identity(self.memory, self.base, &self.countries, reference)?.1)
    }

    pub fn country_and_object(
        &self,
        reference: u32,
    ) -> Result<(CountryIdentity, usize), &'static str> {
        let (country, identity) = identity(self.memory, self.base, &self.countries, reference)?;
        Ok((identity, country.pointer))
    }

    pub fn country_at(&self, reference: usize) -> Result<CountryIdentity, &'static str> {
        self.country(value(self.memory, reference, 0)?)
    }

    pub fn country_object(&self, country: usize) -> Result<CountryIdentity, &'static str> {
        Ok(self.country_record(country)?.1)
    }

    pub fn country_record(
        &self,
        country: usize,
    ) -> Result<(engine_native_memory::country_identity::Country, CountryIdentity), &'static str> {
        verify_vtable(self.memory, self.base, country, ccountry::VTABLE_RVA)?;
        let reference = value(self.memory, country, ccountry::ID)?;
        let (resolved, identity) = identity(self.memory, self.base, &self.countries, reference)?;
        if resolved.pointer != country {
            return Err("event feed country is not its canonical object");
        }
        Ok((resolved, identity))
    }

    #[cfg(test)]
    pub fn diplomacy_owner(&self, diplomacy: usize) -> Result<CountryIdentity, &'static str> {
        verify_vtable(
            self.memory,
            self.base,
            diplomacy,
            engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
        )?;
        self.country_object(pointer(self.memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)?)
    }

    pub fn war_endpoints(&self, war: usize) -> Result<[CountryIdentity; 2], &'static str> {
        verify_vtable(self.memory, self.base, war, cwar_relation::VTABLE_RVA)?;
        if value::<u32>(self.memory, war, crelation::TYPE_ID)? != cwar_relation::TYPE {
            return Err("event feed war type mismatch");
        }
        Ok([
            self.country(value(self.memory, war, crelation::FIRST_COUNTRY)?)?,
            self.country(value(self.memory, war, crelation::SECOND_COUNTRY)?)?,
        ])
    }

    pub fn conference(
        &self,
        conference: usize,
    ) -> Result<(u64, [CountryIdentity; 2]), &'static str> {
        verify_vtable(
            self.memory,
            self.base,
            conference,
            peace_conference::VTABLE_RVA,
        )?;
        let reference = conference_reference(self.memory, self.base, conference)?;
        Ok((
            reference,
            [
                self.country(value(self.memory, conference, MAIN_WINNER)?)?,
                self.country(value(self.memory, conference, MAIN_LOSER)?)?,
            ],
        ))
    }
}

pub const MAIN_LOSER: usize = 0x34;
pub const MAIN_WINNER: usize = 0x38;

pub fn conference_reference(
    memory: &impl Memory,
    base: usize,
    conference: usize,
) -> Result<u64, &'static str> {
    verify_vtable(memory, base, conference, peace_conference::VTABLE_RVA)?;
    match value::<u64>(memory, conference, peace_conference::REFERENCE)? {
        0 => Err("event feed conference reference is null"),
        reference => Ok(reference),
    }
}

pub fn state_owner(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(u32, u32), &'static str> {
    verify_vtable(memory, base, state, cstate::VTABLE_RVA)?;
    Ok((
        value(memory, state, cstate::ID)?,
        value(memory, state, cstate::OWNER)?,
    ))
}
