//! Every prepared family and recording-wide job, once: its identity, version,
//! status rows, query suppliers, same-hour dependents, private reads,
//! sections, tables, content and the backends able to prepare it.

use crate::products::{self, parts_race_prepared::Family as _};
use dashboard_keyed::Observer as _;
use replay_data::rows::TableId;
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
}

/// Whether preparing a family reads the recorded mod stack's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Content {
    Free,
    Bound,
}

/// A family's same-hour state that a reader observes in the walk. While the
/// family is prepared on its own and a reader is still in the walk, the walk
/// keeps its own read under the reader's names (published, kept).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PrivateRead {
    pub readers: &'static [FamilyId],
    pub kept: &'static [(&'static str, &'static str)],
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
    /// Products that wait only for its own coverage while another backend
    /// than the walk prepares it.
    pub gates: &'static [&'static str],
    /// Families observing its state in the same hour: its refusal refuses them.
    pub observers: &'static [FamilyId],
    pub private: Option<PrivateRead>,
    /// The namespaces of every section it writes, by the names its writers
    /// build them from: its data, catalogues, continuation and its own reads
    /// of others' state ([`super::plan::Plan::owner`]).
    pub sections: &'static [&'static str],
    /// Whether the hourly walk can prepare it.
    pub walk: bool,
    /// Whether the segment driver prepares it: its operators read the shared walk world and
    /// its emit writes its series publication ([`dashboard_base::prepared::roots`]), which
    /// its queries read. The hourly walk never observes it.
    pub segments: bool,
    /// Whether a native preparation prepares it ahead of every other family, in a run of its own
    /// over the whole recording that decodes its tables alone: a cheap family the interface
    /// reads at any hour, whose last hours do not wait for the union run to reach them.
    pub ahead: bool,
    /// The tables the hourly walk's stream decodes every hour for it.
    pub walk_tables: &'static [&'static [TableId]],
    pub content: Content,
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
        private: None,
        sections: &[],
        walk: false,
        segments: false,
        ahead: false,
        walk_tables: &[],
        content: Content::Free,
    }
}

/// Where a recording-wide job runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Runner {
    /// The hourly walk runs it with every accepted hour.
    Walk,
    /// It runs on a thread of its own beside the families and publishes
    /// through their group commits, with coverage pages of its own.
    Beside,
}

