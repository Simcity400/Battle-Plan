///! Typed facts shared by the selected-hour panel and prepared faction histories.
pub use super::companions::{PROGRAMME_TABLES, owned};
pub use super::derived::{
    DoctrineMinimum, MemberUpgradeSource, ResearchLevel, doctrine_progress, member_upgrade_sources,
    research_level,
};
use super::*;
use dashboard_world::{Mark, Viewed};
use replay_data::rows::{
    CountryLinkRow, CountryRow, FactionAccountRow, FactionDoctrineRow, FactionManpowerRow,
    FactionProjectionRow, FactionRuleRow, mask,
};
use replay_frame::frame::HourFrame;
use std::collections::BTreeMap;

/// The fields country identity is read by.
const IDENTITY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);

pub fn selected<T>(rows: &[T], faction: u64, owner: impl Fn(&T) -> u64) -> &[T] {
    companions::selected(rows, faction, owner)
}
pub fn manpower_total(rows: &[FactionManpowerRow]) -> Result<i32, String> {
    manpower_total_of(rows)
}
/// [`manpower_total`] of a faction's rows, in ordinal order.
pub fn manpower_total_of<'r>(
    rows: impl IntoIterator<Item = &'r FactionManpowerRow>,
) -> Result<i32, String> {
    let mut total = 0i32;
    for (ordinal, row) in rows.into_iter().enumerate() {
        if ordinal != row.ordinal as usize {
            return Err("Faction manpower ordinals are not contiguous".into());
        }
        total = total.wrapping_add(row.value);
    }
    Ok(total)
}
pub fn influence_share(row: &FactionAccountRow) -> i64 {
    row.influence_share()
}
pub fn influence_total(
    row: &FactionProjectionRow,
    accounts: &[FactionAccountRow],
) -> Result<i64, String> {
    row.influence_total(accounts).map_err(|why| why.to_string())
}
pub fn country_tag(frame: HourFrame<'_>, raw: u32) -> Result<String, String> {
    companions::tag_in(frame, raw)
}
pub fn rule(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionRuleRow,
    cat: Option<&catalogue::Catalogue>,
) -> FactionRuleDetail {
    companions::rule(frame, faction, row, cat, true)
}
pub fn doctrine(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionDoctrineRow,
    cat: Option<&catalogue::Catalogue>,
) -> FactionRuleDetail {
    companions::doctrine(frame, faction, row, cat)
}
/// [`rule`] of any frame's tables, a dynamic name expanded in `held`, the shared text of
/// `frame` and `cat`.
pub fn rule_in(
    frame: HourFrame<'_>,
    faction: u64,
    row: &FactionRuleRow,
    cat: Option<&catalogue::Catalogue>,
    held: Option<&ViewerText<'_>>,
) -> FactionRuleDetail {
    companions::rule_in(frame, faction, row, cat, true, held)
}
/// [`doctrine`] of any frame's tables, a dynamic name expanded in `held`, the shared text of
/// `frame` and `cat`.
pub fn doctrine_in(
    frame: HourFrame<'_>,
    faction: u64,
    row: &FactionDoctrineRow,
    cat: Option<&catalogue::Catalogue>,
    held: Option<&ViewerText<'_>>,
) -> FactionRuleDetail {
    companions::doctrine_in(frame, faction, row, cat, held)
}
pub fn programme(
    frame: &DecodedFrame,
    reference: u64,
) -> Result<&replay_data::rows::ProjectProgramRow, String> {
    companions::programme(frame, reference)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    pub slot: u32,
    pub id: u32,
    pub tag: String,
}
/// Country identity and raw-reference joins; balances are deliberately not operands.
#[derive(Default)]
pub struct Countries {
    keys: Option<crate::metadata::CountryKeys>,
    links: BTreeMap<u32, u32>,
    holders: BTreeMap<u32, Option<Country>>,
    /// The links as last read.
    link_mark: Option<Mark>,
    pub rebuilds: usize,
}
impl Countries {
    /// Reads the countries of `viewed`'s hour again, unless `changes` prove its identity
    /// unchanged since the hour before (`continues`): whether they changed.
    pub fn refresh(
        &mut self,
        viewed: Viewed<'_>,
        changes: &crate::prepared::frame_changes::FrameChanges,
        continues: bool,
    ) -> Result<bool, String> {
        let frame = viewed.frame;
        let inputs = frame.complete::<tables::Countries>(IDENTITY).and_then(|rows| {
            frame
                .complete::<tables::CountryLinks>(LINK)
                .map(|links| (rows, links))
        });
        let (rows, links) = match inputs {
            Ok(inputs) => inputs,
            Err(why) => {
                self.keys = None;
                self.holders.clear();
                self.links.clear();
                self.link_mark = None;
                return Err(why.to_string());
            }
        };
        if continues
            && self.keys.as_ref().is_some_and(|keys| {
                keys.kept_changes(rows.len(), changes.rows(TableId::Countries))
                    || keys.same_in(rows)
            })
            && self
                .link_mark
                .as_ref()
                .is_some_and(|mark| viewed.is(TableId::CountryLinks, mark))
        {
            return Ok(false);
        }
        self.keys = Some(crate::metadata::CountryKeys::read_in(rows).map_err(|why| why.to_string())?);
        self.links = crate::metadata::link_slots_in(links)
            .map_err(|why| why.to_string())?
            .into_iter()
            .collect();
        self.holders.clear();
        for row in rows.iter() {
            let row = row.map_err(|why| why.to_string())?;
            let country = String::from_utf8(row.tag.clone()).ok().map(|tag| Country {
                slot: row.canonical_slot,
                id: row.id,
                tag,
            });
            self.holders
                .entry(row.canonical_slot)
                .and_modify(|entry| *entry = None)
                .or_insert(country);
        }
        self.link_mark = Some(viewed.mark(TableId::CountryLinks));
        self.rebuilds += 1;
        Ok(true)
    }
    pub fn get(&self, raw: u32) -> Option<&Country> {
        self.holders.get(self.links.get(&raw)?)?.as_ref()
    }
}

