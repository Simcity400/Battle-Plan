pub mod a729_hook;

pub const DOMAIN_ID: u16 = 86;
pub const DOMAIN_NAME: &str = "sunk_ship_history_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"SHP1");
pub const SCHEMA_VERSION: u32 = 4;
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"466880cc290e8961dab48c8323346d9d862089d85bbe0f4ba285b924cc959c82");
pub const EXACT20_DATA_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"5e0ed3e76724032f3e2f4cc5bcc9e5c38347ed862e0489821be8b7bbdd45ca21");
pub const WRITER_SENTINEL_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"dccd61e7573317458980fc0796fcba2ba945a434fbacbf3303a588c620307a4a");
pub const ALL_WRITER_ROUTES_MANIFEST_SHA256: [u8; 32] =
    hex32(b"d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0");
pub const CSUNK_SHIP_VTABLE_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"61140fad7a24c150b60935580374129321719555bb5a4e1db88bd2bcd296937e");
pub const CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256: [u8; 32] =
    hex32(b"755a0011376d06bbd38456a2bb009236d99f4481a9b076789d9d57729c5673fd");
pub const ATTACKER_GETTERS_MANIFEST_SHA256: [u8; 32] =
    hex32(b"946eff0fb077863ce2dc1f1607ac2c0b9fcfee8f3377662ff14315101c30cbd6");

pub const PRODUCTION_SCHEDULING_ENABLED: bool = true;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;
pub const MAX_EVENTS: usize = 65_536;
pub const MAX_TEXT_BYTES: usize = 4_096;
pub const EVENT_FLAG_KILLED: u32 = 1;
pub const EVENT_FLAG_ASSISTED_KILL: u32 = 1 << 1;
pub const EVENT_KNOWN_FLAGS: u32 = EVENT_FLAG_KILLED | EVENT_FLAG_ASSISTED_KILL;
pub const FIRST_EVENT_SEQUENCE: u64 = 2;

pub const SNAPSHOT_FLAG_PRODUCER_HOUR: u8 = 1 << 0;
pub const SNAPSHOT_FLAG_PRODUCER_LOSS: u8 = 1 << 1;
const KNOWN_SNAPSHOT_FLAGS: u8 = SNAPSHOT_FLAG_PRODUCER_HOUR | SNAPSHOT_FLAG_PRODUCER_LOSS;

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(input[i * 2]) << 4) | nibble(input[i * 2 + 1]);
        i += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityState {
    Unavailable,
    StaticExact20Receipt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextUnavailable {
    LengthWordUnreadable,
    CapacityWordUnreadable,
    EngineEmpty,
    LengthExceedsCaptureBound,
    CapacityBelowLength,
    PayloadUnreadable,
    EmbeddedNul,
    NotUtf8,
}

/// The code, its reverse and the token are ONE declaration.
///
/// They were three matches over the same mapping, so swapping two tokens
/// compiled and made the attach log and the world fold publish each cause
/// under the other's name (Codex round 6). The token must now spell its own
/// variant, the two code directions are generated from one column, and the
/// codes are a dense set from 1.
macro_rules! text_unavailable_causes {
    ($( $variant:ident = $code:literal => $token:literal, )+) => {
        impl TextUnavailable {
            pub const fn code(self) -> u8 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            pub const fn from_code(code: u8) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub const fn token(self) -> &'static str {
                match self {
                    $(Self::$variant => $token,)+
                }
            }
        }

        const _: () = {
            assert!(
                compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &[$($code - 1,)+],
                    [$($code,)+].len(),
                ),
                "two text-unavailable causes share a code, or one is outside 1..=count",
            );
            $(assert!(
                compact_contract::diagnostic_channel::const_str_is_snake_of(
                    $token,
                    stringify!($variant),
                ),
                "a text-unavailable token does not spell the cause it names",
            );)+
        };
    };
}

text_unavailable_causes! {
    LengthWordUnreadable = 1 => "length_word_unreadable",
    CapacityWordUnreadable = 2 => "capacity_word_unreadable",
    EngineEmpty = 3 => "engine_empty",
    LengthExceedsCaptureBound = 4 => "length_exceeds_capture_bound",
    CapacityBelowLength = 5 => "capacity_below_length",
    PayloadUnreadable = 6 => "payload_unreadable",
    EmbeddedNul = 7 => "embedded_nul",
    NotUtf8 = 8 => "not_utf8",
}

