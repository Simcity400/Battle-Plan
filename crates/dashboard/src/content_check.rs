
use dashboard_effects::ScriptedEffects;
use dashboard_base::catalogue::Catalogue;
use crate::products;
use dashboard_core::findings::{Finding, Kind};
use content_reader_layers::content::ResolvedContent;
use content_reader_layers::content::diagnostics::Once as OnceLock;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct ProductCheck {
    pub key: &'static str,
    pub elapsed: Duration,
    pub error: Option<String>,
    pub findings: Vec<Finding>,
    pub note: String,
}

struct Content {
    resolved: Arc<ResolvedContent>,
    language: String,
    catalogue: OnceLock<Result<Arc<Catalogue>, String>>,
    tags: OnceLock<
        Result<
            std::collections::BTreeMap<String, content_reader_key_names::country_tags::CountryTag>,
            String,
        >,
    >,
    ideologies: OnceLock<Result<content_reader_key_names::ideologies::Ideologies, String>>,
}

impl Content {
    fn new(resolved: Arc<ResolvedContent>, language: &str) -> Self {
        Self {
            resolved,
            language: language.to_owned(),
            catalogue: OnceLock::new(),
            tags: OnceLock::new(),
            ideologies: OnceLock::new(),
        }
    }
    fn tags(
        &self,
    ) -> Result<&std::collections::BTreeMap<String, content_reader_key_names::country_tags::CountryTag>, String>
    {
        self.tags
            .get_or_init(|| {
                content_reader_key_names::country_tags::read_definitions(&self.resolved.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn ideologies(&self) -> Result<&content_reader_key_names::ideologies::Ideologies, String> {
        self.ideologies
            .get_or_init(|| {
                content_reader_key_names::ideologies::read(&self.resolved.layers)
                    .map(|(ideologies, _)| ideologies)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn catalogue(&self) -> Result<&Arc<Catalogue>, String> {
        self.catalogue
            .get_or_init(|| {
                Catalogue::of_layers_for_check(
                    self.resolved.layers.clone(),
                    &self.language,
                    dashboard_effects::effect_describer,
                )
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

pub fn check(content: Arc<ResolvedContent>, language: &str) -> Vec<ProductCheck> {
    let content = Content::new(content, language);
    let checks: Vec<_> = products::PRODUCTS
        .iter()
        .map(|product| {
            let started = Instant::now();
            let mut findings = Vec::new();
            let result = run(product.key, &content, &mut findings);
            if let Some(Ok(catalogue)) = content.catalogue.get() {
                findings.extend(catalogue.take_findings());
            }
            let mut seen = std::collections::BTreeSet::new();
            findings.retain(|finding| seen.insert(finding.clone()));
            aggregate(&mut findings);
            ProductCheck {
                key: product.key,
                elapsed: started.elapsed(),
                error: result.as_ref().err().cloned(),
                findings,
                note: result.unwrap_or_default(),
            }
        })
        .collect();
    checks
}

fn aggregate(findings: &mut Vec<Finding>) {
    use std::collections::{BTreeMap, BTreeSet};
    let mut reasons = BTreeMap::<(Kind, String), BTreeMap<String, BTreeSet<String>>>::new();
    findings.retain(|finding| {
        let reason = match finding.aggregate_reason() {
            Some(reason) => reason.to_owned(),
            None if finding.kind == Kind::Offline => finding.message.clone(),
            None => return true,
        };
        let example = if finding.message == reason || finding.message == finding.subject {
            finding.subject.clone()
        } else {
            format!("{}: {}", finding.subject, finding.message)
        };
        reasons
            .entry((finding.kind, reason))
            .or_default()
            .entry(finding.subject.clone())
            .or_default()
            .insert(example);
        false
    });
    findings.extend(reasons.into_iter().map(|((kind, reason), subjects)| {
        let mut finding = Finding::new(
            kind,
            if kind == Kind::Offline {
                "offline coverage"
            } else {
                "product coverage"
            },
            reason,
        );
        finding.count = Some(subjects.len());
        finding.examples = subjects.into_values().flatten().collect();
        finding
    }));
}

fn run(key: &str, content: &Content, findings: &mut Vec<Finding>) -> Result<String, String> {
    use dashboard_base::products::{Product, artwork, at_hour::{self, AtHour}};
    use dashboard_world::products::{country_flags, identities, subjects};
    use dashboard_land::products::{army, army_losses, divisions, doctrine_mastery, doctrines, forces, leaders, training_queue};
    use dashboard_naval::products::naval;
    use dashboard_wars::products::{casualties, casualty_matrix, diplomacy, faction_state, occupation, peace, scoreboard, territory_transfers, volunteers, wargoals, wars};
    use dashboard_politics::products::{feed, national_focus, stability, technology_artwork};
    use dashboard_economy::products::{convoys, economy, trade_flow};
    use dashboard_production::products::{construction, equipment_losses, equipment_produced, production, production_spending, production_spending_months};
    use dashboard_air::products::aircraft_catalogue;
    use dashboard_players::products::{campaign_clock, player_apm};
    use dashboard_research::products::research;
    use dashboard_intelligence::products::intelligence;
    match key {
        campaign_clock::CampaignClock::KEY => Ok("Campaign clock totals require recorded real-time intervals, pause-state coverage and each session's clock frequency; no authored content is read. Recording-dependent values are unavailable offline.".into()),
        player_apm::PlayerApm::KEY => Ok("eAPM requires recorded accepted-command kinds, player lifetimes, provenance and unpaused timing coverage; no authored content is read. Recording-dependent values are unavailable offline.".into()),
        national_focus::NationalFocus::KEY => {
            let national = artwork_check(
                content,
                artwork::Kind::Focus(dashboard_core::dto::FocusDatabase::National),
                findings,
            )?;
            let continuous = artwork_check(
                content,
                artwork::Kind::Focus(dashboard_core::dto::FocusDatabase::Continuous),
                findings,
            )?;
            Ok(format!("{national} {continuous}"))
        }
        technology_artwork::TechnologyArtwork::KEY => {
            artwork_check(content, artwork::Kind::Technology, findings)
        }
        research::Research::KEY | research::breakdown::ResearchBreakdown::KEY | <research::slots::ResearchSlots as AtHour>::KEY => {
            let names = artwork_check(content, artwork::Kind::Technology, findings)?;
            Ok(format!("{names} Slots, assignments, progress, modifiers and grants require the recorded research tables."))
        }
        doctrines::Doctrines::KEY | <doctrine_mastery::DoctrineMastery as AtHour>::KEY | <doctrine_mastery::rewards::DoctrineRewards as AtHour>::KEY => {
            let database = content_reader_military::doctrines::read(&content.resolved.layers).map_err(|error| error.to_string())?;
            let catalogue = content.catalogue()?;
            for folder in database.entries.values() {
                if let Err(why) = dashboard_base::catalogue::localised_bytes(&catalogue.localisation, &folder.name, dashboard_base::catalogue::View::Plain) { findings.push(Finding::new(Kind::Refused, "doctrine folder name", why)); }
            }
            for name in database.grand.values().chain(database.sub.values()).chain(database.tracks.values()) {
                if let Err(why) = dashboard_base::catalogue::localised_bytes(&catalogue.localisation, &name.value, dashboard_base::catalogue::View::Plain) { findings.push(Finding::new(Kind::Refused, "doctrine name", why)); }
            }
            findings.extend(database.notes.into_iter().map(|note| Finding::new(Kind::Refused, "doctrine content", note)));
            findings.extend(catalogue.take_findings());
            Ok("Doctrine names, authored costs, rewards, unit-stat and modifier forms checked through the recorded content stack; selections, completions, mastery, commander contributions and effective costs require recorded doctrine state and modifier inputs.".into())
        }
        country_flags::CountryFlags::KEY => {
            flags_check(content, findings)?;
            Ok("Authored static-country flags, ideology-group candidates and shield checked; selected flags and shield colours require recorded country appearance, ruling parties and country identities.".into())
        }
        aircraft_catalogue::AircraftCatalogue::KEY => {
            let read = aircraft_catalogue::Content::read(content.catalogue()?)
                .map_err(|why| why.to_string())?;
            let listing = read.catalogue(&[]);
            findings.extend(
                read.warnings
                    .iter()
                    .chain(&listing.diagnostics)
                    .map(|warning| Finding::new(Kind::Warning, "aircraft catalogue", warning.clone())),
            );
            let refused = content_reader_equipment_graphics::equipment_graphics::aircraft::Catalogue::refusal_counts(&listing.entries);
            for (kind, count) in refused {
                let mut finding = Finding::new(Kind::Refused, "aircraft catalogue", kind.label());
                finding.count = Some(count);
                findings.push(finding);
            }
            Ok(format!(
                "{} aircraft names among {} entities and meshes, from the registered descriptor roster, equipment categories and graphics pools; models recorded designs saved require recorded VariantDefinitions and LexicalTokens.",
                listing.entries.iter().filter(|entry| entry.aircraft()).count(),
                listing.entries.len()
            ))
        }
        artwork::sprites::SpriteArtwork::KEY => {
            content.catalogue()?;
            Ok("Sprite declarations and texture decode settings load through the shared catalogue; the keys come from the products that return them, such as doctrine mastery, and are drawn on request.".into())
        }
        army::ArmySize::KEY | <army::ArmyAt as at_hour::AtHour>::KEY => {
            findings.extend(army::check_content(content.catalogue()?).map_err(|why| why.to_string())?);
            Ok("Authored sub-unit groups checked through the army composition; totals, template grids, country identities and supplying dates require the recorded army tables.".into())
        }
        identities::CountryIdentities::KEY => Ok("No content dependency: holders are the recorded identity table.".into()),
        "game-score" => Ok("Scores, their reasons and the multipliers are the recorded CountryGameScores and GameScoreMultipliers rows; only the column hints localise the end screen's sort controls from the recorded content.".into()),
        "factory-groups" | "factory-mix" => Ok("Factory group names are the recorded stack's building types counted by each production flag, localised; the counts are the recorded CountryFactories rows.".into()),
        "factory-parts" => Ok("No content dependency: the three factory groups are the game's own, read from the recorded CountryFactories row.".into()),
        "faction-totals-at" => Ok("No content dependency: faction and group totals sum recorded TerritoryTotals or CountryFactories rows.".into()),
        "world-tension" | "world-tension-stack" => Ok("No content dependency: tension, contributions and the game's own threat text are the recorded WorldTension, ThreatCountryAggregates, ThreatSources and ThreatEvents rows; display text only loses its presentation controls.".into()),
        "nuclear-arsenal" => Ok("Nuclear type names localise the token spellings recorded in LexicalTokens; amounts and type ordinals are the recorded Nukes rows. A type the game names no token for is counted and shown unnamed.".into()),
        <forces::AirForceAt as at_hour::AtHour>::KEY => {
            let units = dashboard_base::catalogue::equipment_units(content.catalogue()?)?;
            Ok(format!("Air parts group recorded variants under their equipment's archetype through the equipment database ({} types); totals and parts require the recorded air force tables.", units.equipment.len()))
        }
        <forces::NavyAt as at_hour::AtHour>::KEY => {
            content_reader_key_names::unit_leader::sub_units(&content.catalogue()?.content.layers).map_err(|why| why.to_string())?;
            Ok("Ship classes and roles resolve recorded class tokens through the sub-unit database; totals and classes require the recorded navy tables.".into())
        }
        <volunteers::VolunteerFlow as at_hour::AtHour>::KEY => Ok("No content dependency: links are the recorded military aid table and the identity table.".into()),
        production_spending::ProductionSpending::KEY | production_spending_months::ProductionSpendingMonths::KEY => {
            let units = dashboard_base::catalogue::equipment_units(content.catalogue()?)?;
            Ok(format!("Equipment types resolve each design's recorded archetype through the equipment database ({} types); design names localise the recorded naming operands; amounts and repair rates are the recorded production tables.", units.equipment.len()))
        }
        <training_queue::TrainingQueue as at_hour::AtHour>::KEY => {
            content.catalogue()?;
            Ok("Deployment orders, divisions in training, manpower and equipment come from the recorded daily queue read; template, priority, location and archetype names resolve through the recorded naming operands and the content stack.".into())
        }
        <divisions::careers::DivisionCareers as at_hour::AtHour>::KEY | <divisions::careers::DecoratedDivisions as at_hour::AtHour>::KEY | <divisions::careers::DivisionRecord as at_hour::AtHour>::KEY => {
            content.catalogue()?;
            Ok("Division histories, medal totals, experience and kill counters come from recorded rows; medal, level, entry and place names resolve through recorded lexical spellings and the content stack's localisation and defines.".into())
        }
        <divisions::templates::DivisionTemplates as at_hour::AtHour>::KEY => {
            content.catalogue()?;
            Ok("Template rosters, settings, grids and base widths come from recorded rows and fielded counts from the division equipment tables; template, sub-unit and artwork names resolve through recorded naming operands and the content stack.".into())
        }
        <divisions::equipment::DivisionEquipment as at_hour::AtHour>::KEY => {
            divisions::equipment::template_needs(&content.catalogue().cloned())?;
            Ok("A country's division equipment is its rows of the derived division equipment tables; a drill compares recorded pools with each template's need from the content stack's sub-unit requirements and recorded local clones. Division, template, archetype and design names share the recorded content stack.".into())
        }
        subjects::Subjects::KEY => {
            let catalogue = content.catalogue()?;
            let (states, _) = content_reader_politics::autonomy::read(&content.resolved.layers).map_err(|error| error.to_string())?;
            let named = states.states.iter().map(|state| state.id.as_str()).collect();
            subjects::levels(&named, Some(&states), Some(catalogue));
            take(catalogue, "subject level names", findings);
            Ok("Every authored autonomy level checked; relations and current levels require a recording.".into())
        }
        wargoals::Wargoals::KEY => wargoals_check(content, findings),
        <intelligence::roster::OperativesRoster as at_hour::AtHour>::KEY => {
            let catalogue = content.catalogue()?;
            let politics = std::sync::Arc::new(dashboard_base::products::politics_content::PoliticsContent::load(std::sync::Arc::clone(catalogue)).map_err(|error| error.to_string())?);
            let operatives = intelligence::operative_content::OperativeContent::load(politics).map_err(|error| error.to_string())?;
            let refused = operatives.check();
            for (subject, why) in &refused {
                findings.push(Finding::new(Kind::Refused, subject, why));
            }
            take(catalogue, "operative names and icons", findings);
            Ok(format!("Every mission, cooldown and operation name and icon checked ({} refused); operatives, portraits and skill levels need a recording.", refused.len()))
        }
        <intelligence::agencies::AgencyComparison as at_hour::AtHour>::KEY => {
            let catalogue = content.catalogue()?;
            let agency = intelligence::agency_content::AgencyContent::load(std::sync::Arc::clone(catalogue));
            let refused = agency.check();
            for (subject, why) in &refused {
                findings.push(Finding::new(Kind::Refused, subject, why));
            }
            take(catalogue, "agency names, defence and upgrades", findings);
            Ok(format!("The default agency name, the defence line and every acquired upgrade's name checked ({} refused); agencies and their levels need a recording.", refused.len()))
        }
        intelligence::espionage::EspionageTypes::KEY => {
            let catalogue = content.catalogue()?;
            let politics = std::sync::Arc::new(dashboard_base::products::politics_content::PoliticsContent::load(std::sync::Arc::clone(catalogue)).map_err(|error| error.to_string())?);
            let operatives = intelligence::operative_content::OperativeContent::load(politics).map_err(|error| error.to_string())?;
            let mut refused = 0usize;
            for token in operatives.operation_tokens() {
                let (name, icon) = operatives.operation_type(token);
                for why in [name.err(), icon.and_then(|art| operatives.draw(&art)).err()].into_iter().flatten() {
                    findings.push(Finding::new(Kind::Refused, &format!("operation {token}"), &why));
                    refused += 1;
                }
            }
            take(catalogue, "operation type names and icons", findings);
            Ok(format!("Every operation type's name and icon checked ({refused} refused); which operations ran, and their tokens, need a recording."))
        }
        diplomacy::DiplomaticPacts::KEY => {
            let catalogue = content.catalogue()?;
            take(catalogue, "relation titles", findings);
            Ok("Relation titles resolve through the recorded content stack by each recorded relation type; which relations a country held requires a recording.".into())
        }
        key if key == <faction_state::details::FactionDetails as at_hour::AtHour>::KEY
            || key == <faction_state::country_goals::CountryFactionGoals as at_hour::AtHour>::KEY
            || key == faction_state::history::FactionState::KEY =>
        {
            let layers = &content.resolved.layers;
            let goals = content_reader_politics::faction_goals::cached(layers)?;
            let upgrades = content_reader_politics::faction_upgrades::cached(layers)?;
            for warning in goals.warnings.iter().chain(&upgrades.warnings) {
                findings.push(Finding::new(Kind::Warning, key, warning));
            }
            findings.push(Finding::new(Kind::Offline, key, "the goals, selections, pools and members a faction holds require recorded faction rows"));
            Ok("Faction goal and member-upgrade content loaded through the production readers; goal rewards and scripted names are described at a recorded hour.".into())
        }
        wars::Wars::KEY => {
            let catalogue = content.catalogue()?;
            catalogue.war_names()?;
            take(catalogue, "native war naming content", findings);
            Ok("Native war naming content loaded through the recorded stack; viewpoint groups, first-goal tokens, naming operands and continuity require recorded rows.".into())
        }
        wars::WarLosses::KEY => Ok("No content dependency: casualties are the recorded CasualtyLifetimes counters of the relations each war held.".into()),
        <wars::contributions::WarContributions as at_hour::AtHour>::KEY => {
            content.catalogue()?.war_participation_factors()?;
            findings.push(Finding::new(Kind::Offline, "war-contributions", "participation ledgers and delivery histories need recorded WarParticipation and WarLendLeaseHistories rows; unavailable offline"));
            Ok("Lend-lease participation factors read from the recorded stack's defines.".into())
        }
        key if key == <economy::fuel_branches::FuelBranches as at_hour::AtHour>::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "consumer requests, receipts and priorities need recorded CountryFuelConsumers rows; unavailable offline"));
            Ok("Consumer labels are the fuel panel's own localisation keys, resolved through the recorded content stack.".into())
        }
        key if key == <economy::resources::ResourceBalance as at_hour::AtHour>::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "pools and trade deals need recorded CountryResources and ResourceExchanges rows; unavailable offline"));
            Ok("Resource names and icons are read through the shared resource naming of the recorded content stack.".into())
        }
        key if key == <economy::lend_lease_balance::LendLeaseBalance as at_hour::AtHour>::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "cumulative lend-lease histories need recorded WarLendLeaseHistories rows; unavailable offline"));
            Ok("No content dependency: countries are named by their recorded tags.".into())
        }
        economy::resources::ResourceSizes::KEY => Ok("No content dependency: bar lengths are computed from the recorded pools.".into()),
        key if key == <economy::fuel::CountryFuel as at_hour::AtHour>::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "the tank, its days and current consumption need recorded CountryFuel, FuelDays and FuelDemandTotals rows; unavailable offline"));
            Ok("Consumer and source labels are the fuel panel's own localisation keys, resolved through the recorded content stack.".into())
        }
        economy::fuel::awards::CountryFuelAwards::KEY | economy::fuel::demand::CountryFuelDemand::KEY => {
            Ok("Recipient, unit, army and fleet names are the game's stored names at the read, shown through the recorded content stack; unavailable offline.".into())
        }
        economy::fuel::awards::CountryFuelAwardHistory::KEY | economy::fuel::demand::CountryFuelDemandHistory::KEY => {
            Ok("No content dependency: receipts and rates are recorded amounts.".into())
        }
        production::ProductionLinesAt::KEY | production::ProductionLineBreakdown::KEY => {
            Ok("Variant and organisation names and icons, and modifier source labels, need the recorded variant, country, organisation and modifier operands; unavailable offline.".into())
        }
        production::ProductionLicences::KEY | production::IndustrialOrganisations::KEY | production::EquipmentDesigns::KEY => {
            Ok("Design, organisation and policy names and icons need the recorded variant, organisation and country operands; unavailable offline.".into())
        }
        <leaders::UnitLeaders as at_hour::AtHour>::KEY | <leaders::career::LeaderHour as at_hour::AtHour>::KEY | leaders::career::LeaderCareer::KEY | leaders::career::LeaderChoices::KEY => {
            let catalogue = content.catalogue()?;
            let read = content_reader_key_names::unit_leader::Leaders::<String>::load(&catalogue.content.layers, &|_| None).map_err(|why| why.to_string())?;
            findings.extend(read.misreads.iter().map(|message| Finding::new(Kind::Misread, "unit leaders", message)));
            findings.extend(read.skills.warnings().iter().map(|message| Finding::new(Kind::Refused, "leader skill definitions", message)));
            findings.extend(read.skills.conditions().iter().map(|condition| Finding::new(Kind::Offline, "leader skill definitions", format!("Maximum skill needs recorded modifier definitions: {condition}"))));
            for effects in read.traits.iter() {
                findings.extend(effects.unsupported.iter().map(|reason| Finding::new(Kind::Refused, &effects.name, reason)));
                findings.extend(effects.conditions.iter().map(|condition| Finding::new(Kind::Offline, &effects.name, format!("Trait conditions need recorded modifier definitions: {condition}"))));
            }
            Ok(format!("Ranks, skill names, trait names and icons and skill ceilings resolve through the content stack ({} traits); names, owners, visibility and battles need the recorded leader, character and combat tables.", read.traits.iter().count()))
        }
        leaders::LeaderPortraits::KEY => Ok("Portraits need each character's recorded portrait holder; unavailable offline.".into()),
        <stability::CountryStabilityBreakdown as at_hour::AtHour>::KEY | <stability::StabilityPartNames as at_hour::AtHour>::KEY => {
            Ok("No offline check: idea, advisor, leader, ideology and modifier names are read at the reply from the recorded lines and parts dictionary, in the country's look at the hour.".into())
        }
        casualties::CasualtyBreakdown::KEY | <casualty_matrix::CasualtyMatrix as at_hour::AtHour>::KEY => Ok("No content dependency: pairs and losses are the recorded casualty lifetimes and totals, named by the identity table.".into()),
        peace::PeaceDeals::KEY | territory_transfers::TerritoryTransfers::KEY => {
            let catalogue = content.catalogue()?;
            if catalogue.localised("PEACE_TAKE_STATES_DEMAND", dashboard_base::catalogue::View::Scripted).is_none() {
                findings.push(Finding::new(Kind::Refused, "peace action names", "localisation PEACE_TAKE_STATES_DEMAND is missing; state transfers are unnamed"));
            }
            take(catalogue, "peace action names", findings);
            Ok("The state-taking action's name checked; other action names come from the recorded settlements' keys and cached names.".into())
        }
        dashboard_world::products::sea_regions::StrategicRegions::KEY => {
            let regions = dashboard_base::content::strategic_regions(&content.resolved.layers).map_err(|why| why.to_string())?;
            for note in &regions.notes {
                findings.push(Finding::new(Kind::Warning, "strategic regions", note.clone()));
            }
            Ok(format!("{} strategic regions checked through the recorded content stack.", regions.regions.len()))
        }
        convoys::ConvoyHealth::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "regional convoy efficiency, resource deals and lend-lease convoy clients need recorded rows; unavailable offline"));
            let regions = dashboard_base::content::strategic_regions(&content.resolved.layers).map_err(|why| why.to_string())?;
            for note in &regions.notes {
                findings.push(Finding::new(Kind::Warning, "strategic regions", note.clone()));
            }
            Ok("Sea region names read the recorded stack's strategic regions; resource names use the shared resource naming.".into())
        }
        <trade_flow::TradeFlow as at_hour::AtHour>::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "seller and buyer flows and their quantities need the recorded TradeFlows rows; unavailable offline"));
            Ok("Resource names and icons are read through the shared resource naming of the recorded content stack.".into())
        }
        key if key == <economy::market::MarketLendLease as AtHour>::KEY
            || key == economy::market::ledger::MarketLedger::KEY
            || key == economy::market::ledger::MarketLedgerTail::KEY =>
        {
            findings.push(Finding::new(Kind::Offline, key, "market listings, contracts, agreements and occurrences need the recorded market tables; unavailable offline"));
            Ok("Equipment names and icons need the recorded variant operands through the content stack; subsidy archetypes are localised from it.".into())
        }
        construction::log::ConstructionLogManifest::KEY | construction::log::ConstructionLog::KEY | construction::breakdown::ConstructionBreakdown::KEY => {
            content.catalogue()?;
            Ok("Building type names localise the token spellings recorded in LexicalTokens through the recorded buildings; icons and the repair marker are the construction queue's line entry art; state names are the recorded states' name keys. Lines, totals and endings are the recorded ConstructionProjects and ConstructionSpend rows.".into())
        }
        naval::battles::NavalBattlesManifest::KEY | naval::battles::NavalBattles::KEY | naval::battles::NavalBattlesRanking::KEY => {
            Ok("No content dependency: battles, sides and results are the recorded naval combat, result and battle tables; places are named by the place names product.".into())
        }
        naval::losses::NavalLosses::KEY => Ok("No content dependency: counts are the recorded naval loss totals.".into()),
        naval::sunk::ShipsSunk::KEY | naval::sunk::ShipsSunkManifest::KEY | naval::battles::NavalBattleDetails::KEY => {
            let catalogue = content.catalogue()?;
            content_reader_key_names::unit_leader::sub_units(&catalogue.content.layers).map_err(|why| why.to_string())?;
            dashboard_base::content::strategic_regions(&catalogue.content.layers).map_err(|why| why.to_string())?;
            Ok("Ship classes resolve recorded class tokens through the sub-unit database, sea zones through the strategic regions; ship, cause and fleet names are the text the game stored, shown in its recorded language.".into())
        }
        feed::EventFeedTail::KEY
        | feed::EventChoicesTail::KEY
        | feed::DecisionsTail::KEY
        | feed::EventFeedSpan::KEY => {
            let catalogue = content.catalogue()?;
            catalogue.scripted_content()?.effects()?;
            take(catalogue, "event feed text", findings);
            Ok("Event and decision titles, bodies, options, effects and pictures resolve through the recorded content stack for each occurrence; who, when and the scopes they ran in require the recorded feed log.".into())
        }
        <occupation::states::OccupiedStates as at_hour::AtHour>::KEY => {
            let catalogue = content.catalogue()?;
            findings.extend(occupation::check_laws(catalogue)?.into_iter().map(|message| Finding::new(Kind::Refused, "occupation laws", message)));
            take(catalogue, "occupation laws", findings);
            Ok("Authored occupation-law names and icons checked; occupied state names, garrisons and law assignments require recorded state identities.".into())
        }
        <occupation::detail::OccupationDetail as at_hour::AtHour>::KEY => Ok("No content dependency: occupier totals and collaboration are recorded tables.".into()),
        key if key == dashboard_production::products::stockpile::EquipmentStockpileCatalogue::KEY
            || key == dashboard_production::products::stockpile::EquipmentStockpileChart::KEY
            || key == <dashboard_production::products::stockpile::EquipmentStockpileAt as dashboard_base::products::at_hour::AtHour>::KEY
            || key == <dashboard_production::products::stockpile::StockpileArtwork as dashboard_base::products::at_hour::AtHour>::KEY =>
        {
            Ok("Design names, equipment types and pictures need the recorded design, country and best-design candidate operands; unavailable offline.".into())
        }
        equipment_losses::EquipmentLosses::KEY => {
            let catalogue = content.catalogue()?;
            let mut warnings = Vec::new();
            catalogue.guis().map(|guis| equipment_losses::content::InterfaceKeys::read(guis, &mut warnings)).map_err(|error| error.to_string())?;
            findings.extend(warnings.into_iter().map(|message| Finding::new(Kind::Warning, "equipment loss interface", message)));
            Ok("Cause names resolve through the recorded content's interface keys and localisation; equipment types and arms through its equipment; totals, ships and coverage are the recorded equipment-loss tables, unavailable offline.".into())
        }
        key if key == <equipment_losses::EquipmentLossHead as at_hour::AtHour>::KEY => Ok("No content dependency: each leaf's total is a recorded EquipmentLossTotals cell.".into()),
        key if key == <army_losses::ArmyLosses as at_hour::AtHour>::KEY => {
            findings.push(Finding::new(Kind::Offline, key, "Combat Log months, armies and amounts need the recorded Combat Log tables; unavailable offline"));
            Ok("Army names are the recorded stored texts localised through the content stack; designs and their types resolve through the recorded definitions and the content's equipment.".into())
        }
        scoreboard::Scoreboard::KEY => Ok("No content dependency: each cell is a declared measure's recorded value.".into()),
        scoreboard::ScoreboardColumns::KEY => {
            let catalogue = content.catalogue()?;
            for key in scoreboard::localised_keys() {
                if catalogue.localised(key, dashboard_base::catalogue::View::Plain).is_none() {
                    findings.push(Finding::new(Kind::Fallback, "scoreboard headings", format!("localisation {key} is missing; the plain heading is shown")));
                }
            }
            take(catalogue, "scoreboard headings", findings);
            Ok("The game's own column headings checked through the recorded content stack.".into())
        }
        equipment_produced::EquipmentProduced::KEY | equipment_produced::EquipmentProducedTypes::KEY | equipment_produced::EquipmentProducedCountry::KEY | equipment_produced::EquipmentProducedAt::KEY | equipment_produced::EquipmentProducedRace::KEY => {
            let units = dashboard_base::catalogue::equipment_units(content.catalogue()?)?;
            Ok(format!("Equipment types resolve the recorded archetype spellings through the equipment database ({} types); design names localise the recorded naming operands; amounts are the recorded production totals and Rocket output.", units.equipment.len()))
        }
        "raid-tally-at" => Ok("Raid availability comes from the recorded content stack's raid types; launch and success counts are the recorded Raids and RaidEnds rows.".into()),
        _ => Err("No content check is registered for this product.".into()),
    }
}