/// Recording-wide work outside the family registry: its failure stops
/// preparation rather than refusing a branch. A job with a status row
/// reports to the row of its name.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Job {
    pub name: &'static str,
    /// As a family's [`Descriptor::version`].
    pub version: u32,
    pub runner: Runner,
    /// Products that wait only for its coverage.
    pub gates: &'static [&'static str],
    /// The namespaces of its sections and of its private fact columns.
    pub sections: &'static [&'static str],
    /// The tables the hourly walk's stream decodes every hour for it.
    pub tables: &'static [&'static [TableId]],
    pub content: Content,
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
        sections: &[products::air_missions::AirMissionsObserver::NAME],
        ..family("air-missions")
    },
    Descriptor {
        status: &["air-statistics"],
        queries: &["air-statistics"],
        gates: &["air-statistics"],
        sections: &[products::air_statistics::AirStatisticsObserver::NAME],
        ..family("air-statistics")
    },
    Descriptor {
        status: &["regional-weather"],
        queries: &["regional-weather"],
        gates: &["regional-weather"],
        sections: &[products::regional_weather::block::Observer::NAME],
        content: Content::Bound,
        ..family("regional-weather")
    },
    Descriptor {
        status: &["minefields"],
        queries: &["minefields"],
        gates: &["minefields"],
        sections: &[products::minefields::MinefieldsObserver::NAME],
        ..family("minefields")
    },
    Descriptor {
        status: &["naval-supremacy"],
        queries: &["naval-supremacy"],
        gates: &["naval-supremacy"],
        sections: &[products::naval_supremacy::NavalSupremacyObserver::NAME],
        ..family("naval-supremacy")
    },
    Descriptor {
        status: &["doctrine-mastery"],
        queries: &["doctrine-mastery"],
        gates: &["doctrine-mastery"],
        sections: &[products::doctrine_mastery::DoctrineMasteryObserver::NAME],
        ..family("doctrine-mastery")
    },
    Descriptor {
        status: &["occupation-reserve"],
        queries: &["occupation-reserve-manpower"],
        gates: &["occupation-reserve-manpower"],
        sections: &[products::occupation_reserve::OccupationReserveObserver::NAME],
        ..family("occupation-reserve")
    },
    Descriptor {
        status: &["intel-network-coverage"],
        queries: &["intel-network-coverage"],
        gates: &["intel-network-coverage"],
        sections: &[products::intel_coverage::IntelCoverageObserver::NAME],
        ..family("intel-network-coverage")
    },
    Descriptor {
        status: &["naval-repairs"],
        queries: &["naval-repairs"],
        gates: &["naval-repairs"],
        // 2: day events on the segment driver, published as a series root.
        version: 2,
        segments: true,
        ..family("naval-repairs")
    },
    Descriptor {
        status: &["naval-refits"],
        queries: &["naval-refits"],
        gates: &["naval-refits"],
        // 2: day events on the segment driver, published as a series root.
        version: 2,
        segments: true,
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
        segments: true,
        content: Content::Bound,
        ..family(products::style_runs::NAME)
    },
    Descriptor {
        status: &["division-equipment"],
        queries: &["division-equipment-history"],
        gates: &[crate::products::division_equipment::history::QUERY],
        // 2: per-division parts on the segment driver, published as a series root.
        version: 2,
        segments: true,
        content: Content::Bound,
        ..family("division-equipment")
    },
    Descriptor {
        status: &["faction-state"],
        queries: &["faction-state"],
        gates: &["faction-state"],
        // Each faction's goals are a catalogue of their own.
        sections: &[products::faction_state::prepared::FAMILY, "faction-goals"],
        content: Content::Bound,
        ..family("faction_state")
    },
    Descriptor {
        status: &["diplomacy-state"],
        queries: &["diplomacy-state"],
        gates: &["diplomacy-state"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        segments: true,
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
        sections: &["focus"],
        walk: true,
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
        walk: true,
        version: 2,
        ..family("production")
    },
    Descriptor {
        status: &["country-measure"],
        queries: &["country-measure", "scoreboard"],
        sections: &["measure"],
        walk: true,
        walk_tables: &[crate::prepared_measure::TABLES],
        ..family("measure")
    },
    // Territory's owned-state observation is the only driver of the identity
    // history; the feed observes it, and these queries read it through
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
        observers: &[TERRITORY, EVENT_FEED],
        // The feed makes its own read of the history prepared on its own.
        private: Some(PrivateRead {
            readers: &[TERRITORY],
            kept: &[("country-identities", "territory-identities")],
        }),
        sections: &["country-identities"],
        walk: true,
        ..family("identities")
    },
    Descriptor {
        status: &["army-size", "volunteers-expeditionaries"],
        queries: &["army-size", "volunteers-expeditionaries", "scoreboard"],
        gates: &["volunteers-expeditionaries"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        segments: true,
        ..family("army")
    },
    Descriptor {
        status: &["air-force-size"],
        queries: &["air-force-size", "scoreboard"],
        gates: &["air-force-size"],
        // The parts race names every section of the air force by it.
        sections: &[products::air_force::prepared::Air::NAME],
        walk: true,
        content: Content::Bound,
        ..family("air")
    },
    Descriptor {
        status: &["navy-size"],
        queries: &["navy-size", "scoreboard"],
        gates: &["navy-size"],
        sections: &[products::navy::prepared::Navy::NAME],
        walk: true,
        content: Content::Bound,
        ..family("navy")
    },
    Descriptor {
        status: &["country-nukes"],
        queries: &["country-nukes", "scoreboard", "event-feed"],
        // 2: stockpiles on the segment driver, published as a series root.
        version: 2,
        segments: true,
        ..family("nukes")
    },
    Descriptor {
        status: &["country-factories"],
        queries: &["country-factories", "scoreboard", "faction-totals"],
        sections: &["factories"],
        walk: true,
        ..family("factories")
    },
    Descriptor {
        status: &["dockyard-repairs"],
        queries: &["dockyard-spending"],
        gates: &["dockyard-spending"],
        sections: &["dockyard-repairs"],
        walk: true,
        ..family("repairs")
    },
    Descriptor {
        status: &["supply-health"],
        queries: &["supply-health"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        segments: true,
        content: Content::Bound,
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
        segments: true,
        content: Content::Bound,
        ..family("stockpile")
    },
    // Air superiority reads the groups in the same hour; prepared on its own,
    // it makes that read itself.
    Descriptor {
        status: &["country-groups"],
        queries: &["country-groups", "faction-totals", "event-feed"],
        gates: &["country-groups"],
        observers: &[AIR_SUPERIORITY],
        private: Some(PrivateRead {
            readers: &[AIR_SUPERIORITY],
            kept: &[
                ("country-groups", "air-superiority-groups"),
                ("country-group-changes", "air-superiority-group-changes"),
            ],
        }),
        sections: &[
            "country-groups",
            "country-group-changes",
            "country-group-state",
        ],
        walk: true,
        content: Content::Bound,
        ..family("groups")
    },
    Descriptor {
        status: &["military-air-ledger"],
        queries: &["military-air-ledger"],
        gates: &["military-air-ledger"],
        sections: &[
            "military-air-ledger",
            "military-air-losses",
            "military-bombing",
        ],
        walk: true,
        ..family("air_ledger")
    },
    Descriptor {
        status: &["air-aces"],
        queries: &["air-aces", "air-ace-portraits"],
        gates: &["air-aces", "air-ace-portraits"],
        sections: &["air-aces"],
        walk: true,
        content: Content::Bound,
        ..family("aces")
    },
    Descriptor {
        status: &["raids"],
        queries: &["raids"],
        gates: &["raids"],
        sections: &["raids"],
        walk: true,
        content: Content::Bound,
        ..family("raids")
    },
    Descriptor {
        status: &["air-superiority"],
        queries: &["air-superiority"],
        gates: &["air-superiority"],
        sections: &["air-superiority"],
        walk: true,
        content: Content::Bound,
        version: 2,
        ..family("air_superiority")
    },
    Descriptor {
        status: &["game-score"],
        queries: &["game-score"],
        sections: &["game-score"],
        walk: true,
        walk_tables: &[
            crate::products::game_score::SCORE_TABLES,
            &installed_content_country_names::country_names::TABLES,
        ],
        content: Content::Bound,
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
        sections: &["construction"],
        walk: true,
        content: Content::Bound,
        ..family("construction")
    },
    Descriptor {
        status: &["resource-balance"],
        queries: &["resource-balance"],
        sections: &[
            "resource-balance",
            "resource-deals",
            "resource-known",
            "resource-pools",
            "resource-values",
        ],
        walk: true,
        content: Content::Bound,
        ..family("resources")
    },
    Descriptor {
        status: &["trade-flows"],
        queries: &["trade-flows"],
        gates: &["trade-flows"],
        sections: &["trade"],
        walk: true,
        content: Content::Bound,
        ..family("trade")
    },
    Descriptor {
        status: &["land-battles"],
        queries: &[
            "land-battles",
            "land-battles-manifest",
            "land-battles-ranking",
            "land-battle-course",
        ],
        sections: &["land-battle", "land-battles"],
        walk: true,
        walk_tables: &[&crate::products::land_battles::CENSUS_TABLES],
        content: Content::Bound,
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
        sections: &[
            "naval-battle",
            "naval-battles",
            "naval-loss",
            "naval-losses",
            "ships-sunk",
            "sinking",
        ],
        walk: true,
        content: Content::Bound,
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
        walk: true,
        content: Content::Bound,
        ..family("divisions")
    },
    Descriptor {
        status: &["subjects"],
        queries: &["subjects", "event-feed"],
        gates: &["subjects"],
        sections: &["subjects", "subject-changes", "subject-state"],
        walk: true,
        ..family("subjects")
    },
    Descriptor {
        status: &["autonomy"],
        queries: &["autonomy"],
        sections: &["autonomy"],
        walk: true,
        walk_tables: &[crate::products::autonomy::TABLES],
        ..family("autonomy")
    },
    Descriptor {
        status: &["world-tension"],
        queries: &["world-tension"],
        gates: &["world-tension"],
        sections: &["world-tension", "tension-series"],
        walk: true,
        ..family("tension")
    },
    Descriptor {
        status: &["wars"],
        queries: &["wars"],
        gates: &["wars"],
        sections: &["wars"],
        walk: true,
        content: Content::Bound,
        // 2: memberships carry their orientation, and no resume points are published.
        version: 2,
        ..family("wars")
    },
    Descriptor {
        status: &["idea-history"],
        queries: &["idea-history", "event-feed"],
        gates: &["idea-history"],
        sections: &["idea-history", "idea-changes", "idea-state"],
        walk: true,
        content: Content::Bound,
        ..family("ideas")
    },
    Descriptor {
        status: &["event-feed"],
        queries: &["event-feed", "event-choices", "decisions", "civil-wars"],
        // The event-choice and decision lists read the feed's own facts alone.
        gates: &["event-choices", "decisions"],
        sections: &["event-feed"],
        walk: true,
        ..family("event_feed")
    },
    Descriptor {
        status: &["party-history"],
        queries: &["party-history"],
        gates: &["party-history"],
        sections: &["party-history", "party-ruling", "party-series"],
        walk: true,
        ..family("parties")
    },
    Descriptor {
        status: &["fuel-branches"],
        queries: &["fuel-branches"],
        gates: &[crate::products::fuel_branches::QUERY],
        sections: &[products::fuel_branches::FAMILY],
        ..family("fuel_branches")
    },
    Descriptor {
        status: &["market"],
        queries: &["market-history", "market-ledger"],
        gates: &[
            crate::products::market::HISTORY,
            crate::products::market::LEDGER,
        ],
        sections: &[products::market::FAMILY],
        // 2: a row whose buyer or receiver has no identity stands, those hours dated.
        version: 2,
        ..family("market")
    },
    Descriptor {
        status: &["lend-lease-balance"],
        queries: &["lend-lease-balance"],
        gates: &[crate::products::lend_lease_balance::QUERY],
        sections: &[products::lend_lease_balance::FAMILY],
        ..family("lend_lease_balance")
    },
    Descriptor {
        status: &["production-catalogue"],
        queries: &["production-catalogue"],
        gates: &[crate::products::production_catalogue::QUERY],
        sections: &[products::production_catalogue::FAMILY],
        // 2: a licence whose licensor has no identity stands; one with no licensee's leaves
        // the roster unknown.
        version: 2,
        ..family("production_catalogue")
    },
    Descriptor {
        status: &["convoys"],
        queries: &["convoy-health"],
        gates: &[crate::products::convoy_health::QUERY],
        sections: &[products::convoy_health::FAMILY],
        ..family("convoys")
    },
    Descriptor {
        status: &["politics-state"],
        queries: &["politics-state-history", "civil-wars"],
        gates: &[crate::products::politics_state::history::QUERY],
        // Its values, and its catalogue by the family's name.
        sections: &["politics-state", products::politics_state::history::FAMILY],
        // 2: refused election eligibility dated per holder.
        version: 2,
        ..family("politics_state")
    },
    Descriptor {
        status: &["unit-leaders"],
        queries: &["unit-leaders"],
        gates: &["unit-leaders"],
        sections: &["unit-leaders"],
        walk: true,
        content: Content::Bound,
        ..family("leaders")
    },
    Descriptor {
        status: &["country-stability"],
        queries: &["country-stability", "scoreboard"],
        gates: &["country-stability"],
        // 2: its measures as facts on the segment driver, published as a series root; its parts
        // a family of their own.
        version: 2,
        segments: true,
        ..family("stability")
    },
    // Stability's parts name a read's characters as that read's hour named them, which a
    // segment's operator cannot see before its first hour: they stay a walked family.
    Descriptor {
        status: &["stability-parts"],
        queries: &["stability-parts"],
        gates: &["stability-parts"],
        sections: &["parts"],
        version: 1,
        walk: true,
        ..family("parts")
    },
    // Its series queries route by the family kind `country-experience`.
    Descriptor {
        status: &["country-experience"],
        queries: &["country-experience"],
        gates: &["country-experience"],
        sections: &["experience"],
        walk: true,
        ..family("experience")
    },
    Descriptor {
        status: &["politics-ledger"],
        queries: &["politics-ledger"],
        gates: &["politics-ledger"],
        // 2: facts on the segment driver, published as a series root.
        version: 2,
        segments: true,
        content: Content::Bound,
        ..family("politics_ledger")
    },
    Descriptor {
        status: &["country-fuel"],
        queries: &["country-fuel"],
        gates: &["country-fuel"],
        // 2: facts on the segment driver, published as a series root.
        // 3: the detail products' recipients and demand packs published with it.
        version: 3,
        segments: true,
        ..family("fuel")
    },
    Descriptor {
        status: &["territory"],
        queries: &["territory", "scoreboard", "faction-totals"],
        sections: &["territory"],
        walk: true,
        content: Content::Bound,
        ..family("territory")
    },
    // The window pages and manifest, and the trend's series by the family
    // kind `occupation-trend` (whose query also waits for the identities).
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
        sections: &["occupation", "occupied-states"],
        walk: true,
        content: Content::Bound,
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
        walk: true,
        content: Content::Bound,
        ..family("intelligence")
    },
    // Its series and events route by the family kind `research`.
    Descriptor {
        status: &["research"],
        queries: &["research"],
        gates: &["research"],
        sections: &["research"],
        // 2: a read of no slots is a known empty slot set.
        version: 2,
        walk: true,
        ..family("research")
    },
    Descriptor {
        status: &["peace-deals"],
        queries: &["peace-deals"],
        gates: &["peace-deals"],
        sections: &["peace-deals", "peace-state"],
        walk: true,
        ..family("peace")
    },
    // The event feed reads the projects' published completions when asked,
    // and refuses with them from this family's own page.
    Descriptor {
        status: &["special-projects"],
        queries: &["special-projects", "event-feed"],
        gates: &["special-projects"],
        sections: &[
            "special-projects",
            products::special_projects::prepared::completion_stream::FAMILY,
        ],
        walk: true,
        content: Content::Bound,
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
        walk: true,
        content: Content::Bound,
        ..family("casualties")
    },
    Descriptor {
        status: &["equipment-losses"],
        queries: &["equipment-losses"],
        sections: &["equipment-loss"],
        walk: true,
        ..family("equipment_losses")
    },
    // Native map pages, published with coverage but no status row or
    // refusing query of their own.
    Descriptor {
        sections: &["map-pages"],
        ..family("map-pages")
    },
    // Provinces' and states' names at hours: their name inputs as events on the segment driver.
    Descriptor {
        status: &["place-names"],
        queries: &["place-names"],
        gates: &["place-names"],
        segments: true,
        ..family(products::place_names::NAME)
    },
    // A country's manpower view: its operands as runs on the segment driver.
    Descriptor {
        status: &["country-people"],
        queries: &["country-people"],
        gates: &["country-people"],
        segments: true,
        ..family(products::country_people::NAME)
    },
];

