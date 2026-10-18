//! Owned simulation-event history and interval coverage. Operation hours filter history; supplying
//! hours govern its first visibility. The country instance word is the native word after the id,
//! never a writer generation.

use crate::{
    codec::{Decoder, invalid_static as invalid},
    rows::Recorded,
    value::{Value, value_struct},
};
use engine_allocation as allocation;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventFeedSource {
    Declaration = 0,
    Join = 1,
    CivilWarWarTransfer = 2,
    Annex = 3,
    /// A war relation ended: actor and target are its first and second endpoints.
    WarEnded = 4,
    /// A country released another through the release helper or release command:
    /// actor releaser, target released.
    Release = 5,
    /// A state's owner changed: actor the new owner, target the former one,
    /// the state in the cause.
    StateTransfer = 6,
    /// A peace conference started: actor the winner, target the loser
    /// the creation named.
    PeaceConference = 7,
}

impl Value for EventFeedSource {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (*self as u8).encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match u8::decode(input)? {
            0 => Ok(Self::Declaration),
            1 => Ok(Self::Join),
            2 => Ok(Self::CivilWarWarTransfer),
            3 => Ok(Self::Annex),
            4 => Ok(Self::WarEnded),
            5 => Ok(Self::Release),
            6 => Ok(Self::StateTransfer),
            7 => Ok(Self::PeaceConference),
            _ => Err(invalid("unknown Event feed source")),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventFeedCoverage {
    #[default]
    Unavailable = 0,
    Partial = 1,
    Complete = 2,
}

impl Value for EventFeedCoverage {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (*self as u8).encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match u8::decode(input)? {
            0 => Ok(Self::Unavailable),
            1 => Ok(Self::Partial),
            2 => Ok(Self::Complete),
            _ => Err(invalid("unknown Event feed coverage")),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedCountry {
    pub reference: u32,
    pub id: u32,
    pub instance: u32,
}
value_struct!(EventFeedCountry {
    reference: u32,
    id: u32,
    instance: u32
});

macro_rules! routes {
    ($($(#[$doc:meta])* $name:ident = $value:literal,)*) => {
        /// The native operation an event happened in, named by the call that
        /// reached it: a value is one call site of the pinned executable,
        /// whose return address the collector matches exactly
        /// ([`engine_events::event_feed_sites::provenance`] names each by this
        /// variant's name), except a conference's application and the
        /// politics scopes (90 to 94), which name the operation whichever
        /// call reached it. One operation's routes share a value range.
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum EventFeedRoute {
            /// No observed operation was running.
            #[default]
            None = 0,
            $($(#[$doc])* $name = $value,)*
        }

        impl EventFeedRoute {
            pub const ALL: [Self; 1 + [$(stringify!($name)),*].len()] =
                [Self::None, $(Self::$name),*];

            /// The route a caller table names.
            pub fn named(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($name) => Some(Self::$name),)*
                    _ => None,
                }
            }

            pub fn name(self) -> &'static str {
                match self {
                    Self::None => "None",
                    $(Self::$name => stringify!($name),)*
                }
            }
        }

        impl Value for EventFeedRoute {
            fn heap_bytes(&self) -> usize { 0 }
            const MINIMUM_ENCODED_BYTES: usize = 1;
            fn encoded_bytes(&self) -> io::Result<usize> {
                Ok(1)
            }
            fn try_clone(&self) -> io::Result<Self> {
                Ok(*self)
            }
            fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
                (*self as u8).encode_reserved(output)
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
                match u8::decode(input)? {
                    0 => Ok(Self::None),
                    $($value => Ok(Self::$name),)*
                    _ => Err(invalid("unknown Event feed route")),
                }
            }
        }
    };
}

routes! {
    // Diplomatic action dispatch, by caller.
    /// A player or AI diplomatic command.
    DiplomaticCommand = 1,
    /// A scripted war declaration.
    DeclareWarEffect = 2,
    /// A civil war's declaration.
    CivilWarDeclaration = 3,
    /// A scripted guarantee.
    GiveGuaranteeEffect = 4,
    /// A scripted grant of military access.
    GiveMilitaryAccessEffect = 5,
    /// An embargo effect.
    EmbargoEffect = 6,
    /// A scripted grant of market access.
    GiveMarketAccessEffect = 7,
    /// An offer to join a faction.
    OfferJoinFaction = 8,
    /// A console diplomacy command.
    ConsoleDiplomacy = 9,
    // Starting a civil war, by caller.
    /// A scripted civil war.
    StartCivilWarEffect = 10,
    /// A completed coup.
    Coup = 11,
    /// The civil war console command.
    ConsoleCivilWar = 12,
    /// The civil war console command's other call.
    ConsoleCivilWarSecond = 13,
    // Joining a war, by caller.
    /// A subject joining its overlord's declaration, reason 1.
    DeclarationJoin = 20,
    /// A scripted addition to a war.
    AddToWarEffect = 21,
    /// A new subject joining its overlord's wars during subject relation
    /// application, reason 1.
    SubjectJoinsOverlord = 22,
    /// An overlord joining its new subject's wars, reason 2.
    OverlordJoinsSubject = 23,
    /// An accepted request to join an ally's war.
    JoinAlly = 24,
    /// A call to arms, reason 4 with a guarantee, else 2.
    CallAlly = 25,
    // Ending a war relation, by caller.
    /// A civil war's transfer of the old country's wars.
    CivilWarTransferEnd = 30,
    /// An annexation.
    AnnexEnd = 31,
    /// A subject relation application ending the war between overlord and
    /// subject.
    SubjectOverlordEnd = 32,
    /// A subject relation application ending the subject's other wars, as its relation's two
    /// war-ending bytes select.
    SubjectWarsEnd = 33,
    /// A relation status reset.
    StatusResetEnd = 34,
    /// A peace conference ending its wars.
    PeaceEnd = 35,
    // Creating a peace conference, by caller, or applying one.
    /// The peace console command.
    ConsolePeace = 40,
    /// A scripted peace conference start.
    StartPeaceConferenceEffect = 41,
    /// A scripted white peace.
    WhitePeaceEffect = 42,
    /// A surrender during the country's daily update.
    Surrender = 43,
    /// An accepted peace proposal.
    PeaceProposal = 44,
    /// A conference's application, whichever call reached it.
    PeaceApplication = 45,
    // Annexing a country, by caller.
    /// A civil war's instant resolution.
    CivilWarAnnex = 50,
    /// The annex console command.
    ConsoleAnnex = 51,
    /// The annex console command's other call.
    ConsoleAnnexSecond = 52,
    /// Integration at the lowest autonomy.
    Integration = 53,
    /// A border war's end.
    BorderWarAnnex = 54,
    /// A border war's end, its other call.
    BorderWarAnnexSecond = 55,
    /// A peace conference's cleanup.
    PeaceAnnex = 56,
    /// A scripted country annexation.
    AnnexEffect = 57,
    /// Annexation through the effect transferring a state to a country.
    TransferStateToAnnex = 58,
    /// Annexation through the state-transfer effect.
    TransferStateAnnex = 59,
    // Releasing a country, through the release helper or command helper.
    /// A scripted puppet release.
    ReleasePuppetEffect = 60,
    /// A scripted puppet release on controlled territory.
    ReleasePuppetOnControlledEffect = 61,
    /// A scripted country release.
    ReleaseEffect = 62,
    /// A scripted country release on controlled territory.
    ReleaseOnControlledEffect = 63,
    /// A scripted release with autonomy.
    ReleaseAutonomyEffect = 64,
    /// The country release command.
    ReleaseCommand = 65,
    // Setting a state's owner, by caller.
    /// The owner and controller consistency fix during in-game initialization.
    StateOwnerFix = 70,
    /// An annexation.
    StateAnnex = 71,
    /// The state-owner effect.
    SetStateOwnerEffect = 72,
    /// A peace conference winner.
    StatePeaceWinner = 73,
    /// The daily state pass during the daily game update.
    StateDaily = 74,
    /// The state reader.
    StateRead = 75,
    /// A civil war's state split.
    StateCivilWar = 76,
    /// A civil war's state split, its other call.
    StateCivilWarSecond = 77,
    /// The state owner console command.
    ConsoleStateOwner = 78,
    /// A country a peace conference liberates during participant-country
    /// creation.
    StatePeaceLiberated = 79,
    /// The effect transferring a state to a country.
    TransferStateToEffect = 80,
    /// A release transferring a released state.
    StateRelease = 81,
    /// A peace conference beneficiary.
    StatePeaceBeneficiary = 82,
    /// A border war's end.
    StateBorderWar = 83,
    /// A border war's end, its other call.
    StateBorderWarSecond = 84,
    /// The state-transfer effect.
    TransferStateEffect = 85,
    /// A civil war's state transfer.
    StateCivilWarTransfer = 86,
    /// An effect changing state ownership.
    SetStateOwnerToEffect = 87,
    /// An impassable state following its neighbours in its owner and controller
    /// update.
    StateImpassable = 88,
    /// A history owner change.
    StateHistory = 89,
    // Script the politics family's scopes run, whichever call reached them.
    /// A decision's execution.
    Decision = 90,
    /// A targeted decision's execution.
    TargetedDecision = 91,
    /// A decision's timeout.
    DecisionTimeout = 92,
    /// A scripted diplomatic action.
    ScriptedDiplomacy = 93,
    /// An on_action's dispatch.
    OnAction = 94,
}

/// What caused an event: the observed native operations running when it
/// happened and the operands the game passed them. A fact the collector did
/// not observe is `None`, never a default.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedCause {
    /// The operation the event happened in; for WarEnded, Release and
    /// StateTransfer, the call that performed it.
    pub route: EventFeedRoute,
    /// The observed operation that one ran inside.
    pub within: EventFeedRoute,
    /// The PeaceConferences reference (the conference's own reference) of the conference starting
    /// or applying when the event happened.
    pub conference: Option<u64>,
    /// StateTransfer: the state's id.
    pub state: Option<u32>,
    /// A join: the country whose war the joiner joined, the join helper's
    /// ally operand in RDX.
    pub ally: Option<EventFeedCountry>,
    /// A join: the hostility reason the game passed in R9D: 0 war, 1 puppet,
    /// 2 ally, 3 asked to join, 4 guarantee, 5 not applicable.
    pub reason: Option<u32>,
    /// The operation's byte operands, bit n set when its byte n is nonzero: a join's expansion (bit
    /// 0, fifth argument: the joiner fights every enemy of the ally's side), a war end's truce (bit
    /// 0, second argument), an annexation's transfer-troops and AI message bytes (bits 0 and 1), a
    /// release's on-controlled and force-controller bytes (bits 0 and 1, read by the released-state
    /// transfer; the release command's exclude-releaser-cores byte, bit 0), a state owner change's
    /// third and fourth bytes (bits 0 and 1) and a conference's cleanup byte (bit 0, seventh
    /// argument, stored on the conference).
    pub flags: u8,
}
value_struct!(EventFeedCause {
    route: EventFeedRoute,
    within: EventFeedRoute,
    conference: Option<u64>,
    state: Option<u32>,
    ally: Option<EventFeedCountry>,
    reason: Option<u32>,
    flags: u8,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedEvent {
    /// Attachment-wide identity seeded from retained replay history, not a pointer.
    pub sequence: u64,
    /// World occurrence supplied by the collector, not a native object field.
    pub generation: u64,
    pub hour: i32,
    pub source: EventFeedSource,
    pub actor: EventFeedCountry,
    pub target: EventFeedCountry,
    /// Not recorded before format
    /// [`EVENT_CAUSE_FORMAT`](crate::rows::compatibility).
    pub cause: Recorded<EventFeedCause>,
}
value_struct!(EventFeedEvent {
    sequence: u64,
    generation: u64,
    hour: i32,
    source: EventFeedSource,
    actor: EventFeedCountry,
    target: EventFeedCountry
} appended EventFeedEvent {
    cause: Recorded<EventFeedCause>
});

/// Cumulative attachment accounting survives drains and world replacement.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedCounts {
    pub acquired: u64,
    pub settled: u64,
    pub failed: u64,
    pub rejected: u64,
}
value_struct!(EventFeedCounts {
    acquired: u64,
    settled: u64,
    failed: u64,
    rejected: u64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventFeedHistoryRow {
    pub generation: u64,
    pub read_hour: i32,
    pub coverage: EventFeedCoverage,
    /// None after count overflow; never a saturated or wrapped known total.
    pub counts: Option<EventFeedCounts>,
    pub outstanding: u64,
    pub failure: Option<Vec<u8>>,
    /// Known facts also survive partial/unavailable coverage. Missing events
    /// in those intervals cannot be interpreted as proof that nothing happened.
    pub events: Vec<EventFeedEvent>,
}

impl EventFeedHistoryRow {
    pub fn validate(&self) -> io::Result<()> {
        if self.coverage == EventFeedCoverage::Complete
            && (self.outstanding != 0 || self.failure.is_some() || self.counts.is_none())
        {
            return Err(invalid("incomplete Event feed accounting marked complete"));
        }
        if self.coverage == EventFeedCoverage::Unavailable && self.failure.is_none() {
            return Err(invalid("unavailable Event feed coverage has no refusal"));
        }
        if let Some(counts) = self.counts
            && (counts.settled > counts.acquired
                || counts.failed > counts.settled
                || counts.acquired - counts.settled != self.outstanding
                || self.events.len() as u64 > counts.acquired - counts.failed
                || self
                    .events
                    .iter()
                    .filter(|event| event.source != EventFeedSource::Annex)
                    .count() as u64
                    > counts.settled - counts.failed)
        {
            return Err(invalid("inconsistent Event feed accounting"));
        }
        let mut previous = None;
        for event in &self.events {
            if event.hour > self.read_hour
                || event.generation != self.generation
                || previous.is_some_and(|sequence| event.sequence <= sequence)
            {
                return Err(invalid("Event feed history identity or order differs"));
            }
            previous = Some(event.sequence);
        }
        Ok(())
    }
}

impl Value for EventFeedHistoryRow {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.counts)
            + Value::heap_bytes(&self.failure)
            + Value::heap_bytes(&self.events)
    }
    const MINIMUM_ENCODED_BYTES: usize = 27;
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.validate()?;
        let size = allocation::add(21, self.counts.encoded_bytes()?)?;
        let size = allocation::add(size, self.failure.encoded_bytes()?)?;
        allocation::add(size, self.events.encoded_bytes()?)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.validate()?;
        Ok(Self {
            generation: self.generation,
            read_hour: self.read_hour,
            coverage: self.coverage,
            counts: self.counts,
            outstanding: self.outstanding,
            failure: self.failure.try_clone()?,
            events: self.events.try_clone()?,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        self.generation.encode_reserved(output)?;
        self.read_hour.encode_reserved(output)?;
        self.coverage.encode_reserved(output)?;
        self.counts.encode_reserved(output)?;
        self.outstanding.encode_reserved(output)?;
        self.failure.encode_reserved(output)?;
        self.events.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let row = Self {
            generation: u64::decode(input)?,
            read_hour: i32::decode(input)?,
            coverage: EventFeedCoverage::decode(input)?,
            counts: Option::decode(input)?,
            outstanding: u64::decode(input)?,
            failure: Option::decode(input)?,
            events: Vec::decode(input)?,
        };
        row.validate()?;
        Ok(row)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod tests {
    use super::*;

    pub fn row() -> EventFeedHistoryRow {
        EventFeedHistoryRow {
            generation: 9,
            read_hour: 12,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts {
                acquired: 2,
                settled: 2,
                failed: 0,
                rejected: 0,
            }),
            outstanding: 0,
            failure: None,
            events: vec![EventFeedEvent {
                sequence: 4,
                generation: 9,
                hour: -17,
                source: EventFeedSource::Annex,
                actor: EventFeedCountry {
                    reference: 3,
                    id: 2,
                    instance: u32::MAX,
                },
                target: EventFeedCountry {
                    reference: 0,
                    id: 0,
                    instance: 7,
                },
                cause: Recorded::Value(EventFeedCause {
                    route: EventFeedRoute::PeaceAnnex,
                    within: EventFeedRoute::PeaceApplication,
                    conference: Some(0x0000_0007_0000_0031),
                    flags: 2,
                    ..Default::default()
                }),
            }],
        }
    }

    #[test]
    fn event_hour_raw_alias_and_zero_holder_survive_later_visibility() {
        let row = row();
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), row.encoded_bytes().unwrap());
        let mut input = Decoder::new(&bytes);
        assert_eq!(EventFeedHistoryRow::decode(&mut input).unwrap(), row);
        input.finish().unwrap();
        for end in 0..bytes.len() {
            assert!(EventFeedHistoryRow::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
    }

    #[test]
    fn complete_empty_partial_empty_and_unavailable_are_distinct() {
        let mut row = row();
        row.events.clear();
        row.validate().unwrap();
        row.coverage = EventFeedCoverage::Partial;
        row.validate().unwrap();
        row.coverage = EventFeedCoverage::Unavailable;
        assert!(row.validate().is_err());
        row.failure = Some(b"missed producer".to_vec());
        row.validate().unwrap();
        row.coverage = EventFeedCoverage::Complete;
        assert!(row.validate().is_err());
    }

    #[test]
    fn annex_before_dispatch_remains_known_with_an_unsettled_native_call() {
        let mut row = row();
        row.coverage = EventFeedCoverage::Unavailable;
        row.failure = Some(b"native dispatcher escaped".to_vec());
        row.counts = Some(EventFeedCounts {
            acquired: 1,
            ..Default::default()
        });
        row.outstanding = 1;
        row.validate().unwrap();
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            row
        );
        row.events[0].source = EventFeedSource::Declaration;
        assert!(row.validate().is_err());
    }

    #[test]
    fn causes_round_trip_and_older_events_decode_without_one() {
        let mut current = row();
        current.events[0].source = EventFeedSource::StateTransfer;
        current.events[0].cause = Recorded::Value(EventFeedCause {
            route: EventFeedRoute::StateAnnex,
            within: EventFeedRoute::AnnexEffect,
            conference: None,
            state: Some(64),
            ally: Some(EventFeedCountry {
                reference: 9,
                id: 9,
                instance: 1,
            }),
            reason: Some(3),
            flags: 1,
        });
        let mut bytes = Vec::new();
        current.encode(&mut bytes).unwrap();
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            current
        );
        // A format-101 event ends at its target; its cause is not recorded.
        let mut older = row();
        let mut bytes = Vec::new();
        older.encode(&mut bytes).unwrap();
        let cause = older.events[0].cause.encoded_bytes().unwrap();
        bytes.truncate(bytes.len() - cause);
        let mut input = Decoder::new(&bytes);
        input.format = crate::rows::compatibility::EVENT_CAUSE_FORMAT - 1;
        older.events[0].cause = Recorded::NotRecorded;
        assert_eq!(EventFeedHistoryRow::decode(&mut input).unwrap(), older);
        input.finish().unwrap();
        for route in EventFeedRoute::ALL {
            assert_eq!(
                EventFeedRoute::named(route.name()),
                (route != EventFeedRoute::None).then_some(route)
            );
            let mut bytes = Vec::new();
            route.encode(&mut bytes).unwrap();
            assert_eq!(
                EventFeedRoute::decode(&mut Decoder::new(&bytes)).unwrap(),
                route
            );
        }
        assert!(EventFeedRoute::decode(&mut Decoder::new(&[95])).is_err());
    }

    #[test]
    fn malformed_kinds_counts_generation_and_duplicate_sequences_refuse() {
        assert!(EventFeedSource::decode(&mut Decoder::new(&[8])).is_err());
        assert!(EventFeedCoverage::decode(&mut Decoder::new(&[3])).is_err());
        let mut row = row();
        row.counts.as_mut().unwrap().settled = 3;
        assert!(row.validate().is_err());
        row.counts.as_mut().unwrap().settled = 2;
        row.events[0].generation = 8;
        assert!(row.validate().is_err());
        row.events[0].generation = 9;
        row.events[0].hour = row.read_hour + 1;
        assert!(row.validate().is_err());
        row.events[0].hour = row.read_hour;
        row.events.push(row.events[0]);
        assert!(row.validate().is_err());
    }
}
