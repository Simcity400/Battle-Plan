//! Owned simulation-event history and interval coverage. Operation hours filter history; supplying
//! hours govern its first visibility. The legacy country instance field retains the raw surrender
//! target, not an incarnation counter.

use crate::{
    codec::{Decoder, invalid_static as invalid},
    rows::Recorded,
    value::{Value, value_struct},
};
use engine_allocation as allocation;
use std::io;
pub(crate) mod columns;
pub use columns::reuse;
pub mod decisions;
pub mod state;
pub use state::{DelayedEvent, PendingEvent, ScriptEventState, ScriptIdentity};

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
    /// An accepted scripted event dispatch, independent of its later choice.
    ScriptEvent = 8,
    /// An accepted news-event dispatch.
    NewsEvent = 9,
    /// An admitted option execution; its signed native ordinal is retained.
    EventOption = 10,
    DecisionSelected = 11,
    DecisionRemoved = 12,
    DecisionCancelled = 13,
    DecisionTimedOut = 14,
    DecisionActivated = 15,
    DecisionCompleted = 16,
    /// Retained event-registry baseline or writer delta, never a fired event.
    ScriptState = 17,
    DecisionReenabled = 18,
    /// Authored effect execution is independent of a later status transition.
    DecisionRemovalEffect = 19,
    DecisionCancellationEffect = 20,
    DecisionCompletionEffect = 21,
    DecisionTimeoutEffect = 22,
    DecisionDurationChanged = 23,
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
            8 => Ok(Self::ScriptEvent),
            9 => Ok(Self::NewsEvent),
            10 => Ok(Self::EventOption),
            11 => Ok(Self::DecisionSelected),
            12 => Ok(Self::DecisionRemoved),
            13 => Ok(Self::DecisionCancelled),
            14 => Ok(Self::DecisionTimedOut),
            15 => Ok(Self::DecisionActivated),
            16 => Ok(Self::DecisionCompleted),
            17 => Ok(Self::ScriptState),
            18 => Ok(Self::DecisionReenabled),
            19 => Ok(Self::DecisionRemovalEffect),
            20 => Ok(Self::DecisionCancellationEffect),
            21 => Ok(Self::DecisionCompletionEffect),
            22 => Ok(Self::DecisionTimeoutEffect),
            23 => Ok(Self::DecisionDurationChanged),
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventFeedCountry {
    pub reference: u32,
    pub id: u32,
    pub instance: u32,
}
impl Value for EventFeedCountry {
    const MINIMUM_ENCODED_BYTES: usize = 3;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok([self.reference, self.id, self.instance]
            .into_iter()
            .map(compact_len)
            .sum())
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        for value in [self.reference, self.id, self.instance] {
            compact_write(value, output);
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            reference: compact_read(input)?,
            id: compact_read(input)?,
            instance: compact_read(input)?,
        })
    }
}

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
    /// Compact operands in this supplying row's token and scope arenas.
    pub script: Option<EventFeedScript>,
}
impl Value for EventFeedEvent {
    const MINIMUM_ENCODED_BYTES: usize = 5;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        let mut bytes = wide_len(self.sequence)
            + wide_len(self.generation)
            + compact_len(signed_word(self.hour))
            + 2;
        for country in [self.actor, self.target] {
            if country != EventFeedCountry::default() {
                bytes += country.encoded_bytes()?;
            }
        }
        if let Recorded::Value(cause) = self.cause
            && cause != EventFeedCause::default()
        {
            bytes += cause.encoded_bytes()?;
        }
        if let Some(script) = self.script {
            bytes += script.encoded_bytes()?;
        }
        Ok(bytes)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        crate::codec::put_varint(output, self.sequence as u128);
        crate::codec::put_varint(output, self.generation as u128);
        compact_write(signed_word(self.hour), output);
        self.source.encode_reserved(output)?;
        let cause = match self.cause {
            Recorded::NotRecorded => 1,
            Recorded::Value(value) if value != EventFeedCause::default() => 2,
            _ => 0,
        };
        let flags = u8::from(self.actor != EventFeedCountry::default())
            | (u8::from(self.target != EventFeedCountry::default()) << 1)
            | (cause << 2)
            | (u8::from(self.script.is_some()) << 4);
        flags.encode_reserved(output)?;
        for country in [self.actor, self.target] {
            if country != EventFeedCountry::default() {
                country.encode_reserved(output)?;
            }
        }
        if cause == 2
            && let Recorded::Value(cause) = self.cause
        {
            cause.encode_reserved(output)?;
        }
        if let Some(script) = self.script {
            script.encode_reserved(output)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let sequence = wide_read(input)?;
        let generation = wide_read(input)?;
        let hour = signed_value(compact_read(input)?);
        let source = EventFeedSource::decode(input)?;
        let flags = u8::decode(input)?;
        if flags >> 5 != 0 || (flags >> 2) & 3 == 3 {
            return Err(invalid("Event feed occurrence flags differ"));
        }
        let actor = if flags & 1 != 0 {
            EventFeedCountry::decode(input)?
        } else {
            Default::default()
        };
        let target = if flags & 2 != 0 {
            EventFeedCountry::decode(input)?
        } else {
            Default::default()
        };
        let cause = match (flags >> 2) & 3 {
            0 => Recorded::Value(Default::default()),
            1 => Recorded::NotRecorded,
            _ => Recorded::Value(EventFeedCause::decode(input)?),
        };
        let script = if flags & 16 != 0 {
            Some(EventFeedScript::decode(input)?)
        } else {
            None
        };
        Ok(Self {
            sequence,
            generation,
            hour,
            source,
            actor,
            target,
            cause,
            script,
        })
    }
}

