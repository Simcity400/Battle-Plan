use replay_data::rows::{NameOperands, Recorded, ScientistTemplateOrigin, UnitLeaderRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Leader<'a> {
    pub reference_type: u32,
    pub id: u32,
    pub character_type: u32,
    pub character_id: u32,
    pub category: i32,
    pub owner: u32,
    pub skill_status: u8,
    pub skill_level: i32,
    pub captured: u8,
    pub visibility_choice: u8,
    pub template_visibility: u8,
    pub inherited_text_a: &'a Vec<u8>,
    pub inherited_text_b: &'a Vec<u8>,
    pub exile_country: &'a Recorded<i32>,
    pub codename: &'a Recorded<Option<NameOperands>>,
    pub visible_state: &'a Recorded<Option<u8>>,
    pub visible_origin: &'a Recorded<Option<ScientistTemplateOrigin>>,
}

impl Leader<'_> {
    pub const FIELDS: &'static [&'static str] = &[
        "character_type",
        "character_id",
        "category",
        "owner",
        "skill_status",
        "skill_level",
        "captured",
        "visibility_choice",
        "template_visibility",
        "inherited_text_a",
        "inherited_text_b",
        "exile_country",
        "codename",
        "visible_state",
        "visible_origin",
    ];
}

impl<'a> From<&'a UnitLeaderRow> for Leader<'a> {
    fn from(row: &'a UnitLeaderRow) -> Self {
        Self {
            reference_type: row.reference_type,
            id: row.id,
            character_type: row.character_type,
            character_id: row.character_id,
            category: row.category,
            owner: row.owner,
            skill_status: row.skill_status,
            skill_level: row.skill_level,
            captured: row.captured,
            visibility_choice: row.visibility_choice,
            template_visibility: row.template_visibility,
            inherited_text_a: &row.inherited_text_a,
            inherited_text_b: &row.inherited_text_b,
            exile_country: &row.exile_country,
            codename: &row.codename,
            visible_state: &row.visible_state,
            visible_origin: &row.visible_origin,
        }
    }
}

impl<'a> From<&Leader<'a>> for Leader<'a> {
    fn from(row: &Leader<'a>) -> Self {
        *row
    }
}

pub struct Leaders<'a>(pub(super) std::slice::Iter<'a, UnitLeaderRow>);
impl Leaders<'_> {
    pub fn iter(self) -> Self {
        self
    }
}
impl<'a> Iterator for Leaders<'a> {
    type Item = Leader<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Leader::from)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