/// Every recording-wide job.
pub(crate) static JOBS: &[Job] = &[
    Job {
        name: "map",
        version: 1,
        runner: Runner::Walk,
        gates: &[],
        sections: &["map.checkpoints", "map.gaps", "map.resume"],
        tables: &[],
        content: Content::Free,
    },
    Job {
        name: "map-export",
        version: 1,
        runner: Runner::Walk,
        gates: &[],
        sections: &["map-export"],
        tables: &[map_snow::TABLES],
        content: Content::Bound,
    },
    Job {
        name: "land-owners",
        version: 1,
        runner: Runner::Walk,
        gates: &[],
        sections: &["land-owners"],
        tables: &[crate::products::land_owners::TABLES],
        content: Content::Bound,
    },
    Job {
        name: "production-lines",
        version: 1,
        runner: Runner::Beside,
        gates: &[
            "production-lines",
            "production-line-detail",
            "production-line-breakdown",
        ],
        sections: products::production_lines::SECTIONS,
        tables: &[],
        content: Content::Bound,
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
        product: "supply-health",
        suppliers: &[FamilyId::new("supply"), IDENTITIES],
    },
    Query {
        product: "army-size",
        suppliers: &[FamilyId::new("army"), IDENTITIES],
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

/// A family's continuation, which the walk keeps in its branch state.
pub(crate) fn continuation_section(family: &str, branch: u64) -> String {
    format!("{family}{CONTINUATION}{branch}")
}

const CONTINUATION: &str = "-continuation-";

/// The family whose continuation `section` names.
pub(crate) fn continuation_owner(section: &str) -> Option<&str> {
    let (name, branch) = section.rsplit_once(CONTINUATION)?;
    digits(branch).then_some(name)
}

/// The hourly walk's own sections of a branch: its coverage and its state.
pub(crate) const WALK: &[&str] = &["preparation-coverage-", "preparation-branch-"];

/// Sections only the commit coordinator writes: the checkpoint and the
/// versions published with it, a native preparation's plan mark, the map
/// binding, the side-work roster and the recording's metadata.
pub(crate) const COORDINATOR: &[&str] = &[
    "preparation-checkpoint-v1",
    dashboard_base::prepared::coverage::VERSIONS,
    dashboard_base::prepared::coverage::PLAN,
    "preparation-map-progress-v1",
    "preparation-side-work-v1",
    "source-metadata-v1",
    "source-metadata-refusals-v1",
];

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `section` is one of the hourly walk's own sections of a branch.
pub(crate) fn walk_owned(section: &str) -> bool {
    WALK.iter()
        .any(|prefix| section.strip_prefix(prefix).is_some_and(digits))
}
