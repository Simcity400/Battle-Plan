//! Entry sites of the native operations that join and end wars, start and apply
//! peace conferences, annex and release countries, and the callers that reach
//! them and the sites other families own (the diplomatic action dispatch, civil
//! war start and state owner assignment), each named by the route the replay
//! records ([`Caller::route`] is a variant name of
//! `replay_data::event_feed_rows::EventFeedRoute`).
//!
//! Every caller list is the complete set of direct calls to its target; a
//! return address outside it is refused by name, never guessed. The diplomatic
//! dispatch and civil war entries are the politics family's patches
//! ([`crate::politics_sites`]), and state owner assignment is the territory
//! family's (`territory_mutations`); they forward their callers here.
//! Readers authenticate each object they read and copy only owned integers.

#[cfg(test)]
use super::pointer;
use super::{CountryIdentity, at, identity, state, value, verify_vtable};
#[cfg(test)]
use crate::cdiplomacy_status;
use crate::{Memory, ccountry, cgame_state, crelation, cstate, cwar_relation, peace_conference};

/// One call site reaching an observed operation: the native return address
/// its CALL pushes, and the replay route that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub return_rva: usize,
    pub route: &'static str,
}

const fn caller(call_rva: usize, route: &'static str) -> Caller {
    // Every listed CALL is the five-byte E8 rel32 form.
    Caller {
        return_rva: call_rva + 5,
        route,
    }
}

/// A function entry the event feed detours. Each keeps its native arguments
/// (at most eight: RCX, RDX, R8, R9 and four stack slots) and returns RAX
/// unchanged from its original.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
    /// Every caller, or empty when the entry names its route itself.
    pub callers: &'static [Caller],
}

/// Joins a war using diplomacy, ally and enemy pointers, a u32 reason and u8 expansion flag,
/// returning the war. The joiner (the diplomacy status's country) joins the ally's war against the
/// enemy with the reason, attacker side and enemies: every enemy of the ally's side when expanding,
/// else the one named. Its five callers are below.
pub const JOIN_WAR: Entry = Entry {
    name: "JoinWar",
    rva: 0x00da_4e00,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
    callers: &[
        caller(0x0118_c2cd, "DeclarationJoin"),
        caller(0x0036_4f9f, "AddToWarEffect"),
        caller(0x00d8_9aeb, "SubjectJoinsOverlord"),
        caller(0x00d8_9dab, "OverlordJoinsSubject"),
        caller(0x0118_e39a, "JoinAlly"),
        caller(0x011c_6142, "CallAlly"),
    ],
};

/// Ends a war using its pointer and a u8 truce flag: applies the truce to both
/// statuses, clears the link and rebuilds both endpoints. The war is linked and
/// readable at entry.
pub const END_WAR: Entry = Entry {
    name: "EndWar",
    rva: 0x00d7_e550,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x48,
    ],
    callers: &[
        caller(0x0116_8838, "CivilWarTransferEnd"),
        caller(0x0071_cffa, "AnnexEnd"),
        caller(0x00d8_96ce, "SubjectOverlordEnd"),
        caller(0x00d8_9861, "SubjectWarsEnd"),
        caller(0x00d8_008b, "StatusResetEnd"),
        caller(0x00eb_1462, "PeaceEnd"),
    ],
};

/// Creates a conference from manager, loser and winner pointers, an i64 factor, two u32 arguments,
/// a u8 argument and a message pointer, returning the conference. Construction stores the loser and
/// then the winner; creation queues it and starts it through [`START_CONFERENCE`] before returning.
pub const CREATE_CONFERENCE: Entry = Entry {
    name: "CreateConference",
    rva: 0x00c1_25c0,
    sentinel: [
        0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x10, 0x49, 0x89, 0x6b, 0x18, 0x49, 0x89, 0x73, 0x20,
        0x57,
    ],
    callers: &[
        caller(0x002b_01d4, "ConsolePeace"),
        caller(0x0037_efd5, "StartPeaceConferenceEffect"),
        caller(0x0038_17bf, "WhitePeaceEffect"),
        caller(0x0072_a896, "Surrender"),
        caller(0x0118_f18d, "PeaceProposal"),
    ],
};

/// Starts the conference, called only by [`CREATE_CONFERENCE`]. It can end its
/// wars and annex its losers at once.
pub const START_CONFERENCE: Entry = Entry {
    name: "StartConference",
    rva: 0x00eb_e370,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
    callers: &[],
};

