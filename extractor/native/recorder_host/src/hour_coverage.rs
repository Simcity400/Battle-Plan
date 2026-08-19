use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

pub const HOUR_COVERAGE_SCHEMA: &str = "hoi4.recording.hour_coverage.v1";

pub const HOUR_COVERAGE_FIELD: &str = "hour_coverage";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HourCoverage {
    Verified { basis_sha256: String },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourCoverageRow {
    pub game_hour_raw: i32,
    pub first_byte: u64,
    pub byte_len: u64,
    pub coverage: HourCoverage,
}

impl HourCoverageRow {
    pub fn is_verified(&self) -> bool {
        self.coverage.is_verified()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HourCoverageMap {
    rows: BTreeMap<i32, HourCoverageRow>,
    unattributed: Vec<String>,
}

impl HourCoverageMap {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn is_hour_verified(&self, game_hour_raw: i32) -> bool {
        self.rows
            .get(&game_hour_raw)
            .is_some_and(HourCoverageRow::is_verified)
    }

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
                    "game_hour_raw"
                        | "first_byte"
                        | "byte_len"
                        | "status"
                        | "basis_sha256"
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
                return Err(format!("hour coverage names hour {game_hour_raw} twice"));
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

        map.record_verified(10, 0, 64, digest(0xab));
        map.validate()
            .expect("a digest-backed verified hour is admissible");
        per_hour_seal_verdict(&map).expect("one verified hour is enough to seal");
    }

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

    #[test]
    fn an_excluded_hour_must_name_its_reason() {
        let mut map = HourCoverageMap::new();
        map.record_verified(1, 0, 10, digest(0x22));
        map.record_excluded(2, 10, 10, "   ");
        let error = map.validate().unwrap_err();
        assert!(
            error.contains("without a named reason"),
            "unexpected: {error}"
        );
    }

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
