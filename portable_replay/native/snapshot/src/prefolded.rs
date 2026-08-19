use crate::{decode_json_strict, Snapshot};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct PrefoldedHourIndex {
    game_id: String,
    schema_version: Value,
    hours: Vec<i64>,
}

impl PrefoldedHourIndex {
    pub fn game_id(&self) -> &str {
        &self.game_id
    }

    pub fn schema_version(&self) -> &Value {
        &self.schema_version
    }

    pub fn hours(&self) -> &[i64] {
        &self.hours
    }
}

#[derive(Debug, Clone)]
pub struct PrefoldedHourEntry {
    hour_index: i64,
    snapshot: Snapshot,
}

impl PrefoldedHourEntry {
    pub fn from_json(hour_index: i64, bytes: &[u8]) -> Result<Self, PrefoldedReplayError> {
        let snapshot: Snapshot = decode_json_strict(bytes).map_err(|error| {
            PrefoldedReplayError::InvalidEntry(format!(
                "hour {hour_index} is not strict snapshot JSON: {error}"
            ))
        })?;
        match snapshot.hour_index {
            Some(body_hour) if body_hour == hour_index => {}
            Some(body_hour) => {
                return Err(PrefoldedReplayError::InvalidEntry(format!(
                    "snapshot body hour_index {body_hour} != entry hour_index {hour_index}"
                )))
            }
            None => {
                return Err(PrefoldedReplayError::InvalidEntry(format!(
                    "snapshot for hour {hour_index} has no body hour_index"
                )))
            }
        }
        Ok(Self {
            hour_index,
            snapshot,
        })
    }

    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[derive(Debug, Clone)]
pub struct PrefoldedReplay {
    index: PrefoldedHourIndex,
    entries: Vec<PrefoldedHourEntry>,
}

impl PrefoldedReplay {
    pub fn try_new(
        game_id: impl Into<String>,
        schema_version: Value,
        entries: Vec<PrefoldedHourEntry>,
    ) -> Result<Self, PrefoldedReplayError> {
        let game_id = game_id.into();
        if game_id.trim().is_empty() {
            return Err(PrefoldedReplayError::InvalidIndex(
                "game_id is empty".to_string(),
            ));
        }
        if entries.is_empty() {
            return Err(PrefoldedReplayError::InvalidIndex(
                "prefolded replay contains no hours".to_string(),
            ));
        }

        let mut hours = Vec::with_capacity(entries.len());
        let mut previous = None;
        for (position, entry) in entries.iter().enumerate() {
            if let Some(previous) = previous {
                if entry.hour_index <= previous {
                    return Err(PrefoldedReplayError::InvalidIndex(format!(
                        "hour at position {position} is {} after {previous}; hours must be strictly increasing",
                        entry.hour_index
                    )));
                }
            }
            previous = Some(entry.hour_index);

            match entry.snapshot.game_id.as_deref() {
                Some(body_game_id) if body_game_id == game_id.as_str() => {}
                Some(body_game_id) => {
                    return Err(PrefoldedReplayError::InvalidEntry(format!(
                        "hour {} snapshot game_id {body_game_id:?} != index game_id {game_id:?}",
                        entry.hour_index
                    )))
                }
                None => {
                    return Err(PrefoldedReplayError::InvalidEntry(format!(
                        "hour {} snapshot has no game_id",
                        entry.hour_index
                    )))
                }
            }
            match entry.snapshot.schema_version.as_ref() {
                Some(body_schema) if body_schema == &schema_version => {}
                Some(body_schema) => {
                    return Err(PrefoldedReplayError::InvalidEntry(format!(
                        "hour {} snapshot schema_version {body_schema} != index schema_version {schema_version}",
                        entry.hour_index
                    )))
                }
                None => {
                    return Err(PrefoldedReplayError::InvalidEntry(format!(
                        "hour {} snapshot has no schema_version",
                        entry.hour_index
                    )))
                }
            }
            hours.push(entry.hour_index);
        }

        Ok(Self {
            index: PrefoldedHourIndex {
                game_id,
                schema_version,
                hours,
            },
            entries,
        })
    }

    pub fn index(&self) -> &PrefoldedHourIndex {
        &self.index
    }

    pub fn entries(&self) -> &[PrefoldedHourEntry] {
        &self.entries
    }

    pub fn entry(&self, hour_index: i64) -> Option<&PrefoldedHourEntry> {
        self.index
            .hours
            .binary_search(&hour_index)
            .ok()
            .map(|position| &self.entries[position])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefoldedReplayError {
    InvalidIndex(String),
    InvalidEntry(String),
}

impl std::fmt::Display for PrefoldedReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidIndex(message) => write!(f, "invalid prefolded hour index: {message}"),
            Self::InvalidEntry(message) => write!(f, "invalid prefolded hour entry: {message}"),
        }
    }
}

impl std::error::Error for PrefoldedReplayError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hour: i64, game: &str, schema: &str) -> PrefoldedHourEntry {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "game_id": game,
            "schema_version": schema,
            "hour_index": hour,
            "date": "1936.1.1.12",
            "countries": {"FRA": {"stability": 0.5}}
        }))
        .unwrap();
        PrefoldedHourEntry::from_json(hour, &bytes).unwrap()
    }

    #[test]
    fn closed_index_and_entries_have_identical_order_and_lookup() {
        let replay = PrefoldedReplay::try_new(
            "g",
            Value::String("1".into()),
            vec![entry(10, "g", "1"), entry(12, "g", "1")],
        )
        .unwrap();
        assert_eq!(replay.index().hours(), &[10, 12]);
        assert_eq!(replay.entries()[1].hour_index(), 12);
        assert_eq!(
            replay.entry(12).unwrap().snapshot().date.as_deref(),
            Some("1936.1.1.12")
        );
        assert!(replay.entry(11).is_none());
    }

    #[test]
    fn construction_rejects_duplicate_or_out_of_order_hours() {
        for entries in [
            vec![entry(10, "g", "1"), entry(10, "g", "1")],
            vec![entry(10, "g", "1"), entry(9, "g", "1")],
        ] {
            assert!(matches!(
                PrefoldedReplay::try_new("g", Value::String("1".into()), entries),
                Err(PrefoldedReplayError::InvalidIndex(_))
            ));
        }
    }

    #[test]
    fn construction_rejects_body_identity_disagreement() {
        assert!(matches!(
            PrefoldedReplay::try_new(
                "wanted",
                Value::String("1".into()),
                vec![entry(10, "other", "1")]
            ),
            Err(PrefoldedReplayError::InvalidEntry(_))
        ));
        assert!(matches!(
            PrefoldedReplay::try_new("g", Value::String("1".into()), vec![entry(10, "g", "2")]),
            Err(PrefoldedReplayError::InvalidEntry(_))
        ));
    }

    #[test]
    fn entry_rejects_missing_mismatched_and_duplicate_hour_members() {
        for bytes in [
            br#"{"game_id":"g","schema_version":"1"}"#.as_slice(),
            br#"{"game_id":"g","schema_version":"1","hour_index":11}"#.as_slice(),
            br#"{"game_id":"g","schema_version":"1","hour_index":10,"hour_index":10}"#.as_slice(),
        ] {
            assert!(PrefoldedHourEntry::from_json(10, bytes).is_err());
        }
    }

    #[test]
    fn portable_manifest_has_no_extractor_or_replayer_dependency() {
        let manifest = include_str!("../Cargo.toml").replace('\\', "/");
        assert!(!manifest.contains("extractor/"), "{manifest}");
        assert!(!manifest.contains("replay_interface/"), "{manifest}");
    }
}
