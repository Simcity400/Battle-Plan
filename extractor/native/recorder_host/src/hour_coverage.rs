//! Per-hour VERIFIED COVERAGE — what a sealed recording asserts.
//!
//! Owner ruling 2026-08-11 (task #188): *"a sealed container asserts exactly
//! which hours are verified; an injured/undecodable hour is EXCLUDED with a
//! named reason; every other hour seals, certifies, and replays."* Whole-stream
//! all-or-nothing certification is retired.
//!
//! What replaced it, precisely:
//!
//!  * The old claim was one bit over the whole recording — *"this stream was
//!    walked end to end and every byte framed"*. One torn tail, one flipped
//!    framing byte or one sequence hole anywhere in a nine-hour capture cost
//!    the entire recording (lane GQ's matrix, cells a/c1/c2/d/h: 15 of 22
//!    failures, and cell (c2) is the sharpest — all nine hours decoded
//!    BYTE-IDENTICALLY to the clean control and the seal refused anyway).
//!  * The new claim is a MAP: one row per game hour the walk reached, each row
//!    either `Verified` — carrying the SHA-256 of exactly the stream bytes that
//!    hour is made of — or `Excluded`, carrying a named reason.
//!
//! Fail-closed is unchanged, only its granularity moved from the run to the
//! hour. Three invariants carry that, and each has a test that fails on its
//! exact regression:
//!
//!  1. **A verified hour needs a digest-backed basis.** [`HourCoverageMap::validate`]
//!     refuses a `Verified` row whose `basis_sha256` is not a 64-character hex
//!     digest. A row cannot claim verification it cannot show the bytes for.
//!  2. **Exclusion is monotone: an hour may be judged DOWN, never up.**
//!     [`HourCoverageMap::record_verified`] on an hour already excluded keeps
//!     the exclusion and its reason. A late clean-looking observation of an
//!     hour some earlier evidence refused must not launder it.
//!  3. **An excluded hour is never guessed, only named.** An `Excluded` row
//!     with an empty reason is refused by `validate`.
//!
//! ## WHAT `verified` ASSERTS, AND WHAT IT DOES NOT (owner ruling 2026-08-12)
//!
//! `Verified` asserts PER-HOUR INTEGRITY and nothing wider: *this hour's own
//! records were framed end to end, its frame closed canonically, and here is
//! the digest of those records.* It does NOT assert transitive purity — that
//! the hour's published STATE was derived only from other verified hours.
//!
//! It cannot, and the owner ruled that it must not pretend to. The fold is a
//! running state and a closed-without-publishing hour deliberately keeps its
//! accumulator mutations (`WorldFold::close_refused_hour`), so a verified hour
//! after an excluded one is legitimately built through it. A consumer that
//! cares about that derives it: the map already names every excluded hour, so
//! "which published hours are downstream of an exclusion" is a question its
//! rows answer. Minting a second status word for it would add vocabulary
//! without adding evidence.
//!
//! Anything the walk could not attribute to a specific hour — bytes between two
//! framing positions that belong to no closed frame — is named in
//! [`HourCoverageMap::unattributed_losses`] rather than silently dropped or
//! guessed onto a neighbouring hour.
//!
//! ## Unassigned bits reject at the decoder
//!
//! [`HourCoverageMap::from_json`] is strict in both directions: an unknown
//! object key, an unknown `status` word, a `verified` row carrying a `reason`,
//! an `excluded` row carrying a `basis_sha256`, or a summary count that
//! disagrees with the rows all refuse. A reader that cannot fully account for
//! the map it was handed must not act on part of it.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

/// The coverage map's own format identifier, written into every serialized
/// copy and required by [`HourCoverageMap::from_json`].
pub const HOUR_COVERAGE_SCHEMA: &str = "hoi4.recording.hour_coverage.v1";

/// The container/index key the map is published under.
pub const HOUR_COVERAGE_FIELD: &str = "hour_coverage";

/// What the recording asserts about one game hour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HourCoverage {
    /// The hour's records were framed end to end and its frame closed
    /// canonically. `basis_sha256` is the SHA-256 of exactly the stream bytes
    /// this hour is made of — the digest-backed basis without which the row is
    /// inadmissible.
    Verified { basis_sha256: String },
    /// The hour is refused, for the named reason. Never guessed.
    Excluded { reason: String },
}

