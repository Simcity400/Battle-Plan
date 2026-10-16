//! Content-only preparation for every registered dashboard product.

use crate::{catalogue::Catalogue, products};
pub use dashboard_core::findings::{Finding, Kind};
use installed_content::content::ResolvedContent;
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct ProductCheck {
    pub key: &'static str,
    pub elapsed: Duration,
    pub error: Option<String>,
    pub findings: Vec<Finding>,
    /// Explicit coverage, including products with no offline content inputs.
    pub note: String,
}

pub const AUTONOMY_CHECK_KEY: &str = "autonomy_content";
pub const CONSTRUCTION_CHECK_KEY: &str = "construction_content";
pub const PRODUCTION_LINES_CHECK_KEY: &str = "production_lines_content";
pub const INTELLIGENCE_CHECK_KEY: &str = "intelligence_content";
/// Preparation channels outside product cells; reader-owned notes are omitted.
pub const SHARED_CHECK_KEYS: &[&str] = &[
    AUTONOMY_CHECK_KEY,
    CONSTRUCTION_CHECK_KEY,
    PRODUCTION_LINES_CHECK_KEY,
    INTELLIGENCE_CHECK_KEY,
];

/// A shared preparation's findings and notes, read once, or why it failed.
type Prepared<T> = OnceLock<Result<(Vec<Finding>, Vec<T>), String>>;

struct Content {
    resolved: Arc<ResolvedContent>,
    language: String,
    catalogue: OnceLock<Result<Arc<Catalogue>, String>>,
    politics: OnceLock<Result<Arc<products::politics_content::PoliticsContent>, String>>,
    equipment: OnceLock<Result<Arc<installed_content::equipment::Database>, String>>,
    autonomy: OnceLock<
        Result<
            (
                installed_content::autonomy::AutonomyStates,
                Vec<installed_content::Diagnostic>,
            ),
            String,
        >,
    >,
    resources: OnceLock<Result<Vec<Finding>, String>>,
    construction: Prepared<Finding>,
    production: Prepared<installed_content::Diagnostic>,
    tags: OnceLock<
        Result<std::collections::BTreeMap<String, installed_content::country_tags::CountryTag>, String>,
    >,
    ideologies: OnceLock<Result<installed_content::ideologies::Ideologies, String>>,
    intelligence:
        OnceLock<Result<Arc<products::intelligence_content::IntelligenceContent>, String>>,
    agency: OnceLock<Result<Arc<products::intelligence_content::IntelligenceContent>, String>>,
    operatives: OnceLock<Result<Vec<Finding>, String>>,
    ship_history: OnceLock<Result<(), String>>,
}

impl Content {
    fn new(resolved: Arc<ResolvedContent>, language: &str) -> Self {
        Self {
            resolved,
            language: language.to_owned(),
            catalogue: OnceLock::new(),
            politics: OnceLock::new(),
            equipment: OnceLock::new(),
            autonomy: OnceLock::new(),
            resources: OnceLock::new(),
            construction: OnceLock::new(),
            production: OnceLock::new(),
            tags: OnceLock::new(),
            ideologies: OnceLock::new(),
            intelligence: OnceLock::new(),
            agency: OnceLock::new(),
            operatives: OnceLock::new(),
            ship_history: OnceLock::new(),
        }
    }

