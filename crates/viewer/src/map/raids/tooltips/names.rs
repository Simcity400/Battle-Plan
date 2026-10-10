//! Use the first resolving army, otherwise the wing.
use super::{content::Content, rules::Text};
use crate::combat::text::localise;
use engine::{cname_group_member as member, raid_tooltip_operands::*};
use installed_content::localisation::Argument as Arg;
use installed_content::localisation::Localisation;
use replay::{
    frame::DecodedFrame,
    rows::{RaidRow, RaidTooltipOperandRow, Recorded, tables},
};

pub struct Name<'a> {
    rows: Vec<&'a RaidTooltipOperandRow>,
    wing_name: Option<&'a Recorded<Option<Vec<u8>>>>,
}

impl<'a> Name<'a> {
    pub fn new(frame: &'a DecodedFrame, raid: &RaidRow) -> Result<Self, String> {
        let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let rows = frame
            .complete::<tables::RaidTooltipOperands>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|r| r.raid == id)
            .collect();
        let wing_name = frame
            .complete::<tables::AirWings>()
            .ok()
            .and_then(|wings| wings.iter().find(|wing| wing.id == raid.air_wing))
            .map(|wing| &wing.name);
        let out = Self { rows, wing_name };
        if out.rows.iter().any(|r| r.kind == UNREADABLE) {
            return Err("raid operands were unreadable when recorded".into());
        }
        for (key, reference) in [(0, raid.unit), (1, raid.air_wing)] {
            if out.value(REFERENCES, key)? as u64 != reference {
                return Err("raid naming reference differs from recorded raid".into());
            }
        }
        Ok(out)
    }
    fn value(&self, kind: u32, key: u32) -> Result<i64, String> {
        self.rows
            .iter()
            .find(|r| r.kind == kind && r.key == key)
            .map(|r| r.raw)
            .ok_or_else(|| format!("raid naming operand {kind}:{key:#x} absent"))
    }
    fn string(&self, kind: u32) -> Result<Option<String>, String> {
        self.bytes(kind)?
            .map(String::from_utf8)
            .transpose()
            .map_err(|e| e.to_string())
    }
    fn bytes(&self, kind: u32) -> Result<Option<Vec<u8>>, String> {
        let mut lengths = self
            .rows
            .iter()
            .filter(|r| r.kind == kind && r.key == u32::MAX);
        let Some(row) = lengths.next() else {
            return Ok(None);
        };
        if lengths.next().is_some() {
            return Err("raid name length operand is duplicated".into());
        }
        let len = usize::try_from(row.raw).map_err(|_| "raid name length invalid")?;
        if len.div_ceil(8) > self.rows.len() {
            return Err("raid name byte chunks were not recorded in full".into());
        }
        let mut text = Vec::new();
        text.try_reserve(len).map_err(|e| e.to_string())?;
        for (index, row) in self
            .rows
            .iter()
            .filter(|r| r.kind == kind && r.key != u32::MAX)
            .enumerate()
        {
            let key = u32::try_from(index).map_err(|_| "raid name chunk overflow")?;
            if index >= len.div_ceil(8) || row.key != key {
                return Err("raid name byte chunks are duplicate, extra or out of order".into());
            }
            text.extend_from_slice(&row.raw.to_le_bytes()[..(len - text.len()).min(8)]);
        }
        if text.len() != len {
            return Err("raid name byte chunks were not recorded in full".into());
        }
        Ok(Some(text))
    }
    pub fn assigned(
        &self,
        content: &Content,
        loc: &Localisation,
        recorded_language: &str,
    ) -> Result<Option<Text>, String> {
        let has_unit =
            self.value(NAME, ARMY_RESOLVED)? != 0 || self.value(NAME, WING_RESOLVED)? != 0;
        if !has_unit || self.value(NAME, UNIT_RAID_RESOLVED)? == 0 {
            return Ok(None);
        }
        Ok(Some(self.description(content, loc, recorded_language)))
    }
    pub fn description(
        &self,
        content: &Content,
        loc: &Localisation,
        recorded_language: &str,
    ) -> Text {
        localise(
            loc,
            "raid_inst_unit_desc",
            &[(
                "NAME",
                Arg::Text(&self.raw(content, loc, recorded_language)?),
            )],
        )
    }
    fn raw(&self, content: &Content, loc: &Localisation, recorded_language: &str) -> Text {
        if self.value(NAME, ARMY_RESOLVED)? != 0 {
            return self.member(content, NAME, NAME_GROUP, CUSTOM_NAME, 0);
        }
        if self.value(NAME, WING_RESOLVED)? != 0 {
            match self.wing_name {
                Some(Recorded::Value(Some(bytes))) => {
                    return dashboard::scripted_text::stored_text(
                        loc,
                        recorded_language,
                        bytes,
                        "raid wing name",
                    );
                }
                Some(Recorded::Value(None)) => {
                    return Err(
                        "AirWings stored wing name unavailable: guarded read or name arena refusal"
                            .into(),
                    );
                }
                Some(Recorded::NotRecorded) | None => {}
            }
            if let Ok(code) = self.value(WING_NAME, u32::MAX)
                && code < 0
            {
                return Err(format!(
                    "raid wing name was not recorded: {}",
                    wing_name_error(code)
                ));
            }
            let bytes = self
                .bytes(WING_NAME)
                .map_err(|reason| format!("raid wing name was not recorded: {reason}"))?
                .ok_or("raid wing name was not recorded")?;
            // Generated inputs and a rename marker are not retained. Share the
            // ship-name fallback, including language refusal.
            return dashboard::scripted_text::stored_text(
                loc,
                recorded_language,
                &bytes,
                "raid wing name",
            );
        }
        Ok(String::new())
    }
    fn member(
        &self,
        content: &Content,
        scalar: u32,
        group: u32,
        custom: u32,
        expected: i64,
    ) -> Text {
        if let Some(text) = self.string(custom)? {
            return Ok(text);
        }
        if self.value(scalar, member::FLAG_B as u32)? != 0
            && self.value(scalar, member::OVERRIDE_NAME_LENGTH as u32)? != 0
        {
            return Err("programmatic unit name has no retained localisation provenance".into());
        }
        if self.value(scalar, member::TYPE as u32)? != expected {
            return Err(format!(
                "unsupported name-group member type for naming family {scalar}"
            ));
        }
        let group = self
            .string(group)?
            .ok_or("name uses generated fallback without retained choice provenance")?;
        content.unit_names.as_ref().map_err(Clone::clone)?.name(
            &group,
            self.value(scalar, member::ORDER as u32)? as i32,
            self.value(scalar, member::FLAG_A as u32)? != 0,
        )
    }
    pub fn source(&self, content: &Content, reference: u64) -> Text {
        if self.value(REFERENCES, 2)? as u64 != reference {
            return Err("source ship name reference differs from raid".into());
        }
        if self.value(NAME, SOURCE_SHIP_RESOLVED)? == 0 {
            return Ok(String::new());
        }
        self.member(
            content,
            SOURCE_NAME,
            SOURCE_NAME_GROUP,
            SOURCE_CUSTOM_NAME,
            1,
        )
    }
    pub fn source_air_base_kind(&self) -> Result<i64, String> {
        self.value(NAME, SOURCE_AIR_BASE_KIND)
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{frame, put};
    use super::*;
    #[test]
    fn raid_names_prefer_recorded_wing_text_and_keep_old_format_fallback() {
        let raid = RaidRow {
            id: 9,
            reference_type: 91,
            air_wing: 123,
            ..Default::default()
        };
        let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let mut frame = frame();
        let mut loc = Localisation::default();
        loc.language = "english".into();
        put::<tables::RaidTooltipOperands>(
            &mut frame,
            [
                (REFERENCES, 0, 0),
                (REFERENCES, 1, 123),
                (NAME, ARMY_RESOLVED, 0),
                (NAME, WING_RESOLVED, 1),
                (WING_NAME, u32::MAX, 6),
                (WING_NAME, 0, i64::from_le_bytes(*b"Legacy\0\0")),
            ]
            .into_iter()
            .enumerate()
            .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                raid: id,
                ordinal: ordinal as u32,
                kind,
                key,
                raw,
            })
            .collect(),
        );
        for (name, expected) in [
            (
                Recorded::Value(Some(b"Stored %d [Root.GetTag]".to_vec())),
                Ok("Stored %d [Root.GetTag]"),
            ),
            (Recorded::Value(Some(Vec::new())), Ok("")),
            (
                Recorded::Value(None),
                Err("guarded read or name arena refusal"),
            ),
            (Recorded::NotRecorded, Ok("Legacy")),
        ] {
            put::<tables::AirWings>(
                &mut frame,
                vec![replay::rows::AirWingRow {
                    id: raid.air_wing,
                    name,
                    ..Default::default()
                }],
            );
            let name = Name::new(&frame, &raid).unwrap();
            match expected {
                Ok(expected) => assert_eq!(
                    name.raw(&Content::default(), &loc, "english").as_deref(),
                    Ok(expected)
                ),
                Err(expected) => assert!(
                    name.raw(&Content::default(), &loc, "english")
                        .unwrap_err()
                        .contains(expected)
                ),
            }
        }
        put::<tables::AirWings>(
            &mut frame,
            vec![replay::rows::AirWingRow {
                id: raid.air_wing,
                name: Recorded::Value(Some(b"Stored".to_vec())),
                ..Default::default()
            }],
        );
        loc.language = "french".into();
        assert!(
            Name::new(&frame, &raid)
                .unwrap()
                .raw(&Content::default(), &loc, "english")
                .unwrap_err()
                .contains("stored in english without its key")
        );
    }
    #[test]
    fn raid_wing_name_recorded_text_produces_native_coloured_find_line() {
        let raid = RaidRow {
            id: 9,
            reference_type: 91,
            air_wing: 123,
            phase: 4,
            ..Default::default()
        };
        let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let mut frame = frame();
        let mut loc: Localisation = [
            ("raid_inst_unit_desc".into(), "$NAME$".into()),
            (
                "tooltip_launch_button_right_click".into(),
                "§GRight-click§! to find §Y$NAME$§!".into(),
            ),
        ]
        .into_iter()
        .collect();
        loc.language = "english".into();
        for text in ["Generated Squadron", "User $literal$ [ROOT.GetName]"] {
            let mut values = vec![
                (REFERENCES, 0, 0),
                (REFERENCES, 1, 123),
                (NAME, ARMY_RESOLVED, 0),
                (NAME, WING_RESOLVED, 1),
                (NAME, UNIT_RAID_RESOLVED, 1),
                (WING_NAME, u32::MAX, text.len() as i64),
            ];
            values.extend(text.as_bytes().chunks(8).enumerate().map(|(index, chunk)| {
                let mut bytes = [0; 8];
                bytes[..chunk.len()].copy_from_slice(chunk);
                (WING_NAME, index as u32, i64::from_le_bytes(bytes))
            }));
            put::<tables::RaidTooltipOperands>(
                &mut frame,
                values
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                        raid: id,
                        ordinal: ordinal as u32,
                        kind,
                        key,
                        raw,
                    })
                    .collect(),
            );
            let draw = |loc: &Localisation, language: &str| {
                super::super::rules::raid(
                    loc,
                    super::super::rules::Raid {
                        ended: false,
                        phase: raid.phase,
                        description: Ok(String::new()),
                        outcome: 0,
                        progress: Ok(None),
                        gate: (Ok(true), vec![]),
                        assigned_unit: Name::new(&frame, &raid).unwrap().assigned(
                            &Content::default(),
                            loc,
                            language,
                        ),
                        owner: Ok(String::new()),
                        success: Ok(0),
                        counterplay: Ok(vec![]),
                    },
                    false,
                )
            };
            let tooltip = draw(&loc, &loc.language);
            assert!(
                tooltip.body.contains(&format!(
                    "\u{11}GRight-click\u{11}! to find \u{11}Y{text}\u{11}!"
                )),
                "{tooltip:?}"
            );
            assert!(tooltip.refused.is_empty(), "{tooltip:?}");
            let mut other = loc.clone();
            other.language = "german".into();
            let mismatch = draw(&other, &loc.language);
            assert!(!mismatch.body.contains(text));
            assert_eq!(mismatch.refused.len(), 1);
            let (reason, count) = mismatch.refused.iter().next().unwrap();
            assert!(reason.contains("tooltip_launch_button_right_click: raid wing name is text the game stored in english"));
            assert!(reason.contains("viewer's language is german"));
            assert_eq!(*count, 1);
        }
    }
    #[test]
    fn raid_wing_name_empty_failure_and_invalid_utf8_remain_distinct() {
        let raid = RaidRow {
            id: 9,
            reference_type: 91,
            air_wing: 123,
            ..Default::default()
        };
        let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let mut frame = frame();
        let loc: Localisation = [("raid_inst_unit_desc".into(), "$NAME$".into())]
            .into_iter()
            .collect();
        for (extra, expected) in [
            (vec![(u32::MAX, 0)], Ok("")),
            (vec![], Err("was not recorded")),
            (
                vec![(u32::MAX, NameReadError::Header.code())],
                Err("length exceeds capacity"),
            ),
            (vec![(u32::MAX, 0), (u32::MAX, 0)], Err("duplicated")),
            (vec![(u32::MAX, 1)], Err("not recorded in full")),
            (vec![(u32::MAX, 0), (0, 0)], Err("extra")),
            (vec![(u32::MAX, 1), (1, 0)], Err("out of order")),
            (vec![(u32::MAX, 1), (0, 255)], Err("not UTF-8")),
        ] {
            let values = [
                (REFERENCES, 0, 0),
                (REFERENCES, 1, 123),
                (NAME, ARMY_RESOLVED, 0),
                (NAME, WING_RESOLVED, 1),
                (NAME, UNIT_RAID_RESOLVED, 1),
            ]
            .into_iter()
            .chain(extra.into_iter().map(|(key, raw)| (WING_NAME, key, raw)));
            put::<tables::RaidTooltipOperands>(
                &mut frame,
                values
                    .enumerate()
                    .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                        raid: id,
                        ordinal: ordinal as u32,
                        kind,
                        key,
                        raw,
                    })
                    .collect(),
            );
            let language = if expected.is_ok() {
                "german"
            } else {
                &loc.language
            };
            let actual = Name::new(&frame, &raid)
                .unwrap()
                .assigned(&Content::default(), &loc, language)
                .unwrap()
                .unwrap();
            match expected {
                Ok(expected) => assert_eq!(actual.unwrap(), expected),
                Err(expected) => assert!(actual.unwrap_err().contains(expected)),
            }
        }
    }
    #[test]
    fn assignment_uses_resolving_back_reference_and_preserves_custom_bytes() {
        let raid = RaidRow {
            id: 9,
            reference_type: 91,
            unit: 123,
            ..Default::default()
        };
        let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let mut frame = frame();
        let loc: Localisation = [("raid_inst_unit_desc".into(), "unit($NAME$)".into())]
            .into_iter()
            .collect();
        let content = Content::default();
        for resolved in [0, 1] {
            let values = [
                (REFERENCES, 0, 123),
                (REFERENCES, 1, 0),
                (NAME, ARMY_RESOLVED, 1),
                (NAME, WING_RESOLVED, 0),
                (NAME, UNIT_RAID_RESOLVED, resolved),
                (CUSTOM_NAME, u32::MAX, 3),
                (CUSTOM_NAME, 0, i64::from_le_bytes(*b"abc\0\0\0\0\0")),
            ];
            put::<tables::RaidTooltipOperands>(
                &mut frame,
                values
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                        raid: id,
                        ordinal: ordinal as u32,
                        kind,
                        key,
                        raw,
                    })
                    .collect(),
            );
            let name = Name::new(&frame, &raid).unwrap();
            assert_eq!(
                name.assigned(&content, &loc, &loc.language).unwrap(),
                (resolved == 1).then(|| Ok("unit(abc)".into()))
            );
        }
    }
    #[test]
    fn unrecorded_wing_name_and_incomplete_chunks_are_named_failures() {
        let raid = RaidRow {
            id: 9,
            reference_type: 91,
            air_wing: 123,
            ..Default::default()
        };
        let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let mut frame = frame();
        let values = [
            (REFERENCES, 0, 0),
            (REFERENCES, 1, 123),
            (NAME, ARMY_RESOLVED, 0),
            (NAME, WING_RESOLVED, 1),
            (WING_NAME, u32::MAX, 12),
            (CUSTOM_NAME, u32::MAX, 9),
        ];
        put::<tables::RaidTooltipOperands>(
            &mut frame,
            values
                .into_iter()
                .enumerate()
                .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                    raid: id,
                    ordinal: ordinal as u32,
                    kind,
                    key,
                    raw,
                })
                .collect(),
        );
        let name = Name::new(&frame, &raid).unwrap();
        assert!(
            name.raw(&Content::default(), &Localisation::default(), "english")
                .unwrap_err()
                .contains("not recorded")
        );
        assert!(
            name.string(CUSTOM_NAME)
                .unwrap_err()
                .contains("not recorded in full")
        );
    }
}
