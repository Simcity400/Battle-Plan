
use super::{Cause, LandSource, LossDomain, LossName, LossSource};
use dashboard_base::{Source, catalogue::{self, Catalogue, View}};
use dashboard_world::products::equipment_names as names;
use dashboard_core::config::LoadContext;
use dashboard_query::Query;
use replay_data::rows::{EquipmentDestructionCause, TableId};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};

const COMBAT_LOG_TAB: &str = "combat_log_equipment_tab";
const COMBAT_LOG_TITLES: [(&str, &str); 3] = [("title_lost_in_combat", "text"), ("title_recovered", "text"), ("title_lost_by_attrition", "text")];
const NAVAL_LOSSES_WINDOW: &str = "navallosseswindow";
const NAVAL_LOSSES_TOTAL: [(&str, &str); 1] = [("total_ships", "pdx_tooltip")];
const RETENTION_UNREAD: &str = "NGame.COMBAT_LOG_MAX_MONTHS is not known from the recorded content, so whether the game deleted Combat Log entries that were still changing is unknown and no land total can be kept.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InterfaceKeys {
    pub combat: Option<String>,
    pub recovered: Option<String>,
    pub attrition: Option<String>,
    pub ships_lost: Option<String>,
}

impl InterfaceKeys {
    pub fn read(guis: &content_reader_gui::gui::native::Guis, warnings: &mut Vec<String>) -> Self {
        let mut resolve = |window: &str, texts: &[(&str, &str)]| -> Vec<Option<String>> {
            match content_reader_gui::gui::native::window_texts(guis, window, texts, warnings) {
                Ok(keys) => keys.into_iter().map(|key| key.map_err(|reason| warnings.push(format!("A loss cause is unnamed: {reason}"))).ok()).collect(),
                Err(error) => {
                    warnings.push(format!("Loss causes from {window} are unnamed: {error}"));
                    vec![None; texts.len()]
                }
            }
        };
        let mut titles = resolve(COMBAT_LOG_TAB, &COMBAT_LOG_TITLES).into_iter();
        let mut total = resolve(NAVAL_LOSSES_WINDOW, &NAVAL_LOSSES_TOTAL).into_iter();
        Self { combat: titles.next().flatten(), recovered: titles.next().flatten(), attrition: titles.next().flatten(), ships_lost: total.next().flatten() }
    }
}

fn app_label(source: LossSource) -> Option<&'static str> {
    match source {
        LossSource::SunkShips => Some("Sunk ships"),
        LossSource::SunkConvoys => Some("Sunk convoys"),
        LossSource::LendLeaseSunk => Some("Lend-lease sunk in transit"),
        LossSource::CombatLog(1) => Some("Attrition: other"),
        LossSource::CombatLog(2) => Some("Attrition: in combat"),
        LossSource::CombatLog(3) => Some("Attrition: in training"),
        LossSource::Destruction(Cause(cause)) => Some(destruction_label(cause)),
        _ => None,
    }
}

pub fn destruction_label(cause: EquipmentDestructionCause) -> &'static str {
    use EquipmentDestructionCause::*;
    match cause {
        Unknown => "Unrecognised destruction caller",
        LandCombat => "Land combat losses",
        NavalTransport => "Naval transport losses",
        Immediate => "Immediate army damage",
        Attrition => "Attrition",
        Garrison => "Resistance attacks on garrisons",
        AirNuclear => "Nuclear air losses",
        AirAccident => "Air accidents",
        AirStrategic => "Strategic air losses",
        AirMissionAa => "Air mission losses",
        AirLandCombatCas => "Land combat anti-air",
        AirRaid => "Raid air losses",
        AirNavalAa => "Naval anti-air",
        DivisionDestroyed => "Divisions destroyed in combat",
        DivisionOverrun => "Divisions overrun",
        DivisionDeleted => "Deleted divisions",
        DivisionDisbanded => "Disconnected disbands",
        DivisionCapitulation => "Divisions lost at capitulation",
        DivisionScripted => "Scripted division removals",
        DivisionTemplateRemoved => "Removed division templates",
        ParadropDisruption => "Paradrop disruption",
        DivisionMerged => "Unreturned equipment from division merges",
        ExileEquipment => "Exile equipment reductions",
        VolunteerReturnEquipment => "Volunteer return equipment reductions",
        CapitulationEscapeEquipment => "Capitulation escape equipment reductions",
        ScriptedEquipmentFraction => "Scripted equipment reductions",
        ScriptedStockpileFraction => "Scripted stockpile reductions",
    }
}