impl TextUnavailable {
    pub const fn describe(self) -> &'static str {
        match self {
            Self::LengthWordUnreadable => "the std::string length word (+0x10) could not be read",
            Self::CapacityWordUnreadable => {
                "the std::string capacity word (+0x18) could not be read"
            }
            Self::EngineEmpty => "the engine holds a well-formed empty string here",
            Self::LengthExceedsCaptureBound => {
                "the std::string length is at or beyond the capture bound"
            }
            Self::CapacityBelowLength => {
                "the std::string capacity is below its length, so the object is not coherent"
            }
            Self::PayloadUnreadable => "the std::string character payload could not be read",
            Self::EmbeddedNul => "the std::string character payload holds an embedded NUL",
            Self::NotUtf8 => "the std::string character payload is not valid UTF-8",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExactText {
    Present(String),
    Unavailable(TextUnavailable),
}

impl ExactText {
    pub fn present(&self) -> Option<&str> {
        match self {
            Self::Present(value) => Some(value.as_str()),
            Self::Unavailable(_) => None,
        }
    }

    pub fn unavailable(&self) -> Option<TextUnavailable> {
        match self {
            Self::Present(_) => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

impl From<String> for ExactText {
    fn from(value: String) -> Self {
        Self::Present(value)
    }
}

impl From<&str> for ExactText {
    fn from(value: &str) -> Self {
        Self::Present(value.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShipEvent {
    pub event_sequence: u64,
    pub combat_join_present: bool,
    pub combat_id: u32,
    pub combat_generation: u64,
    pub victim_country_id_raw: u32,
    pub attacker_country_id_raw: u32,
    pub ui_source_country_id_raw: u32,
    pub sink_date_raw: i32,
    pub victim_member_battle_slot_index: u32,
    pub flags: u32,
    pub victim_ship_reference_raw: [u32; 2],
    pub damage_source_reference_raw: [u32; 2],
    pub task_force_id_raw: u32,
    pub terminal_context_raw: [u32; 2],
    pub cause_record_flag: bool,
    pub ship_name: ExactText,
    pub ship_class_name: ExactText,
    pub attribution_source_name: ExactText,
    pub attribution_source_class_name: ExactText,
}

impl SunkShipEvent {
    pub fn has_unavailable_text(&self) -> bool {
        self.ship_name.is_unavailable()
            || self.ship_class_name.is_unavailable()
            || self.attribution_source_name.is_unavailable()
            || self.attribution_source_class_name.is_unavailable()
    }
}

pub fn records_with_unavailable_text(events: &[SunkShipEvent]) -> u32 {
    u32::try_from(
        events
            .iter()
            .filter(|event| event.has_unavailable_text())
            .count(),
    )
    .unwrap_or(u32::MAX)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SunkShipProducerLoss {
    pub dropped_unbound: u64,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_lock_busy: u64,
    pub dropped_overflow: u64,
    pub sequence_regression: bool,
}

impl SunkShipProducerLoss {
    pub fn dropped_total(&self) -> u64 {
        self.dropped_unbound
            .saturating_add(self.dropped_reentrant)
            .saturating_add(self.dropped_unreadable)
            .saturating_add(self.dropped_lock_busy)
            .saturating_add(self.dropped_overflow)
    }

    pub fn names_a_cause(&self) -> bool {
        self.dropped_total() != 0 || self.sequence_regression
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShipSnapshot {
    pub source_sequence: u64,
    pub producer_complete: bool,
    pub records_with_unavailable_text: u32,
    pub producer_hour_raw: Option<i32>,
    pub producer_loss: Option<SunkShipProducerLoss>,
    pub events: Vec<SunkShipEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShipFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<SunkShipSnapshot>,
}

pub trait ExactSunkShipSource {
    fn is_exact_a729_writer_source(&self) -> bool;
    fn read_sunk_ship_snapshot(&self, frame_hour: u64) -> Result<SunkShipSnapshot, String>;
}

#[cfg(windows)]
pub struct InstalledA729HookSource;

#[cfg(windows)]
impl ExactSunkShipSource for InstalledA729HookSource {
    fn is_exact_a729_writer_source(&self) -> bool {
        a729_hook::installed()
    }

    fn read_sunk_ship_snapshot(&self, _frame_hour: u64) -> Result<SunkShipSnapshot, String> {
        a729_hook::current_complete_snapshot()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    ActivationDisabled,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

fn valid_text(value: &ExactText, required: bool) -> bool {
    match value {
        ExactText::Present(text) => {
            text.len() <= MAX_TEXT_BYTES
                && (!required || !text.is_empty())
                && !text.as_bytes().contains(&0)
        }
        ExactText::Unavailable(_) => true,
    }
}

pub fn validate_snapshot(snapshot: &SunkShipSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0 || snapshot.events.len() > MAX_EVENTS {
        return invalid("SHP1 snapshot sequence/completeness/count invalid");
    }
    if snapshot.producer_complete != snapshot.producer_loss.is_none() {
        return invalid("SHP1 completeness claim and producer loss disagree");
    }
    if snapshot
        .producer_loss
        .is_some_and(|loss| !loss.names_a_cause())
    {
        return invalid("SHP1 partial history names no lost callback");
    }
    if let Some(hour_raw) = snapshot.producer_hour_raw {
        let Some(producer_hour) = calendar_hour(hour_raw) else {
            return invalid("SHP1 producer hour is not a valid engine date");
        };
        if snapshot.events.iter().any(|event| {
            calendar_hour(event.sink_date_raw).map_or(true, |hour| hour > producer_hour)
        }) {
            return invalid("SHP1 producer hour is behind a sinking it dates");
        }
    }
    if snapshot.records_with_unavailable_text != records_with_unavailable_text(&snapshot.events) {
        return invalid(
            "SHP1 unavailable-text accounting does not match the admitted history rows",
        );
    }
    let mut previous_sequence: Option<u64> = None;
    for (index, event) in snapshot.events.iter().enumerate() {
        if snapshot.producer_complete {
            let expected_sequence = u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(FIRST_EVENT_SEQUENCE));
            if expected_sequence != Some(event.event_sequence) {
                return invalid(
                    "SHP1 history sequence is not contiguous, so a sinking may be missing from it",
                );
            }
        } else if event.event_sequence < FIRST_EVENT_SEQUENCE
            || previous_sequence.is_some_and(|prior| prior >= event.event_sequence)
        {
            return invalid("SHP1 partial history sequence is not strictly increasing");
        }
        previous_sequence = Some(event.event_sequence);
        let combat_join_valid = if event.combat_join_present {
            event.combat_id != 0
                && event.combat_generation != 0
                && event.victim_member_battle_slot_index
                    <= naval_combat_live_lane::domain49_50_naval_combat_live::MAX_BATTLE_SLOT_INDEX
        } else {
            event.combat_id == 0
                && event.combat_generation == 0
                && event.victim_member_battle_slot_index == 0
        };
        if event.event_sequence <= 1
            || !combat_join_valid
            || event.victim_country_id_raw == 0
            || (event.attacker_country_id_raw == 0 && !event.cause_record_flag)
            || event.sink_date_raw <= 0
            || event.victim_ship_reference_raw == [0, 0]
            || event.flags & !EVENT_KNOWN_FLAGS != 0
            || (event.flags & EVENT_FLAG_KILLED != 0)
                == (event.flags & EVENT_FLAG_ASSISTED_KILL != 0)
            || (event.cause_record_flag && event.flags & EVENT_FLAG_ASSISTED_KILL != 0)
            || !valid_text(&event.ship_name, true)
            || !valid_text(&event.ship_class_name, true)
            || !valid_text(&event.attribution_source_name, false)
            || !valid_text(&event.attribution_source_class_name, false)
        {
            return invalid("SHP1 event is incomplete, ambiguous, or duplicated");
        }
    }
    if snapshot
        .events
        .last()
        .map_or(snapshot.source_sequence != 1, |event| {
            snapshot.source_sequence != event.event_sequence
        })
    {
        return invalid("SHP1 source sequence does not match the complete ordered history");
    }
    if let Some(loss) = snapshot.producer_loss {
        let span = snapshot.events.last().map_or(0, |event| {
            event
                .event_sequence
                .saturating_sub(FIRST_EVENT_SEQUENCE)
                .saturating_add(1)
        });
        let holes = span.saturating_sub(snapshot.events.len() as u64);
        if holes > loss.dropped_total() {
            return invalid("SHP1 partial history has more sequence holes than it admits losing");
        }
    }
    Ok(())
}

fn calendar_hour(raw: i32) -> Option<u64> {
    compact_contract::calendar::hour_index_from_raw(raw).and_then(|hour| u64::try_from(hour).ok())
}

pub fn receipt_status_word(snapshot: &SunkShipSnapshot) -> u32 {
    if snapshot.producer_loss.is_some() {
        compact_contract::COMPACT_STATUS_PARTIAL_DATA
    } else {
        0
    }
}

fn declares_this_domains_expected_partial(status_flags: u32) -> bool {
    compact_contract::expected_partial_tolerance_applies(DOMAIN_ID, status_flags).is_some()
}

pub fn validate_frame(frame: &SunkShipFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable
            if frame.status_flags != 0
                && !declares_this_domains_expected_partial(frame.status_flags)
                && frame.snapshot.is_none() =>
        {
            Ok(())
        }
        AuthorityState::StaticExact20Receipt
            if frame.frame_hour > 0
                && frame.snapshot.is_some()
                && frame.status_flags == receipt_status_word(frame.snapshot.as_ref().unwrap()) =>
        {
            let snapshot = frame.snapshot.as_ref().unwrap();
            validate_snapshot(snapshot)?;
            if snapshot.events.iter().any(|event| {
                calendar_hour(event.sink_date_raw).map_or(true, |hour| hour > frame.frame_hour)
            }) {
                return invalid("SHP1 sink date is invalid or exceeds the frame hour");
            }
            Ok(())
        }
        _ => invalid("SHP1 authority/availability mismatch"),
    }
}

pub fn collect_and_encode<S: ExactSunkShipSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !PRODUCTION_SCHEDULING_ENABLED {
        return Err(CarrierError::ActivationDisabled);
    }
    if !source.is_exact_a729_writer_source() {
        return Err(CarrierError::Source(
            "source is not the receipt-bound a729 writer bracket".into(),
        ));
    }
    let snapshot = source
        .read_sunk_ship_snapshot(frame_hour)
        .map_err(CarrierError::Source)?;
    let status_flags = receipt_status_word(&snapshot);
    encode_payload(&SunkShipFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::StaticExact20Receipt,
        snapshot: Some(snapshot),
    })
}

pub fn encode_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&SunkShipFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::Unavailable,
        snapshot: None,
    })
}

pub fn encode_payload(frame: &SunkShipFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(
        &mut out,
        match frame.authority {
            AuthorityState::Unavailable => 0,
            AuthorityState::StaticExact20Receipt => 1,
        },
    );
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&EXACT20_FUNCTION_MANIFEST_SHA256);
    out.extend_from_slice(&EXACT20_DATA_ARTIFACT_SHA256);
    out.extend_from_slice(&WRITER_SENTINEL_ARTIFACT_SHA256);
    out.extend_from_slice(&ALL_WRITER_ROUTES_MANIFEST_SHA256);
    out.extend_from_slice(&CSUNK_SHIP_VTABLE_ARTIFACT_SHA256);
    out.extend_from_slice(&CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256);
    out.extend_from_slice(&ATTACKER_GETTERS_MANIFEST_SHA256);
    out.push(u8::from(frame.snapshot.is_some()));
    out.extend_from_slice(&[0; 3]);
    if let Some(snapshot) = &frame.snapshot {
        u64v(&mut out, snapshot.source_sequence);
        out.push(u8::from(snapshot.producer_complete));
        out.push(snapshot_flags(snapshot));
        out.extend_from_slice(&[0; 2]);
        u32v(&mut out, snapshot.records_with_unavailable_text);
        u32v(&mut out, snapshot.events.len() as u32);
        for e in &snapshot.events {
            u64v(&mut out, e.event_sequence);
            out.push(u8::from(e.combat_join_present));
            out.extend_from_slice(&[0; 7]);
            u32v(&mut out, e.combat_id);
            u64v(&mut out, e.combat_generation);
            u32v(&mut out, e.victim_country_id_raw);
            u32v(&mut out, e.attacker_country_id_raw);
            u32v(&mut out, e.ui_source_country_id_raw);
            i32v(&mut out, e.sink_date_raw);
            u32v(&mut out, e.victim_member_battle_slot_index);
            u32v(&mut out, e.flags);
            u32v(&mut out, e.victim_ship_reference_raw[0]);
            u32v(&mut out, e.victim_ship_reference_raw[1]);
            u32v(&mut out, e.damage_source_reference_raw[0]);
            u32v(&mut out, e.damage_source_reference_raw[1]);
            u32v(&mut out, e.task_force_id_raw);
            u32v(&mut out, e.terminal_context_raw[0]);
            u32v(&mut out, e.terminal_context_raw[1]);
            out.push(u8::from(e.cause_record_flag));
            out.extend_from_slice(&[0; 3]);
            exact_textv(&mut out, &e.ship_name);
            exact_textv(&mut out, &e.ship_class_name);
            exact_textv(&mut out, &e.attribution_source_name);
            exact_textv(&mut out, &e.attribution_source_class_name);
        }
        if let Some(hour) = snapshot.producer_hour_raw {
            i64v(&mut out, i64::from(hour));
        }
        if let Some(loss) = snapshot.producer_loss {
            u64v(&mut out, loss.dropped_unbound);
            u64v(&mut out, loss.dropped_reentrant);
            u64v(&mut out, loss.dropped_unreadable);
            u64v(&mut out, loss.dropped_lock_busy);
            u64v(&mut out, loss.dropped_overflow);
            out.push(u8::from(loss.sequence_regression));
            out.extend_from_slice(&[0; 7]);
        }
    }
    Ok(out)
}

fn snapshot_flags(snapshot: &SunkShipSnapshot) -> u8 {
    let mut flags = 0u8;
    if snapshot.producer_hour_raw.is_some() {
        flags |= SNAPSHOT_FLAG_PRODUCER_HOUR
    }
    if snapshot.producer_loss.is_some() {
        flags |= SNAPSHOT_FLAG_PRODUCER_LOSS
    }
    flags
}

pub fn decode_payload(payload: &[u8]) -> Result<SunkShipFrame, CarrierError> {
    let mut r = Reader { payload, at: 0 };
    if r.u32()? != MAGIC {
        return invalid("SHP1 magic/schema mismatch");
    };
    let schema_version = r.u32()?;
    if schema_version != 2 && schema_version != 3 && schema_version != SCHEMA_VERSION {
        return invalid("SHP1 magic/schema mismatch");
    };
    let authority = match r.u32()? {
        0 => AuthorityState::Unavailable,
        1 => AuthorityState::StaticExact20Receipt,
        _ => return invalid("SHP1 authority unknown"),
    };
    let status_flags = r.u32()?;
    let elapsed_wall_ms = r.u64()?;
    let frame_hour = r.u64()?;
    if r.bytes(32)? != EXECUTABLE_SHA256
        || r.bytes(32)? != EXACT20_FUNCTION_MANIFEST_SHA256
        || r.bytes(32)? != EXACT20_DATA_ARTIFACT_SHA256
        || r.bytes(32)? != WRITER_SENTINEL_ARTIFACT_SHA256
        || r.bytes(32)? != ALL_WRITER_ROUTES_MANIFEST_SHA256
        || r.bytes(32)? != CSUNK_SHIP_VTABLE_ARTIFACT_SHA256
        || r.bytes(32)? != CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256
        || r.bytes(32)? != ATTACKER_GETTERS_MANIFEST_SHA256
    {
        return invalid("SHP1 receipt identity mismatch");
    };
    let has = r.u8()?;
    if has > 1 || r.bytes(3)? != [0, 0, 0] {
        return invalid("SHP1 availability marker invalid");
    };
    let snapshot = if has == 1 {
        let source_sequence = r.u64()?;
        let producer_complete = r.u8()? != 0;
        let snapshot_flags = r.u8()?;
        if r.bytes(2)? != [0, 0] {
            return invalid("SHP1 completeness reserved bytes invalid");
        };
        if snapshot_flags & !KNOWN_SNAPSHOT_FLAGS != 0
            || (schema_version < SCHEMA_VERSION && snapshot_flags != 0)
        {
            return invalid("SHP1 snapshot flags unknown");
        };
        let carried_unavailable_text = if schema_version >= 4 { r.u32()? } else { 0 };
        let count = r.u32()? as usize;
        if count > MAX_EVENTS {
            return invalid("SHP1 event count exceeds bound");
        };
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            let event_sequence = r.u64()?;
            let combat_join_present = if schema_version >= 3 {
                let present = r.u8()?;
                if present > 1 || r.bytes(7)? != [0, 0, 0, 0, 0, 0, 0] {
                    return invalid("SHP1 combat-join marker invalid");
                };
                present != 0
            } else {
                true
            };
            let combat_id = r.u32()?;
            let combat_generation = r.u64()?;
            let victim_country_id_raw = r.u32()?;
            let attacker_country_id_raw = r.u32()?;
            let ui_source_country_id_raw = r.u32()?;
            let sink_date_raw = r.i32()?;
            let victim_member_battle_slot_index = r.u32()?;
            let flags = r.u32()?;
            let victim_ship_reference_raw = [r.u32()?, r.u32()?];
            let damage_source_reference_raw = [r.u32()?, r.u32()?];
            let task_force_id_raw = r.u32()?;
            let terminal_context_raw = [r.u32()?, r.u32()?];
            let cause_record_flag = r.u8()? != 0;
            if r.bytes(3)? != [0, 0, 0] {
                return invalid("SHP1 event reserved bytes invalid");
            };
            events.push(SunkShipEvent {
                event_sequence,
                combat_join_present,
                combat_id,
                combat_generation,
                victim_country_id_raw,
                attacker_country_id_raw,
                ui_source_country_id_raw,
                sink_date_raw,
                victim_member_battle_slot_index,
                flags,
                victim_ship_reference_raw,
                damage_source_reference_raw,
                task_force_id_raw,
                terminal_context_raw,
                cause_record_flag,
                ship_name: r.exact_text(schema_version)?,
                ship_class_name: r.exact_text(schema_version)?,
                attribution_source_name: r.exact_text(schema_version)?,
                attribution_source_class_name: r.exact_text(schema_version)?,
            });
        }
        let producer_hour_raw = if snapshot_flags & SNAPSHOT_FLAG_PRODUCER_HOUR != 0 {
            Some(i32::try_from(r.i64()?).map_err(|_| {
                CarrierError::Invalid(
                    "SHP1 producer hour is outside the engine's date range".into(),
                )
            })?)
        } else {
            None
        };
        let producer_loss = if snapshot_flags & SNAPSHOT_FLAG_PRODUCER_LOSS != 0 {
            let loss = SunkShipProducerLoss {
                dropped_unbound: r.u64()?,
                dropped_reentrant: r.u64()?,
                dropped_unreadable: r.u64()?,
                dropped_lock_busy: r.u64()?,
                dropped_overflow: r.u64()?,
                sequence_regression: {
                    let flag = r.u8()?;
                    if flag > 1 {
                        return invalid("SHP1 producer loss regression marker invalid");
                    };
                    flag != 0
                },
            };
            if r.bytes(7)? != [0, 0, 0, 0, 0, 0, 0] {
                return invalid("SHP1 producer loss reserved bytes invalid");
            };
            Some(loss)
        } else {
            None
        };
        Some(SunkShipSnapshot {
            source_sequence,
            producer_complete,
            records_with_unavailable_text: carried_unavailable_text,
            producer_hour_raw,
            producer_loss,
            events,
        })
    } else {
        None
    };
    if r.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    };
    let frame = SunkShipFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn u32v(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes())
}
fn i32v(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes())
}
fn u64v(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes())
}
fn i64v(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes())
}
fn textv(out: &mut Vec<u8>, v: &str) {
    u32v(out, v.len() as u32);
    out.extend_from_slice(v.as_bytes())
}
fn exact_textv(out: &mut Vec<u8>, v: &ExactText) {
    match v {
        ExactText::Present(text) => {
            out.push(0);
            textv(out, text)
        }
        ExactText::Unavailable(reason) => out.push(reason.code()),
    }
}
struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.at.checked_add(n).ok_or(CarrierError::Truncated)?;
        let b = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, CarrierError> {
        Ok(self.bytes(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, CarrierError> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, CarrierError> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, CarrierError> {
        let n = self.u32()? as usize;
        if n > MAX_TEXT_BYTES {
            return invalid("SHP1 text exceeds bound");
        };
        String::from_utf8(self.bytes(n)?.to_vec())
            .map_err(|_| CarrierError::Invalid("SHP1 text is not UTF-8".into()))
    }
    fn exact_text(&mut self, schema_version: u32) -> Result<ExactText, CarrierError> {
        if schema_version < 4 {
            return Ok(ExactText::Present(self.text()?));
        };
        match self.u8()? {
            0 => Ok(ExactText::Present(self.text()?)),
            code => match TextUnavailable::from_code(code) {
                Some(reason) => Ok(ExactText::Unavailable(reason)),
                None => invalid("SHP1 text unavailable reason unknown"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn frame() -> SunkShipFrame {
        SunkShipFrame {
            elapsed_wall_ms: 7,
            frame_hour: 16_959_360,
            status_flags: 0,
            authority: AuthorityState::StaticExact20Receipt,
            snapshot: Some(SunkShipSnapshot {
                source_sequence: 2,
                producer_complete: true,
                records_with_unavailable_text: 0,
                producer_hour_raw: None,
                producer_loss: None,
                events: vec![SunkShipEvent {
                    event_sequence: 2,
                    combat_join_present: true,
                    combat_id: 4,
                    combat_generation: 2,
                    victim_country_id_raw: 3,
                    attacker_country_id_raw: 9,
                    ui_source_country_id_raw: 5,
                    sink_date_raw: compact_contract::calendar::DATE_EPOCH_CONST + 16_959_300,
                    victim_member_battle_slot_index: 7,
                    flags: EVENT_FLAG_KILLED,
                    victim_ship_reference_raw: [11, 12],
                    damage_source_reference_raw: [13, 14],
                    task_force_id_raw: 15,
                    terminal_context_raw: [16, 17],
                    cause_record_flag: false,
                    ship_name: "HMS Exact".into(),
                    ship_class_name: "Destroyer".into(),
                    attribution_source_name: "KMS Exact".into(),
                    attribution_source_class_name: "Cruiser".into(),
                }],
            }),
        }
    }

    #[test]
    fn roundtrip_preserves_exact_epoch_and_writer_fields() {
        let frame = frame();
        assert_eq!(
            decode_payload(&encode_payload(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn optional_attacker_text_and_damage_source_absence_are_preserved() {
        let mut frame = frame();
        let event = &mut frame.snapshot.as_mut().unwrap().events[0];
        event.damage_source_reference_raw = [0, 0];
        event.attribution_source_name = ExactText::Present(String::new());
        event.attribution_source_class_name = ExactText::Present(String::new());
        assert_eq!(
            decode_payload(&encode_payload(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn a_sinking_with_no_attacking_country_is_admitted_only_as_a_cause_record() {
        let mut mine = frame();
        {
            let event = &mut mine.snapshot.as_mut().unwrap().events[0];
            event.cause_record_flag = true;
            event.attacker_country_id_raw = 0;
            event.combat_join_present = false;
            event.combat_id = 0;
            event.combat_generation = 0;
            event.victim_member_battle_slot_index = 0;
            event.attribution_source_name = "SUNK_BY_MINES".into();
        }
        assert_eq!(
            decode_payload(&encode_payload(&mine).unwrap()).unwrap(),
            mine,
            "a mine sinking has no attacking country and no combat join, and is \
             still a complete row"
        );

        let mut not_a_cause_record = mine;
        not_a_cause_record.snapshot.as_mut().unwrap().events[0].cause_record_flag = false;
        assert!(encode_payload(&not_a_cause_record).is_err());
    }

    #[test]
    fn attach_baseline_preserves_an_explicit_absent_combat_join() {
        let mut baseline = frame();
        let event = &mut baseline.snapshot.as_mut().unwrap().events[0];
        event.combat_join_present = false;
        event.combat_id = 0;
        event.combat_generation = 0;
        event.victim_member_battle_slot_index = 0;
        assert_eq!(
            decode_payload(&encode_payload(&baseline).unwrap()).unwrap(),
            baseline
        );

        let mut fabricated = baseline.clone();
        fabricated.snapshot.as_mut().unwrap().events[0].combat_id = 4;
        assert!(encode_payload(&fabricated).is_err());

        let mut missing_live_join = frame();
        missing_live_join.snapshot.as_mut().unwrap().events[0].combat_id = 0;
        assert!(encode_payload(&missing_live_join).is_err());
    }

    #[test]
    fn production_source_is_admitted_without_optional_fixture() {
        struct Source {
            authenticated: bool,
        }
        impl ExactSunkShipSource for Source {
            fn is_exact_a729_writer_source(&self) -> bool {
                self.authenticated
            }
            fn read_sunk_ship_snapshot(&self, _: u64) -> Result<SunkShipSnapshot, String> {
                Ok(frame().snapshot.unwrap())
            }
        }

        assert!(PRODUCTION_SCHEDULING_ENABLED);
        assert!(!LIVE_FIXTURE_ACCEPTED);
        assert!(collect_and_encode(
            &Source {
                authenticated: true
            },
            frame().frame_hour,
            0,
        )
        .is_ok());
        assert!(matches!(
            collect_and_encode(
                &Source {
                    authenticated: false
                },
                frame().frame_hour,
                0,
            ),
            Err(CarrierError::Source(_))
        ));
    }

    #[test]
    fn disappearance_style_ambiguous_flags_are_rejected() {
        let mut frame = frame();
        frame.snapshot.as_mut().unwrap().events[0].flags = 0;
        assert!(encode_payload(&frame).is_err());
    }

    #[test]
    fn date_slot_order_and_history_tail_are_fail_closed() {
        let mut future = frame();
        future.snapshot.as_mut().unwrap().events[0].sink_date_raw =
            compact_contract::calendar::DATE_EPOCH_CONST + 16_959_361;
        assert!(encode_payload(&future).is_err());

        let mut invalid_slot = frame();
        invalid_slot.snapshot.as_mut().unwrap().events[0].victim_member_battle_slot_index = 4_097;
        assert!(encode_payload(&invalid_slot).is_err());

        let mut stale_tail = frame();
        stale_tail.snapshot.as_mut().unwrap().source_sequence = 3;
        assert!(encode_payload(&stale_tail).is_err());

        let mut reordered = frame();
        let mut older = reordered.snapshot.as_ref().unwrap().events[0].clone();
        older.event_sequence = 1;
        reordered.snapshot.as_mut().unwrap().events[0].event_sequence = 2;
        reordered.snapshot.as_mut().unwrap().events.push(older);
        reordered.snapshot.as_mut().unwrap().source_sequence = 1;
        assert!(encode_payload(&reordered).is_err());
    }

    const EVERY_REASON: [TextUnavailable; 8] = [
        TextUnavailable::LengthWordUnreadable,
        TextUnavailable::CapacityWordUnreadable,
        TextUnavailable::EngineEmpty,
        TextUnavailable::LengthExceedsCaptureBound,
        TextUnavailable::CapacityBelowLength,
        TextUnavailable::PayloadUnreadable,
        TextUnavailable::EmbeddedNul,
        TextUnavailable::NotUtf8,
    ];

    #[test]
    fn each_unavailable_reason_has_its_own_code_and_its_own_sentence() {
        let codes: BTreeSet<u8> = EVERY_REASON.iter().map(|reason| reason.code()).collect();
        assert_eq!(
            codes.len(),
            EVERY_REASON.len(),
            "two reasons share a wire code, so the wire cannot tell them apart"
        );
        assert!(
            !codes.contains(&0),
            "zero is the present marker and can never be a reason"
        );

        let sentences: BTreeSet<&str> = EVERY_REASON
            .iter()
            .map(|reason| reason.describe())
            .collect();
        assert_eq!(
            sentences.len(),
            EVERY_REASON.len(),
            "two reasons share a sentence, so an operator cannot tell them apart"
        );
        assert!(sentences.iter().all(|sentence| !sentence.is_empty()));

        for reason in EVERY_REASON {
            assert_eq!(TextUnavailable::from_code(reason.code()), Some(reason));
        }
        assert_eq!(TextUnavailable::from_code(0), None);
        assert_eq!(TextUnavailable::from_code(9), None);
    }

    #[test]
    fn an_unavailable_field_is_admitted_and_its_exact_reason_survives_the_wire() {
        for reason in EVERY_REASON {
            let mut unnamed = frame();
            let snapshot = unnamed.snapshot.as_mut().unwrap();
            snapshot.events[0].ship_name = ExactText::Unavailable(reason);
            snapshot.records_with_unavailable_text = 1;

            let decoded = decode_payload(
                &encode_payload(&unnamed)
                    .unwrap_or_else(|error| panic!("{reason:?} must be admitted: {error:?}")),
            )
            .unwrap_or_else(|error| panic!("{reason:?} must survive decode: {error:?}"));
            assert_eq!(decoded, unnamed, "{reason:?} did not round-trip exactly");
            assert_eq!(
                decoded.snapshot.unwrap().events[0].ship_name,
                ExactText::Unavailable(reason)
            );
        }
    }

    #[test]
    fn an_admitted_unavailable_field_cannot_vanish_from_the_completeness_accounting() {
        let mut honest = frame();
        let snapshot = honest.snapshot.as_mut().unwrap();
        snapshot.events[0].ship_class_name =
            ExactText::Unavailable(TextUnavailable::CapacityBelowLength);
        snapshot.records_with_unavailable_text = 1;
        assert!(validate_snapshot(honest.snapshot.as_ref().unwrap()).is_ok());
        assert_eq!(
            decode_payload(&encode_payload(&honest).unwrap())
                .unwrap()
                .snapshot
                .unwrap()
                .records_with_unavailable_text,
            1,
            "the count travels with the rows it accounts for"
        );

        let mut understated = honest.clone();
        understated
            .snapshot
            .as_mut()
            .unwrap()
            .records_with_unavailable_text = 0;
        assert!(
            encode_payload(&understated).is_err(),
            "a snapshot may not carry an unavailable field and claim none"
        );

        let mut overstated = frame();
        overstated
            .snapshot
            .as_mut()
            .unwrap()
            .records_with_unavailable_text = 1;
        assert!(
            encode_payload(&overstated).is_err(),
            "nor claim an absence it does not carry"
        );

        let mut multi = frame();
        let snapshot = multi.snapshot.as_mut().unwrap();
        snapshot.events[0].ship_name = ExactText::Unavailable(TextUnavailable::EngineEmpty);
        snapshot.events[0].ship_class_name = ExactText::Unavailable(TextUnavailable::NotUtf8);
        snapshot.events[0].attribution_source_name =
            ExactText::Unavailable(TextUnavailable::EmbeddedNul);
        snapshot.records_with_unavailable_text = 1;
        assert!(encode_payload(&multi).is_ok());
    }

    #[test]
    fn present_text_still_fails_closed_on_an_empty_name_a_nul_or_an_overlong_value() {
        let mut empty_name = frame();
        empty_name.snapshot.as_mut().unwrap().events[0].ship_name =
            ExactText::Present(String::new());
        assert!(
            encode_payload(&empty_name).is_err(),
            "an empty *present* name is an encoder defect; emptiness is carried as a reason"
        );

        let mut empty_class = frame();
        empty_class.snapshot.as_mut().unwrap().events[0].ship_class_name =
            ExactText::Present(String::new());
        assert!(encode_payload(&empty_class).is_err());

        let mut nul = frame();
        nul.snapshot.as_mut().unwrap().events[0].ship_name = ExactText::Present("HM\0S".into());
        assert!(encode_payload(&nul).is_err());

        let mut overlong = frame();
        overlong.snapshot.as_mut().unwrap().events[0].attribution_source_name =
            ExactText::Present("x".repeat(MAX_TEXT_BYTES + 1));
        assert!(encode_payload(&overlong).is_err());

        let mut absent_attacker = frame();
        absent_attacker.snapshot.as_mut().unwrap().events[0].attribution_source_name =
            ExactText::Present(String::new());
        assert!(encode_payload(&absent_attacker).is_ok());
    }

    #[test]
    fn schema_3_payloads_decode_as_all_present_with_no_unavailable_records() {
        let mut v3 = Vec::new();
        let source = frame();
        let snapshot = source.snapshot.as_ref().unwrap();
        u32v(&mut v3, MAGIC);
        u32v(&mut v3, 3);
        u32v(&mut v3, 1);
        u32v(&mut v3, source.status_flags);
        u64v(&mut v3, source.elapsed_wall_ms);
        u64v(&mut v3, source.frame_hour);
        for receipt in [
            EXECUTABLE_SHA256,
            EXACT20_FUNCTION_MANIFEST_SHA256,
            EXACT20_DATA_ARTIFACT_SHA256,
            WRITER_SENTINEL_ARTIFACT_SHA256,
            ALL_WRITER_ROUTES_MANIFEST_SHA256,
            CSUNK_SHIP_VTABLE_ARTIFACT_SHA256,
            CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256,
            ATTACKER_GETTERS_MANIFEST_SHA256,
        ] {
            v3.extend_from_slice(&receipt);
        }
        v3.push(1);
        v3.extend_from_slice(&[0; 3]);
        u64v(&mut v3, snapshot.source_sequence);
        v3.push(1);
        v3.extend_from_slice(&[0; 3]);
        u32v(&mut v3, snapshot.events.len() as u32);
        for event in &snapshot.events {
            u64v(&mut v3, event.event_sequence);
            v3.push(u8::from(event.combat_join_present));
            v3.extend_from_slice(&[0; 7]);
            u32v(&mut v3, event.combat_id);
            u64v(&mut v3, event.combat_generation);
            u32v(&mut v3, event.victim_country_id_raw);
            u32v(&mut v3, event.attacker_country_id_raw);
            u32v(&mut v3, event.ui_source_country_id_raw);
            i32v(&mut v3, event.sink_date_raw);
            u32v(&mut v3, event.victim_member_battle_slot_index);
            u32v(&mut v3, event.flags);
            u32v(&mut v3, event.victim_ship_reference_raw[0]);
            u32v(&mut v3, event.victim_ship_reference_raw[1]);
            u32v(&mut v3, event.damage_source_reference_raw[0]);
            u32v(&mut v3, event.damage_source_reference_raw[1]);
            u32v(&mut v3, event.task_force_id_raw);
            u32v(&mut v3, event.terminal_context_raw[0]);
            u32v(&mut v3, event.terminal_context_raw[1]);
            v3.push(u8::from(event.cause_record_flag));
            v3.extend_from_slice(&[0; 3]);
            for text in [
                &event.ship_name,
                &event.ship_class_name,
                &event.attribution_source_name,
                &event.attribution_source_class_name,
            ] {
                textv(
                    &mut v3,
                    text.present().expect("the v3 fixture is all present"),
                );
            }
        }

        assert_eq!(
            decode_payload(&v3).expect("a v3 payload still decodes"),
            source,
            "v3 rows are all present, and a v3 snapshot accounts for no unavailable records"
        );
    }

    #[test]
    fn a_gap_in_the_history_sequence_fails_the_snapshot_closed() {
        let mut two_rows = frame();
        let snapshot = two_rows.snapshot.as_mut().unwrap();
        let mut second = snapshot.events[0].clone();
        second.event_sequence = 3;
        snapshot.events.push(second);
        snapshot.source_sequence = 3;
        assert!(
            encode_payload(&two_rows).is_ok(),
            "a contiguous two-row history is the control"
        );

        let mut gapped = two_rows.clone();
        let snapshot = gapped.snapshot.as_mut().unwrap();
        snapshot.events[1].event_sequence = 4;
        snapshot.source_sequence = 4;
        assert!(
            matches!(encode_payload(&gapped), Err(CarrierError::Invalid(_))),
            "a history that skips sequence 3 is still increasing and its tail still              matches, and it is exactly what a dropped record looks like"
        );

        let mut dropped_head = frame();
        let snapshot = dropped_head.snapshot.as_mut().unwrap();
        snapshot.events[0].event_sequence = 9;
        snapshot.source_sequence = 9;
        assert!(
            matches!(encode_payload(&dropped_head), Err(CarrierError::Invalid(_))),
            "nor may a history begin anywhere but the first sinking"
        );

        let mut duplicated = two_rows;
        let snapshot = duplicated.snapshot.as_mut().unwrap();
        snapshot.events[1].event_sequence = 2;
        snapshot.source_sequence = 2;
        assert!(matches!(
            encode_payload(&duplicated),
            Err(CarrierError::Invalid(_))
        ));
    }

    const SNAPSHOT_COMPLETE_BYTE: usize = 32 + 8 * 32 + 4 + 8;
    const SNAPSHOT_FLAG_BYTE: usize = SNAPSHOT_COMPLETE_BYTE + 1;

    fn hour_raw(hour: u64) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + hour as i32
    }

    fn loss(dropped_unbound: u64) -> SunkShipProducerLoss {
        SunkShipProducerLoss {
            dropped_unbound,
            ..SunkShipProducerLoss::default()
        }
    }

    fn partial(dropped_unbound: u64) -> SunkShipFrame {
        let mut value = frame();
        let snapshot = value.snapshot.as_mut().unwrap();
        snapshot.producer_complete = false;
        snapshot.producer_loss = Some(loss(dropped_unbound));
        value.status_flags = receipt_status_word(value.snapshot.as_ref().unwrap());
        value
    }

    #[test]
    fn a_partial_receipt_declares_partial_data_and_a_complete_one_declares_nothing() {
        let complete = frame();
        assert_eq!(
            receipt_status_word(complete.snapshot.as_ref().unwrap()),
            0,
            "a complete history carries no failure bit"
        );
        assert!(encode_payload(&complete).is_ok());

        let partial = partial(1);
        assert_eq!(
            partial.status_flags,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            "a partial history declares PARTIAL_DATA"
        );
        let bytes = encode_payload(&partial).expect("a partial receipt encodes");
        assert_eq!(
            decode_payload(&bytes).unwrap().status_flags,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            "and the declaration survives the round trip"
        );

        assert!(
            compact_contract::expected_partial_tolerance_applies(
                DOMAIN_ID,
                compact_contract::COMPACT_STATUS_PARTIAL_DATA
            )
            .is_some(),
            "domain {DOMAIN_ID} must be a listed expected-partial domain"
        );
    }

    #[test]
    fn a_receipt_status_word_that_contradicts_its_loss_block_is_refused() {
        let mut silent_partial = partial(1);
        silent_partial.status_flags = 0;
        assert!(
            encode_payload(&silent_partial).is_err(),
            "a partial history may not present a clean status word"
        );

        let mut lying_complete = frame();
        lying_complete.status_flags = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
        assert!(
            encode_payload(&lying_complete).is_err(),
            "and a complete history may not declare a partial it does not carry"
        );
    }

    #[test]
    fn an_unavailable_frame_may_not_declare_the_expected_partial() {
        assert!(
            encode_unavailable(16_959_360, 7, compact_contract::COMPACT_STATUS_PARTIAL_DATA)
                .is_err(),
            "an unavailable frame has no history to be partial about"
        );
        assert!(
            encode_unavailable(
                16_959_360,
                7,
                compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
            )
            .is_ok(),
            "an ordinary failure word is unaffected"
        );
        assert!(
            encode_unavailable(
                16_959_360,
                7,
                compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
                    | compact_contract::COMPACT_STATUS_PARTIAL_DATA
            )
            .is_ok(),
            "and a partial bit beside a genuine failure is not a partial declaration: the \
             tolerance does not apply to that word, so no reader tolerates it"
        );
    }

    #[test]
    fn a_payload_carrying_neither_new_fact_is_byte_for_byte_unchanged() {
        let bare = frame();
        let bytes = encode_payload(&bare).unwrap();
        assert_eq!(
            bytes[SNAPSHOT_FLAG_BYTE], 0,
            "no flag bit is set for a fact that is not carried"
        );
        assert_eq!(
            bytes[SNAPSHOT_FLAG_BYTE + 1..SNAPSHOT_FLAG_BYTE + 3],
            [0, 0],
            "the two bytes after the flag byte stay reserved"
        );
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            SCHEMA_VERSION,
            "an additive fact does not mint a new schema version"
        );
        assert_eq!(decode_payload(&bytes).unwrap(), bare);

        let mut stamped = frame();
        stamped.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_300));
        let stamped_bytes = encode_payload(&stamped).unwrap();
        assert_eq!(
            stamped_bytes.len(),
            bytes.len() + 8,
            "a stamped payload must not move any byte a pre-existing decoder reads"
        );
        let mut prefix = stamped_bytes[..bytes.len()].to_vec();
        assert_eq!(
            prefix[SNAPSHOT_FLAG_BYTE], SNAPSHOT_FLAG_PRODUCER_HOUR,
            "a decoder that predates the bit sees a nonzero reserved byte and refuses"
        );
        prefix[SNAPSHOT_FLAG_BYTE] = 0;
        assert_eq!(prefix, bytes, "no other byte moved");
    }

    #[test]
    fn the_producer_hour_round_trips_and_only_an_engine_date_is_admitted() {
        for hour in [16_959_300, 16_959_330, 16_959_360] {
            let mut value = frame();
            value.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(hour));
            let decoded = decode_payload(&encode_payload(&value).unwrap()).unwrap();
            assert_eq!(
                decoded.snapshot.unwrap().producer_hour_raw,
                Some(hour_raw(hour))
            );
        }

        for not_a_date in [
            0,
            -1,
            i32::MIN,
            compact_contract::calendar::DATE_EPOCH_CONST + 1,
        ] {
            let mut value = frame();
            value.snapshot.as_mut().unwrap().producer_hour_raw = Some(not_a_date);
            assert!(
                encode_payload(&value).is_err(),
                "not a date the engine ever holds: {not_a_date}"
            );
        }

        let mut empty = frame();
        {
            let snapshot = empty.snapshot.as_mut().unwrap();
            snapshot.events.clear();
            snapshot.source_sequence = 1;
            snapshot.producer_hour_raw = Some(compact_contract::calendar::DATE_EPOCH_CONST + 1);
        }
        assert!(
            encode_payload(&empty).is_err(),
            "an out-of-window stamp on an empty history is refused by the window rule alone"
        );
        let mut in_window = empty;
        in_window.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_300));
        assert!(
            encode_payload(&in_window).is_ok(),
            "an empty history with a real stamp is the ordinary pre-first-sinking case"
        );

        let mut value = frame();
        value.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_300));
        let mut bytes = encode_payload(&value).unwrap();
        let trailer = bytes.len() - 8;
        let truncates_to_a_valid_date = (1i64 << 32) + i64::from(hour_raw(16_959_300));
        assert_eq!(truncates_to_a_valid_date as i32, hour_raw(16_959_300));
        bytes[trailer..].copy_from_slice(&truncates_to_a_valid_date.to_le_bytes());
        assert!(
            matches!(decode_payload(&bytes), Err(CarrierError::Invalid(_))),
            "a word outside the engine's date range is refused, not truncated"
        );
    }

    #[test]
    fn a_producer_hour_may_not_be_behind_a_sinking_it_dates() {
        let mut behind = frame();
        {
            let snapshot = behind.snapshot.as_mut().unwrap();
            snapshot.events[0].sink_date_raw = hour_raw(16_959_300);
            snapshot.producer_hour_raw = Some(hour_raw(16_959_299));
        }
        assert!(
            encode_payload(&behind).is_err(),
            "a stamp behind the newest sinking is not a producer observation of it"
        );

        let mut level = behind.clone();
        level.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_300));
        assert!(
            encode_payload(&level).is_ok(),
            "a stamp at the sinking's own hour is the ordinary case"
        );

        let mut ahead = behind;
        ahead.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_360));
        assert!(encode_payload(&ahead).is_ok());
    }

    #[test]
    fn the_producer_hour_does_not_widen_the_frame_hour_ceiling() {
        let mut future = frame();
        {
            let snapshot = future.snapshot.as_mut().unwrap();
            snapshot.events[0].sink_date_raw = hour_raw(16_959_361);
            snapshot.producer_hour_raw = Some(hour_raw(16_959_361));
        }
        assert!(
            encode_payload(&future).is_err(),
            "the stamp does not admit a sinking dated past the frame label"
        );
    }

    #[test]
    fn an_unavailable_frame_carries_no_producer_hour() {
        let mut bytes = encode_unavailable(16_959_360, 7, 256).unwrap();
        assert_eq!(
            bytes[SNAPSHOT_COMPLETE_BYTE - 12],
            0,
            "the availability marker says there is no snapshot"
        );
        assert_eq!(bytes.len(), 292);
        bytes.extend_from_slice(&i64::from(hour_raw(16_959_360)).to_le_bytes());
        assert!(
            decode_payload(&bytes).is_err(),
            "an unavailable frame has no snapshot for a producer hour to date"
        );
    }

    #[test]
    fn a_truncated_producer_trailer_fails_closed() {
        let mut value = partial(3);
        value.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_300));
        let bytes = encode_payload(&value).unwrap();
        for cut in 1..=56 {
            assert!(
                decode_payload(&bytes[..bytes.len() - cut]).is_err(),
                "a trailer short by {cut} bytes must not decode"
            );
        }
        let mut padded = bytes;
        padded.push(0);
        assert!(matches!(
            decode_payload(&padded),
            Err(CarrierError::TrailingBytes)
        ));

        let mut hour_only = frame();
        hour_only.snapshot.as_mut().unwrap().producer_hour_raw = Some(hour_raw(16_959_300));
        let bytes = encode_payload(&hour_only).unwrap();
        for cut in 1..=8 {
            assert!(
                decode_payload(&bytes[..bytes.len() - cut]).is_err(),
                "an hour-only trailer short by {cut} bytes must not decode"
            );
        }
    }

    #[test]
    fn an_unknown_snapshot_flag_bit_fails_closed() {
        let bytes = encode_payload(&frame()).unwrap();
        for bit in [1u8 << 2, 1 << 3, 1 << 7] {
            let mut unknown = bytes.clone();
            unknown[SNAPSHOT_FLAG_BYTE] = bit;
            assert!(
                matches!(decode_payload(&unknown), Err(CarrierError::Invalid(_))),
                "bit {bit:#x} is not a fact this build can size"
            );
        }
        let mut reserved = bytes;
        reserved[SNAPSHOT_FLAG_BYTE + 1] = 1;
        assert!(matches!(
            decode_payload(&reserved),
            Err(CarrierError::Invalid(_))
        ));
    }

    #[test]
    fn a_partial_batch_can_never_present_as_complete() {
        let mut lying = frame();
        {
            let snapshot = lying.snapshot.as_mut().unwrap();
            snapshot.producer_complete = true;
            snapshot.producer_loss = Some(loss(1));
        }
        assert!(
            encode_payload(&lying).is_err(),
            "a batch that lost a sinking may not claim to be whole"
        );

        let mut unexplained = frame();
        unexplained.snapshot.as_mut().unwrap().producer_complete = false;
        assert!(
            encode_payload(&unexplained).is_err(),
            "and completeness may not be withdrawn without naming a loss"
        );

        let honest = partial(1);
        let mut bytes = encode_payload(&honest).unwrap();
        assert_eq!(bytes[SNAPSHOT_COMPLETE_BYTE], 0);
        bytes[SNAPSHOT_COMPLETE_BYTE] = 1;
        assert!(
            matches!(decode_payload(&bytes), Err(CarrierError::Invalid(_))),
            "flipping the completeness byte on the wire is refused too"
        );
    }

    #[test]
    fn a_partial_batch_must_name_at_least_one_cause() {
        let mut nameless = frame();
        {
            let snapshot = nameless.snapshot.as_mut().unwrap();
            snapshot.producer_complete = false;
            snapshot.producer_loss = Some(SunkShipProducerLoss::default());
        }
        assert!(
            encode_payload(&nameless).is_err(),
            "an incompleteness that names nothing is not a partial batch"
        );

        let mut regression = nameless;
        regression.snapshot.as_mut().unwrap().producer_loss = Some(SunkShipProducerLoss {
            sequence_regression: true,
            ..SunkShipProducerLoss::default()
        });
        regression.status_flags = receipt_status_word(regression.snapshot.as_ref().unwrap());
        let decoded = decode_payload(&encode_payload(&regression).unwrap()).unwrap();
        assert_eq!(decoded, regression);
        assert!(
            decoded
                .snapshot
                .unwrap()
                .producer_loss
                .unwrap()
                .sequence_regression
        );
    }

    #[test]
    fn each_loss_cause_round_trips_as_its_own_number() {
        let carried = SunkShipProducerLoss {
            dropped_unbound: 1,
            dropped_reentrant: 2,
            dropped_unreadable: 3,
            dropped_lock_busy: 4,
            dropped_overflow: 5,
            sequence_regression: true,
        };
        let mut value = frame();
        {
            let snapshot = value.snapshot.as_mut().unwrap();
            snapshot.producer_complete = false;
            snapshot.producer_loss = Some(carried);
        }
        value.status_flags = receipt_status_word(value.snapshot.as_ref().unwrap());
        let decoded = decode_payload(&encode_payload(&value).unwrap()).unwrap();
        assert_eq!(
            decoded.snapshot.as_ref().unwrap().producer_loss,
            Some(carried)
        );
        assert_eq!(decoded, value);
        assert_eq!(carried.dropped_total(), 15);
    }

    #[test]
    fn a_partial_history_may_hole_only_as_far_as_it_admits_losing() {
        let mut budgeted = partial(1);
        {
            let snapshot = budgeted.snapshot.as_mut().unwrap();
            let mut second = snapshot.events[0].clone();
            second.event_sequence = 4;
            snapshot.events.push(second);
            snapshot.source_sequence = 4;
        }
        assert!(
            encode_payload(&budgeted).is_ok(),
            "one hole against one admitted drop is exactly what a lost callback leaves"
        );
        assert_eq!(
            decode_payload(&encode_payload(&budgeted).unwrap()).unwrap(),
            budgeted
        );

        let mut overdrawn = budgeted.clone();
        {
            let snapshot = overdrawn.snapshot.as_mut().unwrap();
            snapshot.events[1].event_sequence = 9;
            snapshot.source_sequence = 9;
        }
        assert!(
            matches!(encode_payload(&overdrawn), Err(CarrierError::Invalid(_))),
            "a partial history may not hide holes it did not admit"
        );

        let mut reordered = budgeted.clone();
        {
            let snapshot = reordered.snapshot.as_mut().unwrap();
            snapshot.events[1].event_sequence = 2;
            snapshot.source_sequence = 2;
        }
        assert!(matches!(
            encode_payload(&reordered),
            Err(CarrierError::Invalid(_))
        ));

        let mut complete_with_a_hole = budgeted;
        {
            let snapshot = complete_with_a_hole.snapshot.as_mut().unwrap();
            snapshot.producer_complete = true;
            snapshot.producer_loss = None;
        }
        assert!(
            matches!(
                encode_payload(&complete_with_a_hole),
                Err(CarrierError::Invalid(_))
            ),
            "contiguity is still what a completeness claim means"
        );
    }

    #[test]
    fn an_unknown_unavailable_reason_code_fails_closed() {
        let mut unnamed = frame();
        let snapshot = unnamed.snapshot.as_mut().unwrap();
        snapshot.events[0].ship_name = ExactText::Unavailable(TextUnavailable::EngineEmpty);
        snapshot.records_with_unavailable_text = 1;
        let mut payload = encode_payload(&unnamed).unwrap();

        let marker = payload
            .iter()
            .rposition(|byte| *byte == TextUnavailable::EngineEmpty.code())
            .expect("the reason byte is on the wire");
        payload[marker] = 9;
        assert!(
            matches!(decode_payload(&payload), Err(CarrierError::Invalid(_))),
            "a reason this build does not know is not silently dropped"
        );
    }
}