/// Authored bytes and native identities, never rendered text or game pointers.
/// Scope binding IDs are local to this occurrence's scope span.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct EventFeedEventIdentity {
    pub kind: u32,
    pub id: u32,
}
impl std::fmt::Display for EventFeedEventIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (type {})", self.id, self.kind)
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedScript {
    pub token_start: u32,
    pub token_len: u32,
    pub scope_start: u32,
    pub scope_len: u32,
    pub native_event: Option<EventFeedEventIdentity>,
    pub instance: Option<u32>,
    pub option: Option<i32>,
    pub country_target: Option<u32>,
    /// Raw selector operand; definition-specific meaning is not assumed.
    pub numeric_target: Option<i32>,
    pub token_unrecorded: bool,
    pub scope_unrecorded: bool,
    pub change: Option<EventFeedScriptChange>,
    /// Actual writer result, present only when that boundary supplies it.
    pub lifecycle: Option<i32>,
    pub days: Option<i32>,
}

/// Retained registry operations share the occurrence stream's ordering,
/// world and delivery receipts. Baseline membership is never timed history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFeedScriptChange {
    Decision(decisions::DecisionChange),
    PendingBaseline,
    PendingStored {
        due_hour: i32,
    },
    PendingReady,
    PendingRemoved,
    FiredBaseline,
    FiredMember,
    FiredRemoved,
    FiredReady,
    DelayedBaseline,
    DelayedStored {
        remaining: i32,
    },
    DelayedReady,
    /// One certified empty owner baseline; no separate begin/end payload.
    DelayedEmpty,
    /// Decrements derived from a pass anchor and sparse native cursor boundaries.
    /// Quiet passes emit one final count, with no per-timer observation.
    DelayedAdvance {
        count: u32,
        finished: bool,
    },
    DelayedRemoved {
        iteration: bool,
        visited: bool,
    },
    /// The capture refused one owner's retained queue (the event's target);
    /// the token is the reason. Other owners and registries keep theirs.
    DelayedRefused,
}
impl Value for EventFeedScriptChange {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1 + match self {
            Self::Decision(change) => change.encoded_bytes()?,
            Self::PendingStored { due_hour } => compact_len(signed_word(*due_hour)),
            Self::DelayedStored { remaining } => compact_len(signed_word(*remaining)),
            Self::DelayedAdvance { count, .. } => compact_len(*count),
            _ => 0,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let code = match self {
            Self::Decision(_) => 17,
            Self::PendingBaseline => 0_u8,
            Self::PendingStored { .. } => 1,
            Self::PendingReady => 2,
            Self::PendingRemoved => 3,
            Self::FiredBaseline => 4,
            Self::FiredMember => 5,
            Self::FiredReady => 6,
            Self::FiredRemoved => 16,
            Self::DelayedBaseline => 7,
            Self::DelayedStored { .. } => 8,
            Self::DelayedReady => 9,
            Self::DelayedAdvance {
                finished: false, ..
            } => 10,
            Self::DelayedAdvance { finished: true, .. } => 11,
            Self::DelayedRemoved {
                iteration: false,
                visited: false,
            } => 12,
            Self::DelayedRemoved {
                iteration: true,
                visited: false,
            } => 13,
            Self::DelayedRemoved {
                iteration: true,
                visited: true,
            } => 14,
            Self::DelayedEmpty => 15,
            Self::DelayedRefused => 18,
            Self::DelayedRemoved {
                iteration: false,
                visited: true,
            } => {
                return Err(invalid(
                    "delayed-event cancellation cannot be a native visit",
                ));
            }
        };
        code.encode_reserved(output)?;
        if let Self::Decision(change) = self {
            change.encode_reserved(output)?;
        }
        if let Self::PendingStored { due_hour } = self {
            compact_write(signed_word(*due_hour), output);
        }
        if let Self::DelayedStored { remaining } = self {
            compact_write(signed_word(*remaining), output);
        }
        if let Self::DelayedAdvance { count, .. } = self {
            compact_write(*count, output);
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let code = u8::decode(input)?;
        Self::of(code, input)
    }
}
/// Where a stored registry change's operands are read from.
pub(crate) trait ChangeOperands {
    /// The change's one integer operand.
    fn operand(&mut self) -> io::Result<u32>;
    fn decision(&mut self) -> io::Result<decisions::DecisionChange>;
}
impl ChangeOperands for Decoder<'_> {
    fn operand(&mut self) -> io::Result<u32> {
        compact_read(self)
    }
    fn decision(&mut self) -> io::Result<decisions::DecisionChange> {
        decisions::DecisionChange::decode(self)
    }
}
impl EventFeedScriptChange {
    /// The change stored as `code`, its operands read from `operands`.
    pub(crate) fn of(code: u8, operands: &mut impl ChangeOperands) -> io::Result<Self> {
        match code {
            17 => Ok(Self::Decision(operands.decision()?)),
            0 => Ok(Self::PendingBaseline),
            1 => Ok(Self::PendingStored {
                due_hour: signed_value(operands.operand()?),
            }),
            2 => Ok(Self::PendingReady),
            3 => Ok(Self::PendingRemoved),
            4 => Ok(Self::FiredBaseline),
            5 => Ok(Self::FiredMember),
            6 => Ok(Self::FiredReady),
            16 => Ok(Self::FiredRemoved),
            7 => Ok(Self::DelayedBaseline),
            8 => Ok(Self::DelayedStored {
                remaining: signed_value(operands.operand()?),
            }),
            9 => Ok(Self::DelayedReady),
            code @ (10 | 11) => Ok(Self::DelayedAdvance {
                count: operands.operand()?,
                finished: code == 11,
            }),
            code @ (12..=14) => Ok(Self::DelayedRemoved {
                iteration: code != 12,
                visited: code == 14,
            }),
            15 => Ok(Self::DelayedEmpty),
            18 => Ok(Self::DelayedRefused),
            _ => Err(invalid("Event feed script registry change differs")),
        }
    }
}
impl EventFeedScript {
    fn flags(&self) -> u32 {
        [
            self.instance,
            self.option.map(signed_word),
            self.country_target,
            self.numeric_target.map(signed_word),
        ]
        .iter()
        .enumerate()
        .fold(0_u32, |flags, (n, value)| {
            flags | (u32::from(value.is_some()) << (n + 1))
        }) | u32::from(self.native_event.is_some())
            | (u32::from(self.scope_unrecorded) << 5)
            | (u32::from(self.token_unrecorded) << 6)
            | (u32::from(self.change.is_some()) << 7)
            | (u32::from(self.lifecycle.is_some()) << 8)
            | (u32::from(self.days.is_some()) << 9)
    }
}
impl Value for EventFeedScript {
    const MINIMUM_ENCODED_BYTES: usize = 5;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(compact_len(self.flags())
            + self
                .lifecycle
                .map(|n| compact_len(signed_word(n)))
                .unwrap_or(0)
            + self.days.map(|n| compact_len(signed_word(n))).unwrap_or(0)
            + self
                .native_event
                .map(|id| compact_len(id.kind) + compact_len(id.id))
                .unwrap_or(0)
            + self
                .change
                .map(|change| change.encoded_bytes())
                .transpose()?
                .unwrap_or(0)
            + [
                self.token_start,
                self.token_len,
                self.scope_start,
                self.scope_len,
            ]
            .into_iter()
            .map(compact_len)
            .sum::<usize>()
            + [
                self.instance,
                self.option.map(signed_word),
                self.country_target,
                self.numeric_target.map(signed_word),
            ]
            .into_iter()
            .flatten()
            .map(compact_len)
            .sum::<usize>())
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        for value in [
            self.token_start,
            self.token_len,
            self.scope_start,
            self.scope_len,
        ] {
            compact_write(value, output);
        }
        let values = [
            self.instance,
            self.option.map(signed_word),
            self.country_target,
            self.numeric_target.map(signed_word),
        ];
        compact_write(self.flags(), output);
        if let Some(id) = self.native_event {
            compact_write(id.kind, output);
            compact_write(id.id, output);
        }
        for value in values.into_iter().flatten() {
            compact_write(value, output);
        }
        if let Some(change) = self.change {
            change.encode_reserved(output)?;
        }
        for value in [self.lifecycle, self.days].into_iter().flatten() {
            compact_write(signed_word(value), output);
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let token_start = compact_read(input)?;
        let token_len = compact_read(input)?;
        let scope_start = compact_read(input)?;
        let scope_len = compact_read(input)?;
        let flags = compact_read(input)?;
        if flags >> 10 != 0 {
            return Err(invalid("script operand flags differ"));
        }
        let native_event = if flags & 1 != 0 {
            Some(EventFeedEventIdentity {
                kind: compact_read(input)?,
                id: compact_read(input)?,
            })
        } else {
            None
        };
        let mut values = [None; 4];
        for (n, value) in values.iter_mut().enumerate() {
            if flags & (1 << (n + 1)) != 0 {
                *value = Some(compact_read(input)?);
            }
        }
        Ok(Self {
            token_start,
            token_len,
            scope_start,
            scope_len,
            native_event,
            instance: values[0],
            option: values[1].map(signed_value),
            country_target: values[2],
            numeric_target: values[3].map(signed_value),
            scope_unrecorded: flags & 0x20 != 0,
            token_unrecorded: flags & 0x40 != 0,
            change: if flags & 0x80 != 0 {
                Some(EventFeedScriptChange::decode(input)?)
            } else {
                None
            },
            lifecycle: if flags & 0x100 != 0 {
                Some(signed_value(compact_read(input)?))
            } else {
                None
            },
            days: if flags & 0x200 != 0 {
                Some(signed_value(compact_read(input)?))
            } else {
                None
            },
        })
    }
}