impl HourCoverage {
    pub fn is_verified(&self) -> bool {
        matches!(self, HourCoverage::Verified { .. })
    }

    pub fn status_word(&self) -> &'static str {
        match self {
            HourCoverage::Verified { .. } => "verified",
            HourCoverage::Excluded { .. } => "excluded",
        }
    }
}

/// One hour's row: which hour, which stream bytes it was read from, and the
/// verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourCoverageRow {
    pub game_hour_raw: i32,
    /// Offset of the hour's first record in the decoded stream.
    pub first_byte: u64,
    /// Total decoded bytes attributed to this hour. Zero is legal for an hour
    /// the walk knows existed but read no bytes of (a sequence hole).
    pub byte_len: u64,
    pub coverage: HourCoverage,
}

impl HourCoverageRow {
    pub fn is_verified(&self) -> bool {
        self.coverage.is_verified()
    }
}

/// The recording's per-hour verdict, in game-hour order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HourCoverageMap {
    rows: BTreeMap<i32, HourCoverageRow>,
    unattributed: Vec<String>,
}

impl HourCoverageMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an hour whose bytes were framed end to end and whose frame closed
    /// canonically.
    ///
    /// EXCLUSION WINS. An hour already excluded stays excluded and keeps its
    /// reason: coverage may be judged down, never up. Without this a walk that
    /// injured an hour and then re-reached a clean-looking `FRAME_END` for the
    /// same hour — which a resynchronising framing loop can do — would launder
    /// the injury away.
    pub fn record_verified(
        &mut self,
        game_hour_raw: i32,
        first_byte: u64,
        byte_len: u64,
        basis_sha256: String,
    ) {
        if let Some(existing) = self.rows.get(&game_hour_raw) {
            if !existing.is_verified() {
                return;
            }
        }
        self.rows.insert(
            game_hour_raw,
            HourCoverageRow {
                game_hour_raw,
                first_byte,
                byte_len,
                coverage: HourCoverage::Verified { basis_sha256 },
            },
        );
    }

    /// Refuse an hour, naming why. Overwrites a verified row (see
    /// [`HourCoverageMap::record_verified`]); keeps the FIRST reason when the
    /// hour is already excluded, because the first statement is the diagnosis
    /// and later ones are its consequences.
    pub fn record_excluded(
        &mut self,
        game_hour_raw: i32,
        first_byte: u64,
        byte_len: u64,
        reason: impl Into<String>,
    ) {
        if let Some(existing) = self.rows.get(&game_hour_raw) {
            if !existing.is_verified() {
                return;
            }
        }
        self.rows.insert(
            game_hour_raw,
            HourCoverageRow {
                game_hour_raw,
                first_byte,
                byte_len,
                coverage: HourCoverage::Excluded {
                    reason: reason.into(),
                },
            },
        );
    }

    /// Name a loss that belongs to no single hour — bytes that framed into no
    /// closed frame, or a span whose hours the walk cannot enumerate. Naming it
    /// here is what stops it from being silently dropped or, worse, attributed
    /// to a neighbouring hour that is actually intact.
    pub fn note_unattributed_loss(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.unattributed.iter().any(|existing| existing == &reason) {
            self.unattributed.push(reason);
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = &HourCoverageRow> {
        self.rows.values()
    }

    pub fn row(&self, game_hour_raw: i32) -> Option<&HourCoverageRow> {
        self.rows.get(&game_hour_raw)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.unattributed.is_empty()
    }

    pub fn verified_hours(&self) -> impl Iterator<Item = i32> + '_ {
        self.rows
            .values()
            .filter(|row| row.is_verified())
            .map(|row| row.game_hour_raw)
    }

    pub fn verified_count(&self) -> usize {
        self.rows.values().filter(|row| row.is_verified()).count()
    }

    pub fn excluded_count(&self) -> usize {
        self.rows.values().filter(|row| !row.is_verified()).count()
    }

    /// Is this hour one the recording vouches for? An hour with no row at all
    /// is NOT verified — absence is not a claim.
    pub fn is_hour_verified(&self, game_hour_raw: i32) -> bool {
        self.rows
            .get(&game_hour_raw)
            .is_some_and(HourCoverageRow::is_verified)
    }

    /// Every excluded hour with its reason, in hour order. This is the named
    /// gap list a replay consumer composes with the partial-data law.
    pub fn excluded_hours(&self) -> Vec<(i32, &str)> {
        self.rows
            .values()
            .filter_map(|row| match &row.coverage {
                HourCoverage::Excluded { reason } => Some((row.game_hour_raw, reason.as_str())),
                HourCoverage::Verified { .. } => None,
            })
            .collect()
    }

    pub fn unattributed_losses(&self) -> &[String] {
        &self.unattributed
    }

    /// Structural admissibility of the map itself, asked by every writer before
    /// it seals and by every reader after it decodes.
    pub fn validate(&self) -> Result<(), String> {
        for (hour, row) in &self.rows {
            if *hour != row.game_hour_raw {
                return Err(format!(
                    "hour coverage row keyed {hour} carries game_hour_raw {}",
                    row.game_hour_raw
                ));
            }
            match &row.coverage {
                HourCoverage::Verified { basis_sha256 } => {
                    if basis_sha256.len() != 64
                        || !basis_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(format!(
                            "hour {hour} claims verified coverage without a digest-backed \
                             basis (basis_sha256 {basis_sha256:?} is not a 64-character hex \
                             SHA-256)"
                        ));
                    }
                    if row.byte_len == 0 {
                        return Err(format!(
                            "hour {hour} claims verified coverage over zero stream bytes; a \
                             verified hour is made of the bytes its digest covers"
                        ));
                    }
                }
                HourCoverage::Excluded { reason } => {
                    if reason.trim().is_empty() {
                        return Err(format!(
                            "hour {hour} is excluded without a named reason; an injured hour \
                             is never guessed, only named"
                        ));
                    }
                }
            }
        }
        for loss in &self.unattributed {
            if loss.trim().is_empty() {
                return Err("an unattributed loss carries no reason".to_string());
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let hours: Vec<Value> = self
            .rows
            .values()
            .map(|row| {
                let mut object = Map::new();
                object.insert("game_hour_raw".to_string(), json!(row.game_hour_raw));
                object.insert("first_byte".to_string(), json!(row.first_byte));
                object.insert("byte_len".to_string(), json!(row.byte_len));
                object.insert("status".to_string(), json!(row.coverage.status_word()));
                match &row.coverage {
                    HourCoverage::Verified { basis_sha256 } => {
                        object.insert("basis_sha256".to_string(), json!(basis_sha256));
                    }
                    HourCoverage::Excluded { reason } => {
                        object.insert("reason".to_string(), json!(reason));
                    }
                }
                Value::Object(object)
            })
            .collect();
        json!({
            "schema": HOUR_COVERAGE_SCHEMA,
            "verified_hours": self.verified_count(),
            "excluded_hours": self.excluded_count(),
            "hours": hours,
            "unattributed_losses": self.unattributed.clone(),
        })
    }

    /// Strict decode. Every key is accounted for; anything unassigned rejects.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "hour coverage is not a JSON object".to_string())?;
        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "schema" | "verified_hours" | "excluded_hours" | "hours" | "unattributed_losses"
            ) {
                return Err(format!("hour coverage carries unassigned key {key:?}"));
            }
        }
        let schema = object.get("schema").and_then(Value::as_str);
        if schema != Some(HOUR_COVERAGE_SCHEMA) {
            return Err(format!(
                "hour coverage schema is {schema:?}, expected {HOUR_COVERAGE_SCHEMA:?}"
            ));
        }
        let rows_json = object
            .get("hours")
            .and_then(Value::as_array)
            .ok_or_else(|| "hour coverage has no \"hours\" array".to_string())?;
        let mut map = Self::new();
        for entry in rows_json {
            let row_object = entry
                .as_object()
                .ok_or_else(|| "hour coverage row is not a JSON object".to_string())?;
            for key in row_object.keys() {
                if !matches!(
                    key.as_str(),
                    "game_hour_raw" | "first_byte" | "byte_len" | "status" | "basis_sha256"
                        | "reason"
                ) {
                    return Err(format!("hour coverage row carries unassigned key {key:?}"));
                }
            }
            let game_hour_raw: i32 = row_object
                .get("game_hour_raw")
                .and_then(Value::as_i64)
                .and_then(|raw| i32::try_from(raw).ok())
                .ok_or_else(|| "hour coverage row has no i32 game_hour_raw".to_string())?;
            let first_byte = row_object
                .get("first_byte")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("hour {game_hour_raw} coverage row has no first_byte"))?;
            let byte_len = row_object
                .get("byte_len")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("hour {game_hour_raw} coverage row has no byte_len"))?;
            let status = row_object
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("hour {game_hour_raw} coverage row has no status"))?;
            let coverage = match status {
                "verified" => {
                    if row_object.contains_key("reason") {
                        return Err(format!(
                            "hour {game_hour_raw} is verified but carries an exclusion reason"
                        ));
                    }
                    HourCoverage::Verified {
                        basis_sha256: row_object
                            .get("basis_sha256")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                format!("hour {game_hour_raw} is verified with no basis_sha256")
                            })?
                            .to_string(),
                    }
                }
                "excluded" => {
                    if row_object.contains_key("basis_sha256") {
                        return Err(format!(
                            "hour {game_hour_raw} is excluded but carries a verification basis"
                        ));
                    }
                    HourCoverage::Excluded {
                        reason: row_object
                            .get("reason")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                format!("hour {game_hour_raw} is excluded with no reason")
                            })?
                            .to_string(),
                    }
                }
                other => {
                    return Err(format!(
                        "hour {game_hour_raw} carries unassigned coverage status {other:?}"
                    ))
                }
            };
            if map.rows.contains_key(&game_hour_raw) {
                return Err(format!(
                    "hour coverage names hour {game_hour_raw} twice"
                ));
            }
            map.rows.insert(
                game_hour_raw,
                HourCoverageRow {
                    game_hour_raw,
                    first_byte,
                    byte_len,
                    coverage,
                },
            );
        }
        for loss in object
            .get("unattributed_losses")
            .and_then(Value::as_array)
            .ok_or_else(|| "hour coverage has no \"unattributed_losses\" array".to_string())?
        {
            map.unattributed.push(
                loss.as_str()
                    .ok_or_else(|| "unattributed loss is not a string".to_string())?
                    .to_string(),
            );
        }
        // The summary counts are a self-check, not a second source: a map whose
        // header disagrees with its rows is one a reader would summarize
        // differently from the writer that sealed it.
        let declared_verified = object
            .get("verified_hours")
            .and_then(Value::as_u64)
            .ok_or_else(|| "hour coverage has no verified_hours count".to_string())?;
        let declared_excluded = object
            .get("excluded_hours")
            .and_then(Value::as_u64)
            .ok_or_else(|| "hour coverage has no excluded_hours count".to_string())?;
        if declared_verified != map.verified_count() as u64
            || declared_excluded != map.excluded_count() as u64
        {
            return Err(format!(
                "hour coverage header claims {declared_verified} verified / \
                 {declared_excluded} excluded hour(s), but carries {} / {}",
                map.verified_count(),
                map.excluded_count()
            ));
        }
        map.validate()?;
        Ok(map)
    }

    /// One-line human account, for refusal strings and receipts.
    pub fn summary(&self) -> String {
        let excluded = self.excluded_hours();
        if excluded.is_empty() && self.unattributed.is_empty() {
            return format!("{} hour(s) verified, none excluded", self.verified_count());
        }
        let named: Vec<String> = excluded
            .iter()
            .map(|(hour, reason)| format!("hour {hour}: {reason}"))
            .chain(self.unattributed.iter().cloned())
            .collect();
        format!(
            "{} hour(s) verified, {} excluded — {}",
            self.verified_count(),
            excluded.len(),
            named.join("; ")
        )
    }
}

