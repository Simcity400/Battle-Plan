#![forbid(unsafe_code)]

use std::collections::BTreeMap;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const ROOT_PLAYER_COUNTRY: u32 = 1 << 0;
pub const ROOT_PAUSE_CLOCK: u32 = 1 << 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tri<T> {
    Known(T),
    Unavailable(&'static str),
    NotSourced(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedPlayerCountryRow {
    pub country_tag: String,
    pub tag_db_id: u32,
    pub human_status_raw: u8,
    pub valid_from_hour_raw: i32,
    pub player_name_payload: String,
    pub player_key_hash_payload: u64,
    pub session_slot_payload: u32,
    pub flags_payload: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedPauseRow {
    pub is_paused: bool,
    pub event_hour_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDomain57Frame {
    pub game_hour_raw: i32,
    pub status_flags: u32,
    pub root_evidence_mask: u32,
    pub player_country_rows: Vec<DecodedPlayerCountryRow>,
    pub pause: Option<DecodedPauseRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R2PlayerCountryProjection {
    pub is_human_controlled: Tri<bool>,
    pub tag_db_id: Tri<u32>,
    pub human_status_raw: Tri<u8>,
    pub human_name: Tri<String>,
    pub player_key_hash: Tri<u64>,
    pub session_slot: Tri<u32>,
    pub host: Tri<bool>,
    pub connected: Tri<bool>,
    pub valid_from_hour_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R2SessionProjection {
    pub game_hour_raw: i32,
    pub players_by_country: Tri<BTreeMap<String, R2PlayerCountryProjection>>,
    pub pause_is_paused: Tri<bool>,
}

pub fn fold_domain57_to_r2(frame: &DecodedDomain57Frame) -> R2SessionProjection {
    let player_root_valid = frame.root_evidence_mask & ROOT_PLAYER_COUNTRY != 0
        && frame.status_flags & STATUS_SOURCE_UNAVAILABLE == 0;

    let players_by_country = if !player_root_valid {
        Tri::Unavailable("player-country root evidence missing or source unavailable")
    } else if frame.player_country_rows.is_empty() {
        Tri::Unavailable("zero human country rows is implausible after validated root read")
    } else {
        let mut players = BTreeMap::new();
        for row in &frame.player_country_rows {
            players.insert(
                row.country_tag.clone(),
                R2PlayerCountryProjection {
                    is_human_controlled: Tri::Known(true),
                    tag_db_id: Tri::Known(row.tag_db_id),
                    human_status_raw: Tri::Known(row.human_status_raw),
                    human_name: Tri::NotSourced("players.json lobby persona is not read by domain 57"),
                    player_key_hash: Tri::NotSourced("MP lobby player key is not validated from SP"),
                    session_slot: Tri::NotSourced("MP lobby session slot is not validated from SP"),
                    host: Tri::NotSourced("MP host flag is not validated from SP"),
                    connected: Tri::NotSourced("MP connected flag is not validated from SP"),
                    valid_from_hour_raw: row.valid_from_hour_raw,
                },
            );
        }
        Tri::Known(players)
    };

    let pause_is_paused = if frame.root_evidence_mask & ROOT_PAUSE_CLOCK == 0
        || frame.status_flags & STATUS_SOURCE_UNAVAILABLE != 0
    {
        Tri::Unavailable("pause clock root evidence missing or source unavailable")
    } else {
        match &frame.pause {
            Some(row) => Tri::Known(row.is_paused),
            None => Tri::Unavailable("pause row absent despite validated clock root"),
        }
    };

    R2SessionProjection {
        game_hour_raw: frame.game_hour_raw,
        players_by_country,
        pause_is_paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tag: &str, tag_db_id: u32) -> DecodedPlayerCountryRow {
        DecodedPlayerCountryRow {
            country_tag: tag.to_string(),
            tag_db_id,
            human_status_raw: 1,
            valid_from_hour_raw: 60_828_184,
            player_name_payload: String::new(),
            player_key_hash_payload: 0,
            session_slot_payload: u32::MAX,
            flags_payload: 1 << 1,
        }
    }

    #[test]
    fn folds_validated_sp_player_country_and_pause_without_human_name() {
        let projection = fold_domain57_to_r2(&DecodedDomain57Frame {
            game_hour_raw: 60_828_184,
            status_flags: 0,
            root_evidence_mask: ROOT_PLAYER_COUNTRY | ROOT_PAUSE_CLOCK,
            player_country_rows: vec![row("IRE", 29)],
            pause: Some(DecodedPauseRow {
                is_paused: true,
                event_hour_raw: 60_828_184,
            }),
        });

        let players = match projection.players_by_country {
            Tri::Known(players) => players,
            other => panic!("expected known players, got {other:?}"),
        };
        let ireland = players.get("IRE").expect("IRE row");
        assert_eq!(ireland.is_human_controlled, Tri::Known(true));
        assert_eq!(ireland.tag_db_id, Tri::Known(29));
        assert!(matches!(ireland.human_name, Tri::NotSourced(_)));
        assert!(matches!(ireland.player_key_hash, Tri::NotSourced(_)));
        assert!(matches!(ireland.session_slot, Tri::NotSourced(_)));
        assert_eq!(projection.pause_is_paused, Tri::Known(true));
    }

    #[test]
    fn keeps_multiple_mp_human_rows_when_the_same_shape_appears() {
        let projection = fold_domain57_to_r2(&DecodedDomain57Frame {
            game_hour_raw: 60_828_185,
            status_flags: 0,
            root_evidence_mask: ROOT_PLAYER_COUNTRY | ROOT_PAUSE_CLOCK,
            player_country_rows: vec![row("GER", 1), row("ENG", 2), row("IRE", 29)],
            pause: Some(DecodedPauseRow {
                is_paused: false,
                event_hour_raw: 60_828_185,
            }),
        });

        let players = match projection.players_by_country {
            Tri::Known(players) => players,
            other => panic!("expected known players, got {other:?}"),
        };
        assert_eq!(players.len(), 3);
        assert_eq!(players["ENG"].human_status_raw, Tri::Known(1));
        assert_eq!(projection.pause_is_paused, Tri::Known(false));
    }

    #[test]
    fn zero_human_rows_after_validated_root_is_unavailable_not_empty() {
        let projection = fold_domain57_to_r2(&DecodedDomain57Frame {
            game_hour_raw: 60_828_186,
            status_flags: 0,
            root_evidence_mask: ROOT_PLAYER_COUNTRY | ROOT_PAUSE_CLOCK,
            player_country_rows: Vec::new(),
            pause: Some(DecodedPauseRow {
                is_paused: true,
                event_hour_raw: 60_828_186,
            }),
        });

        assert!(matches!(projection.players_by_country, Tri::Unavailable(_)));
        assert_eq!(projection.pause_is_paused, Tri::Known(true));
    }

    #[test]
    fn missing_pause_clock_root_fails_closed_independently() {
        let projection = fold_domain57_to_r2(&DecodedDomain57Frame {
            game_hour_raw: 60_828_187,
            status_flags: 0,
            root_evidence_mask: ROOT_PLAYER_COUNTRY,
            player_country_rows: vec![row("IRE", 29)],
            pause: None,
        });

        assert!(matches!(projection.players_by_country, Tri::Known(_)));
        assert!(matches!(projection.pause_is_paused, Tri::Unavailable(_)));
    }
}