fn wargoals_check(content: &Content, findings: &mut Vec<Finding>) -> Result<String, String> {
    let catalogue = content.catalogue()?;
    let held = catalogue
        .content
        .layers
        .derived::<OnceLock<Result<content_reader_wargoals::wargoals::Database, String>>>();
    let definitions = held
        .get_or_init(|| {
            content_reader_wargoals::wargoals::read(&catalogue.content.layers)
                .map_err(|why| why.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    findings.extend(
        definitions
            .notes
            .iter()
            .map(|note| Finding::new(Kind::Warning, "war-goal definitions", note)),
    );
    let mut seen = std::collections::BTreeSet::new();
    for name in &definitions.names {
        if !seen.insert(content_reader_parser::tokens::Token::c_str(name)) {
            continue;
        }
        let key = String::from_utf8_lossy(name);
        let subject = format!("war-goal type {key}");
        if let Err(why) = definitions.terms(name) {
            findings.push(Finding::new(Kind::Refused, &subject, why));
        }
        if std::str::from_utf8(name).is_err() {
            findings.push(Finding::new(
                Kind::Refused,
                subject,
                "War-goal definition name is not UTF-8",
            ));
            continue;
        }
        if !name.is_empty() && catalogue.localisation.get(&key).is_none() {
            findings.push(Finding::new(
                Kind::Fallback,
                subject,
                format!("localisation {key} is missing; the key is shown"),
            ));
        } else if !catalogue
            .localisation
            .requires_arguments(name, &["ACTOR", "TARGET"])
        {
            catalogue.localised(&key, dashboard_base::catalogue::View::Plain);
            take(catalogue, &subject, findings);
        }
    }
    Ok("Authored war-goal definitions, term operands and scope-independent name rendering checked through the runtime product's shared content cache; ACTOR/TARGET names, allowed/available descriptions, conditional peace terms and justify/declare eligibility require recorded country scope and inventory operands.".into())
}


fn artwork_check(
    content: &Content,
    kind: dashboard_base::products::artwork::Kind,
    findings: &mut Vec<Finding>,
) -> Result<String, String> {
    let catalogue = content.catalogue()?;
    let context = dashboard_base::products::artwork::Context::load(Arc::clone(catalogue), kind)
        .map_err(|error| error.to_string())?;
    let (tokens, _) = context.tokens();
    let total = tokens.len();
    let mut without_description = 0;
    for token in tokens {
        let subject = format!(
            "{} {token}",
            match kind {
                dashboard_base::products::artwork::Kind::Focus(dashboard_core::dto::FocusDatabase::National) => "focus",
                dashboard_base::products::artwork::Kind::Focus(dashboard_core::dto::FocusDatabase::Continuous) => {
                    "continuous focus"
                }
                dashboard_base::products::artwork::Kind::Technology => "technology",
            }
        );
        let (_, problems) = context.prepare(token);
        without_description += usize::from(!context.description_is_defined(token));
        findings.extend(problems.into_iter().map(|mut problem| {
            problem.subject = subject.clone();
            problem
        }));
        take(catalogue, &subject, findings);
    }
    Ok(match kind {
        dashboard_base::products::artwork::Kind::Focus(database) => format!(
            "{total} declared {} checked; {without_description} missing description localisation keys (descriptions are unavailable).",
            match database {
                dashboard_core::dto::FocusDatabase::National => "national focuses",
                dashboard_core::dto::FocusDatabase::Continuous => "continuous focuses",
            }
        ),
        dashboard_base::products::artwork::Kind::Technology => {
            format!("{total} declared technologies checked (text and icons).")
        }
    })
}

fn flags_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    use dashboard_world::products::country_flags::{CountryFlagSize, FlagCache, FlagSearch, ShieldStyle, find_flag, flag_cells};
    match content
        .catalogue()
        .and_then(|catalogue| ShieldStyle::load(catalogue).map_err(|error| error.to_string()))
    {
        Ok(style) => findings.extend(
            style
                .warnings
                .into_iter()
                .map(|message| Finding::new(Kind::Warning, "shield", message)),
        ),
        Err(error) => findings.push(Finding::new(Kind::Refused, "shield", error)),
    }
    let cells = flag_cells(&content.resolved.layers).map_err(|error| error.to_string())?;
    let tags = content.tags()?;
    let ideologies = content.ideologies()?;
    let (groups, _) = ideologies.groups();
    let mut cache = FlagCache::default();
    for (tag, definition) in tags {
        if definition.dynamic {
            findings.push(Finding::new(
                Kind::Offline,
                format!("country {tag:?}"),
                "dynamic country flags need recorded appearance; unavailable offline",
            ));
            continue;
        }
        let mut any = false;
        for group in groups.iter().map(|group| Some(group.token.as_str())) {
            let candidates =
                content_reader_country_names::country_names::flag_keys(Some(tag), "", group);
            for size in [
                CountryFlagSize::Full,
                CountryFlagSize::Medium,
                CountryFlagSize::Small,
            ] {
                let subject = format!("flag {tag:?} group {group:?} {size:?}");
                let (found, problems) = find_flag(
                    &content.resolved.layers,
                    size.sets(),
                    &cells,
                    &candidates,
                    &mut cache,
                )
                .map_err(|error| error.to_string())?;
                findings.extend(
                    problems
                        .into_iter()
                        .filter(|problem| problem.file.is_some() || problem.kind == Kind::Refused)
                        .map(|mut problem| {
                            problem.subject = subject.clone();
                            problem
                        }),
                );
                match found {
                    FlagSearch::Found { .. } => any = true,
                    FlagSearch::Unresolvable | FlagSearch::Absent(_) => {}
                }
            }
        }
        if !any {
            let candidates = content_reader_country_names::country_names::flag_keys(Some(tag), "", None);
            let mut plain_found = false;
            for size in [
                CountryFlagSize::Full,
                CountryFlagSize::Medium,
                CountryFlagSize::Small,
            ] {
                let (found, problems) = find_flag(
                    &content.resolved.layers,
                    size.sets(),
                    &cells,
                    &candidates,
                    &mut cache,
                )
                .map_err(|error| error.to_string())?;
                plain_found |= matches!(found, FlagSearch::Found { .. });
                findings.extend(
                    problems
                        .into_iter()
                        .filter(|problem| problem.file.is_some() || problem.kind == Kind::Refused),
                );
            }
            if !plain_found {
                findings.push(Finding::new(Kind::Fallback, format!("flag {tag:?} group None"),
                    "no decodable flag without a ruling group or in any group; flag artwork is unavailable"));
            }
        }
    }
    Ok(())
}

fn take(catalogue: &Catalogue, subject: &str, findings: &mut Vec<Finding>) {
    findings.extend(catalogue.take_findings().into_iter().map(|mut finding| {
        finding.subject = subject.to_owned();
        finding
    }));
}