/// Applies a conference's outcome from its pointer and a u8 argument, ending
/// its wars and annexing losers left without states. Reached from the
/// conference queue, turn processing and a tail jump from conference
/// processing, so it is one route.
pub const APPLY_CONFERENCE: Entry = Entry {
    name: "ApplyConference",
    rva: 0x00eb_0c70,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54, 0x41,
        0x56,
    ],
    callers: &[],
};

/// Annexes a target using the taker pointer, u32 target, i64 output pointer, u8
/// troop-transfer flag and another u8 argument. It moves the target's states
/// and ends its wars before the `on_annex` dispatch observed by the event
/// feed's [`super::ANNEX`] call.
pub const ANNEX: Entry = Entry {
    name: "Annex",
    rva: 0x0071_b9e0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x44, 0x88, 0x4c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x48,
        0x89,
    ],
    callers: &[
        caller(0x0116_7057, "CivilWarAnnex"),
        caller(0x0025_62a9, "ConsoleAnnex"),
        caller(0x0025_64d8, "ConsoleAnnexSecond"),
        caller(0x006b_fdf3, "Integration"),
        caller(0x0148_a93a, "BorderWarAnnex"),
        caller(0x0148_aa9c, "BorderWarAnnexSecond"),
        caller(0x014f_1a76, "PeaceAnnex"),
        caller(0x0036_5f7e, "AnnexEffect"),
        caller(0x0032_b125, "TransferStateToAnnex"),
        caller(0x0038_00c7, "TransferStateAnnex"),
    ],
};

/// Releases a nation using releaser and target pointers and two u8 arguments,
/// moving each core state of the target through released-state transfer.
pub const RELEASE: Entry = Entry {
    name: "Release",
    rva: 0x0075_4aa0,
    sentinel: [
        0x40, 0x56, 0x57, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x48, 0x80, 0x3d, 0x5f, 0xab,
        0xf3,
    ],
    callers: &[
        caller(0x0035_e81f, "ReleasePuppetEffect"),
        caller(0x0035_e57f, "ReleasePuppetOnControlledEffect"),
        caller(0x0035_e4f8, "ReleaseEffect"),
        caller(0x0035_e4a8, "ReleaseOnControlledEffect"),
        caller(0x0037_4caa, "ReleaseAutonomyEffect"),
    ],
};

/// The release command's release operation uses releaser and target pointers
/// and a u8 argument, returns a u8 result and follows the same state walk.
pub const RELEASE_COMMAND: Entry = Entry {
    name: "ReleaseCommand",
    rva: 0x0075_4bc0,
    sentinel: [
        0x40, 0x56, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x40, 0x80, 0x3d, 0x41, 0xaa, 0xf3, 0x02,
        0x00,
    ],
    callers: &[caller(0x011e_2259, "ReleaseCommand")],
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

/// The callers of diplomatic action dispatch, which takes an action pointer.
pub const DIPLOMACY_CALLERS: [Caller; 9] = [
    caller(0x0118_ac47, "DiplomaticCommand"),
    caller(0x0036_c8d9, "DeclareWarEffect"),
    caller(0x0116_7674, "CivilWarDeclaration"),
    caller(0x0037_08e0, "GiveGuaranteeEffect"),
    caller(0x0037_0b33, "GiveMilitaryAccessEffect"),
    caller(0x0036_e4d3, "EmbargoEffect"),
    caller(0x0031_d6d3, "GiveMarketAccessEffect"),
    caller(0x0169_c880, "OfferJoinFaction"),
    caller(0x0025_b863, "ConsoleDiplomacy"),
];

/// The callers of civil-war start.
pub const CIVIL_WAR_CALLERS: [Caller; 4] = [
    caller(0x0037_eb26, "StartCivilWarEffect"),
    caller(0x00b2_d8c6, "Coup"),
    caller(0x0025_a413, "ConsoleCivilWar"),
    caller(0x0025_a4cb, "ConsoleCivilWarSecond"),
];

/// The callers of state owner assignment, which takes a state pointer, u32
/// owner and two u8 arguments.
pub const STATE_OWNER_CALLERS: [Caller; 20] = [
    caller(0x0020_7c4c, "StateOwnerFix"),
    caller(0x0071_c84d, "StateAnnex"),
    caller(0x0037_cc2b, "SetStateOwnerEffect"),
    caller(0x014f_1fdd, "StatePeaceWinner"),
    caller(0x00a2_22f2, "StateDaily"),
    caller(0x00a2_6bb4, "StateRead"),
    caller(0x0116_7902, "StateCivilWar"),
    caller(0x0116_7b38, "StateCivilWarSecond"),
    caller(0x0029_c9ba, "ConsoleStateOwner"),
    caller(0x014f_5cae, "StatePeaceLiberated"),
    caller(0x0032_b08c, "TransferStateToEffect"),
    caller(0x0075_4d91, "StateRelease"),
    caller(0x014f_13ff, "StatePeaceBeneficiary"),
    caller(0x0148_a8e3, "StateBorderWar"),
    caller(0x0148_aa45, "StateBorderWarSecond"),
    caller(0x0038_0046, "TransferStateEffect"),
    caller(0x0116_7c73, "StateCivilWarTransfer"),
    caller(0x0032_9b10, "SetStateOwnerToEffect"),
    caller(0x00a2_86d1, "StateImpassable"),
    caller(0x0163_d57c, "StateHistory"),
];

/// The route of a call returning to `return_rva`, if `callers` lists it.
pub fn route(callers: &[Caller], return_rva: usize) -> Option<&'static str> {
    callers
        .iter()
        .find(|caller| caller.return_rva == return_rva)
        .map(|caller| caller.route)
}