/// Current membership is independent of a retained account's supplying faction.
pub fn membership(
    frame: HourFrame<'_>,
    countries: &Countries,
) -> Result<BTreeMap<u32, Option<u64>>, String> {
    let rows = frame
        .complete::<tables::FactionMembers>(u64::MAX)
        .map_err(|why| why.to_string())?;
    let mut owners = BTreeMap::new();
    for row in rows.iter() {
        let row = row.map_err(|why| why.to_string())?;
        if let Some(country) = countries.get(row.country) {
            owners
                .entry(country.slot)
                .and_modify(|owner: &mut Option<u64>| {
                    if *owner != Some(row.faction) {
                        *owner = None;
                    }
                })
                .or_insert(Some(row.faction));
        }
    }
    Ok(owners)
}

pub fn upgrade(
    frame: HourFrame<'_>,
    group: u32,
    token: u32,
    cat: Option<&catalogue::Catalogue>,
) -> FactionUpgradeDetail {
    let mut detail = FactionUpgradeDetail {
        group,
        token: None,
        name: None,
        reason: None,
    };
    let mut read = || -> Result<String, String> {
        let spelling = faction_names::template_spelling_in(&frame, Some(token))?
            .ok_or("Faction member upgrade token is absent")?;
        detail.token = Some(spelling.clone());
        let cat = cat.ok_or("Faction member upgrade needs recorded localisation")?;
        let definitions = installed_content_core::faction_upgrades::cached(&cat.content.layers)?;
        let definition = definitions.upgrade(spelling.as_bytes())?;
        let key = definition.name.as_deref().unwrap_or(spelling.as_bytes());
        let text = cat
            .localisation
            .localise(key)
            .map_err(|why| why.to_string())?;
        if text.contains('[') {
            return Err(format!(
                "Faction upgrade {spelling}: dynamic member name environment is unavailable"
            ));
        }
        catalogue::plain_text(&text)
    };
    match read() {
        Ok(name) => detail.name = Some(name),
        Err(reason) => detail.reason = Some(reason),
    }
    detail
}
pub fn group_name(
    frame: HourFrame<'_>,
    group: u32,
    cat: Option<&catalogue::Catalogue>,
) -> Result<Option<String>, String> {
    let spelling = faction_names::template_spelling_in(&frame, Some(group))?
        .ok_or("Faction upgrade group spelling is absent")?;
    let cat =
        cat.ok_or_else(|| format!("Faction upgrade group {spelling} requires recorded content"))?;
    let definitions = installed_content_core::faction_upgrades::cached(&cat.content.layers)?;
    let definition = definitions.group(spelling.as_bytes())?;
    let key = definition
        .name
        .as_deref()
        .ok_or_else(|| format!("Faction upgrade group {spelling}: authored name is unavailable"))?;
    let text = cat
        .localisation
        .localise(key)
        .map_err(|why| why.to_string())?;
    if text.contains('[') {
        return Err(format!(
            "Faction upgrade group {spelling}: dynamic name environment is unavailable"
        ));
    }
    catalogue::plain_text(&text).map(Some)
}