/// An occurrence's bound scope. Unproved pointer-owned operands are named
/// demands, independent of the values that were acquired successfully.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventFeedScope {
    pub country: u32,
    pub random: [u32; 2],
    /// ROOT, FROM and PREV, in that order.
    pub bindings: [u32; 3],
    pub state: i32,
    pub references: [u64; 9],
    pub strategic_region: Option<u32>,
    pub combat_context_unrecorded: bool,
    pub target_storage_unrecorded: bool,
    pub tooltip_storage_unrecorded: bool,
    /// Set by collectors that record the two fields below, even when they could not.
    pub country_attempted: bool,
    /// The full native ID a positive `country` resolved to when the scope was copied.
    pub country_id: Option<u32>,
    /// That country's name, definite name and adjective then, in the recording client's language.
    pub country_names: Option<[EventFeedText; 3]>,
}

/// Exact bytes of a native string, held inline so a scope stays `Copy`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EventFeedText {
    len: u16,
    bytes: [u8; Self::CAPACITY],
}
impl EventFeedText {
    pub const CAPACITY: usize = 256;
    pub fn new(bytes: &[u8]) -> Option<Self> {
        Self::filled(bytes.len(), |output| {
            output.copy_from_slice(bytes);
            true
        })
    }
    /// `len` bytes `fill` writes in place; `None` beyond the capacity or when it fails.
    pub fn filled(len: usize, fill: impl FnOnce(&mut [u8]) -> bool) -> Option<Self> {
        let mut text = Self::default();
        let output = text.bytes.get_mut(..len)?;
        fill(output).then_some(())?;
        text.len = len as u16;
        Some(text)
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}
impl Default for EventFeedText {
    fn default() -> Self {
        Self {
            len: 0,
            bytes: [0; Self::CAPACITY],
        }
    }
}
impl std::fmt::Debug for EventFeedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.as_bytes()))
    }
}
impl serde::Serialize for EventFeedText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(self.as_bytes(), serializer)
    }
}
impl<'de> serde::Deserialize<'de> for EventFeedText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8> as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(&bytes)
            .ok_or_else(|| serde::de::Error::custom("Event feed text exceeds its capacity"))
    }
}