/// Verify an entry's sentinel before publication, alongside
/// `build::PINNED_BUILD`; a sentinel is not installation or unwind approval.
pub fn verify(memory: &impl Memory, base: usize, entry: Entry) -> Result<usize, &'static str> {
    let address = at(base, entry.rva)?;
    if crate::read::<[u8; 16]>(memory, address) != Some(entry.sentinel) {
        return Err("event feed entry sentinel unreadable or differs");
    }
    Ok(address)
}

/// The current world's countries, read once per observation.
pub struct World<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    countries: ccountry::Reader<'a, M>,
}

impl<'a, M: Memory> World<'a, M> {
    /// Acquisition-only authenticated world identity, never a replay key.
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

    /// `cgame_state::CURRENT_HOUR`, the hour every event feed operand carries.
    pub fn hour(&self) -> Result<i32, &'static str> {
        value(self.memory, self.state, cgame_state::CURRENT_HOUR)
    }

    /// A raw country reference with its resolved full ID.
    pub fn country(&self, reference: u32) -> Result<CountryIdentity, &'static str> {
        Ok(identity(self.memory, self.base, &self.countries, reference)?.1)
    }

    /// [`Self::country`] with the address of the native country object it resolved to.
    pub fn country_and_object(
        &self,
        reference: u32,
    ) -> Result<(CountryIdentity, usize), &'static str> {
        let (country, identity) = identity(self.memory, self.base, &self.countries, reference)?;
        Ok((identity, country.pointer))
    }

    /// The country a `u32` reference pointer names.
    pub fn country_at(&self, reference: usize) -> Result<CountryIdentity, &'static str> {
        self.country(value(self.memory, reference, 0)?)
    }

    /// A `CCountry*`, which must be its reference's canonical object.
    pub fn country_object(&self, country: usize) -> Result<CountryIdentity, &'static str> {
        Ok(self.country_record(country)?.1)
    }

    /// [`Self::country_object`] with the canonical census record it resolved
    /// to, so a caller can key per-country state by its census slot without
    /// another read.
    pub fn country_record(
        &self,
        country: usize,
    ) -> Result<(ccountry::Country, CountryIdentity), &'static str> {
        verify_vtable(self.memory, self.base, country, ccountry::VTABLE_RVA)?;
        let reference = value(self.memory, country, ccountry::ID)?;
        let (resolved, identity) = identity(self.memory, self.base, &self.countries, reference)?;
        if resolved.pointer != country {
            return Err("event feed country is not its canonical object");
        }
        Ok((resolved, identity))
    }

    /// The owner of a `CDiplomacyStatus*`.
    #[cfg(test)]
    pub fn diplomacy_owner(&self, diplomacy: usize) -> Result<CountryIdentity, &'static str> {
        verify_vtable(
            self.memory,
            self.base,
            diplomacy,
            cdiplomacy_status::VTABLE_RVA,
        )?;
        self.country_object(pointer(self.memory, diplomacy, cdiplomacy_status::COUNTRY)?)
    }

    /// A linked war relation's first and second endpoints.
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

    /// A conference's PeaceConferences reference (nonzero) and its main winner and loser.
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

/// Conference construction stores the creation's loser here.
pub const MAIN_LOSER: usize = 0x34;
/// Conference construction stores the creation's winner here.
pub const MAIN_WINNER: usize = 0x38;

/// A conference's PeaceConferences reference, without a country read.
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

/// A state's id and owner reference.
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
