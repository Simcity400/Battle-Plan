//! A replay and the verified local game content used to draw its current hour.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use installed_content::{
    ResolveOptions,
    map::{self, MapData, ProvinceKind},
};
use replay::{
    container::{Replay, SeekResult},
    frame::DecodedFrame,
    identity::Verification,
    rows::{CountryRow, tables},
};

use crate::playback::Playback;

const WATER: [u8; 4] = [31, 56, 77, 255];
const UNKNOWN_LAND: [u8; 4] = [100, 103, 107, 255];

pub struct Document {
    pub playback: Playback,
    pub map: MapData,
    /// Persistent content verification and presentation limitations.
    pub warnings: Vec<String>,
    pub source_path: PathBuf,
}

pub struct Palette {
    /// Indexed by province id, including the province-zero sentinel.
    pub colors: Vec<[u8; 4]>,
    /// Resolved country slot plus one; zero means no resolved country.
    pub owner_keys: Vec<u32>,
    /// Limitations at this hour, separate from persistent content warnings.
    pub warnings: Vec<String>,
}

impl Document {
    pub fn load(path: &Path, options: &ResolveOptions, allow_unverified: bool) -> io::Result<Self> {
        let replay = Replay::open(fs::read(path)?)?;
        let mut warnings = Vec::new();
        check_verification(
            "replay header",
            &replay.identity.verification,
            allow_unverified,
            &mut warnings,
        )?;
        for session in &replay.sessions {
            check_verification(
                &format!("session {}", session.id),
                &session.identity.verification,
                allow_unverified,
                &mut warnings,
            )?;
        }
        // Replay::open requires every session to have the same content identity.
        // The recorded layer ids and hashes, rather than launcher state, select it.
        let content = installed_content::resolve(&replay.identity, options)?;
        let map = map::load(&content.layers)?;
        warnings.push(
            "Country colors come from the game's static content; runtime and cosmetic color overrides were not captured.".into(),
        );
        Ok(Self {
            playback: Playback::new(replay)?,
            map,
            warnings,
            source_path: path.to_path_buf(),
        })
    }

