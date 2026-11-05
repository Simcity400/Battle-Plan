//! Every prepared family and recording-wide job, once: its identity, version,
//! status rows, query suppliers, same-hour dependents and sections.

use crate::products;
use dashboard_keyed::Observer as _;
use std::io;

/// A prepared family's stable identity. Its spelling names the family's
/// refusals, progress, coverage pages and status entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FamilyId(&'static str);

impl FamilyId {
    pub(crate) const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn name(self) -> &'static str {
        self.0
    }

    /// The family `spelling` names, by its name or one of its aliases.
    pub fn parse(spelling: &str) -> io::Result<Self> {
        FAMILIES
            .iter()
            .find(|family| family.id.0 == spelling || family.aliases.contains(&spelling))
            .map(|family| family.id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Unknown prepared family {spelling:?}"),
                )
            })
    }

    /// The family `spelling` names, or else the recording-wide job of that name, which a
    /// native preparation runs among the families ([`super::native`]).
    pub fn publisher(spelling: &str) -> io::Result<Self> {
        Self::parse(spelling).or_else(|error| job(spelling).map(|job| Self(job.name)).ok_or(error))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Descriptor {
    pub id: FamilyId,
    /// The version of its algorithm and of what it publishes: raise it when
    /// either changes, and a store keeps every other family and prepares it
    /// again alone, with the families observing it in the same hour.
    pub version: u32,
    /// Other spellings, which tools accept for it.
    pub aliases: &'static [&'static str],
    /// The status rows it reports to.
    pub status: &'static [&'static str],
    /// Products and series kinds whose replies read its pages and answer
    /// with its refusals.
    pub queries: &'static [&'static str],
    /// Products that wait only for its own coverage.
    pub gates: &'static [&'static str],
    /// Families observing its state in the same hour: its refusal refuses them.
    pub observers: &'static [FamilyId],
    /// The namespaces of every section it writes, by the names its writers
    /// build them from: its data, catalogues, continuation and its own reads
    /// of others' state ([`super::plan::Plan::owner`]).
    pub sections: &'static [&'static str],
    /// Whether a native preparation prepares it ahead of every other family, in a run of its own
    /// over the whole recording that decodes its tables alone: a cheap family the interface
    /// reads at any hour, whose last hours do not wait for the union run to reach them.
    pub ahead: bool,
}

/// A family with nothing but its name.
pub(crate) const fn family(name: &'static str) -> Descriptor {
    Descriptor {
        id: FamilyId::new(name),
        version: 1,
        aliases: &[],
        status: &[],
        queries: &[],
        gates: &[],
        observers: &[],
        sections: &[],
        ahead: false,
    }
}

/// Recording-wide work outside the family registry. A native recording's run prepares it
/// among the families when it has a native backend ([`super::native`]), with coverage pages
/// of its own; it is refused by name otherwise. A job with a status row reports to the row of
/// its name.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Job {
    pub name: &'static str,
    /// As a family's [`Descriptor::version`].
    pub version: u32,
    /// Products that wait only for its coverage.
    pub gates: &'static [&'static str],
    /// The namespaces of its sections and of its private fact columns.
    pub sections: &'static [&'static str],
}

/// A product whose replies read the pages of several families up to the
/// hours they answer: it waits for the coverage every supplier has, while
/// any supplier is prepared on its own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Query {
    pub product: &'static str,
    pub suppliers: &'static [FamilyId],
}

const IDENTITIES: FamilyId = FamilyId::new("identities");
const TERRITORY: FamilyId = FamilyId::new("territory");
const EVENT_FEED: FamilyId = FamilyId::new("event_feed");
const GROUPS: FamilyId = FamilyId::new("groups");
const AIR_SUPERIORITY: FamilyId = FamilyId::new("air_superiority");