fn cause_key(source: LossSource, keys: &InterfaceKeys) -> Option<&str> {
    match source {
        LossSource::Destruction(Cause(EquipmentDestructionCause::LandCombat)) => keys.combat.as_deref(),
        LossSource::Destruction(Cause(EquipmentDestructionCause::Attrition)) => keys.attrition.as_deref(),
        LossSource::Destruction(_) => None,
        LossSource::CombatLog(0) => keys.combat.as_deref(),
        LossSource::CombatLog(1) => Some("COMBAT_LOG_EQ_LOST_ATTR_OTHER"),
        LossSource::CombatLog(2) => Some("COMBAT_LOG_EQ_LOST_ATTR_COMBAT"),
        LossSource::CombatLog(_) => Some("COMBAT_LOG_EQ_LOST_ATTR_TRAINING"),
        LossSource::Recovered => keys.recovered.as_deref(),
        LossSource::SunkShips => keys.ships_lost.as_deref(),
        LossSource::SunkConvoys => Some("LEDGER_CHART_DAMAGED_CONVOYS"),
        LossSource::LendLeaseSunk => Some("LEND_LEASE_DETAILS_EQUIPMENT_SUNK"),
        LossSource::LendLeaseSent => Some("LOGISTICS_NEED_LEND_LEASE"),
    }
}

fn name(catalogue: Option<&Catalogue>, key: Option<&str>) -> LossName {
    let (Some(catalogue), Some(key)) = (catalogue, key) else { return LossName { key: key.map(str::to_owned), ..LossName::default() } };
    let template = catalogue.localisation.get(key).and_then(|text| catalogue::plain_text(text).inspect_err(|reason| catalogue.refuse(reason, key)).ok());
    let text = template.as_ref().filter(|template| !template.contains('$')).and_then(|_| catalogue.localised(key, View::Plain));
    LossName { key: Some(key.to_owned()), template, text }
}

fn retention_unknown(months: i32) -> String {
    format!("NGame.COMBAT_LOG_MAX_MONTHS is {months}: the game deletes every Combat Log entry each day, the current month's included, so land losses written after an entry's last read are lost and no total can be kept.")
}

pub struct Content {
    pub catalogue: Option<Arc<Catalogue>>,
    pub keys: InterfaceKeys,
    pub units: Option<Arc<content_reader_key_names::equipment::Database>>,
    pub retained_months: Option<i32>,
    pub names: names::Context,
    pub warnings: Vec<String>,
}

pub(super) struct DatedDesign {
    pub first_hour: Option<i32>,
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
    pub domain: Option<LossDomain>,
}

impl Content {
    pub fn load(source: &Source, load: &LoadContext, land: LandSource) -> io::Result<Self> {
        let mut warnings = Vec::new();
        let catalogue = match load.content().and_then(|options| catalogue::catalogue(source, options)) {
            Ok(catalogue) => Some(catalogue),
            Err(error) => {
                warnings.push(format!("Cause, type and design names are unavailable: {error}"));
                None
            }
        };
        let units = catalogue.as_ref().and_then(|catalogue| catalogue::equipment_units(catalogue).map_err(|error| warnings.push(format!("Equipment types are unavailable: {error}"))).ok());
        let retained_months = (land == LandSource::CombatLog).then_some(catalogue.as_ref()).flatten().and_then(|catalogue| {
            catalogue.read_defines()
                .and_then(|defines| defines.int32("NGame", "COMBAT_LOG_MAX_MONTHS"))
                .map_err(|error| warnings.push(format!("The Combat Log's retention is unavailable: {error}")))
                .ok()
        });
        let keys = catalogue
            .as_ref()
            .and_then(|catalogue| catalogue.guis().map(|guis| InterfaceKeys::read(guis, &mut warnings)).map_err(|error| warnings.push(format!("Equipment loss interface names are unavailable: {error}"))).ok())
            .unwrap_or_default();
        let names = names::context(source, load)?;
        Ok(Self { catalogue, keys, units, retained_months, names, warnings })
    }