fn compact_len(value: u32) -> usize {
    ((32 - value.leading_zeros()).max(1) as usize).div_ceil(7)
}
fn wide_len(value: u64) -> usize {
    ((64 - value.leading_zeros()).max(1) as usize).div_ceil(7)
}
fn wide_read(input: &mut Decoder<'_>) -> io::Result<u64> {
    u64::try_from(input.varint()?).map_err(|_| invalid("Event feed integer exceeds u64"))
}
fn compact_write(value: u32, output: &mut Vec<u8>) {
    crate::codec::put_varint(output, value as u128);
}
fn compact_read(input: &mut Decoder<'_>) -> io::Result<u32> {
    u32::try_from(input.varint()?).map_err(|_| invalid("Event feed integer exceeds u32"))
}
fn signed_word(value: i32) -> u32 {
    (value as u32).wrapping_shl(1) ^ (value >> 31) as u32
}
fn signed_value(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

impl Value for EventFeedScope {
    const MINIMUM_ENCODED_BYTES: usize = 16;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(11
            + compact_len(self.country)
            + self
                .bindings
                .iter()
                .map(|id| compact_len(*id))
                .sum::<usize>()
            + compact_len(signed_word(self.state))
            + self
                .references
                .iter()
                .filter(|reference| **reference != 0)
                .count()
                * 8
            + self.strategic_region.map(compact_len).unwrap_or(0)
            + self.country_id.map(compact_len).unwrap_or(0)
            + self
                .country_names
                .iter()
                .flatten()
                .map(|text| compact_len(text.len.into()) + text.as_bytes().len())
                .sum::<usize>())
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        compact_write(self.country, output);
        self.random.encode_reserved(output)?;
        for binding in self.bindings {
            compact_write(binding, output);
        }
        compact_write(signed_word(self.state), output);
        let mask = self
            .references
            .iter()
            .enumerate()
            .fold(0_u16, |mask, (n, reference)| {
                mask | (u16::from(*reference != 0) << n)
            });
        mask.encode_reserved(output)?;
        for reference in self.references {
            if reference != 0 {
                reference.encode_reserved(output)?;
            }
        }
        let flags = u8::from(self.strategic_region.is_some())
            | (u8::from(self.combat_context_unrecorded) << 1)
            | (u8::from(self.target_storage_unrecorded) << 2)
            | (u8::from(self.tooltip_storage_unrecorded) << 3)
            | (u8::from(self.country_id.is_some()) << 4)
            | (u8::from(self.country_names.is_some()) << 5)
            | (u8::from(self.country_attempted) << 6);
        flags.encode_reserved(output)?;
        for value in [self.strategic_region, self.country_id].into_iter().flatten() {
            compact_write(value, output);
        }
        for text in self.country_names.iter().flatten() {
            compact_write(text.len.into(), output);
            output.extend_from_slice(text.as_bytes());
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let country = compact_read(input)?;
        let random = <[u32; 2]>::decode(input)?;
        let bindings = [
            compact_read(input)?,
            compact_read(input)?,
            compact_read(input)?,
        ];
        let state = signed_value(compact_read(input)?);
        let mask = u16::decode(input)?;
        if mask >> 9 != 0 {
            return Err(invalid("Event feed scope reference mask differs"));
        }
        let mut references = [0; 9];
        for (n, reference) in references.iter_mut().enumerate() {
            if mask & (1 << n) != 0 {
                *reference = u64::decode(input)?;
            }
        }
        let flags = u8::decode(input)?;
        if flags >> 7 != 0 {
            return Err(invalid("Event feed scope flags differ"));
        }
        let strategic_region = if flags & 1 != 0 {
            Some(compact_read(input)?)
        } else {
            None
        };
        let country_id = if flags & 0x10 != 0 {
            Some(compact_read(input)?)
        } else {
            None
        };
        let country_names = if flags & 0x20 != 0 {
            let mut texts = [EventFeedText::default(); 3];
            for text in &mut texts {
                let len = compact_read(input)? as usize;
                *text = EventFeedText::new(input.take(len)?)
                    .ok_or_else(|| invalid("Event feed scope name exceeds its capacity"))?;
            }
            Some(texts)
        } else {
            None
        };
        Ok(Self {
            country_attempted: flags & 0x40 != 0,
            country_id,
            country_names,
            country,
            random,
            bindings,
            state,
            references,
            strategic_region,
            combat_context_unrecorded: flags & 2 != 0,
            target_storage_unrecorded: flags & 4 != 0,
            tooltip_storage_unrecorded: flags & 8 != 0,
        })
    }
}

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

/// Families whose unobserved conditional writers prevent interval carry.
/// These bits qualify family coverage independently of transport/flight coverage:
/// known occurrences and independently certified event registries remain usable.
pub const DECISION_FAMILY_REFUSALS: u8 = 0x1f;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScriptFamily {
    DecisionMembership = 1,
    DecisionTimers = 2,
    DecisionLifecycle = 4,
    DecisionRoster = 8,
    DecisionRandomCount = 16,
}
pub const SCRIPT_FAMILY_NAMES: [(ScriptFamily, &str); 5] = [
    (
        ScriptFamily::DecisionMembership,
        "decision and mission membership (selection retention, targeted cleanup, loading and refresh)",
    ),
    (
        ScriptFamily::DecisionTimers,
        "decision and mission durations, cooldowns, removal and re-enable timers (conditional countdowns and timer transfers)",
    ),
    (
        ScriptFamily::DecisionLifecycle,
        "decision and mission lifecycle (targeted reactivation and selection result stores)",
    ),
    (
        ScriptFamily::DecisionRoster,
        "visible and available decision roster (native trigger inputs, target rebuild and ignore commands)",
    ),
    (
        ScriptFamily::DecisionRandomCount,
        "decision description random-count state (selection, loading and reset)",
    ),
];

/// The names of the families `refusals` marks unavailable.
fn refused_script_families(refusals: u8) -> impl Iterator<Item = &'static str> {
    SCRIPT_FAMILY_NAMES
        .into_iter()
        .filter_map(move |(bit, name)| (refusals & bit as u8 != 0).then_some(name))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventFeedHistoryRow {
    pub generation: u64,
    pub read_hour: i32,
    /// Observation/transport coverage; consult script_family_coverage before family carry.
    pub coverage: EventFeedCoverage,
    /// None after count overflow; never a saturated or wrapped known total.
    pub counts: Option<EventFeedCounts>,
    pub outstanding: u64,
    pub failure: Option<Vec<u8>>,
    /// Named/countable unavailable families for this supplying interval.
    pub script_refusals: u8,
    /// Known facts also survive partial/unavailable coverage. Missing events
    /// in those intervals cannot be interpreted as proof that nothing happened.
    pub events: Vec<EventFeedEvent>,
    pub script_tokens: Vec<u8>,
    pub script_scopes: Vec<EventFeedScope>,
}

impl EventFeedHistoryRow {
    pub fn script_family_coverage(&self, family: ScriptFamily) -> EventFeedCoverage {
        if self.script_refusals & family as u8 != 0 {
            EventFeedCoverage::Unavailable
        } else {
            self.coverage
        }
    }
    pub fn refused_script_families(&self) -> impl Iterator<Item = &'static str> {
        refused_script_families(self.script_refusals)
    }
    /// Keep each distinct token's bytes once in the arena, in first-use order,
    /// and point every occurrence of an equal token at that one span. Every
    /// occurrence still resolves the same bytes; a roster rebuild naming one
    /// decision for every target stores that name once.
    pub fn intern_tokens(&mut self) -> io::Result<()> {
        self.validate()?;
        let (events, tokens) = (&self.events, &self.script_tokens);
        let span = |index: usize| {
            events[index]
                .script
                .map(|script| &tokens[script.token_start as usize..][..script.token_len as usize])
        };
        let mut order = allocation::vec(events.len())?;
        order.extend((0..events.len()).filter(|&index| events[index].script.is_some()));
        order.sort_unstable_by(|&a, &b| span(a).cmp(&span(b)).then(a.cmp(&b)));
        // Each occurrence's first occurrence with an equal token.
        let mut first = allocation::vec(events.len())?;
        first.resize(events.len(), 0);
        let mut bytes = 0;
        for run in order.chunk_by(|&a, &b| span(a) == span(b)) {
            bytes += span(run[0]).map_or(0, <[u8]>::len);
            for &index in run {
                first[index] = run[0];
            }
        }
        let mut interned = allocation::vec(bytes)?;
        let mut starts = allocation::vec(events.len())?;
        starts.resize(events.len(), 0);
        for index in 0..events.len() {
            if let Some(token) = span(index) {
                starts[index] = if first[index] == index {
                    let start = u32::try_from(interned.len())
                        .map_err(|_| invalid("Event feed token arena exceeds u32"))?;
                    interned.extend_from_slice(token);
                    start
                } else {
                    starts[first[index]]
                };
            }
        }
        for (event, start) in self.events.iter_mut().zip(starts) {
            if let Some(script) = event.script.as_mut() {
                script.token_start = start;
            }
        }
        self.script_tokens = interned;
        Ok(())
    }
    pub fn validate(&self) -> io::Result<()> {
        if self.script_refusals & !DECISION_FAMILY_REFUSALS != 0 {
            return Err(invalid("unknown Event feed script family refusal"));
        }
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
            let scripted = matches!(
                event.source,
                EventFeedSource::ScriptEvent
                    | EventFeedSource::NewsEvent
                    | EventFeedSource::EventOption
                    | EventFeedSource::DecisionSelected
                    | EventFeedSource::DecisionRemoved
                    | EventFeedSource::DecisionCancelled
                    | EventFeedSource::DecisionTimedOut
                    | EventFeedSource::DecisionActivated
                    | EventFeedSource::DecisionCompleted
                    | EventFeedSource::ScriptState
                    | EventFeedSource::DecisionReenabled
                    | EventFeedSource::DecisionRemovalEffect
                    | EventFeedSource::DecisionCancellationEffect
                    | EventFeedSource::DecisionCompletionEffect
                    | EventFeedSource::DecisionTimeoutEffect
                    | EventFeedSource::DecisionDurationChanged
            );
            if scripted != event.script.is_some() {
                return Err(invalid("Event feed script operands differ from source"));
            }
            if let Some(script) = event.script {
                if event.source == EventFeedSource::ScriptState && script.change.is_none()
                    || script.change.is_some()
                        && event.source != EventFeedSource::ScriptState
                        && (!matches!(script.change, Some(EventFeedScriptChange::Decision(_)))
                            || !matches!(
                                event.source,
                                EventFeedSource::DecisionActivated
                                    | EventFeedSource::DecisionCompleted
                                    | EventFeedSource::DecisionTimedOut
                                    | EventFeedSource::DecisionRemoved
                                    | EventFeedSource::DecisionCancelled
                                    | EventFeedSource::DecisionReenabled
                                    | EventFeedSource::DecisionDurationChanged
                            ))
                {
                    return Err(invalid("Event feed registry change differs from source"));
                }
                if let Some(change) = script.change {
                    use EventFeedScriptChange::*;
                    if let Decision(change) = change {
                        change.validate(script)?;
                    }
                    if matches!(
                        change,
                        DelayedRemoved {
                            iteration: false,
                            visited: true
                        }
                    ) {
                        return Err(invalid(
                            "delayed-event cancellation cannot be a native visit",
                        ));
                    }
                    let (identity, instance, scope) = match change {
                        Decision(_) => (false, script.instance.is_some(), false),
                        PendingStored { .. } | DelayedStored { .. } => (true, true, true),
                        PendingRemoved | DelayedRemoved { .. } => (false, true, false),
                        FiredMember | FiredRemoved => (true, false, false),
                        PendingBaseline
                        | PendingReady
                        | FiredBaseline
                        | FiredReady
                        | DelayedBaseline
                        | DelayedReady
                        | DelayedEmpty
                        | DelayedAdvance { .. }
                        | DelayedRefused => (false, false, false),
                    };
                    // A refusal's token is its recorded reason, never a definition.
                    let reason = change == DelayedRefused;
                    if identity != script.native_event.is_some()
                        || instance != script.instance.is_some()
                        || scope != (script.scope_len != 0 || script.scope_unrecorded)
                        || reason && (script.token_len == 0 || script.token_unrecorded)
                        || !matches!(change, Decision(_))
                            && !identity
                            && !reason
                            && (script.token_len != 0 || script.token_unrecorded)
                        || !matches!(change, Decision(_))
                            && (script.country_target.is_some() || script.numeric_target.is_some())
                    {
                        return Err(invalid("Event feed registry operands differ from change"));
                    }
                }
                if script.token_unrecorded && script.token_len != 0
                    || script.scope_unrecorded && script.scope_len != 0
                {
                    return Err(invalid(
                        "Event feed unavailable script operand carries a value",
                    ));
                }
                let span = |start: u32, len: u32, size: usize| {
                    let start = start as usize;
                    let end = start
                        .checked_add(len as usize)
                        .filter(|end| *end <= size)
                        .ok_or_else(|| invalid("Event feed script arena span differs"))?;
                    Ok::<_, io::Error>(start..end)
                };
                span(
                    script.token_start,
                    script.token_len,
                    self.script_tokens.len(),
                )?;
                let scopes = span(
                    script.scope_start,
                    script.scope_len,
                    self.script_scopes.len(),
                )?;
                for scope in &self.script_scopes[scopes] {
                    if scope.bindings.iter().any(|id| *id >= script.scope_len) {
                        return Err(invalid(
                            "Event feed scope binding is outside its occurrence",
                        ));
                    }
                    if scope.country_id.is_some() && scope.country as i32 <= 0
                        || scope.country_names.is_some() && scope.country_id.is_none()
                        || scope.country_id.is_some() && !scope.country_attempted
                    {
                        return Err(invalid("Event feed scope identity has no country"));
                    }
                }
                if (event.source == EventFeedSource::EventOption) != script.option.is_some() {
                    return Err(invalid("Event feed option operand differs from source"));
                }
            }
        }
        Ok(())
    }
}