    pub fn palette(&self, control: bool) -> io::Result<Palette> {
        match self.playback.current() {
            SeekResult::Frame(frame) => palette(&self.map, frame, control),
            SeekResult::Gap {
                branch,
                hour,
                reason,
            } => Err(invalid(format!(
                "branch {branch}, hour {hour} is a gap: {reason}"
            ))),
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn check_verification(
    context: &str,
    state: &Verification,
    allow_unverified: bool,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let reason = match state {
        Verification::Verified { .. } => return Ok(()),
        Verification::Unverified { reason, .. } => reason.as_str(),
        Verification::Unchecked => "the collector did not verify the loaded content roster",
    };
    if !allow_unverified {
        return Err(invalid(format!(
            "{context} has unverified content: {reason}; --allow-unverified-content permits viewing with a persistent warning"
        )));
    }
    warnings.push(format!("UNVERIFIED CONTENT — {context}: {reason}"));
    Ok(())
}

fn palette(map: &MapData, frame: &DecodedFrame, control: bool) -> io::Result<Palette> {
    let countries = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect::<BTreeMap<_, _>>();
    let links = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect::<BTreeMap<_, _>>();
    let provinces = frame.complete::<tables::Provinces>()?;
    let count = map
        .definitions
        .last_key_value()
        .and_then(|(&id, _)| usize::try_from(id).ok().and_then(|id| id.checked_add(1)))
        .ok_or_else(|| {
            invalid("map has no definitions or its province ids exceed address space")
        })?;
    let mut result = Palette {
        colors: vec![UNKNOWN_LAND; count],
        owner_keys: vec![0; count],
        warnings: Vec::new(),
    };
    let mut observed = BTreeSet::new();
    let mut unknown = Vec::new();
    for province in provinces {
        let definition = map.definitions.get(&province.id).ok_or_else(|| {
            invalid(format!(
                "recorded province {} is absent from map/definition.csv",
                province.id
            ))
        })?;
        observed.insert(province.id);
        let index = province.id as usize;
        if matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake) {
            result.colors[index] = WATER;
            continue;
        }
        let reference = if control {
            province.controller
        } else {
            province.owner
        };
        if reference == 0 {
            unknown.push(province.id);
            continue;
        }
        let slot = links.get(&reference).ok_or_else(|| {
            invalid(format!(
                "province {} references missing country link {reference}",
                province.id
            ))
        })?;
        let country = countries.get(slot).ok_or_else(|| {
            invalid(format!(
                "province {} country link {reference} resolves to missing country slot {slot}",
                province.id
            ))
        })?;
        result.colors[index] = country_color(map, country)?;
        result.owner_keys[index] = slot
            .checked_add(1)
            .ok_or_else(|| invalid("country slot exceeds the map border key range"))?;
    }
    for &id in map.definitions.keys() {
        if id != 0 && !observed.contains(&id) {
            return Err(invalid(format!(
                "hour {} has no recorded row for map province {id}",
                frame.hour
            )));
        }
    }
    if !unknown.is_empty() {
        let field = if control { "controller" } else { "owner" };
        let examples = unknown
            .iter()
            .take(12)
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        result.warnings.push(format!(
            "{} land provinces have a zero {field} reference and are shown grey; province ids {examples}{}",
            unknown.len(),
            if unknown.len() > 12 { ", …" } else { "" }
        ));
    }
    Ok(result)
}

fn country_color(map: &MapData, country: &CountryRow) -> io::Result<[u8; 4]> {
    let tag = String::from_utf8_lossy(&country.tag);
    let rgb = map.country_colors.rgb.get(&country.tag).ok_or_else(|| {
        let reason = map
            .country_colors
            .unavailable
            .get(&country.tag)
            .map(String::as_str)
            .unwrap_or("no static color exists in the recorded game content");
        invalid(format!("country {tag} color is unavailable: {reason}"))
    })?;
    Ok([rgb[0], rgb[1], rgb[2], 255])
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::map::{CountryColors, ProvinceDefinition};
    use replay::{
        frame::{Metrics, TableView},
        identity::Roster,
        rows::{CountryLinkRow, ProvinceRow, TableId, TableRows},
    };

    fn map() -> MapData {
        MapData {
            width: 3,
            height: 1,
            province_ids: vec![1, 2, 3],
            definitions: [
                (0, ProvinceKind::Land),
                (1, ProvinceKind::Land),
                (2, ProvinceKind::Land),
                (3, ProvinceKind::Sea),
            ]
            .into_iter()
            .map(|(id, kind)| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind,
                    },
                )
            })
            .collect(),
            country_colors: CountryColors {
                rgb: [(b"GER".to_vec(), [102, 96, 87])].into(),
                unavailable: BTreeMap::new(),
            },
        }
    }

    fn frame() -> DecodedFrame {
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for map".into(),
        });
        tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 100,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                ..Default::default()
            }]));
        tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            },
        ]));
        tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
            ProvinceRow {
                id: 1,
                owner: 100,
                controller: 0,
                victory_points: 10,
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                owner: 101,
                controller: 100,
                victory_points: -5,
                ..Default::default()
            },
            ProvinceRow {
                id: 3,
                ..Default::default()
            },
        ]));
        DecodedFrame {
            branch: 1,
            hour: 42,
            tables,
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn aliases_share_country_borders_and_unrelated_table_failure_does_not_hide_map() {
        let palette = palette(&map(), &frame(), false).unwrap();
        assert_eq!(palette.colors[1], [102, 96, 87, 255]);
        assert_eq!(palette.colors[1], palette.colors[2]);
        assert_eq!(palette.owner_keys[1], palette.owner_keys[2]);
        assert_ne!(palette.owner_keys[1], 0);
        assert_eq!(palette.colors[3], WATER);
        assert!(palette.warnings.is_empty());
    }

    #[test]
    fn zero_control_is_named_and_never_inherits_owner() {
        let palette = palette(&map(), &frame(), true).unwrap();
        assert_eq!(palette.colors[1], UNKNOWN_LAND);
        assert_eq!(palette.owner_keys[1], 0);
        assert_eq!(palette.colors[2], [102, 96, 87, 255]);
        assert!(palette.warnings[0].contains("zero controller"));
        assert!(palette.warnings[0].contains("province ids 1"));
    }

    #[test]
    fn required_table_failure_and_absent_province_are_refused() {
        let mut frame = frame();
        frame.tables[TableId::CountryLinks.index()] = TableView::Truncated {
            rows: TableRows::CountryLinks(Vec::new()),
            missing: 7,
        };
        assert!(
            palette(&map(), &frame, false)
                .err()
                .unwrap()
                .to_string()
                .contains("CountryLinks is truncated (7 missing rows)")
        );
        let mut frame = super::tests::frame();
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        rows.pop();
        assert!(
            palette(&map(), &frame, false)
                .err()
                .unwrap()
                .to_string()
                .contains("no recorded row for map province 3")
        );
    }

    #[test]
    fn missing_country_color_and_dangling_links_are_named_errors() {
        let mut map = map();
        map.country_colors.rgb.clear();
        map.country_colors
            .unavailable
            .insert(b"GER".to_vec(), "runtime color only".into());
        assert!(
            palette(&map, &frame(), false)
                .err()
                .unwrap()
                .to_string()
                .contains("country GER color is unavailable: runtime color only")
        );
        let mut frame = frame();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(Vec::new()));
        assert!(
            palette(&super::tests::map(), &frame, false)
                .err()
                .unwrap()
                .to_string()
                .contains("province 1 references missing country link 100")
        );
    }

    #[test]
    fn every_unverified_identity_requires_override_and_keeps_its_reason() {
        let mut warnings = Vec::new();
        let state = Verification::Unverified {
            reason: "DLC roster differs".into(),
            launcher: Roster::default(),
            memory: Roster::default(),
        };
        assert!(check_verification("session 2", &state, false, &mut warnings).is_err());
        check_verification("session 2", &state, true, &mut warnings).unwrap();
        assert!(warnings[0].contains("session 2: DLC roster differs"));
        assert!(
            check_verification("session 3", &Verification::Unchecked, false, &mut warnings)
                .is_err()
        );
        check_verification("session 3", &Verification::Unchecked, true, &mut warnings).unwrap();
        assert_eq!(warnings.len(), 2);
    }
}
