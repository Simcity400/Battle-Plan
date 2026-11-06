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
use super::{CountryIdentity, identity, state, value, verify_vtable};
#[cfg(test)]
use crate::cdiplomacy_status;
use crate::{Memory, ccountry, cgame_state, crelation, cstate, cwar_relation, peace_conference};
use engine_sites::Rva;

/// One call site reaching an observed operation: the native return address
/// its CALL pushes, and the replay route that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub return_rva: Rva,
    pub route: &'static str,
}

const fn caller(call_rva: Rva, route: &'static str) -> Caller {
    // Every listed CALL is the five-byte E8 rel32 form.
    Caller {
        return_rva: call_rva.after(5),
        route,
    }
}

/// A function entry the event feed detours. Each keeps its native arguments
/// (at most eight: RCX, RDX, R8, R9 and four stack slots) and returns RAX
/// unchanged from its original.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    pub rva: Rva,
    /// Every caller, or empty when the entry names its route itself.
    pub callers: &'static [Caller],
}

/// Joins a war using diplomacy, ally and enemy pointers, a u32 reason and u8 expansion flag,
/// returning the war. The joiner (the diplomacy status's country) joins the ally's war against the
/// enemy with the reason, attacker side and enemies: every enemy of the ally's side when expanding,
/// else the one named. Its five callers are below.
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

/// Ends a war using its pointer and a u8 truce flag: applies the truce to both
/// statuses, clears the link and rebuilds both endpoints. The war is linked and
/// readable at entry.
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

/// Creates a conference from manager, loser and winner pointers, an i64 factor, two u32 arguments,
/// a u8 argument and a message pointer, returning the conference. Construction stores the loser and
/// then the winner; creation queues it and starts it through [`START_CONFERENCE`] before returning.
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

/// Starts the conference, called only by [`CREATE_CONFERENCE`]. It can end its
/// wars and annex its losers at once.
pub const START_CONFERENCE: Entry = Entry {
    name: "StartConference",
    rva: Rva(0x00eb_e3e0),
    callers: &[],
};

/// Applies a conference's outcome from its pointer and a u8 argument, ending
/// its wars and annexing losers left without states. Reached from the
/// conference queue, turn processing and a tail jump from conference
/// processing, so it is one route.
pub const APPLY_CONFERENCE: Entry = Entry {
    name: "ApplyConference",
    rva: Rva(0x00eb_0ce0),
    callers: &[],
};

/// Annexes a target using the taker pointer, u32 target, i64 output pointer, u8
/// troop-transfer flag and another u8 argument. It moves the target's states
/// and ends its wars before the `on_annex` dispatch observed by the event
/// feed's [`super::ANNEX`] call.
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

/// Releases a nation using releaser and target pointers and two u8 arguments,
/// moving each core state of the target through released-state transfer.
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

/// The release command's release operation uses releaser and target pointers
/// and a u8 argument, returns a u8 result and follows the same state walk.
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

/// The callers of diplomatic action dispatch, which takes an action pointer.
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

/// The callers of civil-war start.
pub const CIVIL_WAR_CALLERS: [Caller; 4] = [
    caller(Rva(0x0037_eb96), "StartCivilWarEffect"),
    caller(Rva(0x00b2_d936), "Coup"),
    caller(Rva(0x0025_a483), "ConsoleCivilWar"),
    caller(Rva(0x0025_a53b), "ConsoleCivilWarSecond"),
];

/// The callers of state owner assignment, which takes a state pointer, u32
/// owner and two u8 arguments.
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

/// The route of a call returning to the live `return_address`, if `callers` lists it. Outside every
/// located function a withheld caller cannot be ruled out, so the read scope is tainted.
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
