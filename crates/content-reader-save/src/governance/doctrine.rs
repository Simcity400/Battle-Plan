use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctrineFolder {
    pub folder: Vec<u8>,
    pub grand_doctrine: Option<Vec<u8>>,
    pub tracks: Vec<DoctrineTrack>,
    pub completed_subdoctrines: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctrineTrack {
    pub sub_doctrine: Option<Vec<u8>>,
    pub rewards: i32,
    pub mastery: Decimal,
    pub mastery_bank: Decimal,
    pub daily_mastery: Decimal,
    pub leaders_daily_mastery: Vec<DoctrineLeaderMastery>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctrineLeaderMastery {
    pub leader: (u32, u32),
    pub mastery: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctrineCostReduction {
    pub uses: i32,
    pub name: Vec<u8>,
    pub cost_factor: Decimal,
    pub folder: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctrineTrackFilter {
    pub track: Option<Vec<u8>>,
    pub folder: Option<Vec<u8>>,
    pub grand_doctrine: Option<Vec<u8>>,
    pub sub_doctrine: Option<Vec<u8>>,
    pub track_index: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctrineMasteryGrant {
    pub name: Option<Vec<u8>>,
    pub relevant_tracks: DoctrineTrackFilter,
    pub daily_mastery: Decimal,
    pub bonus: Decimal,
    pub days: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctrineEquipmentBonus {
    pub id: u32,
    pub index: i32,
    pub equipment_bonus: i32,
}

fn entries<T>(
    fields: &[Item],
    key: &[u8],
    mut parse: impl FnMut(&[Item]) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    optional_block(fields, key)?
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid(format!(
                    "{} entry must be anonymous",
                    String::from_utf8_lossy(key)
                )));
            }
            parse(block(&entry.value)?)
        })
        .collect()
}

fn emitted_bytes(fields: &[Item], key: &[u8]) -> io::Result<Vec<u8>> {
    bytes(fields, key)?
        .ok_or_else(|| invalid(format!("missing doctrine {}", String::from_utf8_lossy(key))))
}

fn required_integer<T: std::str::FromStr>(fields: &[Item], key: &[u8]) -> io::Result<T> {
    integer(&required_bytes(fields, key)?)
}

fn required_decimal(fields: &[Item], key: &[u8]) -> io::Result<Decimal> {
    decimal(fields, key)?
        .ok_or_else(|| invalid(format!("missing doctrine {}", String::from_utf8_lossy(key))))
}

pub(super) fn folders(fields: &[Item]) -> io::Result<Vec<DoctrineFolder>> {
    entries(fields, b"folder", |fields| {
        Ok(DoctrineFolder {
            folder: emitted_bytes(fields, b"folder")?,
            grand_doctrine: bytes(fields, b"grand_doctrine")?,
            tracks: entries(fields, b"tracks", |fields| {
                Ok(DoctrineTrack {
                    sub_doctrine: bytes(fields, b"sub_doctrine")?,
                    rewards: required_integer(fields, b"rewards")?,
                    mastery: required_decimal(fields, b"mastery")?,
                    mastery_bank: required_decimal(fields, b"mastery_bank")?,
                    daily_mastery: required_decimal(fields, b"daily_mastery")?,
                    leaders_daily_mastery: entries(fields, b"leaders_daily_mastery", |fields| {
                        let reference = block(
                            field(fields, b"leader")?
                                .ok_or_else(|| invalid("missing doctrine leader"))?,
                        )?;
                        Ok(DoctrineLeaderMastery {
                            leader: (
                                required_integer(reference, b"id")?,
                                required_integer(reference, b"type")?,
                            ),
                            mastery: required_decimal(fields, b"mastery")?,
                        })
                    })?,
                })
            })?,
            completed_subdoctrines: optional_block(fields, b"completed_subdoctrines")?
                .unwrap_or_default()
                .iter()
                .map(|entry| {
                    if entry.key.is_some() {
                        return Err(invalid("completed subdoctrine must be an unkeyed token"));
                    }
                    Ok(scalar(&entry.value)?.to_vec())
                })
                .collect::<io::Result<_>>()?,
        })
    })
}

pub(super) fn cost_reductions(fields: &[Item]) -> io::Result<Vec<Option<DoctrineCostReduction>>> {
    entries(fields, b"cost_reduction", |fields| {
        if fields.is_empty() {
            return Ok(None);
        }
        Ok(Some(DoctrineCostReduction {
            uses: required_integer(fields, b"uses")?,
            name: emitted_bytes(fields, b"name")?,
            cost_factor: required_decimal(fields, b"cost_factor")?,
            folder: emitted_bytes(fields, b"folder")?,
        }))
    })
}

pub(super) fn mastery_grants(fields: &[Item]) -> io::Result<Vec<Option<DoctrineMasteryGrant>>> {
    entries(fields, b"daily_mastery", |fields| {
        if fields.is_empty() {
            return Ok(None);
        }
        let filter = optional_block(fields, b"relevant_tracks")?
            .ok_or_else(|| invalid("missing doctrine relevant_tracks"))?;
        Ok(Some(DoctrineMasteryGrant {
            name: bytes(fields, b"name")?,
            relevant_tracks: DoctrineTrackFilter {
                track: bytes(filter, b"track")?,
                folder: bytes(filter, b"folder")?,
                grand_doctrine: bytes(filter, b"grand_doctrine")?,
                sub_doctrine: bytes(filter, b"sub_doctrine")?,
                track_index: field(filter, b"track_index")?
                    .map(|value| integer(scalar(value)?))
                    .transpose()?,
            },
            daily_mastery: required_decimal(fields, b"daily_mastery")?,
            bonus: required_decimal(fields, b"bonus")?,
            days: required_integer(fields, b"days")?,
        }))
    })
}

pub(super) fn equipment_bonuses(fields: &[Item]) -> io::Result<Vec<DoctrineEquipmentBonus>> {
    entries(fields, b"equipment_bonus", |fields| {
        Ok(DoctrineEquipmentBonus {
            id: required_integer(fields, b"id")?,
            index: required_integer(fields, b"index")?,
            equipment_bonus: required_integer(fields, b"equipment_bonus")?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctrine_state_keeps_tracks_banks_sources_grants_and_repeated_completions() {
        let text = br#"countries={} doctrine={countries={{country=AAA
            folder={{folder=mod_folder grand_doctrine=mod_grand tracks={{
                sub_doctrine=mod_sub rewards=7 mastery=123.45678 mastery_bank=-0.00001 daily_mastery=0.2
                leaders_daily_mastery={{leader={id=4294967295 type=4713} mastery=-0.12345}}
            }} completed_subdoctrines={mod_sub mod_other mod_sub}} {folder=empty}}
            cost_reduction={{uses=-1 name="" cost_factor=-0.35 folder=mod_folder}}
            daily_mastery={{name=grant relevant_tracks={folder=mod_folder grand_doctrine=mod_grand sub_doctrine=mod_sub track=mod_track track_index=0} daily_mastery=0.125 bonus=-0.5 days=-1}}
            equipment_bonus={{id=4294967295 index=-1 equipment_bonus=4}}
        }}}"#;
        let pack =
            super::super::parse(&content_reader_parser::parser::parse(text).unwrap().items).unwrap();
        let state = &pack.doctrine.unwrap()[0];
        assert_eq!(state.folders.len(), 2);
        assert_eq!(
            state.folders[0].completed_subdoctrines,
            [
                b"mod_sub".to_vec(),
                b"mod_other".to_vec(),
                b"mod_sub".to_vec()
            ]
        );
        let track = &state.folders[0].tracks[0];
        assert_eq!(track.rewards, 7);
        assert_eq!(track.mastery, Decimal::parse(b"123.45678").unwrap());
        assert_eq!(track.mastery_bank, Decimal::parse(b"-0.00001").unwrap());
        assert_eq!(track.leaders_daily_mastery[0].leader, (u32::MAX, 4713));
        assert!(state.folders[1].tracks.is_empty());
        assert_eq!(state.folders[1].grand_doctrine, None);
        assert_eq!(state.cost_reductions[0].as_ref().unwrap().name, b"");
        assert_eq!(
            state.daily_mastery[0]
                .as_ref()
                .unwrap()
                .relevant_tracks
                .track,
            Some(b"mod_track".to_vec())
        );
        assert_eq!(state.daily_mastery[0].as_ref().unwrap().days, -1);
        assert_eq!(state.equipment_bonuses[0].id, u32::MAX);
    }

    #[test]
    fn doctrine_grant_filters_keep_authored_track_tokens_separate_from_indices() {
        let document = content_reader_parser::parser::parse(br#"countries={} doctrine={countries={{
            daily_mastery={
                {relevant_tracks={track=mod_screens track_index=0} daily_mastery=1 bonus=0 days=5}
                {relevant_tracks={track=mod_support} daily_mastery=1 bonus=0 days=5}
                {relevant_tracks={track="mod_infantry" track_index=3} daily_mastery=1 bonus=0 days=5}
                {relevant_tracks={track=mod_operations} daily_mastery=1 bonus=0 days=5}
                {relevant_tracks={track_index=0} daily_mastery=1 bonus=0 days=5}
            }
        }}}"#).unwrap();
        let pack = super::super::parse(&document.items).unwrap();
        let grants = &pack.doctrine.as_ref().unwrap()[0].daily_mastery;
        for (ordinal, token, index) in [
            (0, Some(b"mod_screens".as_slice()), Some(0)),
            (1, Some(b"mod_support".as_slice()), None),
            (2, Some(b"mod_infantry".as_slice()), Some(3)),
            (3, Some(b"mod_operations".as_slice()), None),
            (4, None, Some(0)),
        ] {
            let filter = &grants[ordinal].as_ref().unwrap().relevant_tracks;
            assert_eq!(filter.track.as_deref(), token);
            assert_eq!(filter.track_index, index);
        }
    }

    #[test]
    fn doctrine_missing_track_values_and_keyed_entries_are_errors() {
        for text in [
            &b"countries={} doctrine={countries={{folder={{folder=x tracks={{rewards=0}}}}}}}"[..],
            &b"countries={} doctrine={countries={{cost_reduction={named={}}}}}"[..],
        ] {
            assert!(
                super::super::parse(&content_reader_parser::parser::parse(text).unwrap().items).is_err()
            );
        }
    }
}