/// An owner witness is independent of the faction's name/header acquisition. A row that cannot
/// be built witnesses none.
pub fn owners(frame: HourFrame<'_>) -> std::collections::BTreeSet<u64> {
    let mut owners = std::collections::BTreeSet::new();
    if let Ok(rows) = frame.complete::<tables::Factions>(0) {
        owners.extend(rows.keys().map(|key| key as u64));
    }
    // Each of these tables names its faction in its key.
    if let Ok(rows) = frame.complete::<tables::FactionProjection>(0) {
        owners.extend(rows.keys().map(|key| key as u64));
    }
    if let Ok(rows) = frame.complete::<tables::FactionUnlocks>(0) {
        owners.extend(rows.keys().map(|key| key as u64));
    }
    if let Ok(rows) = frame.complete::<tables::FactionGoalStatus>(0) {
        owners.extend(rows.keys().map(|key| key as u64));
    }
    macro_rules! ordinals { ($($table:ident),+ $(,)?) => { $(if let Ok(rows) = frame.complete::<tables::$table>(0) { owners.extend(rows.keys().map(|key| (key >> 32) as u64)); })+ }; }
    ordinals!(
        FactionRules,
        FactionPrograms,
        FactionMinimumUpgrades,
        FactionDoctrine,
        FactionManpower,
    );
    if let Ok(rows) = frame.complete::<tables::FactionMembers>(u64::MAX) {
        owners.extend(rows.iter().filter_map(|row| row.ok().map(|row| row.faction)));
    }
    owners.remove(&0);
    owners
}

pub fn programme_detail(
    supplying: Option<HourFrame<'_>>,
    ordinal: u32,
    reference: u64,
    cat: Option<&catalogue::Catalogue>,
) -> FactionProgramDetail {
    companions::programme_detail(supplying, ordinal, reference, cat)
}

/// The supplying read's tables with those a scripted project name reads.
pub fn programme_text_tables() -> Vec<TableId> {
    let mut tables = PROGRAMME_TABLES.to_vec();
    tables.extend_from_slice(crate::scripted_text::TABLES);
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// Whether any faction's programme in `frame` names a scripted project at the supplying read `details`.
pub fn programmes_need_environment(
    frame: HourFrame<'_>,
    details: HourFrame<'_>,
    cat: &catalogue::Catalogue,
) -> bool {
    frame
        .complete::<tables::FactionPrograms>(0)
        .is_ok_and(|rows| {
            let factions: std::collections::BTreeSet<u64> =
                rows.keys().map(|key| (key >> 32) as u64).collect();
            factions
                .into_iter()
                .any(|faction| companions::programmes_need_environment(frame, details, faction, cat))
        })
}
/// Whether any programme the supplying read `details` records names a scripted project: the
/// widest [`programmes_need_environment`] any faction's programmes could ask of that read.
pub fn read_names_scripted(details: HourFrame<'_>, cat: &catalogue::Catalogue) -> bool {
    companions::read_names_scripted(details, cat)
}
#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{LexicalTokenRow, TableRows};
    use replay_frame::frame::TableView;
    #[test]
    fn faction_upgrade_names_use_authored_mod_labels_and_keep_named_dynamic_refusals() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (cat, _folder) = catalogue::tests::folder_catalogue(&[
            (
                "common/factions/member_upgrades/mod.txt",
                "mod_upgrade={name=UPGRADE_NAME bonus=0.25} dynamic_upgrade={name=DYNAMIC_NAME}",
            ),
            (
                "common/factions/member_upgrades/member_groups/mod.txt",
                "mod_group={name=GROUP_NAME upgrade_type=mod_kind upgrades={mod_upgrade dynamic_upgrade}} unnamed_group={upgrades={}}",
            ),
            (
                "localisation/mod_l_english.yml",
                // A localisation file is read only after its leading UTF-8 BOM.
                "\u{feff}l_english:
 UPGRADE_NAME:0 \"Authored mod upgrade\"
 GROUP_NAME:0 \"Authored mod group\"
 DYNAMIC_NAME:0 \"[ROOT.GetName]\"
",
            ),
        ]);
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                [
                    (1, "mod_group"),
                    (2, "mod_upgrade"),
                    (3, "dynamic_upgrade"),
                    (4, "unnamed_group"),
                ]
                .into_iter()
                .map(|(token, literal)| LexicalTokenRow {
                    token,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
            ));
        assert_eq!(
            upgrade(HourFrame::from(&frame), 1, 2, Some(&cat)).name.as_deref(),
            Some("Authored mod upgrade")
        );
        assert_eq!(
            group_name(HourFrame::from(&frame), 1, Some(&cat)).unwrap().as_deref(),
            Some("Authored mod group")
        );
        let dynamic = upgrade(HourFrame::from(&frame), 1, 3, Some(&cat));
        assert_eq!(dynamic.token.as_deref(), Some("dynamic_upgrade"));
        assert!(dynamic.name.is_none());
        assert!(dynamic.reason.unwrap().contains("dynamic member name"));
        assert!(
            group_name(HourFrame::from(&frame), 4, Some(&cat))
                .unwrap_err()
                .contains("unnamed_group")
        );
    }
}