impl Value for EventFeedHistoryRow {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.counts)
            + Value::heap_bytes(&self.failure)
            + Value::heap_bytes(&self.events)
            + Value::heap_bytes(&self.script_tokens)
            + Value::heap_bytes(&self.script_scopes)
    }
    const MINIMUM_ENCODED_BYTES: usize = 36;
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.validate()?;
        let size = allocation::add(22, self.counts.encoded_bytes()?)?;
        let size = allocation::add(size, self.failure.encoded_bytes()?)?;
        let size = allocation::add(
            size,
            crate::value::EncodeValue::encoded_bytes(&columns::Batch(self))?,
        )?;
        let size = allocation::add(size, self.script_tokens.encoded_bytes()?)?;
        allocation::add(size, self.script_scopes.encoded_bytes()?)
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
            script_refusals: self.script_refusals,
            events: self.events.try_clone()?,
            script_tokens: self.script_tokens.try_clone()?,
            script_scopes: self.script_scopes.try_clone()?,
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
        self.script_refusals.encode_reserved(output)?;
        crate::value::EncodeValue::encode_value(&columns::Batch(self), output)?;
        self.script_tokens.encode_reserved(output)?;
        self.script_scopes.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let mut row = Self {
            generation: u64::decode(input)?,
            read_hour: i32::decode(input)?,
            coverage: EventFeedCoverage::decode(input)?,
            counts: Option::decode(input)?,
            outstanding: u64::decode(input)?,
            failure: Option::decode(input)?,
            script_refusals: u8::decode(input)?,
            events: Vec::new(),
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
        };
        row.events = columns::decode(&row, input)?;
        row.script_tokens = Vec::decode(input)?;
        row.script_scopes = Vec::decode(input)?;
        row.validate()?;
        Ok(row)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod tests {
    use super::*;

    #[test]
    fn family_refusals_roundtrip_without_inventing_transport_or_registry_loss() {
        let mut row = row();
        row.script_refusals = DECISION_FAMILY_REFUSALS;
        assert_eq!(row.refused_script_families().count(), 5);
        for (family, _) in SCRIPT_FAMILY_NAMES {
            assert_eq!(
                row.script_family_coverage(family),
                EventFeedCoverage::Unavailable
            );
        }
        assert_eq!(row.coverage, EventFeedCoverage::Complete);
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(row.encoded_bytes().unwrap(), bytes.len());
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            row
        );
        row.script_refusals = 0x80;
        assert!(row.validate().is_err());
    }
    pub fn row() -> EventFeedHistoryRow {
        EventFeedHistoryRow {
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
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
            script_refusals: 0,
            events: vec![EventFeedEvent {
                script: None,
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
    fn interned_tokens_keep_every_occurrence_and_the_retained_state_it_reduces_to() {
        use EventFeedScriptChange::*;
        use decisions::{DecisionChange, Operation};
        let owner = EventFeedCountry {
            reference: 3,
            id: 3,
            instance: 0,
        };
        let mut row = row();
        row.events.clear();
        row.counts = None;
        row.coverage = EventFeedCoverage::Partial;
        // The seal's layout: every occurrence's token and scopes appended in order.
        let mut push = |source, target, token: &[u8], scopes: u32, script: EventFeedScript| {
            let sequence = row.events.last().map_or(1, |event| event.sequence + 1);
            let script = EventFeedScript {
                token_start: row.script_tokens.len() as u32,
                token_len: token.len() as u32,
                scope_start: row.script_scopes.len() as u32,
                scope_len: scopes,
                ..script
            };
            row.script_tokens.extend_from_slice(token);
            for _ in 0..scopes {
                row.script_scopes.push(EventFeedScope {
                    country: 3,
                    ..Default::default()
                });
            }
            row.events.push(EventFeedEvent {
                sequence,
                generation: row.generation,
                hour: row.read_hour,
                source,
                actor: owner,
                target,
                cause: Recorded::Value(Default::default()),
                script: Some(script),
            });
        };
        let event = |id| Some(EventFeedEventIdentity { kind: 50, id });
        let change = |change| EventFeedScript {
            change: Some(change),
            ..Default::default()
        };
        let decision = |operation, container, ordinal| {
            change(Decision(DecisionChange {
                operation,
                container,
                flags: 0,
                ordinal,
                political_power: None,
            }))
        };
        let none = EventFeedCountry::default();
        let (state, fired) = (EventFeedSource::ScriptState, b"synthetic.2".as_slice());
        push(
            EventFeedSource::ScriptEvent,
            owner,
            b"synthetic.1",
            1,
            EventFeedScript {
                native_event: event(1),
                instance: Some(5),
                ..Default::default()
            },
        );
        push(
            EventFeedSource::EventOption,
            owner,
            b"synthetic.1",
            1,
            EventFeedScript {
                native_event: event(1),
                option: Some(-1),
                ..Default::default()
            },
        );
        push(state, none, b"", 0, change(PendingBaseline));
        push(
            state,
            none,
            b"synthetic.1",
            1,
            EventFeedScript {
                native_event: event(1),
                instance: Some(5),
                ..change(PendingStored { due_hour: 40 })
            },
        );
        push(state, none, b"", 0, change(PendingReady));
        push(state, none, b"", 0, change(FiredBaseline));
        for token in [b"synthetic.1".as_slice(), fired, b"synthetic.1"] {
            push(
                state,
                none,
                token,
                0,
                EventFeedScript {
                    native_event: event(token[10].into()),
                    ..change(FiredMember)
                },
            );
        }
        push(state, none, b"", 0, change(FiredReady));
        push(state, owner, b"", 0, change(DelayedBaseline));
        push(
            state,
            owner,
            fired,
            1,
            EventFeedScript {
                native_event: event(2),
                instance: Some(7),
                ..change(DelayedStored { remaining: 30 })
            },
        );
        push(state, owner, b"", 0, change(DelayedReady));
        push(state, none, b"", 0, decision(Operation::Baseline, 0, 0));
        push(
            state,
            none,
            b"decision.a",
            0,
            EventFeedScript {
                instance: Some(9),
                days: Some(4),
                ..decision(Operation::Store, 0, 0)
            },
        );
        push(state, none, b"", 0, decision(Operation::Ready, 0, 0));
        push(
            state,
            none,
            b"decision.a",
            0,
            EventFeedScript {
                days: Some(2),
                ..decision(Operation::RandomSet, 0, 6)
            },
        );
        push(
            state,
            none,
            b"decision.b",
            0,
            change(Decision(DecisionChange {
                flags: decisions::IGNORED,
                ..DecisionChange::new(Operation::Ignore, 0)
            })),
        );
        for target in 1..=3 {
            let target = EventFeedCountry {
                reference: target,
                id: target,
                instance: 0,
            };
            push(
                state,
                target,
                b"decision.a",
                0,
                EventFeedScript {
                    country_target: Some(3),
                    ..decision(Operation::Candidate, 0, target.reference)
                },
            );
        }
        push(state, none, b"", 0, decision(Operation::Rebuild, 3, 0));
        row.validate().unwrap();

        let mut interned = row.clone();
        interned.intern_tokens().unwrap();
        // Each distinct token once, in first-use order; empty tokens add nothing.
        assert_eq!(
            interned.script_tokens,
            b"synthetic.1synthetic.2decision.adecision.b"
        );
        let resolve = |row: &EventFeedHistoryRow| {
            row.events
                .iter()
                .map(|event| {
                    let script = event.script.unwrap();
                    row.script_tokens[script.token_start as usize..][..script.token_len as usize]
                        .to_vec()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(resolve(&interned), resolve(&row));
        let mut offsets = interned.clone();
        for (event, original) in offsets.events.iter_mut().zip(&row.events) {
            event.script.as_mut().unwrap().token_start = original.script.unwrap().token_start;
        }
        offsets.script_tokens = row.script_tokens.clone();
        assert_eq!(offsets, row, "only token offsets and the arena change");
        // Retained registries, delayed queues and decision rosters reduce alike.
        let reduce = |row: &EventFeedHistoryRow| {
            let mut state = ScriptEventState::default();
            let mut visits = Vec::new();
            state
                .observe_decisions(row, |owner, event| {
                    visits.push(format!("{} {owner:?}", event.sequence));
                    Ok(())
                })
                .unwrap();
            (format!("{state:?}"), visits)
        };
        assert_eq!(reduce(&interned), reduce(&row));
        assert_eq!(reduce(&row).1.len(), 9);
        let mut bytes = Vec::new();
        interned.encode(&mut bytes).unwrap();
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            interned
        );
        let mut again = interned.clone();
        again.intern_tokens().unwrap();
        assert_eq!(again, interned);
    }

    #[test]
    fn delayed_owner_refusal_carries_its_reason_and_nothing_else() {
        let mut row = row();
        row.script_tokens = b"synthetic delayed owner refusal".to_vec();
        row.events[0].source = EventFeedSource::ScriptState;
        row.events[0].target = row.events[0].actor;
        let reason = EventFeedScript {
            token_len: row.script_tokens.len() as u32,
            change: Some(EventFeedScriptChange::DelayedRefused),
            ..Default::default()
        };
        row.events[0].script = Some(reason);
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            row
        );
        row.script_scopes = vec![EventFeedScope::default()];
        for script in [
            EventFeedScript {
                token_len: 0,
                ..reason
            },
            EventFeedScript {
                token_len: 0,
                token_unrecorded: true,
                ..reason
            },
            EventFeedScript {
                instance: Some(1),
                ..reason
            },
            EventFeedScript {
                native_event: Some(EventFeedEventIdentity { kind: 1, id: 1 }),
                ..reason
            },
            EventFeedScript {
                scope_len: 1,
                ..reason
            },
            EventFeedScript {
                scope_unrecorded: true,
                ..reason
            },
        ] {
            row.events[0].script = Some(script);
            assert!(row.validate().is_err(), "{script:?}");
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
    fn script_occurrences_preserve_repeat_instances_signed_options_and_scope_ids() {
        let mut row = row();
        row.script_tokens = b"synthetic.1".to_vec();
        row.script_scopes = vec![
            EventFeedScope {
                country: 0,
                random: [u32::MAX, 7],
                bindings: [0, 1, 1],
                state: i32::MIN,
                references: [u64::MAX, 0, 4, 0, 0, 0, 0, 0, 0],
                ..Default::default()
            },
            EventFeedScope {
                bindings: [1, 1, 0],
                country: 5_000,
                country_attempted: true,
                country_id: Some(u32::MAX),
                country_names: Some(
                    [b"Reich".as_slice(), b"the Reich", b""]
                        .map(|text| EventFeedText::new(text).unwrap()),
                ),
                ..Default::default()
            },
        ];
        row.events[0].source = EventFeedSource::EventOption;
        row.events[0].script = Some(EventFeedScript {
            token_len: row.script_tokens.len() as u32,
            scope_len: 2,
            native_event: Some(EventFeedEventIdentity {
                kind: u32::MAX,
                id: u32::MAX,
            }),
            instance: Some(0),
            option: Some(i32::MIN),
            numeric_target: Some(i32::MIN),
            lifecycle: Some(i32::MAX),
            days: Some(-123),
            ..Default::default()
        });
        let mut second = row.events[0];
        second.sequence += 1;
        row.events.push(second);
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(row.encoded_bytes().unwrap(), bytes.len());
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            row
        );
        for end in 0..bytes.len() {
            assert!(EventFeedHistoryRow::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
        row.script_scopes[0].bindings[1] = 2;
        assert!(row.validate().is_err());
        row.script_scopes[0].bindings[1] = 1;
        row.script_scopes[0].country_id = Some(1);
        assert!(row.validate().is_err());
        row.script_scopes[0].country_id = None;
        row.script_scopes[1].country_id = None;
        assert!(row.validate().is_err());
        row.script_scopes[1].country_id = Some(u32::MAX);
        row.script_scopes[1].country_attempted = false;
        assert!(row.validate().is_err());
        row.script_scopes[1].country_attempted = true;
        row.events[0].script.as_mut().unwrap().token_len += 1;
        assert!(row.validate().is_err());
    }

    #[test]
    fn scope_codec_omits_zero_references_and_rejects_unbounded_ids_and_masks() {
        let scope = EventFeedScope::default();
        let mut bytes = Vec::new();
        scope.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(scope.encoded_bytes().unwrap(), bytes.len());
        assert_eq!(
            EventFeedScope::decode(&mut Decoder::new(&bytes)).unwrap(),
            scope
        );
        // The nine-bit object-reference mask starts after country, random,
        // three bindings and signed state. Higher bits have no native operand.
        bytes[14] = 2;
        assert!(EventFeedScope::decode(&mut Decoder::new(&bytes)).is_err());
        bytes[14] = 0;
        bytes[15] = 0x80;
        assert!(EventFeedScope::decode(&mut Decoder::new(&bytes)).is_err());
        // A failed attempt still marks the scope as from a recording that tries.
        let attempted = EventFeedScope {
            country: 7,
            country_attempted: true,
            ..Default::default()
        };
        let mut bytes = Vec::new();
        attempted.encode(&mut bytes).unwrap();
        assert_eq!(
            EventFeedScope::decode(&mut Decoder::new(&bytes)).unwrap(),
            attempted
        );
        let long = [b'x'; EventFeedText::CAPACITY + 1];
        assert!(EventFeedText::new(&long).is_none());
        assert!(EventFeedText::new(&long[1..]).is_some());
        let identified = EventFeedScope {
            country: 7,
            country_attempted: true,
            country_id: Some(9_000),
            country_names: Some(
                [b"Name".as_slice(), b"the Name", b"Named"]
                    .map(|text| EventFeedText::new(text).unwrap()),
            ),
            ..Default::default()
        };
        let mut bytes = Vec::new();
        identified.encode(&mut bytes).unwrap();
        assert_eq!(identified.encoded_bytes().unwrap(), bytes.len());
        assert_eq!(
            EventFeedScope::decode(&mut Decoder::new(&bytes)).unwrap(),
            identified
        );
        let mut bytes = Vec::new();
        crate::codec::put_varint(&mut bytes, u32::MAX as u128 + 1);
        assert!(EventFeedScope::decode(&mut Decoder::new(&bytes)).is_err());
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
    fn causes_and_routes_round_trip() {
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
        assert!(EventFeedSource::decode(&mut Decoder::new(&[24])).is_err());
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