    pub fn cause_name(&self, source: LossSource) -> LossName {
        let mut named = name(self.catalogue.as_deref(), cause_key(source, &self.keys));
        let named_by_game = matches!(source, LossSource::Destruction(_)) && named.text.is_some();
        if let Some(label) = app_label(source).filter(|_| !named_by_game) {
            named.text = Some(label.to_owned());
        }
        named
    }

    pub fn retention_refusal(&self) -> Option<String> {
        match self.retained_months {
            Some(months) if months < 1 => Some(retention_unknown(months)),
            Some(_) => None,
            None => Some(RETENTION_UNREAD.to_owned()),
        }
    }

    pub fn domain(&self, archetype: &str) -> Option<LossDomain> {
        let units = self.units.as_ref()?;
        let category = units.category(units.named(archetype)?);
        Some(if category & engine_executable::native_values::equipment_type::PLANE_MASK != 0 {
            LossDomain::Air
        } else if category & engine_executable::native_values::equipment_type::SHIP_MASK != 0 {
            LossDomain::Naval
        } else {
            LossDomain::Land
        })
    }

    pub(super) fn designs_dated(&self, query: &Query<'_>, variants: &BTreeSet<u32>, warnings: &mut Vec<String>) -> io::Result<HashMap<u32, DatedDesign>> {
        if variants.is_empty() {
            return Ok(HashMap::new());
        }
        let recording = query.replay.recording();
        let handle = |variant: u32| u64::from(variant) << 32 | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE);
        let handles: Vec<u64> = variants.iter().map(|&variant| handle(variant)).collect();
        let keys: Vec<u128> = handles.iter().map(|&raw| u128::from(raw)).collect();
        let mut since: HashMap<u32, i32> = HashMap::new();
        for stretch in &query.timeline.stretches {
            let history = dashboard_query::cost::storage(|| {
                replay_lanes::lanes::history(recording, stretch.run, TableId::VariantDefinitions, &[0], replay_lanes::lanes::Keys::These(&keys), stretch.first, stretch.last)
            })?;
            for (key, steps) in &history.keys {
                if let Some(step) = steps.iter().find(|step| step.cells.is_some())
                    && let Some(variant) = replay_data::rows::EquipmentLossTotalRow::variant_id(*key as u64)
                {
                    let at = step.hour.max(stretch.first);
                    since.entry(variant).and_modify(|prev| *prev = (*prev).min(at)).or_insert(at);
                }
            }
        }
        let mut named = names::Named::default();
        for stretch in query.timeline.stretches.iter().rev() {
            let missing: Vec<u64> = handles.iter().copied().filter(|raw| !named.variants.contains_key(raw)).collect();
            if missing.is_empty() {
                break;
            }
            names::name_at(recording, stretch.run, stretch.last, &missing, &[], &self.names, &mut named)?;
        }
        warnings.extend(named.warnings.iter().cloned());
        let mut out = HashMap::new();
        for &variant in variants {
            let Some(found) = named.variants.get(&handle(variant)) else { continue };
            let equipment = (!found.token.is_empty()).then(|| found.token.clone());
            let archetype = equipment.as_deref().and_then(|token| {
                let units = self.units.as_ref()?;
                let ordinal = units.named(token)?.archetype?;
                units.equipment.get(ordinal)?.name_str().ok().map(str::to_owned)
            });
            let archetype_name = archetype.as_deref().and_then(|key| self.catalogue.as_ref()?.localised(key, View::Plain)).or_else(|| found.type_name.clone());
            let domain = archetype.as_deref().and_then(|key| self.domain(key));
            let name = found.known.then(|| found.name.clone()).flatten();
            let first_hour = name.is_some().then(|| since.get(&variant).copied().or(found.hour)).flatten();
            out.insert(variant, DatedDesign { first_hour, name, archetype, archetype_name, domain });
        }
        Ok(out)
    }
}
