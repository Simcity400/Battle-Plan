use super::{
    LoadContext,
    at_hour::{At, AtHour},
    game_text::{EntityKind, GameRun, runs},
    politics_content::{PoliticsContent, politics_content},
};
use crate::{Source, catalogue};
use content_reader_core::{
    combat_tactics::Tactic,
    equipment::{Database, ModuleDesign, STAT_COUNT, StatComponents, StatGroup},
    fixed5::format_fixed5,
};
use content_reader_localisation::localisation::{Argument, Localisation};
use dashboard_text::subunit_requirements::Content as SubUnitSheets;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub struct EntityStatsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntityStatsParams {
    pub branch_id: String,
    pub kind: EntityKind,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EntityStat {
    #[doc = " The game's label for the stat."]
    pub label: Vec<GameRun>,
    #[doc = " The value as the game formats it, coloured as the game colours it."]
    pub value: Vec<GameRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EntityStats {
    pub kind: EntityKind,
    pub token: String,
    pub stats: Vec<EntityStat>,
    #[doc = " Why stats were withheld, with how often each was met."]
    pub refusals: BTreeMap<String, usize>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EntityStats);

pub struct Context {
    content: Result<Arc<PoliticsContent>, String>,
}

type Metadata = [(&'static str, &'static str, bool)];

struct Sheet<'l> {
    localisation: &'l Localisation,
    stats: Vec<EntityStat>,
    refusals: BTreeMap<String, usize>,
}

impl<'l> Sheet<'l> {
    fn new(localisation: &'l Localisation) -> Self {
        Self { localisation, stats: Vec::new(), refusals: BTreeMap::new() }
    }

    fn text(&self, key: &str, args: &[(&str, Argument<'_>)]) -> Result<String, String> {
        if self.localisation.get(key).is_none() {
            return Err(format!("localisation {key} is not defined"));
        }
        self.localisation.localise_with(key.as_bytes(), args).map_err(|why| why.to_string())
    }

    fn refuse(&mut self, why: String) {
        *self.refusals.entry(why).or_default() += 1;
    }

    fn push(&mut self, label: Result<String, String>, value: Result<String, String>) {
        let line = label.and_then(|label| Ok(EntityStat { label: runs(&label)?, value: runs(&value?)? }));
        match line {
            Ok(line) => self.stats.push(line),
            Err(why) => self.refuse(why),
        }
    }

    fn stat(&mut self, metadata: &Metadata, index: usize, value: i64, formats: &[&str]) {
        let Some(&(_, label, _)) = metadata.get(index) else {
            return self.refuse(format!("stat {index} has no game name"));
        };
        let format = formats.iter().map(|format| format.replace("{}", label)).find(|key| self.localisation.get(key).is_some());
        let value = match format {
            Some(format) => self.text(&format, &[("VALUE", Argument::Fixed5(value))]),
            None => Err(format!("{label} has no value format")),
        };
        self.push(self.text(label, &[]), value);
    }

    fn equipment(&mut self, metadata: &Metadata, stats: &[i64]) {
        for (index, &value) in stats.iter().enumerate().filter(|(_, value)| **value != 0) {
            self.stat(metadata, index, value, &["{}_VALUE"]);
        }
    }

    fn module(&mut self, metadata: &Metadata, stats: &StatGroup) {
        for term in &stats.add {
            self.stat(metadata, term.index, term.value, &["{}_DIFF"]);
        }
        for term in &stats.average {
            let before = self.stats.len();
            self.stat(metadata, term.index, term.value, &["{}_DIFF"]);
            if self.stats.len() > before {
                match self.text("EQUIPMENT_MODULE_STAT_AVERAGED", &[]).and_then(|suffix| runs(&suffix)) {
                    Ok(suffix) => self.stats.last_mut().expect("a pushed stat").value.extend(suffix),
                    Err(why) => self.refuse(why),
                }
            }
        }
        for term in &stats.multiply {
            let positive = metadata.get(term.index).is_some_and(|(_, _, positive)| *positive);
            let fallback = if positive { "STAT_MODIFIER_DIFF" } else { "STAT_MODIFIER_DIFF_NEG" };
            self.stat(metadata, term.index, term.value, &["{}_MOD_DIFF", fallback]);
        }
    }

    fn tactic(&mut self, tactic: &Tactic, warnings: &mut Vec<String>) {
        let mut attacker = false;
        for item in tactic.fields.iter().filter(|item| item.keyword_is("is_attacker")) {
            match item.value.scalar().and_then(content_reader_core::parser::read_bool) {
                Some(value) => attacker = value,
                None => warnings.push(format!("tactic {}'s is_attacker is not exactly yes or no; the game keeps {}", String::from_utf8_lossy(&tactic.token), if attacker { "yes" } else { "no" })),
            }
        }
        for (label, slot, favourable) in [
            ("TACTICS_DAMAGE_ATTACKER", 2, Some(attacker)),
            ("TACTICS_DAMAGE_DEFENDER", 4, Some(!attacker)),
            ("TACTICS_COMBATWIDTH", 0, None),
            ("TACTICS_MOVEMENTSPEED_ATTACKER", 1, Some(attacker)),
            ("TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER", 3, Some(attacker)),
            ("TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER", 5, Some(attacker)),
        ] {
            let format = match favourable {
                None => "TACTIC_LIST_VALUE_NEUTRAL",
                Some(true) => "TACTIC_LIST_VALUE_GOOD",
                Some(false) => "TACTIC_LIST_VALUE_BAD",
            };
            self.push(self.text(label, &[]), self.text(format, &[("VAL", Argument::Fixed5(tactic.effects[slot]))]));
        }
    }

    fn sub_unit(&mut self, metadata: &Metadata, sheet: &dashboard_text::subunit_requirements::Sheet) {
        let refused = |field: &str| sheet.refused.iter().find(|refusal| refusal.field == field).map(|refusal| refusal.why.clone());
        let width = match refused("combat_width") {
            Some(why) => Err(why),
            None => self.text("DESIGNER_COMBATWIDTH_VALUE", &[("VALUE", Argument::Fixed5(sheet.combat_width))]),
        };
        self.push(self.text("DESIGNER_COMBATWIDTH", &[]), width);
        for (label, field, value) in [("DESIGNER_MANPOWER", "manpower", sheet.manpower), ("DESIGNER_TRAINING_TIME", "training_time", sheet.training_time)] {
            self.push(self.text(label, &[]), refused(field).map_or_else(|| Ok(value.to_string()), Err));
        }
        if sheet.words.iter().any(|(word, _)| metadata.iter().any(|(name, _, _)| name.eq_ignore_ascii_case(word))) {
            self.refuse("Combat stats of a sub-unit come from its equipment in the game; they are not derived here".into());
        }
    }
}

fn lossy(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

fn off_sheet(metadata: &Metadata, design: &ModuleDesign) -> Vec<String> {
    let fixed = |value| format_fixed5(value, true, 5);
    let mut out = Vec::new();
    for (missions, group) in &design.mission_terms {
        let missions: Vec<_> = missions.iter().map(|mission| lossy(mission)).collect();
        let only = if missions.is_empty() { String::new() } else { format!(" on {} missions", missions.join(", ")) };
        for (terms, kind) in [(&group.add, "added"), (&group.average, "averaged"), (&group.multiply, "multiplied")] {
            for term in terms {
                let stat = metadata.get(term.index).map_or_else(|| format!("stat {}", term.index), |(name, _, _)| (*name).to_owned());
                out.push(format!("{stat} {} ({kind}) applies only{only} under mission_type_stats; mission-conditional stats are not shown", fixed(term.value)));
            }
        }
    }
    if let Some(manpower) = design.manpower {
        out.push(format!("manpower {} is outside the stat block and is not shown", fixed(manpower)));
    }
    for (resource, amount) in &design.build_resources {
        out.push(format!("resource cost {} {} is outside the stat block and is not shown", lossy(resource), fixed(*amount)));
    }
    out
}

struct Totals {
    stats: [i64; STAT_COUNT],
    withheld: Vec<String>,
}

fn equipment_totals(database: &Database, metadata: &Metadata, token: &str) -> Result<Totals, String> {
    let at = database.get(token).ok_or_else(|| format!("Equipment {token} is not in the recorded content"))?;
    let equipment = &database.equipment[at];
    let mut components = StatComponents::base(&equipment.stats);
    let mut withheld = Vec::new();
    for held in database.lineage(equipment)? {
        for keyword in &held.off_sheet {
            let why = format!("{} of {} is outside the stat block and is not shown", lossy(keyword), lossy(&held.name));
            if !withheld.contains(&why) {
                withheld.push(why);
            }
        }
    }
    for (slot, module) in database.default_modules(equipment)? {
        let (shown, slot) = (lossy(module), lossy(slot));
        let design = database.module_design(module).ok_or_else(|| format!("Default module {shown} in slot {slot} of equipment {token} is not in the recorded content, so its stats are withheld"))?;
        if let Some(group) = database.module_stats(module) {
            components.apply_module(group, equipment)?;
        }
        withheld.extend(off_sheet(metadata, design).into_iter().map(|why| format!("Default module {shown} in slot {slot}: {why}")));
    }
    Ok(Totals { stats: components.finish_for(equipment)?, withheld })
}

fn stat_metadata() -> Result<&'static Metadata, String> {
    engine_executable::executable_tables::auxiliary_part(|tables| &tables.stat_metadata)
        .map(Vec::as_slice)
        .map_err(|why| format!("The game's stat names are withheld: {why}"))
}

impl AtHour for EntityStatsAt {
    const KEY: &'static str = "content-entity-stats";
    const TITLE: &'static str = "Content entity stats";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EntityStatsParams;
    type Reply = EntityStats;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.token.is_empty() {
            return Err("Entity stats need a content token");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EntityStatsParams::decl(), EntityStats::decl(), EntityStat::decl()]
    }

    const CLOCK_DEPENDENT: bool = false;
    const REPLY_NAMES_NO_HOUR: bool = true;

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        Vec::new()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let content = match load.content.as_ref() {
            Some(options) => politics_content(source, options).map_err(|error| format!("Recorded content is unavailable: {error}")),
            None => Err("Entity stats need the recorded content stack".to_owned()),
        };
        Ok(Context { content })
    }

    fn reply(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut out = EntityStats { kind: params.kind, token: params.token.clone(), stats: Vec::new(), refusals: BTreeMap::new(), unavailable: None, warnings: Vec::new() };
        let content = match &at.context.content {
            Ok(content) => content,
            Err(why) => {
                out.unavailable = Some(why.clone());
                return Ok(out);
            }
        };
        let catalogue = &content.catalogue;
        let localisation = &catalogue.localisation;
        let mut sheet = Sheet::new(localisation);
        let token = params.token.as_str();
        let read = (|| -> Result<(), String> {
            let effects = catalogue.scripted_content()?.effects()?;
            let view = effects.view(None);
            let technologies = || view.doctrines.and_then(|doctrines| doctrines.technologies.as_ref()).ok_or_else(|| "The recorded technology content was not read".to_owned());
            match params.kind {
                EntityKind::Equipment => {
                    let metadata = stat_metadata()?;
                    let totals = equipment_totals(&technologies()?.equipment, metadata, token)?;
                    sheet.equipment(metadata, &totals.stats);
                    totals.withheld.into_iter().for_each(|why| sheet.refuse(why));
                }
                EntityKind::Module => {
                    let (database, metadata) = (&technologies()?.equipment, stat_metadata()?);
                    let missing = || format!("Module {token} is not in the recorded content");
                    let (stats, design) = (database.module_stats(token).ok_or_else(missing)?, database.module_design(token).ok_or_else(missing)?);
                    sheet.module(metadata, stats);
                    off_sheet(metadata, design).into_iter().for_each(|why| sheet.refuse(why));
                }
                EntityKind::Tactic => {
                    let tactics = technologies()?.tactics.as_ref().ok_or("The recorded combat tactics were not read")?.as_ref().map_err(Clone::clone)?;
                    let tactic = tactics.tactics.iter().find(|tactic| tactic.token == token.as_bytes()).ok_or_else(|| format!("Combat tactic {token} is not in the recorded content"))?;
                    if let Some(why) = &tactic.unproven {
                        return Err(why.clone());
                    }
                    sheet.tactic(tactic, &mut out.warnings);
                }
                EntityKind::SubUnit => {
                    let sheets = catalogue.content.layers.derived::<catalogue::CachedResult<SubUnitSheets>>().get_or_init(|| SubUnitSheets::load(&catalogue.content.layers)).map_err(|why| format!("Sub-unit content unavailable: {why}"))?;
                    let unit = sheets.subunit(token).ok_or_else(|| format!("Sub-unit {token} is not in the recorded content"))?;
                    sheet.sub_unit(stat_metadata()?, &unit.sheet);
                }
                EntityKind::Building => {
                    return Err("Building stats are not read from the recorded content".into());
                }
                EntityKind::Technology => {
                    return Err("A technology has no stats sheet; its effects describe it".into());
                }
            }
            Ok(())
        })();
        if let Err(why) = read {
            out.unavailable = Some(why);
        }
        out.stats = sheet.stats;
        out.refusals = sheet.refusals;
        Ok(out)
    }
}
