//! Entry sites of the native operations that join and end wars, start and apply
//! peace conferences, annex and release countries, and the callers that reach
//! them and the sites other families own (the diplomatic action dispatch, civil
//! war start and state owner assignment), each named by the route the replay
//! records ([`Caller::route`] is a variant name of
//! `replay::event_feed_rows::EventFeedRoute`).
//!
//! Every caller list is the complete set of direct calls to its target; a
//! return address outside it is refused by name, never guessed. The diplomatic
//! dispatch and civil war entries are the politics family's patches
//! ([`crate::politics_sites`]), and state owner assignment is the territory
//! family's ([`crate::territory_mutations`]); they forward their callers here.
//! Readers authenticate each object they read and copy only owned integers.

use super::{CountryIdentity, at, identity, pointer, state, value, verify_vtable};
use crate::{
    Memory, ccountry, cdiplomacy_status, cgame_state, crelation, cstate, cwar_relation,
    peace_conference,
};

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

/// Joins a war using diplomacy, ally and enemy pointers, a u32 reason and u8
/// expansion flag, returning the war. The joiner (`diplomacy+0x288`) joins the
/// ally's war against the enemy with the reason, attacker side and enemies:
/// every enemy of the ally's side when expanding, else the one named. Its five
/// callers are below.
pub const JOIN_WAR: Entry = Entry {
    name: "JoinWar",
    rva: 0x00d2_bec0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
    callers: &[
        caller(0x010f_112d, "DeclarationJoin"),
        caller(0x0034_c3af, "AddToWarEffect"),
        caller(0x00d1_3a0b, "SubjectJoinsOverlord"),
        caller(0x00d1_3b7b, "OverlordJoinsSubject"),
        caller(0x010f_308a, "JoinAlly"),
        caller(0x0112_7982, "CallAlly"),
    ],
};

/// Ends a war using its pointer and a u8 truce flag: applies the truce to both
/// statuses, clears the link and rebuilds both endpoints. The war is linked and
/// readable at entry.
pub const END_WAR: Entry = Entry {
    name: "EndWar",
    rva: 0x00d0_8520,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x48,
    ],
    callers: &[
        caller(0x010c_bb48, "CivilWarTransferEnd"),
        caller(0x006c_89da, "AnnexEnd"),
        caller(0x00d1_360e, "SubjectOverlordEnd"),
        caller(0x00d1_3795, "SubjectWarsEnd"),
        caller(0x00d0_a05b, "StatusResetEnd"),
        caller(0x00e2_daa2, "PeaceEnd"),
    ],
};

/// Creates a conference from manager, loser and winner pointers, an i64 factor,
/// two u32 arguments, a u8 argument and a message pointer, returning the
/// conference. Construction stores the loser at `+0x34` and winner at `+0x38`;
/// creation queues it and starts it through [`START_CONFERENCE`] before
/// returning.
pub const CREATE_CONFERENCE: Entry = Entry {
    name: "CreateConference",
    rva: 0x00ba_f880,
    sentinel: [
        0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x10, 0x49, 0x89, 0x6b, 0x18, 0x49, 0x89, 0x73, 0x20,
        0x57,
    ],
    callers: &[
        caller(0x0029_8844, "ConsolePeace"),
        caller(0x0036_6365, "StartPeaceConferenceEffect"),
        caller(0x0036_8b4f, "WhitePeaceEffect"),
        caller(0x006d_6146, "Surrender"),
        caller(0x010f_3e7d, "PeaceProposal"),
    ],
};

/// Starts the conference, called only by [`CREATE_CONFERENCE`]. It can end its
/// wars and annex its losers at once.
pub const START_CONFERENCE: Entry = Entry {
    name: "StartConference",
    rva: 0x00e3_a940,
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
    rva: 0x00e2_d2b0,
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
    rva: 0x006c_73c0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x44, 0x88, 0x4c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x48,
        0x89,
    ],
    callers: &[
        caller(0x010c_a367, "CivilWarAnnex"),
        caller(0x0023_e849, "ConsoleAnnex"),
        caller(0x0023_ea78, "ConsoleAnnexSecond"),
        caller(0x0066_bd43, "Integration"),
        caller(0x013d_98e6, "BorderWarAnnex"),
        caller(0x013d_9a48, "BorderWarAnnexSecond"),
        caller(0x0144_32e6, "PeaceAnnex"),
        caller(0x0034_d38e, "AnnexEffect"),
        caller(0x0031_2625, "TransferStateToAnnex"),
        caller(0x0036_7457, "TransferStateAnnex"),
    ],
};

/// Releases a nation using releaser and target pointers and two u8 arguments,
/// moving each core state of the target through released-state transfer.
pub const RELEASE: Entry = Entry {
    name: "Release",
    rva: 0x006f_fe20,
    sentinel: [
        0x40, 0x56, 0x57, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x48, 0x80, 0x3d, 0xc0, 0x8d,
        0xec,
    ],
    callers: &[
        caller(0x0034_5c1f, "ReleasePuppetEffect"),
        caller(0x0034_597f, "ReleasePuppetOnControlledEffect"),
        caller(0x0034_58f8, "ReleaseEffect"),
        caller(0x0034_58a8, "ReleaseOnControlledEffect"),
        caller(0x0035_c0ca, "ReleaseAutonomyEffect"),
    ],
};

