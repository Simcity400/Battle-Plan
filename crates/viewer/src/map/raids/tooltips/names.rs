//! 1414536D0/1414545D0: the first resolving army, otherwise wing.
use super::{
    content::Content,
    rules::Text,
    text::{Arg, localise},
};
use engine::{cname_group_member as member, raid_tooltip_operands::*};
use installed_content::localisation::Localisation;
use replay::{
    frame::DecodedFrame,
    rows::{RaidRow, RaidTooltipOperandRow, tables},
};

pub struct Name<'a> {
    rows: Vec<&'a RaidTooltipOperandRow>,
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
        let out = Self { rows };
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
        let Some(row) = self
            .rows
            .iter()
            .find(|r| r.kind == kind && r.key == u32::MAX)
        else {
            return Ok(None);
        };
        let len = usize::try_from(row.raw).map_err(|_| "raid name length invalid")?;
        let mut text = Vec::new();
        text.try_reserve(len).map_err(|e| e.to_string())?;
        for i in 0..len.div_ceil(8) {
            let value = self.value(
                kind,
                u32::try_from(i).map_err(|_| "raid name chunk overflow")?,
            )?;
            text.extend_from_slice(&value.to_le_bytes()[..(len - text.len()).min(8)]);
        }
        String::from_utf8(text).map(Some).map_err(|e| e.to_string())
    }
    pub fn assigned(&self, content: &Content, loc: &Localisation) -> Result<Option<Text>, String> {
        let has_unit =
            self.value(NAME, ARMY_RESOLVED)? != 0 || self.value(NAME, WING_RESOLVED)? != 0;
        if !has_unit || self.value(NAME, UNIT_RAID_RESOLVED)? == 0 {
            return Ok(None);
        }
        Ok(Some(self.description(content, loc)))
    }
    pub fn description(&self, content: &Content, loc: &Localisation) -> Text {
        localise(
            loc,
            "raid_inst_unit_desc",
            &[("NAME", Arg::Text(self.raw(content)?))],
        )
    }
    fn raw(&self, content: &Content) -> Text {
        if self.value(NAME, ARMY_RESOLVED)? != 0 {
            return self.member(content, NAME, NAME_GROUP, CUSTOM_NAME, 0);
        }
        if self.value(NAME, WING_RESOLVED)? != 0 && self.value(NAME, WING_NAME_LENGTH)? != 0 {
            return Err("wing name 140F51FB0 retains neither generated-name key nor user/generated provenance".into());
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
            return Err(
                "programmatic unit name 1409B4C40 has no retained localisation provenance".into(),
            );
        }
        if self.value(scalar, member::TYPE as u32)? != expected {
            return Err(format!(
                "unsupported CNameGroupMember type for naming family {scalar}"
            ));
        }
        let group = self
            .string(group)?
            .ok_or("name uses generated fallback 1409B4CE0 without retained choice provenance")?;
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
                name.assigned(&content, &loc).unwrap(),
                (resolved == 1).then(|| Ok("unit(abc)".into()))
            );
        }
    }
    #[test]
    fn generated_wing_name_and_incomplete_chunks_are_named_failures() {
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
            (NAME, WING_NAME_LENGTH, 12),
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
            name.raw(&Content::default())
                .unwrap_err()
                .contains("provenance")
        );
        assert!(name.string(CUSTOM_NAME).unwrap_err().contains("absent"));
    }
}
