use dashboard_effects::ScriptedEffects;
use dashboard_base::{
    Source, catalogue,
    products::{
        LoadContext,
        at_hour::{At, AtHour},
        game_text::{EntityKind, GameRun, runs},
        politics_content::{PoliticsContent, politics_content},
    },
};
use content_reader_key_names::equipment::{Amount, Database, Equipment, ModuleDesign, ResourcePool, StatComponents, StatGroup, StatValue};
use content_reader_military::combat_tactics::Tactic;
use content_reader_parser::fixed5::format_fixed5;
use content_reader_script::equipment_stats::{RELIABILITY_STAT, STAT_COUNT};
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
    #[doc = " The game's label for the stat; a plain one where the game labels it only in a state's own tooltip."]
    pub label: Vec<GameRun>,
    #[doc = " The value as the game formats it, coloured as the game colours it."]
    pub value: Vec<GameRun>,
    #[doc = " Where the value comes from or when it applies; null for the entity's own stat."]
    pub source: Option<String>,
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
    source: Option<String>,
}

impl<'l> Sheet<'l> {
    fn new(localisation: &'l Localisation) -> Self {
        Self { localisation, stats: Vec::new(), refusals: BTreeMap::new(), source: None }
    }

    fn text(&self, key: &str, args: &[(&str, Argument<'_>)]) -> Result<String, String> {
        localised(self.localisation, key, args)
    }

    fn refuse(&mut self, why: String) {
        let why = match &self.source {
            Some(source) => format!("{source}: {why}"),
            None => why,
        };
        *self.refusals.entry(why).or_default() += 1;
    }

    fn push(&mut self, label: Result<String, String>, value: Result<String, String>) {
        self.push_runs(label, value.and_then(|value| runs(&value)));
    }

    fn push_runs(&mut self, label: Result<String, String>, value: Result<Vec<GameRun>, String>) {
        let line = label.and_then(|label| Ok(EntityStat { label: runs(&label)?, value: value?, source: self.source.clone() }));
        match line {
            Ok(line) => self.stats.push(line),
            Err(why) => self.refuse(why),
        }
    }

    fn from(&mut self, source: Option<String>) -> &mut Self {
        self.source = source;
        self
    }

    fn amount(&mut self, label: Result<String, String>, amount: &Amount) {
        match amount {
            Ok(value) => self.push(label, Ok(format_fixed5(*value, true, 5))),
            Err(why) => self.refuse(why.clone()),
        }
    }

    fn costs(&mut self, manpower: Option<&Amount>, resources: &ResourcePool) {
        if let Some(manpower) = manpower {
            self.amount(self.text("DESIGNER_MANPOWER", &[]), manpower);
        }
        for (resource, amount) in resources {
            let key = format!("PRODUCTION_MATERIALS_{}", content_reader_localisation::localisation::crt_case_text(&lossy(resource), true, false));
            self.amount(self.text(&key, &[]), amount);
        }
    }

    fn stat(&mut self, metadata: &Metadata, index: usize, value: i64, formats: &[&str], suffix: Option<&str>) {
        let Some(&(_, label, _)) = metadata.get(index) else {
            return self.refuse(format!("stat {index} has no game name"));
        };
        let format = formats.iter().map(|format| format.replace("{}", label)).find(|key| self.localisation.get(key).is_some());
        let value = match format {
            Some(format) => self.text(&format, &[("VALUE", Argument::Fixed5(value))]),
            None => Err(format!("{label} has no value format")),
        };
        let value = value.and_then(|value| runs(&value)).and_then(|mut value| {
            if let Some(key) = suffix {
                value.extend(runs(&self.text(key, &[])?)?);
            }
            Ok(value)
        });
        self.push_runs(self.text(label, &[]), value);
    }

    fn equipment(&mut self, metadata: &Metadata, stats: &[i64], withheld: &BTreeMap<usize, String>) {
        for (index, &value) in stats.iter().enumerate().filter(|(index, value)| **value != 0 && !withheld.contains_key(index)) {
            self.stat(metadata, index, value, &["{}_VALUE"], None);
        }
        withheld.values().for_each(|why| self.refuse(why.clone()));
    }

    fn module(&mut self, metadata: &Metadata, stats: &StatGroup) {
        for (_, why) in &stats.unread {
            self.refuse(why.clone());
        }
        let read = |term: &&StatValue| stats.unread.iter().all(|(index, _)| *index != term.index);
        for term in stats.add.iter().filter(read) {
            self.stat(metadata, term.index, term.value, &["{}_DIFF"], None);
        }
        for term in stats.average.iter().filter(read) {
            self.stat(metadata, term.index, term.value, &["{}_DIFF"], Some("EQUIPMENT_MODULE_STAT_AVERAGED"));
        }
        for term in stats.multiply.iter().filter(read) {
            let positive = metadata.get(term.index).is_some_and(|(_, _, positive)| *positive);
            let fallback = if positive { "STAT_MODIFIER_DIFF" } else { "STAT_MODIFIER_DIFF_NEG" };
            self.stat(metadata, term.index, term.value, &["{}_MOD_DIFF", fallback], None);
        }
    }

    fn tactic(&mut self, tactic: &Tactic, warnings: &mut Vec<String>) {
        let mut attacker = false;
        for item in tactic.fields.iter().filter(|item| item.keyword_is("is_attacker")) {
            match item.value.scalar().and_then(content_reader_parser::parser::read_bool) {
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

    fn building(&mut self, building: &content_reader_key_names::buildings::BuildingType, defines: &content_reader_defines::defines::Defines) {
        let scope = if building.provincial() { "Per province" } else { "Per state" };
        let maximum = building.base_maximum(defines).map(|level| level.to_string()).map_err(|why| format!("The building's maximum level is unavailable: {why}"));
        self.from(Some(scope.into())).push(Ok("Maximum level".into()), maximum);
        self.from(None);
        if !building.state_modifiers.is_empty() {
            self.refuse(format!("The state modifiers of {} are not shown", building.name));
        }
        if !building.local_resources.is_empty() {
            self.refuse(format!("The local resources of {} are not shown", building.name));
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

fn localised(localisation: &Localisation, key: &str, args: &[(&str, Argument<'_>)]) -> Result<String, String> {
    if localisation.get(key).is_none() {
        return Err(format!("localisation {key} is not defined"));
    }
    let (text, notes) = localisation.localise_with_fallbacks(key.as_bytes(), args);
    let text = text.map_err(|why| why.to_string())?;
    if !notes.is_empty() {
        return Err(notes.join("; "));
    }
    if text.trim().is_empty() {
        return Err(format!("localisation {key} gives no text"));
    }
    Ok(text)
}

fn named(localisation: &Localisation, key: &str, token: &str) -> String {
    localised(localisation, key, &[]).unwrap_or_else(|_| format!("{token} (name unavailable)"))
}

fn missions(localisation: &Localisation, missions: &[Vec<u8>]) -> String {
    if missions.is_empty() {
        return "On missions, with no mission limit".into();
    }
    let names: Vec<String> = missions
        .iter()
        .map(|mission| {
            let token = lossy(mission);
            named(localisation, &format!("AIRWING_MISSION_TYPE_{}", content_reader_localisation::localisation::crt_case_text(&token, true, false)), &token)
        })
        .collect();
    format!("On {} missions", names.join(", "))
}

fn joined(prefix: Option<&str>, part: &str) -> String {
    prefix.map_or_else(|| part.to_owned(), |prefix| format!("{prefix} · {part}"))
}

fn module_design(sheet: &mut Sheet<'_>, metadata: &Metadata, design: &ModuleDesign, prefix: Option<&str>) {
    for (limit, group) in &design.mission_terms {
        let condition = joined(prefix, &missions(sheet.localisation, limit));
        sheet.from(Some(condition)).module(metadata, group);
    }
    sheet.from(prefix.map(str::to_owned)).costs(design.manpower.as_ref(), &Vec::new());
    sheet.from(Some(joined(prefix, "Production resources"))).costs(None, &design.build_resources);
    sheet.from(None);
}

struct Totals<'d> {
    stats: [i64; STAT_COUNT],
    withheld: BTreeMap<usize, String>,
    modules: Vec<(String, &'d ModuleDesign)>,
    lineage: Vec<&'d Equipment>,
}

fn equipment_totals<'d>(database: &'d Database, metadata: &Metadata, localisation: &Localisation, token: &str) -> Result<Totals<'d>, String> {
    let at = database.get(token).ok_or_else(|| format!("Equipment {token} is not in the recorded content"))?;
    let equipment = &database.equipment[at];
    let mut components = StatComponents::base(&equipment.stats);
    let mut withheld: BTreeMap<usize, String> = BTreeMap::new();
    for (index, why) in &equipment.unread_terms {
        withheld.entry(*index).or_insert_with(|| why.clone());
    }
    let mut modules = Vec::new();
    for (slot, module) in database.default_modules(equipment)? {
        let (shown, slot) = (lossy(module), lossy(slot));
        let design = database.module_design(module).ok_or_else(|| format!("Default module {shown} in slot {slot} of equipment {token} is not in the recorded content, so its stats are withheld"))?;
        let source = format!("Default module {} in {}", named(localisation, &shown, &shown), named(localisation, &format!("EQ_MOD_SLOT_{slot}_TITLE"), &slot));
        if let Some(group) = database.module_stats(module) {
            components.apply_module(group, equipment)?;
            for (index, why) in group.unread.iter().map(|(index, why)| (*index, why.as_str())).chain(equipment.unread_multipliers_for(group)) {
                withheld.entry(index).or_insert_with(|| format!("{source}: {why}"));
            }
        }
        modules.push((source, design));
    }
    if let Some(why) = withheld.get(&RELIABILITY_STAT).cloned() {
        for term in &equipment.scaled_by_reliability {
            let stat = metadata.get(term.index).map_or_else(|| format!("stat {}", term.index), |(name, _, _)| (*name).to_owned());
            withheld.entry(term.index).or_insert_with(|| format!("{stat} scales with reliability, which is withheld: {why}"));
        }
    }
    Ok(Totals { stats: components.finish_for(equipment)?, withheld, modules, lineage: database.lineage(equipment)? })
}

fn equipment_sheet(sheet: &mut Sheet<'_>, metadata: &Metadata, totals: &Totals<'_>) {
    sheet.equipment(metadata, &totals.stats, &totals.withheld);
    for (at, held) in totals.lineage.iter().enumerate() {
        let name = lossy(&held.name);
        let set_by = (at != 0).then(|| format!("Set by {}", named(sheet.localisation, &name, &name)));
        sheet.from(set_by.clone()).costs(held.manpower.as_ref(), &Vec::new());
        sheet.from(Some(joined(set_by.as_deref(), "Production resources"))).costs(None, &held.resources);
        for keyword in &held.off_sheet {
            sheet.from(None).refuse(format!("{} of {name} is not shown", lossy(keyword)));
        }
    }
    for (source, design) in &totals.modules {
        module_design(sheet, metadata, design, Some(source));
    }
    sheet.from(None);
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
            let equipment = || view.equipment.ok_or_else(|| "The recorded equipment content was not read".to_owned());
            match params.kind {
                EntityKind::Equipment => {
                    let metadata = stat_metadata()?;
                    let totals = equipment_totals(equipment()?, metadata, localisation, token)?;
                    equipment_sheet(&mut sheet, metadata, &totals);
                }
                EntityKind::Module => {
                    let (database, metadata) = (equipment()?, stat_metadata()?);
                    let missing = || format!("Module {token} is not in the recorded content");
                    let (stats, design) = (database.module_stats(token).ok_or_else(missing)?, database.module_design(token).ok_or_else(missing)?);
                    sheet.module(metadata, stats);
                    module_design(&mut sheet, metadata, design, None);
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
                    let buildings = view.buildings.ok_or("The recorded building content was not read")?;
                    let building = buildings.by_token(token).map_err(|why| why.to_string())?.ok_or_else(|| format!("Building {token} is not in the recorded content"))?;
                    sheet.building(building, catalogue.defines()?);
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