/// THE SEAL GATE, per-hour. Replaces the whole-stream all-or-nothing verdict.
///
/// A recording seals when it has at least one hour it can vouch for and its
/// coverage map is structurally admissible. It refuses only when there is
/// nothing to publish — which is not a narrowing of fail-closed but a
/// relocation of it: every hour this gate does not vouch for is carried in the
/// map as `Excluded` with a named reason, and the replay authority refuses to
/// serve those hours.
pub fn per_hour_seal_verdict(map: &HourCoverageMap) -> Result<(), String> {
    map.validate()
        .map_err(|error| format!("hour coverage map is inadmissible: {error}"))?;
    if map.verified_count() == 0 {
        return Err(format!(
            "no game hour in this recording is verified, so there is nothing to seal ({})",
            map.summary()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> String {
        std::iter::repeat(format!("{byte:02x}")).take(32).collect()
    }

    /// kills: dropping the `basis_sha256` shape check in
    /// `HourCoverageMap::validate` — a verified hour could then be claimed with
    /// no digest to back it, which is the seal asserting coverage it cannot
    /// show the bytes for.
    #[test]
    fn a_verified_hour_without_a_digest_backed_basis_is_inadmissible() {
        let mut map = HourCoverageMap::new();
        map.rows.insert(
            10,
            HourCoverageRow {
                game_hour_raw: 10,
                first_byte: 0,
                byte_len: 64,
                coverage: HourCoverage::Verified {
                    basis_sha256: String::new(),
                },
            },
        );
        let error = map.validate().unwrap_err();
        assert!(
            error.contains("without a digest-backed basis"),
            "unexpected: {error}"
        );
        assert!(per_hour_seal_verdict(&map).is_err());

        // A well-formed digest over real bytes is the admissible shape.
        map.record_verified(10, 0, 64, digest(0xab));
        map.validate().expect("a digest-backed verified hour is admissible");
        per_hour_seal_verdict(&map).expect("one verified hour is enough to seal");
    }

    /// kills: making `record_verified` overwrite an excluded row — an hour some
    /// earlier evidence refused could then be laundered clean by a later
    /// clean-looking observation of the same hour, which is exactly what a
    /// resynchronising framing loop produces.
    #[test]
    fn an_excluded_hour_cannot_be_relabelled_verified() {
        let mut map = HourCoverageMap::new();
        map.record_excluded(7, 100, 40, "framing damage at byte 100");
        map.record_verified(7, 100, 40, digest(0x11));
        assert_eq!(
            map.excluded_hours(),
            vec![(7, "framing damage at byte 100")],
            "the exclusion and its ORIGINAL reason must survive a later verified claim"
        );
        assert!(!map.is_hour_verified(7));
        assert_eq!(map.verified_count(), 0);
        assert!(
            per_hour_seal_verdict(&map).is_err(),
            "a recording with no verified hour has nothing to seal"
        );
    }

    /// kills: dropping the empty-reason check — an hour could then be excluded
    /// with nothing said about why, which is silent loss wearing the exclusion
    /// label.
    #[test]
    fn an_excluded_hour_must_name_its_reason() {
        let mut map = HourCoverageMap::new();
        map.record_verified(1, 0, 10, digest(0x22));
        map.record_excluded(2, 10, 10, "   ");
        let error = map.validate().unwrap_err();
        assert!(error.contains("without a named reason"), "unexpected: {error}");
    }

    /// kills: relaxing `from_json` to ignore unknown keys or unknown status
    /// words — a decoder would then act on part of a map it cannot fully
    /// account for.
    #[test]
    fn unassigned_coverage_bits_reject_at_the_decoder() {
        let mut map = HourCoverageMap::new();
        map.record_verified(1, 0, 24, digest(0x33));
        map.record_excluded(2, 24, 24, "torn tail");
        map.note_unattributed_loss("11 byte(s) after hour 2 framed into no hour");
        let encoded = map.to_json();
        assert_eq!(
            HourCoverageMap::from_json(&encoded).expect("round trip"),
            map
        );

        let mut unknown_key = encoded.clone();
        unknown_key
            .as_object_mut()
            .unwrap()
            .insert("probably_fine".to_string(), json!(true));
        assert!(HourCoverageMap::from_json(&unknown_key)
            .unwrap_err()
            .contains("unassigned key"));

        let mut unknown_status = encoded.clone();
        unknown_status["hours"][1]
            .as_object_mut()
            .unwrap()
            .insert("status".to_string(), json!("probably_ok"));
        assert!(HourCoverageMap::from_json(&unknown_status)
            .unwrap_err()
            .contains("unassigned coverage status"));

        let mut laundered = encoded.clone();
        laundered["hours"][1]
            .as_object_mut()
            .unwrap()
            .insert("basis_sha256".to_string(), json!(digest(0x44)));
        assert!(HourCoverageMap::from_json(&laundered)
            .unwrap_err()
            .contains("excluded but carries a verification basis"));

        let mut miscounted = encoded;
        miscounted
            .as_object_mut()
            .unwrap()
            .insert("verified_hours".to_string(), json!(2));
        assert!(HourCoverageMap::from_json(&miscounted)
            .unwrap_err()
            .contains("header claims"));
    }
}