/// Every family, in the order a query shared by several answers their
/// refusals in.
pub(crate) static FAMILIES: &[Descriptor] = &[
    Descriptor {
        status: &["air-missions"],
        queries: &["air-missions"],
        gates: &["air-missions"],
        // 2: the scan's runs on the segment driver, published as a series root.
        // 3: refused observations on unchanged hours are counted again.
        version: 3,
        ..family("air-missions")
    },
    Descriptor {
        status: &["air-statistics"],
        queries: &["air-statistics"],
        gates: &["air-statistics"],
        // 2: the scan's runs on the segment driver, published as a series root.
        // 3: refused observations on unchanged hours are counted again.
        // 4: its operators observe and hand on what moved; each unit an actor of 26 measures.
        version: 4,
        ..family("air-statistics")
    },
    Descriptor {
        status: &["regional-weather"],
        queries: &["regional-weather"],
        gates: &["regional-weather"],
        // 3: hours certify against the province temperature witness; older recordings refuse.
        // 4: the observer stores empty regions and unavailable provinces apart, so older state is rebuilt.
        // 5: refused observations on unchanged hours are counted again.
        version: 5,
        ..family("regional-weather")
    },
    Descriptor {
        status: &["minefields"],
        queries: &["minefields"],
        gates: &["minefields"],
        // 2: the scan's runs on the segment driver, published as a series root.
        // 3: refused observations on unchanged hours are counted again.
        version: 3,
        ..family("minefields")
    },
    Descriptor {
        status: &["naval-supremacy"],
        queries: &["naval-supremacy"],
        gates: &["naval-supremacy"],
        // 2: the scan's runs on the segment driver, published as a series root.
        // 3: refused observations on unchanged hours are counted again.
        version: 3,
        ..family("naval-supremacy")
    },
    Descriptor {
        status: &["doctrine-mastery"],
        queries: &["doctrine-mastery"],
        gates: &["doctrine-mastery"],
        // 2: the scan's runs on the segment driver, published as a series root.
        // 3: refused observations on unchanged hours are counted again.
        version: 3,
        ..family("doctrine-mastery")
    },
    Descriptor {
        status: &["occupation-reserve"],
        queries: &["occupation-reserve-manpower"],
        gates: &["occupation-reserve-manpower"],
        // 4: every reserve view requires existence at its supplying read.
        // 5: refused observations on unchanged hours are counted again.
        version: 5,
        ..family("occupation-reserve")
    },
    Descriptor {
        status: &["intel-network-coverage"],
        queries: &["intel-network-coverage"],
        gates: &["intel-network-coverage"],
        // 2: the scan's runs on the segment driver, published as a series root.
        // 3: refused observations on unchanged hours are counted again.
        version: 3,
        ..family("intel-network-coverage")
    },
    Descriptor {
        status: &["naval-repairs"],
        queries: &["naval-repairs"],
        gates: &["naval-repairs"],
        // 2: day events on the segment driver, published as a series root.
        version: 2,
        ..family("naval-repairs")
    },
    Descriptor {
        status: &["naval-refits"],
        queries: &["naval-refits"],
        gates: &["naval-refits"],
        // 2: day events on the segment driver, published as a series root.
        version: 2,
        ..family("naval-refits")
    },
    // Interactive products read it hour by hour through its coverage; `country-facts` whole.
    Descriptor {
        status: &["country-styles"],
        queries: &["country-facts"],
        gates: &["country-styles", "country-colours", "country-facts"],
        // Flags and the country picker read each country's last observed hour.
        ahead: true,
        // 2: facts on the segment driver, published as a series root, each run's flag chosen in
        // the recorded mod stack.
        version: 2,
        ..family(products::style_runs::NAME)
    },
    Descriptor {
        status: &["division-equipment"],
        queries: &["division-equipment-history"],
        gates: &[crate::products::division_equipment::history::QUERY],
        // 4: the engine handed on holds its reads alone, and saves its context.
        version: 4,
        ..family("division-equipment")
    },
    Descriptor {
        status: &["faction-state"],
        queries: &["faction-state"],
        gates: &["faction-state"],
        // 2: lanes on the segment driver, published as a series root.
        // 3: stored scripted names: is_country_leader is now evaluated.
        // 4: programmes a native capture holds are read from the change-run index.
        version: 4,
        ..family("faction_state")
    },
    Descriptor {
        status: &["diplomacy-state"],
        queries: &["diplomacy-state"],
        gates: &["diplomacy-state"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        ..family("diplomacy_state")
    },
    Descriptor {
        status: &["focus", "focus-display"],
        queries: &[
            "focus-hour",
            "focus-unavailable",
            "focus-order-page",
            "focus-completions",
            "event-feed",
        ],
        gates: &[
            "focus-hour",
            "focus-unavailable",
            "focus-order-page",
            "focus-completions",
        ],
        // 2: orders, progress and completions on the segment driver, published as a series root.
        // 3: each completion keeps its event text and icon choice, made at its hour.
        version: 3,
        ..family("focus")
    },
    Descriptor {
        status: &["production"],
        queries: &[
            "production-select",
            "production-spending",
            "dockyard-spending",
            "military-factory-spending",
            "scoreboard",
        ],
        gates: &[
            "production-select",
            "production-spending",
            "military-factory-spending",
        ],
        sections: &[
            "production-spent",
            crate::prepared_production::FACTS,
            crate::prepared_production::STATE,
        ],
        version: 8,
        ..family("production")
    },
    Descriptor {
        status: &["country-measure"],
        queries: &["country-measure", "scoreboard"],
        gates: &["country-measure"],
        // 3: tag numbers follow the world's lifetime across starts.
        version: 3,
        ..family(products::country_balances::NAME)
    },
    // The feed observes the identity history, and these queries read it through
    // `country_history::prepared_branch_histories`.
    Descriptor {
        status: &["country-identities"],
        queries: &[
            "country-identities",
            "unit-leaders",
            "world-tension",
            "supply-health",
            "army-size",
            "raids",
            "scoreboard",
            "faction-totals",
            "naval-losses",
            "equipment-losses",
            "occupation-trend",
            "agency-comparison",
            "operatives-roster",
        ],
        gates: &[crate::products::country_history::IDENTITIES_PRODUCT],
        observers: &[EVENT_FEED],
        sections: &["country-identities"],
        ..family("identities")
    },
    Descriptor {
        status: &["army-size", "volunteers-expeditionaries"],
        queries: &["army-size", "volunteers-expeditionaries", "scoreboard"],
        gates: &["volunteers-expeditionaries"],
        // 3: each total starts a segment after its own unknown hour.
        version: 3,
        ..family("army")
    },
    Descriptor {
        status: &["air-force-size"],
        queries: &["air-force-size", "scoreboard"],
        gates: &["air-force-size"],
        // 2: wings on the segment driver, published as a series root.
        version: 2,
        ..family("air")
    },
    Descriptor {
        status: &["navy-size"],
        queries: &["navy-size", "scoreboard"],
        gates: &["navy-size"],
        // 2: ships on the segment driver, published as a series root.
        version: 2,
        ..family("navy")
    },
    Descriptor {
        status: &["country-nukes"],
        queries: &["country-nukes", "scoreboard", "event-feed"],
        // 2: stockpiles on the segment driver, published as a series root.
        version: 2,
        ..family("nukes")
    },
    Descriptor {
        status: &["country-factories"],
        queries: &["country-factories", "scoreboard", "faction-totals"],
        // 2: values on the segment driver, published as a series root.
        version: 2,
        ..family("factories")
    },
    Descriptor {
        status: &["dockyard-repairs"],
        queries: &["dockyard-spending"],
        gates: &["dockyard-spending"],
        sections: &["dockyard-repairs"],
        // 3: native holder inputs; mismatches count country-hours.
        version: 3,
        ..family("repairs")
    },
    Descriptor {
        status: &["supply-health"],
        queries: &["supply-health"],
        // 4: display-step supply units; tag numbers follow the world's lifetime; shares restart after an hour without divisions.
        version: 4,
        ..family("supply")
    },
    Descriptor {
        status: &["equipment-stockpile"],
        queries: &[
            "equipment-stockpile-at",
            "equipment-stockpile-catalogue",
            "equipment-stockpile-chart",
        ],
        gates: &[
            "equipment-stockpile-at",
            "equipment-stockpile-catalogue",
            "equipment-stockpile-chart",
        ],
        // 2: snapshot events on the segment driver, published as a series root.
        version: 2,
        ..family("stockpile")
    },
    // Air superiority reads the group tables with the groups' scan in the same hour, and its
    // replies read the groups' pages.
    Descriptor {
        status: &["country-groups"],
        queries: &["country-groups", "faction-totals", "event-feed", "air-superiority"],
        gates: &["country-groups"],
        observers: &[AIR_SUPERIORITY],
        sections: &[
            "country-groups",
            "country-group-changes",
            "country-group-state",
        ],
        // 2: an unchanged hour no longer repeats the last hour's boundary refusals.
        // 3: the operator hands on what moved of the rows the scan reads, not the tables.
        version: 3,
        ..family("groups")
    },
    Descriptor {
        status: &["military-air-ledger"],
        queries: &["military-air-ledger"],
        gates: &["military-air-ledger"],
        // 2: events on the segment driver, published as a series root.
        version: 2,
        ..family("air_ledger")
    },
    Descriptor {
        status: &["air-aces"],
        queries: &["air-aces", "air-ace-portraits"],
        gates: &["air-aces", "air-ace-portraits"],
        sections: &["air-aces"],
        // 2: on the segment driver, its pages published as the walk published them.
        // 3: its operators hand on each read's changed aces and named designs, never tables.
        version: 3,
        ..family("aces")
    },
    Descriptor {
        status: &["raids"],
        queries: &["raids"],
        gates: &["raids"],
        // 2: on the segment driver, published as a series root.
        version: 2,
        ..family("raids")
    },
    Descriptor {
        status: &["air-superiority"],
        queries: &["air-superiority"],
        sections: &["air-superiority"],
        // 3: on the segment driver: its operators hand on what moved, never the score table.
        // 4: each segment's operator reads the groups and hands on the day's roster; its replies
        // read the groups' pages, never a read of its own.
        version: 4,
        ..family("air_superiority")
    },
    Descriptor {
        status: &["game-score"],
        queries: &["game-score"],
        gates: &["game-score"],
        // 2: scores on the segment driver, published as a series root.
        // 3: each score part breaks after an unknown hour.
        version: 3,
        ..family("game_score")
    },
    Descriptor {
        status: &["construction"],
        queries: &[
            "construction-log",
            "construction-log-manifest",
            "construction-spending",
        ],
        gates: &[
            "construction-log",
            "construction-log-manifest",
            "construction-spending",
        ],
        // 3: output-only projects and dated census metadata.
        version: 3,
        ..family("construction")
    },
    Descriptor {
        status: &["resource-balance"],
        queries: &["resource-balance"],
        // Its replies wait on its own coverage, not the walk's.
        gates: &["resource-balance"],
        // 2: pools and deals on the segment driver, published as a series root.
        version: 2,
        ..family("resources")
    },
    Descriptor {
        status: &["trade-flows"],
        queries: &["trade-flows"],
        // 2: flows on the segment driver, published as a series root.
        version: 2,
        ..family("trade")
    },
    Descriptor {
        // 2: battles on the segment driver, their pages published block by block.
        version: 2,
        status: &["land-battles"],
        queries: &[
            "land-battles",
            "land-battles-manifest",
            "land-battles-ranking",
            "land-battle-course",
        ],
        gates: &[
            "land-battles",
            "land-battles-manifest",
            "land-battles-ranking",
            "land-battle-course",
        ],
        sections: &["land-battle", "land-battles"],
        ..family("land")
    },
    Descriptor {
        status: &["naval-battles"],
        queries: &[
            "naval-battles",
            "naval-battles-manifest",
            "naval-battles-ranking",
            "naval-battle-details",
            "ships-sunk",
            "ships-sunk-manifest",
            "ships-sunk-battle-titles",
            "naval-losses",
        ],
        gates: &[
            "naval-battles",
            "naval-battles-manifest",
            "naval-battles-ranking",
            "naval-battle-details",
            "ships-sunk",
            "ships-sunk-manifest",
            "ships-sunk-battle-titles",
            "naval-losses",
        ],
        // Battles' and ships sunk's window pages; naval losses publish on its series.
        sections: &["naval-battle", "naval-battles", "ships-sunk", "sinking"],
        // 2: on the segment driver, naval losses published as a series root.
        version: 2,
        ..family("naval")
    },
    Descriptor {
        aliases: &["division-log"],
        status: &["division-log"],
        queries: &[
            "division-log",
            "division-log-manifest",
            "division-log-totals",
        ],
        gates: &[
            "division-log",
            "division-log-manifest",
            "division-log-totals",
        ],
        sections: &["division-log"],
        // 2: on the segment driver, its pages published as the walk published them.
        version: 2,
        ..family("divisions")
    },
    Descriptor {
        status: &["subjects"],
        queries: &["subjects", "event-feed"],
        gates: &["subjects"],
        // 3: known coverage includes unobserved runs across recording gaps.
        version: 3,
        ..family("subjects")
    },
    Descriptor {
        status: &["autonomy"],
        queries: &["autonomy"],
        // 2: scores and bands on the segment driver, published as a series root.
        version: 2,
        ..family("autonomy")
    },
    Descriptor {
        status: &["world-tension"],
        queries: &["world-tension"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        ..family("tension")
    },
    Descriptor {
        status: &["wars"],
        queries: &["wars"],
        gates: &["wars"],
        // 3: change inputs, counters and participation parts on the segment driver, published
        // as a series root. 4: inputs and parts packed as only their own tables. 5: prepared
        // layers and each viewpoint's groups instead of change inputs. 6: labels' refusals
        // without their hour, and the hours naming reads again marked on coverage.
        version: 6,
        ..family("wars")
    },
    Descriptor {
        status: &["idea-history"],
        queries: &["idea-history", "event-feed"],
        gates: &["idea-history"],
        // 2: holdings on the segment driver, published as a series root; additions are
        // classified as laws when read.
        // 3: naming follows civil-war inputs, war end hours and reserved slot-zero tags.
        version: 3,
        ..family(products::ideas::NAME)
    },
    Descriptor {
        status: &["event-feed"],
        queries: &["event-feed", "event-choices", "decisions", "civil-wars"],
        // The event-choice and decision lists read the feed's own facts alone.
        gates: &["event-choices", "decisions"],
        sections: &["event-feed"],
        // 2: facts presented in the walk, stored MessagePack behind their opener; recorder
        // observations removed.
        // 3: a script event's picture is kept as its sprite key, drawn as the reply shows it.
        version: 3,
        ..family("event_feed")
    },
    Descriptor {
        status: &["party-history"],
        queries: &["party-history"],
        gates: &["party-history"],
        // 2: parties on the segment driver, published as a series root.
        version: 2,
        ..family(products::party_history::NAME)
    },
    Descriptor {
        status: &["fuel-branches"],
        queries: &["fuel-branches"],
        gates: &[crate::products::fuel_branches::QUERY],
        // 2: consumers on the segment driver, published as a series root.
        version: 2,
        ..family("fuel_branches")
    },
    Descriptor {
        status: &["market"],
        queries: &["market-history", "market-ledger"],
        gates: &[
            crate::products::market::HISTORY,
            crate::products::market::LEDGER,
        ],
        // 2: a row whose buyer or receiver has no identity stands, those hours dated.
        // 3: on the segment driver, published as a series root; refusals count the hours held.
        version: 4,
        ..family("market")
    },
    Descriptor {
        status: &["lend-lease-balance"],
        queries: &["lend-lease-balance"],
        gates: &[crate::products::lend_lease_balance::QUERY],
        // 2: histories on the segment driver, published as a series root.
        version: 2,
        ..family("lend_lease_balance")
    },
    Descriptor {
        status: &["production-catalogue"],
        queries: &["production-catalogue"],
        gates: &[crate::products::production_catalogue::QUERY],
        // 3: its domains on the segment driver, published as a series root; each refusal
        // counted per hour it held, its reason without the hour.
        version: 3,
        ..family("production_catalogue")
    },
    Descriptor {
        status: &["convoys"],
        queries: &["convoy-health"],
        gates: &[crate::products::convoy_health::QUERY],
        sections: &[products::convoy_health::FAMILY],
        // 2: the keyed census on the segment driver, its pages published as the walk's were.
        version: 2,
        ..family("convoys")
    },
    Descriptor {
        status: &["politics-state"],
        queries: &["politics-state-history", "civil-wars", "country-people"],
        gates: &[crate::products::politics_state::history::QUERY],
        // 3: its parts on the segment driver, published as a series root; each refusal counted
        // per hour it held.
        version: 3,
        ..family("politics_state")
    },
    Descriptor {
        status: &["unit-leaders"],
        queries: &["unit-leaders"],
        gates: &["unit-leaders"],
        sections: &["unit-leaders"],
        // 2: replies and saved scans hold per-leader withheld refusals.
        // 3: unit leader visibility: is_country_leader is now evaluated.
        version: 3,
        ..family("leaders")
    },
    Descriptor {
        status: &["country-stability"],
        queries: &["country-stability", "scoreboard"],
        gates: &["country-stability"],
        // 3: tag numbers follow the world's lifetime across starts.
        version: 3,
        ..family("stability")
    },
    Descriptor {
        status: &["stability-parts"],
        queries: &["stability-parts"],
        gates: &["stability-parts"],
        // 4: tag numbers follow the world's lifetime across starts.
        version: 4,
        ..family("parts")
    },
    // Its series queries route by the family kind `country-experience`.
    Descriptor {
        status: &["country-experience"],
        queries: &["country-experience"],
        gates: &["country-experience"],
        // 3: tag numbers follow the world's lifetime across starts.
        version: 3,
        ..family("experience")
    },
    Descriptor {
        status: &["politics-ledger"],
        queries: &["politics-ledger"],
        gates: &["politics-ledger"],
        // 2: facts on the segment driver, published as a series root.
        // 3: stored names and icon choices: is_country_leader is now evaluated.
        // 4: rows, items and holds stored as replies show them; presence by tag.
        // 5: only what widget replies read is stored: no capture coverage, revisions or zero costs.
        version: 5,
        ..family("politics_ledger")
    },
    Descriptor {
        status: &["country-fuel"],
        queries: &["country-fuel"],
        gates: &["country-fuel"],
        // 2: facts on the segment driver, published as a series root.
        // 3: the detail products' recipients and demand packs published with it.
        version: 3,
        ..family("fuel")
    },
    // Totals, ranks and regions on the segment driver, the native owned-state counter with them.
    Descriptor {
        status: &["territory"],
        queries: &["territory", "scoreboard", "faction-totals"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        ..family("territory")
    },
    // The windows and manifest, and the trend's series by the family kind
    // `occupation-trend` (whose query also waits for the identities).
    Descriptor {
        status: &["occupation"],
        queries: &[
            "occupied-states",
            "occupied-states-manifest",
            "occupation-trend",
        ],
        gates: &[
            "occupied-states",
            "occupied-states-manifest",
            "occupation-trend",
        ],
        // 2: on the segment driver, published as a series root.
        // 3: occupation law eligibility: is_country_leader is now evaluated.
        version: 3,
        ..family("occupation")
    },
    Descriptor {
        status: &[
            "decryption",
            "operative-placements",
            "agency-comparison",
            "operatives-roster",
            "espionage-log",
        ],
        queries: &[
            "decryption",
            "operative-placements",
            "agency-comparison",
            "operatives-roster",
            "espionage-log",
        ],
        gates: &[
            "decryption",
            "operative-placements",
            "agency-comparison",
            "operatives-roster",
            "espionage-log",
        ],
        sections: &[
            "agency-comparison",
            "decryption",
            "espionage-log",
            "operative-placements",
            "operatives-roster",
        ],
        // 2: its observers on the segment driver, their pages published as the walk did.
        // 3: its operators hand on what their reads changed, never tables beside the countries.
        version: 3,
        ..family("intelligence")
    },
    // Its series and events route by the family kind `research`.
    Descriptor {
        status: &["research"],
        queries: &["research"],
        gates: &["research"],
        // 2: a read of no slots is a known empty slot set.
        // 3: on the segment driver, published as a series root with its grants.
        version: 3,
        ..family("research")
    },
    Descriptor {
        status: &["peace-deals"],
        queries: &["peace-deals"],
        gates: &["peace-deals"],
        // 2: conferences and outcomes on the segment driver, published as a series root.
        // 3: the operator hands on what the observer's reads changed, not the tables.
        version: 3,
        ..family("peace")
    },
    // The event feed reads the projects' published completions when asked,
    // and refuses with them from this family's own page.
    Descriptor {
        status: &["special-projects"],
        queries: &["special-projects", "event-feed"],
        gates: &["special-projects"],
        // 2: observations and completions on the segment driver, published as a series root.
        // 3: stored project names: is_country_leader is now evaluated.
        // 4: each country's episodes built in preparation as a ledger; events stored binary.
        version: 4,
        ..family("projects")
    },
    // Its series queries route by the family kind `war-casualties`.
    Descriptor {
        status: &["war-casualties"],
        queries: &[
            "war-casualties",
            "casualty-breakdown",
            "casualty-pairs",
            "scoreboard",
        ],
        gates: &["war-casualties", "casualty-pairs", "casualty-breakdown"],
        sections: &["casualty", "war-casualties"],
        // 2: on the segment driver, its pages published as the walk published them.
        version: 2,
        ..family("casualties")
    },
    Descriptor {
        status: &["equipment-losses"],
        queries: &["equipment-losses"],
        sections: &["equipment-loss"],
        // 2: its operators hand on compact reads, and its fold runs in shards by country.
        version: 2,
        ..family("equipment_losses")
    },
    // Provinces' and states' names at hours: their name inputs as events on the segment driver.
    Descriptor {
        status: &["place-names"],
        queries: &["place-names"],
        gates: &["place-names"],
        ..family(products::place_names::NAME)
    },
    // A country's manpower view: its operands as runs on the segment driver.
    Descriptor {
        status: &["country-people"],
        queries: &["country-people"],
        gates: &["country-people"],
        ..family(products::country_people::NAME)
    },
];

/// Every recording-wide job.
pub(crate) static JOBS: &[Job] = &[
    Job {
        name: "map",
        version: 1,
        gates: &[],
        sections: &[],
    },
    // Prepared among the families by its native backend, its catalogue waiting for its coverage.
    Job {
        name: "land-owners",
        version: 1,
        gates: &["land-owners"],
        sections: &["land-owners"],
    },
    Job {
        name: "production-lines",
        // 2: its operators hand on census deltas, and its fold runs in shards by country.
        version: 2,
        gates: &[
            "production-lines",
            "production-line-detail",
            "production-line-breakdown",
        ],
        sections: products::production_lines::SECTIONS,
    },
];

/// Every product whose coverage several families supply. Its refusals are
/// those of the families listing it in [`Descriptor::queries`].
pub(crate) static QUERIES: &[Query] = &[
    Query {
        product: "operatives-roster",
        suppliers: &[FamilyId::new("intelligence"), IDENTITIES],
    },
    Query {
        product: "agency-comparison-identities",
        suppliers: &[FamilyId::new("intelligence"), IDENTITIES],
    },
    Query {
        product: "raids-tally",
        suppliers: &[FamilyId::new("raids"), IDENTITIES],
    },
    Query {
        product: "dockyard-spending",
        suppliers: &[FamilyId::new("production"), FamilyId::new("repairs")],
    },
    // Named through the identity history up to their coverage.
    Query {
        product: "country-factories",
        suppliers: &[FamilyId::new("factories"), IDENTITIES],
    },
    Query {
        product: "country-nukes",
        suppliers: &[FamilyId::new("nukes"), IDENTITIES],
    },
    Query {
        product: "autonomy",
        suppliers: &[FamilyId::new("autonomy"), IDENTITIES],
    },
    Query {
        product: "supply-health",
        suppliers: &[FamilyId::new("supply"), IDENTITIES],
    },
    Query {
        product: "army-size",
        suppliers: &[FamilyId::new("army"), IDENTITIES],
    },
    Query {
        product: "air-force-size",
        suppliers: &[FamilyId::new("air"), IDENTITIES],
    },
    Query {
        product: "navy-size",
        suppliers: &[FamilyId::new("navy"), IDENTITIES],
    },
    // Every family with a scoreboard column, and the identities naming its rows.
    Query {
        product: "scoreboard",
        suppliers: &[
            FamilyId::new("air"),
            FamilyId::new("army"),
            FamilyId::new("casualties"),
            FamilyId::new("factories"),
            IDENTITIES,
            FamilyId::new("measure"),
            FamilyId::new("navy"),
            FamilyId::new("nukes"),
            FamilyId::new("production"),
            FamilyId::new("stability"),
            TERRITORY,
        ],
    },
    Query {
        product: "equipment-losses",
        suppliers: &[FamilyId::new("equipment_losses"), IDENTITIES],
    },
    Query {
        product: "territory",
        suppliers: &[TERRITORY, IDENTITIES],
    },
    // Factory faction totals read the faction rosters, the identity history
    // and the factories (dense factory summaries read territory too, and
    // wait for its coverage through their window).
    Query {
        product: crate::products::faction_totals::FACTORIES_GATE,
        suppliers: &[GROUPS, IDENTITIES, FamilyId::new("factories")],
    },
    // Air superiority's samples are classified and named by the groups' pages.
    Query {
        product: "air-superiority",
        suppliers: &[AIR_SUPERIORITY, GROUPS],
    },
    // Casualty lifetimes joined with the wars' app continuity.
    Query {
        product: "casualty-breakdown",
        suppliers: &[FamilyId::new("casualties"), FamilyId::new("wars")],
    },
    // Territory faction totals read the faction rosters, the identity
    // history and territory's pages.
    Query {
        product: <crate::products::faction_totals::FactionTotals as crate::products::Product>::KEY,
        suppliers: &[GROUPS, IDENTITIES, TERRITORY],
    },
    // A country's manpower view with its holder's elections and exiles.
    Query {
        product: "country-people",
        suppliers: &[
            FamilyId::new(products::country_people::NAME),
            FamilyId::new("politics_state"),
        ],
    },
    // Civil-war lists with the feed's annexations as outcome evidence.
    Query {
        product: "civil-wars",
        suppliers: &[FamilyId::new("politics_state"), EVENT_FEED],
    },
    // The feed's own facts and the changes it merges with them.
    Query {
        product: "event-feed",
        suppliers: &[
            EVENT_FEED,
            GROUPS,
            FamilyId::new("subjects"),
            FamilyId::new("ideas"),
            FamilyId::new("focus"),
            FamilyId::new("nukes"),
            FamilyId::new("projects"),
        ],
    },
    // Contributors named through the identity history up to their coverage.
    Query {
        product: "world-tension",
        suppliers: &[FamilyId::new("tension"), IDENTITIES],
    },
    // Named through the identity history up to their coverage.
    Query {
        product: "trade-flows",
        suppliers: &[FamilyId::new("trade"), IDENTITIES],
    },
];

/// Every status row, in the order preparation reports them.
pub(crate) static STATUS: &[(&str, &str)] = &[
    ("focus", "National focus"),
    ("focus-display", "Focus titles and icons"),
    ("production", "Produced equipment"),
    ("map", "Campaign map"),
    ("country-measure", "Country balances"),
    ("country-identities", "Country identities"),
    ("army-size", "Army size"),
    (
        "volunteers-expeditionaries",
        "Volunteers and expeditionaries",
    ),
    ("air-force-size", "Air force size"),
    ("navy-size", "Navy size"),
    ("country-nukes", "Nuclear arsenal"),
    ("country-factories", "Factories"),
    ("dockyard-repairs", "Dockyard repairs"),
    ("supply-health", "Supply health"),
    ("equipment-stockpile", "Equipment stockpile"),
    ("country-groups", "Country groups"),
    ("military-air-ledger", "Aircraft losses and bombing damage"),
    ("air-aces", "Aces and portraits"),
    ("raids", "Raid episodes and tallies"),
    ("air-superiority", "Regional air scores"),
    ("game-score", "Game score"),
    ("construction", "Construction"),
    ("resource-balance", "Resources"),
    ("trade-flows", "Trade flows"),
    ("land-battles", "Land battles"),
    ("naval-battles", "Naval battles"),
    ("division-log", "Division log"),
    ("subjects", "Subject relations"),
    ("autonomy", "Subject autonomy"),
    ("world-tension", "World tension"),
    ("wars", "Wars"),
    ("idea-history", "Ideas and advisors"),
    ("event-feed", "Event feed"),
    ("party-history", "Parties and leaders"),
    ("politics-state", "Elections, exiles and balances of power"),
    ("unit-leaders", "Generals and admirals"),
    ("country-stability", "Stability and war support"),
    ("stability-parts", "Stability and war support parts"),
    ("country-experience", "Experience"),
    ("politics-ledger", "Political and command power accounting"),
    (
        "diplomacy-state",
        "Opinions, pacts and diplomatic proposals",
    ),
    ("country-fuel", "Fuel and consumption"),
    ("fuel-branches", "Fuel by military branch"),
    ("market", "International market and lend-lease"),
    ("lend-lease-balance", "Lend-lease balances"),
    (
        "production-catalogue",
        "Licences, industrial organisations and designs",
    ),
    ("convoys", "Convoy and trade efficiency"),
    ("territory", "Territory and regions"),
    ("occupation", "Occupied states and occupation trends"),
    ("land-owners", "Land-owning country catalogue"),
    ("decryption", "Decryption"),
    ("operative-placements", "Operative placements"),
    ("agency-comparison", "Agency comparison"),
    ("operatives-roster", "Operatives roster"),
    ("espionage-log", "Espionage operations"),
    ("research", "Research"),
    ("peace-deals", "Peace deals and territory transfers"),
    ("special-projects", "Special projects"),
    ("war-casualties", "Casualties"),
    ("equipment-losses", "Equipment losses"),
    ("air-missions", "Air missions"),
    ("air-statistics", "Air statistics"),
    ("regional-weather", "Regional weather"),
    ("minefields", "Minefields"),
    ("naval-supremacy", "Naval supremacy"),
    ("doctrine-mastery", "Doctrine mastery"),
    ("occupation-reserve", "Occupation reserve manpower"),
    ("intel-network-coverage", "Intel network coverage"),
    ("naval-repairs", "Naval repairs"),
    ("naval-refits", "Refits"),
    ("country-styles", "Country names, colours and flags"),
    ("division-equipment", "Division equipment"),
];

/// The family named `name`.
#[cfg(test)]
pub(crate) fn family_named(name: &str) -> Option<&'static Descriptor> {
    FAMILIES.iter().find(|family| family.id.0 == name)
}

/// The job named `name`.
pub(crate) fn job(name: &str) -> Option<&'static Job> {
    JOBS.iter().find(|job| job.name == name)
}

/// A family's coverage page for one branch, with its refusals there.
pub(crate) fn coverage_section(family: &str, branch: u64) -> String {
    dashboard_base::prepared::coverage::family_section(family, branch)
}

/// The family or job whose coverage page `section` is.
pub(crate) fn coverage_owner(section: &str) -> Option<&str> {
    dashboard_base::prepared::coverage::family_of(section).map(|(name, _)| name)
}

/// A family's continuation, which it takes up again from.
pub(crate) fn continuation_section(family: &str, branch: u64) -> String {
    format!("{family}{CONTINUATION}{branch}")
}

const CONTINUATION: &str = "-continuation-";

/// The family whose continuation `section` names.
pub(crate) fn continuation_owner(section: &str) -> Option<&str> {
    let (name, branch) = section.rsplit_once(CONTINUATION)?;
    digits(branch).then_some(name)
}

/// Sections only the commit coordinator writes: the plan mark and the recording's metadata.
pub(crate) const COORDINATOR: &[&str] = &[
    dashboard_base::prepared::coverage::PLAN,
    "source-metadata-v1",
    "source-metadata-refusals-v1",
];

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}