/// The release command's release operation uses releaser and target pointers
/// and a u8 argument, returns a u8 result and follows the same state walk.
pub const RELEASE_COMMAND: Entry = Entry {
    name: "ReleaseCommand",
    rva: 0x006f_ff40,
    sentinel: [
        0x40, 0x56, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x40, 0x80, 0x3d, 0xa2, 0x8c, 0xec, 0x02,
        0x00,
    ],
    callers: &[caller(0x0114_37fb, "ReleaseCommand")],
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
    caller(0x010e_fb80, "DiplomaticCommand"),
    caller(0x0035_3ce9, "DeclareWarEffect"),
    caller(0x010c_a984, "CivilWarDeclaration"),
    caller(0x0035_7d00, "GiveGuaranteeEffect"),
    caller(0x0035_7f53, "GiveMilitaryAccessEffect"),
    caller(0x0035_58e3, "EmbargoEffect"),
    caller(0x0030_4aae, "GiveMarketAccessEffect"),
    caller(0x015f_1930, "OfferJoinFaction"),
    caller(0x0024_3e1c, "ConsoleDiplomacy"),
];

/// The callers of civil-war start.
pub const CIVIL_WAR_CALLERS: [Caller; 4] = [
    caller(0x0036_5eb6, "StartCivilWarEffect"),
    caller(0x00ac_bb86, "Coup"),
    caller(0x0024_29b3, "ConsoleCivilWar"),
    caller(0x0024_2a6b, "ConsoleCivilWarSecond"),
];

/// The callers of state owner assignment, which takes a state pointer, u32
/// owner and two u8 arguments.
pub const STATE_OWNER_CALLERS: [Caller; 20] = [
    caller(0x001f_17ac, "StateOwnerFix"),
    caller(0x006c_822d, "StateAnnex"),
    caller(0x0036_402b, "SetStateOwnerEffect"),
    caller(0x0144_384d, "StatePeaceWinner"),
    caller(0x009c_a7e2, "StateDaily"),
    caller(0x009c_f0a3, "StateRead"),
    caller(0x010c_ac12, "StateCivilWar"),
    caller(0x010c_ae48, "StateCivilWarSecond"),
    caller(0x0028_501a, "ConsoleStateOwner"),
    caller(0x0144_760e, "StatePeaceLiberated"),
    caller(0x0031_258c, "TransferStateToEffect"),
    caller(0x0070_0111, "StateRelease"),
    caller(0x0144_2c6f, "StatePeaceBeneficiary"),
    caller(0x013d_988f, "StateBorderWar"),
    caller(0x013d_99f1, "StateBorderWarSecond"),
    caller(0x0036_73d6, "TransferStateEffect"),
    caller(0x010c_af83, "StateCivilWarTransfer"),
    caller(0x0031_1000, "SetStateOwnerToEffect"),
    caller(0x009d_0bc1, "StateImpassable"),
    caller(0x0158_e92c, "StateHistory"),
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
    pub fn new(memory: &'a M, base: usize) -> Result<Self, &'static str> {
        let state = state(memory, base)?;
        Ok(Self {
            memory,
            base,
            state,
            countries: ccountry::Reader::new(memory, base, state)?,
        })
    }

    /// `CGameState+0x468`, the hour every event feed operand carries.
    pub fn hour(&self) -> Result<i32, &'static str> {
        value(self.memory, self.state, cgame_state::CURRENT_HOUR)
    }

    /// A raw country reference with its resolved full ID.
    pub fn country(&self, reference: u32) -> Result<CountryIdentity, &'static str> {
        Ok(identity(self.memory, self.base, &self.countries, reference)?.1)
    }

    /// The country a `u32` reference pointer names (`ccountry_tag`).
    pub fn country_at(&self, reference: usize) -> Result<CountryIdentity, &'static str> {
        self.country(value(self.memory, reference, 0)?)
    }

    /// A `CCountry*`, which must be its reference's canonical object.
    pub fn country_object(&self, country: usize) -> Result<CountryIdentity, &'static str> {
        verify_vtable(self.memory, self.base, country, ccountry::VTABLE_RVA)?;
        let reference = value(self.memory, country, ccountry::ID)?;
        let (resolved, identity) = identity(self.memory, self.base, &self.countries, reference)?;
        if resolved.pointer != country {
            return Err("event feed country is not its canonical object");
        }
        Ok(identity)
    }

    /// The owner (`+0x288`) of a `CDiplomacyStatus*`.
    pub fn diplomacy_owner(&self, diplomacy: usize) -> Result<CountryIdentity, &'static str> {
        verify_vtable(
            self.memory,
            self.base,
            diplomacy,
            cdiplomacy_status::VTABLE_RVA,
        )?;
        self.country_object(pointer(self.memory, diplomacy, cdiplomacy_status::COUNTRY)?)
    }

    /// A linked war relation's first and second endpoints (`+0x10`/`+0x14`).
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

    /// A conference's PeaceConferences reference (`+0x8`, nonzero) and its
    /// main winner (`+0x38`) and loser (`+0x34`).
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

/// A state's id (`+0x58`) and owner reference (`+0xC8`).
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