    fn equipment(&self) -> Result<&Arc<installed_content::equipment::Database>, String> {
        self.equipment
            .get_or_init(|| {
                installed_content::equipment::Database::load_complete(&self.resolved.layers)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn autonomy(&self) -> Result<&installed_content::autonomy::AutonomyStates, String> {
        self.autonomy
            .get_or_init(|| {
                installed_content::autonomy::read(&self.resolved.layers).map_err(|error| error.to_string())
            })
            .as_ref()
            .map(|(states, _)| states)
            .map_err(Clone::clone)
    }

    fn intelligence(
        &self,
    ) -> Result<&Arc<products::intelligence_content::IntelligenceContent>, String> {
        self.intelligence
            .get_or_init(|| {
                products::intelligence_content::IntelligenceContent::load(Arc::clone(
                    self.catalogue()?,
                ))
                .map(Arc::new)
                .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn agency(&self) -> Result<&Arc<products::intelligence_content::IntelligenceContent>, String> {
        self.agency
            .get_or_init(|| {
                Ok(Arc::new(
                    products::intelligence_content::IntelligenceContent::load_agency(Arc::clone(
                        self.catalogue()?,
                    )),
                ))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn operatives(&self) -> Result<&Vec<Finding>, String> {
        self.operatives
            .get_or_init(|| {
                products::operative_content::OperativeContent::load(Arc::clone(self.politics()?))
                    .map(|content| content.check_content())
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn ship_history(&self) -> Result<(), String> {
        self.ship_history
            .get_or_init(|| {
                products::ships_sunk::check_content(self.catalogue()?)
                    .map_err(|error| error.to_string())
            })
            .clone()
    }

    fn tags(
        &self,
    ) -> Result<&std::collections::BTreeMap<String, installed_content::country_tags::CountryTag>, String>
    {
        self.tags
            .get_or_init(|| {
                installed_content::country_tags::read_definitions(&self.resolved.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn ideologies(&self) -> Result<&installed_content::ideologies::Ideologies, String> {
        // The corpus ideologies reader owns these typed native notes.
        self.ideologies
            .get_or_init(|| {
                installed_content::ideologies::read(&self.resolved.layers)
                    .map(|(ideologies, _)| ideologies)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn catalogue(&self) -> Result<&Arc<Catalogue>, String> {
        self.catalogue
            .get_or_init(|| {
                Catalogue::of_layers_for_check(self.resolved.layers.clone(), &self.language)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn politics(&self) -> Result<&Arc<products::politics_content::PoliticsContent>, String> {
        self.politics
            .get_or_init(|| {
                products::politics_content::PoliticsContent::load_with_ideologies(
                    Arc::clone(self.catalogue()?),
                    self.ideologies()?.clone(),
                )
                .map(Arc::new)
                .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Runs in registry order, sharing the catalogue and product readers. No
/// recorded operands are invented; runtime-only checks name their limitation.
pub fn check(content: Arc<ResolvedContent>, language: &str) -> Vec<ProductCheck> {
    let content = Content::new(content, language);
    let mut checks: Vec<_> = products::PRODUCTS
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
    // Equipment's full misreads and lost-read channel are already emitted by
    // the corpus equipment reader, which calls the same Database::load.
    if let Some(autonomy) = content.autonomy.get() {
        let findings = autonomy
            .as_ref()
            .map(|(_, warnings)| {
                warnings
                    .iter()
                    .cloned()
                    .map(|note| Finding::reader("autonomy states", note))
                    .collect()
            })
            .unwrap_or_default();
        checks.push(ProductCheck {
            key: AUTONOMY_CHECK_KEY,
            elapsed: Duration::ZERO,
            error: autonomy.as_ref().err().cloned(),
            findings,
            note: "Shared autonomy preparation; time included in its first product consumer."
                .into(),
        });
    }
    for (key, prepared) in [
        (
            CONSTRUCTION_CHECK_KEY,
            content.construction.get().map(|prepared| {
                prepared
                    .as_ref()
                    .map(|(_, notes)| notes.clone())
                    .map_err(Clone::clone)
            }),
        ),
        (
            PRODUCTION_LINES_CHECK_KEY,
            content.production.get().map(|prepared| {
                prepared
                    .as_ref()
                    .map(|(_, notes)| {
                        notes
                            .iter()
                            .cloned()
                            .map(|note| Finding::reader("production-line preparation", note))
                            .collect()
                    })
                    .map_err(Clone::clone)
            }),
        ),
        (
            INTELLIGENCE_CHECK_KEY,
            content.intelligence.get().map(|prepared| {
                prepared
                    .as_ref()
                    .map(|prepared| prepared.check_reader_notes())
                    .map_err(Clone::clone)
            }),
        ),
    ] {
        if let Some(prepared) = prepared {
            let mut findings = prepared.as_ref().cloned().unwrap_or_default();
            aggregate(&mut findings);
            checks.push(ProductCheck {
                key,
                elapsed: Duration::ZERO,
                error: prepared.err(),
                findings,
                note: "Shared preparation; time included in its first product consumer.".into(),
            });
        }
    }
    checks
}

/// Aggregate declared reasons without putting changing inventories in the message.
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

fn take(catalogue: &Catalogue, subject: &str, findings: &mut Vec<Finding>) {
    findings.extend(catalogue.take_findings().into_iter().map(|mut finding| {
        finding.subject = subject.to_owned();
        finding
    }));
}

fn run(key: &str, content: &Content, findings: &mut Vec<Finding>) -> Result<String, String> {
    use products::*;
    match key {
        national_focus::NationalFocus::KEY | focus_artwork::FocusArtwork::KEY => {
            artwork_check(content, artwork::Kind::Focus, findings)
        }
        research::Research::KEY | technology_artwork::TechnologyArtwork::KEY => {
            artwork_check(content, artwork::Kind::Technology, findings)
        }
        country_flags::CountryFlags::KEY => {
            flags_check(content, findings)?;
            Ok("Authored static-country flags, ideology-group candidates and shield checked; selected flags and shield colours require recorded country appearance, ruling parties and country identities.".into())
        }
        country_factories::CountryFactories::KEY => {
            let catalogue = content.catalogue()?;
            country_factories::content_groups(catalogue);
            findings.extend(catalogue.take_findings());
            Ok("Every authored factory building and group name checked; country totals require recorded factory rows.".into())
        }
        party_history::PartyHistory::KEY => {
            parties_check(content, findings)?;
            Ok("Authored ideology names/colours and static-country party-name ladders checked; party popularity, ruling parties, elections, custom names and leader names/portraits require recorded politics, country identities and character holders.".into())
        }
        idea_history::IdeaHistory::KEY => ideas_check(content, findings),
        country_stability::CountryStability::KEY => {
            stability_check(content, findings)?;
            Ok("Authored idea, advisor, ideology and dynamic-modifier formula-line names/groups checked; stability, war support, command-power formulas and scripted line text require recorded inputs, modifier definitions, country scope and character holders.".into())
        }
        stability_parts::StabilityParts::KEY => {
            stability_check(content, findings)?;
            Ok("Authored idea, advisor, ideology and dynamic-modifier part names/groups checked; part values, identity changes, scripted text and observation dates require recorded stability lines, inputs, modifier definitions, country scope and character holders.".into())
        }
        resource_balance::ResourceBalance::KEY | trade_flows::TradeFlows::KEY => {
            let prepared = content.resources.get_or_init(|| {
                let mut prepared = Vec::new();
                resources_check(content, &mut prepared).map(|()| prepared)
            });
            findings.extend(prepared.as_ref().map_err(Clone::clone)?.iter().cloned());
            Ok(if key == resource_balance::ResourceBalance::KEY {
                "Authored resource names and icons checked; country pools, trade quantities and recorded resource order require recorded resource definitions, lexical tokens, country resources, exchanges and country identities."
            } else {
                "Authored resource names and icons checked; seller/buyer flows, quantities and recorded resource order require recorded resource definitions, lexical tokens, exchanges and country identities."
            }.into())
        }
        subjects::Subjects::KEY => {
            let catalogue = content.catalogue()?;
            let states = content.autonomy()?;
            let named = states
                .states
                .iter()
                .map(|state| state.id.as_str())
                .collect();
            subjects::levels(&named, Some(states), Some(catalogue));
            take(catalogue, "subject level names", findings);
            Ok("Every authored autonomy level checked; active relations and current levels require a recording.".into())
        }
        occupied_states::OccupiedStates::KEY | occupied_states::OccupiedStatesManifest::KEY => {
            let catalogue = content.catalogue()?;
            let laws = installed_content::occupation_laws::read(&content.resolved.layers)
                .map_err(|why| why.to_string())?;
            // Laws::warnings contains failed file reads, which settle no law.
            findings.extend(
                laws.warnings
                    .iter()
                    .map(|message| Finding::new(Kind::Refused, "occupation laws", message)),
            );
            if laws.warnings.is_empty() {
                for token in laws.definitions.keys() {
                    occupied_states::law_content(catalogue, &laws, token);
                    take(catalogue, &format!("occupation law {token}"), findings);
                }
            }
            Ok("Authored occupation-law names and icons checked; occupied state names and law assignments require recorded state identities.".into())
        }
        equipment_stockpile::EquipmentStockpile::KEY => {
            let catalogue = content.catalogue()?;
            let equipment = content.equipment()?;
            for archetype in equipment.equipment.iter().filter(|kind| kind.is_archetype) {
                let key = archetype.name_str().map_err(|why| why.to_string())?;
                equipment_stockpile::type_name(catalogue, key);
                take(catalogue, &format!("equipment type {key}"), findings);
            }
            Ok("Authored equipment archetype names checked; variant names and country localisation ladders require recorded variants and country identities.".into())
        }
        equipment_produced::EquipmentProduced::KEY
        | production_spending::ProductionSpending::KEY => {
            equipment_production_check(content, key, findings)?;
            Ok("Authored equipment types, archetypes, domains and type names checked through shared equipment production preparation; produced/spent totals, costs and variant names require recorded lines, outputs, costs, variants and country identities.".into())
        }
        special_projects::SpecialProjects::KEY => {
            findings.extend(
                special_projects::check_content(Arc::clone(content.politics()?))
                    .map_err(|why| why.to_string())?,
            );
            Ok("Authored project names, specialization names and generic icons checked through special-project preparation; country overrides, scripted names, iteration counts and scientist names/portraits require recorded country, project and character context.".into())
        }
        autonomy::Autonomy::KEY => {
            autonomy_check(content, findings)?;
            Ok("Every authored autonomy level named and ordered through autonomy preparation; subject relations, scores and move limits require recorded Autonomy and Subjects rows.".into())
        }
        construction_log::ConstructionLog::KEY
        | construction_log::ConstructionLogManifest::KEY
        | construction_spending::ConstructionSpending::KEY => {
            let prepared = content
                .construction
                .get_or_init(|| construction_check(content));
            findings.extend(prepared.as_ref().map_err(Clone::clone)?.0.iter().cloned());
            Ok("Every authored building and state named, and queue building/repair icons drawn through the shared construction preparation; recorded queue lines, state identities, custom state names and building/progress reads determine active projects and spending.".into())
        }
        military_air_ledger::MilitaryAirLedger::KEY => {
            findings.extend(military_air_ledger::check_content(content.catalogue()?)
                .map_err(|why| why.to_string())?);
            Ok("Air regions and bombing building names checked through the recorded content stack; native event context determines recorded amounts.".into())
        }
        equipment_losses::EquipmentLosses::KEY => {
            findings.extend(
                equipment_losses::check_content(content.catalogue()?, content.equipment()?)
                    .map_err(|why| why.to_string())?,
            );
            Ok("Loss cause interface keys, cause names, equipment/design base names and Combat Log retention checked through equipment-losses preparation; recorded designs, loss records and writer coverage determine totals and active selections.".into())
        }
        world_tension::WorldTension::KEY => {
            Ok("No content dependency: tension, country contributions and already-built threat text come from recorded WorldTension, ThreatCountryAggregates and ThreatSources rows, with Countries/CountryLinks naming the countries; display_text only strips their presentation controls.".into())
        }
        decryption::Decryption::KEY => {
            decryption::content_defines(&content.resolved.layers)?;
            Ok("Cipher threshold defines checked through shared decryption preparation; target strengths, amounts, bonus states and country identities require recorded RaidIntelInputs, Cryptology, Countries and CountryLinks rows, with Campaign dating the observations.".into())
        }
        country_nukes::CountryNukes::KEY => {
            Ok("Nuclear type names localise the native token spellings recorded in LexicalTokens; no authored type-name inventory exists offline. Amounts, type ordinals, country identities and observation dates require recorded Nukes, Countries, CountryLinks and Campaign rows.".into())
        }
        raids::Raids::KEY => {
            findings
                .extend(raids::check_content(content.catalogue()?).map_err(|why| why.to_string())?);
            Ok("Every authored raid type named and its generic icon drawn through raid preparation; phase/outcome names use the product's native keys. Active episodes, target names, building-specific icons and tallies require recorded Raids, RaidTargets, RaidCountries, Countries, CountryLinks, States, Provinces, LexicalTokens and Campaign context.".into())
        }
        espionage_log::EspionageLog::KEY => {
            findings.extend(content.intelligence()?.check_operations());
            Ok("Every authored operation name and icon checked through shared intelligence preparation; active episodes, assignments, type tokens, country identities and dates require recorded Operations, OperationAssignments, LexicalTokens, Countries, CountryLinks and Campaign rows.".into())
        }
        operative_placements::OperativePlacements::KEY => {
            Ok("Spy counts and saved operative names require recorded OperativeMemberships, SimulationFeatures, Countries and CountryLinks, with Campaign and the supplying membership read dating each observation. Names localise through the recorded content stack; no per-pair network strength is inferred.".into())
        }
        operatives_roster::OperativesRoster::KEY => {
            findings.extend(content.operatives()?.iter().cloned());
            Ok("Native operative skill definitions, operation assignment names/icons, mission text icons and cooldown/capture-duration content checked through shared operative preparation. Names, experience, assignments, full-reference character portraits, captured state/captor/date and country names require recorded OperativeMemberships, CharacterOperatives, CharacterPortraits, Operations, LexicalTokens, SimulationFeatures, country naming tables and actual supplying reads.".into())
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
            let refused = installed_content::equipment_graphics::aircraft::Catalogue::refusal_counts(&listing.entries);
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
        agency_comparison::AgencyComparison::KEY => {
            findings.extend(content.agency()?.check_agency());
            Ok("Uncreated agency name and defense-label template checked through shared intelligence preparation; created names, defense values, signed acquired upgrade inventory, agency/operative rosters, country identities and dates require recorded IntelligenceAgencies, AgencyUpgradeLevels, LexicalTokens, OperativeMemberships, Countries, CountryLinks and Campaign rows.".into())
        }
        production_lines::ProductionLines::KEY
        | production_lines::ProductionLineDetail::KEY
        | production_lines::ProductionLineBreakdown::KEY
        | equipment_stockpile::artwork::StockpileArtwork::KEY => {
            let prepared = content.production.get_or_init(|| {
                production_lines::check_content(
                    Arc::clone(content.catalogue()?),
                    Arc::clone(content.equipment()?),
                )
                .map_err(|why| why.to_string())
            });
            findings.extend(prepared.as_ref().map_err(Clone::clone)?.0.iter().cloned());
            Ok("Shared production-line icon content loaded; authored equipment base names and generic icon candidates named and drawn through production preparation. Active line/detail inventory, variant overrides/creator/origin/classifier flags, country appearance/factions/capital and organisation name/icon/owner operands require recorded ProductionLines, VariantDefinitions and IndustrialOrganisations rows.".into())
        }
        land_owners::LandOwnersProduct::KEY => {
            names_check(content, false, findings)?;
            Ok("Static-country name ladders checked through country-name preparation; land totals, owners, dynamic-country names and cosmetic identities require recorded states and country naming operands.".into())
        }
        country_styles::CountryStyles::KEY => {
            names_check(content, true, findings)?;
            Ok("Static-country names checked through the product's plain-text country-name preparation; map colours, dynamic-country names and cosmetic identities require recorded country appearance and naming operands.".into())
        }
        country_groups::CountryGroups::KEY => {
            Ok("Faction names use recorded keys localised through the content stack; their inventory requires a recording.".into())
        }
        peace_deals::PeaceDeals::KEY => {
            Ok("Peace action names use captured native cached type names in the recording client's language; their inventory requires recorded PeaceSettlements, PeaceBids, PeaceConferences and LexicalTokens. Exact event and publication dates, ordered actions, signed state-transfer values and country identities require recorded rows.".into())
        }
        country_measure::CountryMeasure::KEY => {
            Ok("No content dependency: political and command power balances and their net drops use recorded Countries and CountryLinks rows.".into())
        }
        country_fuel::CountryFuel::KEY => {
            Ok("No content dependency: tank quantities, capacity and daily gain/burn rings use recorded CountryFuel rows and Countries/CountryLinks identities.".into())
        }
        country_fuel::CountryFuelAwards::KEY | country_fuel::CountryFuelAwardHistory::KEY => {
            findings.push(Finding::new(Kind::Offline, key,
                "receipt inventory, amounts and cached recipient names need supplying recorded fuel-distribution rows and matching recorded language; unavailable offline"));
            Ok("Fuel award presentation reads recorded receipt rows; no authored receipt-name inventory exists offline.".into())
        }
        country_fuel::CountryFuelConsumption::KEY => {
            Ok("Current required-use rates use recorded FuelDemandRead native rates, independently of completed-day burn and receipts; old or refused inputs remain unknown.".into())
        }
        country_fuel::CountryFuelDemand::KEY | country_fuel::CountryFuelDemandHistory::KEY | country_fuel::CountryFuelDemandStack::KEY => {
            Ok("Individual consumption names are supplying-read stored bytes guarded by the recording language; divisions, commands, fleets, task forces and repeated air-site/wing occurrences require recorded FuelDemandRead. No authored or later names replace missing native names.".into())
        }
        war_casualties::WarCasualties::KEY => {
            let catalogue = content.catalogue()?;
            let label = war_casualties::UNKNOWN_LABEL;
            if catalogue
                .localised(label, crate::catalogue::View::Plain)
                .is_some_and(|text| text.trim().is_empty())
            {
                findings.push(Finding::new(
                    Kind::Fallback,
                    format!("casualty label {label}"),
                    "localisation is blank; the key is shown",
                ));
            }
            take(catalogue, &format!("casualty label {label}"), findings);
            Ok("Unknown-casualty label checked; campaign names, casualty counters and rankings require recorded Campaign, WarRelations, country identities and politics rows.".into())
        }
        wars::Wars::KEY | war_casualties::breakdown::CasualtyBreakdown::KEY => {
            names_check(content, false, findings)?;
            let catalogue = content.catalogue()?;
            catalogue.war_names()?;
            take(catalogue, "native war naming content", findings);
            Ok("Native war naming content loaded through the shared Wars catalogue; viewpoint groups, first-goal tokens, naming operands, episode continuity and signed casualty lifetimes require recorded rows. Domain and unknown-attacker attribution are not invented.".into())
        }
        war_casualties::pairs::CasualtyPairs::KEY => {
            Ok("No content dependency: signed country-pair totals use the shared recorded identity and casualty-lifetime scan.".into())
        }
        scoreboard::Scoreboard::KEY => {
            let catalogue = content.catalogue()?;
            scoreboard::registry(Some(catalogue));
            take(catalogue, "scoreboard column labels", findings);
            Ok("Column labels the executable binds checked through the product's registry; measures, rankings and country identities require recorded rows.".into())
        }
        occupation_trend::OccupationTrend::KEY => {
            Ok("No content dependency: occupier resistance and compliance means use occupied-states' recorded state, garrison and resistance scan and Countries/CountryLinks identities.".into())
        }
        country_experience::CountryExperience::KEY => {
            Ok("No content dependency: banked army, navy and air experience and daily changes use recorded CountryExperience rows and Countries/CountryLinks identities.".into())
        }
        army_size::ArmySize::KEY => {
            findings.extend(army_size::check_content(content.catalogue()?).map_err(|why| why.to_string())?);
            Ok("Authored sub-unit groups checked through Army composition preparation; totals, template grids, country identities and supplying dates require recorded army rows.".into())
        }
        unit_leaders::UnitLeaders::KEY => {
            findings.extend(unit_leaders::check_content(Arc::clone(content.catalogue()?), Arc::clone(content.politics()?)).map_err(|why| why.to_string())?);
            Ok("Leader ranks, attributes, trait names/icons and skill definitions checked through leader preparation; visibility, held traits, participation, decided outcomes, command names and modifier conditions require recorded rows.".into())
        }
        unit_leaders::portraits::UnitLeaderPortraits::KEY => {
            findings.extend(unit_leaders::portraits::check_content(content.politics()?));
            Ok("Every authored large portrait alternative checked through the shared Politics portrait loader; selected kind/identity, generated portraits and native fallback order require recorded portrait holders.".into())
        }
        supply_health::SupplyHealth::KEY => {
            supply_health::check_content(&content.resolved.layers).map_err(|why| why.to_string())?;
            Ok("Supply icon thresholds and grace defines checked through the product loader; supply/grace operands, owner attribution and hourly counts require recorded division inputs.".into())
        }
        division_log::DivisionLog::KEY | division_log::DivisionLogManifest::KEY => {
            Ok("Recorded cached division names and template operands require supplying rows; historical naming operands resolve through the shared four-directory name database only when reached.".into())
        }
        division_log::DivisionLogTotals::KEY => {
            Ok("No content dependency: totals count witnessed events with recorded country and template identities.".into())
        }
        land_battles::LandBattles::KEY
        | land_battles::LandBattlesManifest::KEY
        | land_battles::LandBattlesRanking::KEY => {
            Ok("No content dependency: battle census, observation bounds, participant counts and rankings use recorded Campaign, LandCombats, LandCombatSides and Countries/CountryLinks rows. Place names are resolved by the separate place-names product; casualties and winners remain unavailable without their recorded operands.".into())
        }
        land_battles::LandBattleCourses::KEY => {
            Ok("Equipment design names resolve through the shared recorded variant catalogue only when a battle's loss reads name the design; runs, losses and verdicts require recorded LandCombats, LandCombatSides, LandCombatProgress and LandCombatResults rows.".into())
        }
        land_battles::LandBattleTerrains::KEY => {
            let catalogue = content.catalogue()?;
            for warning in land_battles::check_terrain_content(catalogue).map_err(|why| why.to_string())? {
                findings.push(Finding::new(Kind::Warning, "land battle terrain", warning));
            }
            take(catalogue, "land battle terrain titles", findings);
            Ok("Every province terrain the content map definitions use checked against the terrain database and its localised category title, as the land combat window titles it.".into())
        }
        naval_battles::NavalBattles::KEY
        | naval_battles::NavalBattlesManifest::KEY
        | naval_battles::NavalBattlesRanking::KEY => {
            Ok("No content dependency: live combats and retained results join only through recorded full NavalResultSources references. Peak ships and length use their observed combat reads; place names resolve separately through place-names.".into())
        }
        naval_losses::NavalLosses::KEY => {
            let catalogue = content.catalogue()?;
            for warning in sinkings::retention(&sinkings::Scanned::default(), Some(catalogue)).warnings {
                findings.push(Finding::new(Kind::Warning, "naval history retention", warning));
            }
            Ok("Native retention defines checked through the shared sinking scan. Signed convoy corrections, ship/convoy read dates, country identities and loss histories require recorded rows.".into())
        }
        ships_sunk::ShipsSunk::KEY | naval_battles::NavalBattleDetails::KEY => {
            content.ship_history()?;
            Ok("Ship class definitions, strategic-region names and retention checked through shared ship-name preparation. Class tokens, ship/fleet names, recording language and exact battle references require recorded rows.".into())
        }
        ships_sunk::ShipsSunkBattleTitles::KEY => {
            let catalogue = content.catalogue()?;
            for warning in ships_sunk::battle_titles::check_content(catalogue)? {
                findings.push(Finding::new(Kind::Warning, "naval battle title", warning));
            }
            place_names_check(content, findings)?;
            Ok("Native naval title geography, content defines and title formatter checked through the production loader. Selected victory points and the current recorded PlaceNames original/reserved-tag name ladder require the supplying read.".into())
        }
        place_names::PlaceNames::KEY => {
            place_names_check(content, findings)?;
            Ok("Authored state-template and strategic-region names checked through the shared state content, region reader and plain localiser. Province overrides, owner-qualified victory-point names, stored state names and their language require recorded Provinces, ProvinceNames, States, Countries, CountryLinks, CountryAppearance and Campaign context.".into())
        }
        territory::Territory::KEY => {
            installed_content::province_area::ProvinceAreas::read(&content.resolved.layers)
                .map_err(|error| error.to_string())?;
            Ok("Native land map-cell weights loaded; totals and owned/controlled identities need recorded States and Provinces rows.".into())
        }
        territory_breakdown::TerritoryBreakdown::KEY => {
            place_names_check(content, findings)?;
            Ok("State-template names checked; selected ownership, stored names, victory points and values need the supplying recorded rows.".into())
        }
        faction_totals::FactionTotals::KEY => {
            country_factories::content_groups(content.catalogue()?);
            findings.extend(content.catalogue()?.take_findings());
            findings.push(Finding::new(Kind::Offline, key,
                "faction names, membership, native holders and the selected totals need recorded faction and measure rows; unavailable offline"));
            Ok("Authored factory-group names checked; faction name-template operands and country membership need a recording.".into())
        }
        air_aces::AirAces::KEY | air_aces::AirAcePortraits::KEY => {
            findings.push(Finding::new(Kind::Offline, key,
                "ace and wing identity, stored names, aircraft variants and selected/cached portrait-pool operands need recorded AirAces and companion rows; unavailable offline"));
            Ok("No authored ace roster or selected portrait inventory exists offline; supplying recorded operands select content names and sprites.".into())
        }
        air_force::AirForceSize::KEY | navy::NavySize::KEY => {
            let catalogue = content.catalogue()?;
            if key == air_force::AirForceSize::KEY {
                content.equipment()?;
            } else {
                let units = installed_content::unit_leader::sub_units(&content.resolved.layers)
                    .map_err(|error| error.to_string())?;
                for token in units.registered() {
                    let token = std::str::from_utf8(token).map_err(|error| error.to_string())?;
                    if catalogue.localisation.get(token).is_some() {
                        catalogue.localised(token, crate::catalogue::View::Plain);
                        take(catalogue, token, findings);
                    }
                }
            }
            findings.push(Finding::new(Kind::Offline, key,
                "recorded equipment/class tokens and country inventories select displayed labels; unavailable offline"));
            Ok("Native equipment/sub-unit database prepared; selected composition and supplying dates need recorded inventories.".into())
        }
        game_score::GameScore::KEY => {
            findings.extend(game_score::check_content(content.catalogue()?));
            Ok("Native score hints and compatible disabled/older-recording multiplier defines checked; current score inputs and identities require recorded rows.".into())
        }
        air_superiority::AirSuperiority::KEY => {
            place_names_check(content, findings)?;
            findings.push(Finding::new(Kind::Offline, key,
                "regional scores, faction/war-side assignment, theatres and historical anchors require recorded inputs; unavailable offline"));
            Ok("Authored strategic-region names checked; country/faction naming and regional observations need a recording.".into())
        }
        politics_ledger::PoliticsLedger::KEY => {
            findings.push(Finding::new(Kind::Offline, key,
                "spend category/item inventory and captured native descriptions, scopes, costs and names require recorded political-power streams; unavailable offline"));
            Ok("Ledger presentation selects idea, decision, ability, trait and organisation content from recorded spend operands; no country-independent spend inventory exists offline.".into())
        }
        operation_description::OperationDescription::KEY => {
            Ok("No authored content dependency: cached native operation descriptions use recorded operation-description rows and their captured text.".into())
        }
        event_feed::EventFeed::KEY => {
            findings.push(Finding::new(
                Kind::Offline,
                "event feed",
                "event transitions and their exact or observation-bound dates need the recorded topic histories; unavailable offline",
            ));
            findings.push(Finding::new(
                Kind::Offline,
                "event row presentation",
                "row-hour country names, flags and country-scoped item presentation need recorded identities, appearances and event context; unavailable offline",
            ));
            findings.push(Finding::new(
                Kind::Offline,
                "nuclear stockpile artwork",
                "nuclear topbar GUI artwork and client visibility are outside the reused topic checks and have not been evaluated offline",
            ));
            Ok("Static content diagnostics are reported separately by the existing focus, party, idea, project, subject, country-name and flag checks; the feed reuses their topic helpers without another content inventory. Feed transitions, row-hour presentation and the nuclear indicator remain offline coverage limits.".into())
        }
        _ => Err("No content preparation check is registered for this product.".into()),
    }
}

fn place_names_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    let catalogue = content.catalogue()?;
    // State reader notes belong to the corpus states reader, as in construction.
    for (&id, key) in &catalogue.cached_states()?.names {
        let subject = format!("state {id}");
        match key {
            Ok(key) => {
                // Template text and successfully localised text remain
                // known even when empty.
                catalogue.localised(key, crate::catalogue::View::Plain);
                take(catalogue, &subject, findings);
            }
            Err(reason) => findings.push(Finding::new(Kind::Refused, subject, reason)),
        }
    }
    // The corpus regions reader owns its native skip/replacement notes.
    let regions = installed_content::regions::read(&content.resolved.layers)
        .map_err(|error| format!("Strategic-region names unavailable: {error}"))?;
    for region in regions.regions.values() {
        let subject = format!("strategic region {}", region.id);
        // Stored localised text includes successful empty results and
        // the empty constructor default.
        catalogue.localised(&region.name_key, crate::catalogue::View::Plain);
        take(catalogue, &subject, findings);
    }
    Ok(())
}

fn equipment_production_check(
    content: &Content,
    product: &str,
    findings: &mut Vec<Finding>,
) -> Result<(), String> {
    let catalogue = content.catalogue()?;
    let units = content.equipment()?;
    let named = products::equipment::production::Named {
        units: Some(units),
        texts: Some(catalogue.as_ref()),
    };
    for kind in &units.equipment {
        let token = kind.name_str().map_err(|why| why.to_string())?;
        let subject = format!("equipment type {token}");
        match products::equipment::archetype_of(units, token) {
            Ok((archetype, _, _)) => {
                if named.type_name(&archetype).is_none() {
                    findings.push(Finding::grouped(
                        Kind::Fallback,
                        &subject,
                        "equipment type has no usable name; its token is shown",
                        format!(
                            "{archetype}: {product} has no usable type name; the token is shown"
                        ),
                    ));
                }
            }
            Err(reason) => findings.push(Finding::new(Kind::Refused, &subject, reason)),
        }
        take(catalogue, &subject, findings);
    }
    Ok(())
}

fn autonomy_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    let catalogue = content.catalogue()?;
    let states = content.autonomy()?;
    products::autonomy::content_levels(states, catalogue);
    take(catalogue, "autonomy level names", findings);
    Ok(())
}

fn construction_check(content: &Content) -> Result<(Vec<Finding>, Vec<Finding>), String> {
    let catalogue = content.catalogue()?;
    let localise = |key: &str| catalogue.localised(key, crate::catalogue::View::Plain);
    let named = products::construction_log::named_all(
        &products::construction_log::ContentView {
            layers: &catalogue.content.layers,
            sprites: &catalogue.sprites,
            guis: catalogue.guis().map_err(|error| error.to_string()),
            gui_decode: &catalogue.gui_decode,
            localise: &localise,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|why| why.to_string())?;
    // Typed state notes belong to the corpus states reader, including its refusals.
    let products::construction_log::Named {
        warnings,
        declared_types,
        ..
    } = named;
    let shared = warnings
        .into_iter()
        .map(|note| Finding::reader("construction preparation", note))
        .collect();
    let mut findings = Vec::new();
    for key in declared_types {
        findings.push(Finding::new(Kind::Offline, format!("building {key}"),
            "active construction and completed/spent amounts need recorded queue and progress reads; unavailable offline"));
    }
    take(catalogue, "construction names", &mut findings);
    Ok((findings, shared))
}

fn artwork_check(
    content: &Content,
    kind: products::artwork::Kind,
    findings: &mut Vec<Finding>,
) -> Result<String, String> {
    let catalogue = content.catalogue()?;
    let context = products::artwork::Context::load(Arc::clone(catalogue), kind)
        .map_err(|error| error.to_string())?;
    let (tokens, _) = context.tokens();
    let total = tokens.len();
    let mut without_description = 0;
    for token in tokens {
        let subject = format!(
            "{} {token}",
            match kind {
                products::artwork::Kind::Focus => "focus",
                products::artwork::Kind::Technology => "technology",
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
        products::artwork::Kind::Focus => format!(
            "{total} declared items checked; {without_description} missing description localisation keys (descriptions are unavailable)."
        ),
        products::artwork::Kind::Technology => {
            format!("{total} declared technologies checked (text and icons).")
        }
    })
}

fn flags_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    use products::country_flags::{
        CountryFlagSize, FlagCache, FlagSearch, ShieldStyle, find_flag, flag_cells,
    };
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
    // Flag preparation needs ideology groups alone, not the idea databases.
    let ideologies = content.ideologies()?;
    let (groups, _) = ideologies.groups();
    let mut cache = FlagCache::default();
    for (tag, definition) in tags {
        if definition.dynamic {
            // Dynamic tags declare runtime slots; the product's choices()
            // resolves their recorded appearance.
            findings.push(Finding::new(
                Kind::Offline,
                format!("country {tag:?}"),
                "dynamic country flags need recorded appearance; unavailable offline",
            ));
            continue;
        }
        let mut any = false;
        for group in groups.iter().map(|group| Some(group.token.as_str())) {
            let candidates = installed_content::country_names::flag_keys(Some(tag), "", group);
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
                // A completely missing group/set authored no image, and the
                // game skips it while searching for a decodable flag.
                // Keep file refusals, but no diagnostic for authored absence.
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
                    FlagSearch::Unresolvable | FlagSearch::Absent => {}
                }
            }
        }
        // Probe the null-group candidate only if every group/set was absent,
        // and retain one visible-fallback note per tag, rather than three
        // impossible requests.
        if !any {
            let candidates = installed_content::country_names::flag_keys(Some(tag), "", None);
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

fn parties_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    use crate::party_names::{PartyNaming, party_key_checked};
    use products::party_history::{ideology_infos, ideology_type_info};
    let politics = content.politics()?;
    let mut warnings = Vec::new();
    ideology_infos(politics, &mut warnings);
    // ideology_infos emits typed product findings; its strings are live warnings.
    // Localisation diagnostics retain the ideology token in their messages.
    findings.extend(politics.catalogue.take_findings());
    let tags = content.tags()?;
    for (tag, definition) in tags {
        if definition.dynamic {
            findings.push(Finding::new(
                Kind::Offline,
                format!("country {tag:?}"),
                "dynamic country parties need recorded identity; unavailable offline",
            ));
        }
    }
    for group in politics.ideologies.groups().0 {
        for token in &group.types {
            ideology_type_info(Some(politics), token);
            take(
                &politics.catalogue,
                &format!("ideology type {token}"),
                findings,
            );
        }
        for (tag, _) in tags.iter().filter(|(_, definition)| !definition.dynamic) {
            let subject = format!("party {tag:?} / {}", group.token);
            let naming = PartyNaming {
                token: group.token.clone(),
                tag: Some(tag.clone()),
                original_tag: Some(String::new()),
                key: None,
            };
            let (key, reason) = party_key_checked(&naming, |key| politics.defines(key))
                .ok_or_else(|| "static party naming operands were refused".to_owned())?;
            if let Some(reason) = reason {
                // The group token is the final party-naming rung.
                findings.push(Finding::grouped(
                    Kind::Content,
                    &subject,
                    "party name keys are missing; the ideology group name is shown",
                    reason,
                ));
            }
            politics.localised(&key, crate::catalogue::View::Plain);
            take(&politics.catalogue, &subject, findings);
        }
    }
    Ok(())
}

fn ideas_check(content: &Content, findings: &mut Vec<Finding>) -> Result<String, String> {
    use products::idea_history::{Joined, categories, effect_keys, prepare_info};
    let politics = content.politics()?;
    let localise = |key: &str, view| politics.localised(key, view);
    let joined = Joined::of(politics, &localise);
    categories(&joined);
    take(&politics.catalogue, "idea categories and slots", findings);
    if let Some(gap) = politics.ideas.tags.categories().1 {
        findings.push(Finding::new(
            Kind::Refused,
            "idea category inventory",
            gap.to_string(),
        ));
    }
    let (tokens, gap) = politics.ideas.tokens();
    if let Some(gap) = gap {
        findings.push(Finding::new(
            Kind::Refused,
            "idea inventory",
            gap.to_string(),
        ));
    }
    let (advisors, advisor_gap) = politics.characters.advisor_tokens();
    if let Some(gap) = advisor_gap {
        findings.push(Finding::new(
            Kind::Refused,
            "advisor idea inventory",
            gap.to_string(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut without_description = 0;
    for token in tokens.chain(advisors).filter(|token| seen.insert(*token)) {
        let subject = format!("idea {token}");
        without_description += usize::from(politics.idea_description_key(token).is_none());
        let (found, problems) = prepare_info(
            &joined,
            token,
            None,
            Err("recorded advisor holder is unavailable offline"),
            true,
        );
        findings.extend(problems);
        // Artwork creates typed findings at its source. Its returned string
        // serves the live reply only; emitting it here would count portraits twice.
        politics.idea_artwork_offline(token);
        let (modifiers, texts) = effect_keys(&[found]);
        findings.extend(products::idea_format::content_formats(politics, &modifiers));
        products::idea_format::texts(politics, &texts);
        take(&politics.catalogue, &subject, findings);
    }
    Ok(format!(
        "{} declared ideas/advisors checked; {without_description} without an authored description key (no finding).",
        seen.len()
    ))
}

fn stability_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    use products::country_stability::{BreakdownLine, line_group_offline, line_name_offline};
    let politics = content.politics()?;
    let check_line = |kind: &str, identity: &str, findings: &mut Vec<Finding>| {
        let line = BreakdownLine {
            line: 0,
            kind: kind.to_owned(),
            identity: identity.to_owned(),
            identity_id: 0,
            secondary_raw: String::new(),
            value_raw: String::new(),
            text: None,
            label: None,
            group: None,
            group_kind: None,
        };
        let subject = format!("{kind} {identity}");
        line_group_offline(&line, politics);
        line_name_offline(&line, politics);
        let problems = politics.catalogue.take_findings();
        findings.extend(problems.into_iter().map(|mut problem| {
            problem.subject = subject.clone();
            problem
        }));
    };
    let (tokens, gap) = politics.ideas.tokens();
    if let Some(gap) = gap {
        findings.push(Finding::new(
            Kind::Refused,
            "idea inventory",
            gap.to_string(),
        ));
    }
    for token in tokens {
        check_line("idea", token, findings);
    }
    let (advisors, gap) = politics.characters.advisor_tokens();
    if let Some(gap) = gap {
        findings.push(Finding::new(
            Kind::Refused,
            "advisor idea inventory",
            gap.to_string(),
        ));
    }
    for token in advisors {
        check_line("advisor", token, findings);
    }
    let (groups, _) = politics.ideologies.groups();
    for group in groups {
        check_line("ideology", &group.token, findings);
        for token in &group.types {
            check_line("ideology", token, findings);
        }
    }
    for definition in politics.modifier_definitions.iter() {
        check_line("dynamicModifier", &definition.token, findings);
    }
    Ok(())
}

fn resources_check(content: &Content, findings: &mut Vec<Finding>) -> Result<(), String> {
    use products::resources::{ContentView, ordinals, resource_content};
    let catalogue = content.catalogue()?;
    let (resources, _) =
        installed_content::resources::read(&content.resolved.layers).map_err(|error| error.to_string())?;
    let numbered: Vec<_> = resources
        .into_iter()
        .enumerate()
        .map(|(index, resource)| (index as u32, resource.token))
        .collect();
    let wanted: Vec<_> = numbered
        .iter()
        .zip(ordinals(&numbered))
        .map(|((_, token), ordinal)| (token.clone(), ordinal))
        .collect();
    let mut cached = std::collections::BTreeMap::new();
    let mut warnings = Vec::new();
    findings.extend(
        resource_content(
            &ContentView::of(catalogue),
            &wanted,
            &mut cached,
            &mut warnings,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map_err(|error| error.to_string())?,
    );

    Ok(())
}

fn names_check(content: &Content, plain: bool, findings: &mut Vec<Finding>) -> Result<(), String> {
    use installed_content::country_names::{CountryNames, Operands};
    let catalogue = content.catalogue()?;
    let tags = content.tags()?;
    let ideologies = content.ideologies()?;
    let (groups, _) = ideologies.groups();
    for (tag, definition) in tags {
        if definition.dynamic {
            // Dynamic tags mark runtime slots. CountryNames' recorded operands
            // (country_names.rs) supply the runtime naming/cosmetic identity.
            findings.push(Finding::new(
                Kind::Offline,
                format!("country {tag:?}"),
                "dynamic country names need recorded naming operands; unavailable offline",
            ));
            continue;
        }
        for group in
            std::iter::once(None).chain(groups.iter().map(|group| Some(group.token.as_str())))
        {
            let subject = format!("country name {tag:?} group {group:?}");
            let operands = Operands::for_tag(tag, group);
            let key = operands.name_key("", |key| catalogue.localisation.get(key).is_some());
            let names = CountryNames::of_operands(
                &catalogue.localisation,
                std::collections::BTreeMap::from([(0, Ok(operands))]),
            );
            match if plain {
                products::country_styles::country_name(&names, 0)
            } else {
                names.name(0)
            } {
                Ok(_) => {}
                Err(reason) => {
                    let message =
                        format!("{reason}; the country name is unavailable and the tag is shown");
                    findings.push(if key.is_none() {
                        Finding::grouped(
                            Kind::Content,
                            subject,
                            "country-name ladder has no localisation; the bare tag key is copied",
                            message,
                        )
                    } else {
                        Finding::new(Kind::Refused, subject, message)
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_construction_state_read_is_omitted_offline() {
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
            "common/buildings/test.txt",
            "buildings={mod_building={icon_frame=-1}}",
        )]);
        let error = crate::scripted_text::read_state_content(&catalogue.content.layers)
            .err()
            .unwrap();
        let expected = format!("State names are unavailable: {error}");
        let checks = check(Arc::clone(&catalogue.content), "english");
        let shared = checks
            .iter()
            .find(|check| check.key == CONSTRUCTION_CHECK_KEY)
            .unwrap();
        assert!(shared.error.is_none(), "{shared:?}");
        assert!(
            shared
                .findings
                .iter()
                .all(|finding| !finding.message.contains(&expected)),
            "{shared:?}"
        );
    }

    #[test]
    fn place_name_inventory_accepts_successful_empty_text_and_native_defaults() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/mod.lua",
                "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
            ),
            (
                "map/default.map",
                "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp",
            ),
            ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
            ("history/states/empty.txt", "state={ id=11 name=EMPTY }"),
            ("history/states/default.txt", "state={ id=12 }"),
            ("history/states/absent.txt", "state={ id=13 name=ABSENT }"),
            (
                "map/strategicregions/empty.txt",
                "strategic_region={ id=21 name=EMPTY }",
            ),
            (
                "map/strategicregions/default.txt",
                "strategic_region={ id=22 }",
            ),
            (
                "map/strategicregions/absent.txt",
                "strategic_region={ id=23 name=ABSENT }",
            ),
            (
                "localisation/mod_l_english.yml",
                "\u{feff}l_english:\n EMPTY:0 \"\"\n",
            ),
        ]);
        assert_eq!(
            catalogue
                .cached_states()
                .unwrap()
                .names
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [11, 12, 13]
        );
        assert_eq!(
            installed_content::regions::read(&catalogue.content.layers)
                .unwrap()
                .regions
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [21, 22, 23]
        );
        let content = Content::new(Arc::clone(&catalogue.content), "english");
        for key in ["place-names", "territory-breakdown", "air-superiority"] {
            let mut findings = Vec::new();
            run(key, &content, &mut findings).unwrap();
            assert!(
                findings
                    .iter()
                    .all(|finding| matches!(finding.kind, Kind::Offline | Kind::Content)),
                "{key}: {findings:?}"
            );
        }
    }

    #[test]
    fn operative_content_checks_prepare_roster_and_keep_placements_recording_only() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (operative, _folder) = products::operative_content::test_content(&[
            (
                "common/operations/fixture.txt",
                "synthetic_operation = { name = SYN_NAME }",
            ),
            (
                "localisation/english/fixture_l_english.yml",
                "\u{feff}l_english:\n SYN_NAME:0 \"Authored operation\"\n",
            ),
        ]);
        let content = Content::new(Arc::clone(&operative.catalogue.content), "english");
        let mut findings = Vec::new();
        let placement = run("operative-placements", &content, &mut findings).unwrap();
        assert!(placement.contains("supplying membership read"));
        assert!(content.operatives.get().is_none());
        let roster = run("operatives-roster", &content, &mut findings).unwrap();
        assert!(roster.contains("full-reference character portraits"));
        assert!(content.operatives.get().is_some());
        assert!(std::ptr::eq(
            content.operatives().unwrap(),
            content.operatives().unwrap()
        ));
        let agency = run("agency-comparison", &content, &mut findings).unwrap();
        assert!(agency.contains("AgencyUpgradeLevels"));
        assert!(!agency.contains("not captured"));
    }

    #[test]
    fn equipment_and_autonomy_preparation_reuse_the_same_loaded_objects() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let content = Content::new(
            Arc::new(ResolvedContent {
                layers: installed_content::ResolvedLayers::from_folders(&[]).unwrap(),
                stats: Default::default(),
            }),
            "english",
        );
        assert!(Arc::ptr_eq(
            content.equipment().unwrap(),
            content.equipment().unwrap(),
        ));
        assert!(std::ptr::eq(
            content.autonomy().unwrap(),
            content.autonomy().unwrap(),
        ));
        assert!(std::ptr::eq(
            content.tags().unwrap(),
            content.tags().unwrap()
        ));
        assert!(std::ptr::eq(
            content.ideologies().unwrap(),
            content.ideologies().unwrap()
        ));
    }

    #[test]
    fn reader_kinds_preserve_the_producer_without_reading_message_words() {
        for (producer, expected) in [
            (installed_content::DiagnosticKind::ParseFailure, Kind::ParseFailure),
            (installed_content::DiagnosticKind::Misread, Kind::Misread),
            (installed_content::DiagnosticKind::Unsettled, Kind::Unsettled),
            (installed_content::DiagnosticKind::Unsupported, Kind::Unsupported),
            (installed_content::DiagnosticKind::Refused, Kind::Refused),
            (installed_content::DiagnosticKind::Logged, Kind::Logged),
            (installed_content::DiagnosticKind::Warning, Kind::Warning),
        ] {
            let note = Finding::reader(
                "fixture",
                (producer, "unknown unsupported parse failure".into()),
            );
            assert_eq!(note.kind, expected);
            assert_eq!(note.message, "unknown unsupported parse failure");
        }
    }

    #[test]
    fn offline_reasons_count_distinct_items_with_every_example() {
        let mut findings: Vec<_> = (0..8)
            .map(|index| {
                Finding::new(
                    Kind::Offline,
                    format!("item {index}"),
                    "recorded context unavailable offline",
                )
            })
            .collect();
        findings.push(findings[0].clone());
        findings.push(Finding::new(
            Kind::Unsupported,
            "script",
            "reader cannot handle this form",
        ));
        aggregate(&mut findings);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[1].message, "recorded context unavailable offline");
        assert_eq!(findings[1].count, Some(8));
        assert_eq!(
            findings[1].examples,
            [
                "item 0", "item 1", "item 2", "item 3", "item 4", "item 5", "item 6", "item 7"
            ]
        );
    }

    #[test]
    fn recorded_holder_reasons_do_not_split_by_authored_advisor_name() {
        let reason = "advisor role needs its recorded holder template; unavailable offline";
        let mut findings = vec![
            Finding::grouped(
                Kind::Offline,
                "idea first",
                reason,
                "advisor first's template missing",
            ),
            Finding::grouped(
                Kind::Offline,
                "idea second",
                reason,
                "advisor second's template missing",
            ),
        ];
        aggregate(&mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].message, reason);
        assert_eq!(findings[0].count, Some(2));
        assert_eq!(
            findings[0].examples,
            [
                "idea first: advisor first's template missing",
                "idea second: advisor second's template missing",
            ]
        );
    }
}
