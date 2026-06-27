//! Executable interaction-contract accounting for registered replay views.
//!
//! This is deliberately conservative: a row in this inventory does not claim that the
//! interaction works. It records that every registered view has an explicit status for
//! each interaction-parity category, so missing per-view proof cannot hide outside the
//! code path the app already uses to collect views.

use crate::host::registry::{GuiSymbolRef, MenuCluster, ViewDescriptor, ViewId, ViewRegistry};

pub const INTERACTION_CATEGORY_COUNT: usize = 9;

/// The view-level interaction categories tracked for product-parity work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InteractionCategory {
    OpenClose,
    TabSwitch,
    ListSelection,
    Scroll,
    HoverTooltip,
    Drilldown,
    DisabledMutation,
    ScrubState,
    ScreenshotProof,
}

impl InteractionCategory {
    pub const ALL: [InteractionCategory; INTERACTION_CATEGORY_COUNT] = [
        InteractionCategory::OpenClose,
        InteractionCategory::TabSwitch,
        InteractionCategory::ListSelection,
        InteractionCategory::Scroll,
        InteractionCategory::HoverTooltip,
        InteractionCategory::Drilldown,
        InteractionCategory::DisabledMutation,
        InteractionCategory::ScrubState,
        InteractionCategory::ScreenshotProof,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            InteractionCategory::OpenClose => "open_close",
            InteractionCategory::TabSwitch => "tab_switch",
            InteractionCategory::ListSelection => "list_selection",
            InteractionCategory::Scroll => "scroll",
            InteractionCategory::HoverTooltip => "hover_tooltip",
            InteractionCategory::Drilldown => "drilldown",
            InteractionCategory::DisabledMutation => "disabled_mutation",
            InteractionCategory::ScrubState => "scrub_state",
            InteractionCategory::ScreenshotProof => "screenshot_proof",
        }
    }
}

/// Conservative scaffold status. `Open` means the gap is explicitly tracked, not
/// implemented. `NotProven` means no executable interaction proof exists here yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionStatus {
    Open,
    NotProven,
}

impl InteractionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            InteractionStatus::Open => "open",
            InteractionStatus::NotProven => "not_proven",
        }
    }
}

/// Descriptor-derived reachability context for the open/close contract row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractRoute {
    Menubar {
        cluster: MenuCluster,
        label: &'static str,
        order: i32,
    },
    NonMenuRegistered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryContract {
    pub category: InteractionCategory,
    pub status: InteractionStatus,
    /// Human-readable status note for reports. This note is not proof of product parity.
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionTargetContract {
    pub category: InteractionCategory,
    /// Stable contract-side target id. This is intentionally namespaced by view id so it
    /// can survive `.gui` symbol reshuffles without losing the accounting row.
    pub id: &'static str,
    /// Human label from the visible widget/tab definition.
    pub label: &'static str,
    pub status: InteractionStatus,
    /// Human-readable status note for reports. This note is not click/scrub proof.
    pub note: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewInteractionContract {
    pub view_id: ViewId,
    pub route: ContractRoute,
    pub gui_root: Option<GuiSymbolRef>,
    pub categories: [CategoryContract; INTERACTION_CATEGORY_COUNT],
    pub targets: Vec<InteractionTargetContract>,
}

impl ViewInteractionContract {
    pub fn category(&self, category: InteractionCategory) -> Option<&CategoryContract> {
        self.categories.iter().find(|c| c.category == category)
    }

    pub fn targets(
        &self,
        category: InteractionCategory,
    ) -> impl Iterator<Item = &InteractionTargetContract> {
        self.targets.iter().filter(move |t| t.category == category)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InteractionStatusCounts {
    pub open: usize,
    pub not_proven: usize,
}

impl InteractionStatusCounts {
    pub const fn total(self) -> usize {
        self.open + self.not_proven
    }

    pub const fn count(self, status: InteractionStatus) -> usize {
        match status {
            InteractionStatus::Open => self.open,
            InteractionStatus::NotProven => self.not_proven,
        }
    }

    fn add(&mut self, status: InteractionStatus) {
        match status {
            InteractionStatus::Open => self.open += 1,
            InteractionStatus::NotProven => self.not_proven += 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionCategoryCoverage {
    pub category: InteractionCategory,
    pub category_status_counts: InteractionStatusCounts,
    pub target_count: usize,
    pub target_status_counts: InteractionStatusCounts,
    /// Rows whose notes cite executable proof. This is a partial-proof accounting aid,
    /// not a completed/implemented status.
    pub target_rows_with_executable_proof_notes: usize,
}

impl InteractionCategoryCoverage {
    const fn empty(category: InteractionCategory) -> Self {
        Self {
            category,
            category_status_counts: InteractionStatusCounts {
                open: 0,
                not_proven: 0,
            },
            target_count: 0,
            target_status_counts: InteractionStatusCounts {
                open: 0,
                not_proven: 0,
            },
            target_rows_with_executable_proof_notes: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionCoverageReport {
    pub registered_view_count: usize,
    pub contract_count: usize,
    pub category_row_count: usize,
    pub category_status_counts: InteractionStatusCounts,
    pub target_row_count: usize,
    pub target_status_counts: InteractionStatusCounts,
    /// `None` means the current contract status model cannot represent completed
    /// implemented/proven target rows without overclaiming product parity.
    pub implemented_or_proven_target_count: Option<usize>,
    /// `None` means intentionally inert mutation controls are not represented as a
    /// first-class target/status kind yet.
    pub intentionally_inert_mutation_count: Option<usize>,
    pub missing_not_proven_category_count: usize,
    /// Partial-proof notes are tracked separately from implemented/proven status.
    pub target_rows_with_executable_proof_notes: usize,
    pub per_category: [InteractionCategoryCoverage; INTERACTION_CATEGORY_COUNT],
}

impl InteractionCoverageReport {
    pub fn category(&self, category: InteractionCategory) -> Option<&InteractionCategoryCoverage> {
        self.per_category.iter().find(|c| c.category == category)
    }
}

/// Build an executable coverage report from the live registry and the conservative
/// interaction contracts. The report is accounting only: current statuses are
/// `Open`/`NotProven`, so it intentionally does not claim implemented target parity.
pub fn interaction_coverage_report(registry: &ViewRegistry) -> InteractionCoverageReport {
    let contracts = interaction_contracts(registry);
    report_from_contracts(registry.len(), &contracts)
}

fn report_from_contracts(
    registered_view_count: usize,
    contracts: &[ViewInteractionContract],
) -> InteractionCoverageReport {
    let mut report = InteractionCoverageReport {
        registered_view_count,
        contract_count: contracts.len(),
        category_row_count: 0,
        category_status_counts: InteractionStatusCounts::default(),
        target_row_count: 0,
        target_status_counts: InteractionStatusCounts::default(),
        implemented_or_proven_target_count: None,
        intentionally_inert_mutation_count: None,
        missing_not_proven_category_count: 0,
        target_rows_with_executable_proof_notes: 0,
        per_category: InteractionCategory::ALL.map(InteractionCategoryCoverage::empty),
    };

    for contract in contracts {
        for row in &contract.categories {
            report.category_row_count += 1;
            report.category_status_counts.add(row.status);
            if row.status == InteractionStatus::NotProven {
                report.missing_not_proven_category_count += 1;
            }
            category_report_mut(&mut report.per_category, row.category)
                .category_status_counts
                .add(row.status);
        }

        for target in &contract.targets {
            report.target_row_count += 1;
            report.target_status_counts.add(target.status);
            let has_proof_note = target.note.contains("tests prove");
            if has_proof_note {
                report.target_rows_with_executable_proof_notes += 1;
            }

            let category = category_report_mut(&mut report.per_category, target.category);
            category.target_count += 1;
            category.target_status_counts.add(target.status);
            if has_proof_note {
                category.target_rows_with_executable_proof_notes += 1;
            }
        }
    }

    report
}

fn category_report_mut(
    categories: &mut [InteractionCategoryCoverage; INTERACTION_CATEGORY_COUNT],
    category: InteractionCategory,
) -> &mut InteractionCategoryCoverage {
    categories
        .iter_mut()
        .find(|entry| entry.category == category)
        .expect("interaction category must be present in report")
}

/// Build the interaction contract inventory from the same registry the app uses to open
/// views. There is no separate static view-id list: adding a registered view makes it
/// appear here, with every category conservatively marked until a targeted proof exists.
pub fn interaction_contracts(registry: &ViewRegistry) -> Vec<ViewInteractionContract> {
    registry.iter_views().map(contract_for_descriptor).collect()
}

fn contract_for_descriptor(d: &'static ViewDescriptor) -> ViewInteractionContract {
    let route = d
        .route
        .map(|r| ContractRoute::Menubar {
            cluster: r.cluster,
            label: r.label,
            order: r.order,
        })
        .unwrap_or(ContractRoute::NonMenuRegistered);
    let targets = known_interaction_targets(d.id);

    ViewInteractionContract {
        view_id: d.id,
        route,
        gui_root: d.gui_root,
        categories: [
            open_close_contract(route),
            tab_switch_contract(d.id, !known_tab_switch_targets(d.id).is_empty()),
            list_selection_contract(d.id),
            not_proven(
                InteractionCategory::Scroll,
                "no executable wheel/drag scroll proof attached for this view",
            ),
            not_proven(
                InteractionCategory::HoverTooltip,
                "no per-view hover/tooltip target proof attached",
            ),
            drilldown_contract(d.id),
            not_proven(
                InteractionCategory::DisabledMutation,
                "no mutating-control disabled/inert target inventory or click-no-op proof attached",
            ),
            not_proven(
                InteractionCategory::ScrubState,
                "render scrub tests do not prove future tab/selection/scroll state persistence",
            ),
            not_proven(
                InteractionCategory::ScreenshotProof,
                "`--shot-view=<view_id>` can target a registered view for deterministic app screenshots; no per-view screenshot or side-by-side interaction proof attached",
            ),
        ],
        targets,
    }
}

fn open_close_contract(route: ContractRoute) -> CategoryContract {
    let note = match route {
        ContractRoute::Menubar { .. } => {
            "descriptor has a menubar route; per-view close affordance remains unaudited"
        }
        ContractRoute::NonMenuRegistered => {
            "descriptor has no menubar route; seeded/drill/shortcut open-close proof remains unaudited"
        }
    };
    CategoryContract {
        category: InteractionCategory::OpenClose,
        status: InteractionStatus::Open,
        note,
    }
}

fn not_proven(category: InteractionCategory, note: &'static str) -> CategoryContract {
    CategoryContract {
        category,
        status: InteractionStatus::NotProven,
        note,
    }
}

fn tab_switch_contract(view_id: ViewId, has_targets: bool) -> CategoryContract {
    if has_targets {
        let note = if view_id.0 == "politics.politics" {
            "politics.politics has executable click switching, unavailable-tab inert-click, and selected-tab scrub proof for Government/National Spirits/Ideas & Laws/Autonomy; other tab inventories remain target accounting only"
        } else if view_id.0 == "chrome.find" {
            "chrome.find has executable click switching, inert-click, and selected-tab scrub proof for States/Countries/Victory Points; other tab inventories remain target accounting only"
        } else if view_id.0 == "military.army" {
            "military.army has executable click switching, unavailable-tab inert-click, and selected-tab scrub proof for Army/Navy/Air; other tab inventories remain target accounting only"
        } else if view_id.0 == "designers.division" {
            "designers.division has executable read-only click switching for Division/Army HQ, inert non-HQ handling, and selected-tab scrub proof; mutating designer edits remain inert/open"
        } else if view_id.0 == "intel.intel_agency" {
            "intel.intel_agency has executable click switching, unavailable-tab inert-click, and selected-tab scrub proof for Agency/Operations/Cryptology; mutating agency controls remain inert/open"
        } else {
            "known tab ids/labels are inventoried; click switching, selected-state persistence, and scrub proof remain open"
        };
        CategoryContract {
            category: InteractionCategory::TabSwitch,
            status: InteractionStatus::Open,
            note,
        }
    } else {
        not_proven(
            InteractionCategory::TabSwitch,
            "no executable tab/subtab target inventory or click proof attached",
        )
    }
}

fn list_selection_contract(view_id: ViewId) -> CategoryContract {
    if view_id.0 == "chrome.find" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "chrome.find has executable row selection/highlight proof for snapshot-derived catalog rows; text filtering, recenter selection, and other row selections remain open",
        }
    } else if view_id.0 == "chrome.mapmodes" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "chrome.mapmodes has executable click-to-switch proof for rail rows backed by existing render::modes::MapMode variants; Intel/non-core mode, active-state echo, legend hover, screenshots, and broader map/chrome parity remain open",
        }
    } else {
        not_proven(
            InteractionCategory::ListSelection,
            "no executable row/list selection target inventory or detail-update proof attached",
        )
    }
}

fn drilldown_contract(view_id: ViewId) -> CategoryContract {
    if view_id.0 == "chrome.find" {
        CategoryContract {
            category: InteractionCategory::Drilldown,
            status: InteractionStatus::Open,
            note: "chrome.find state and Victory Point rows emit read-only OpenView(core.state_panel, state_id); country drill, province drill, and map recenter remain open",
        }
    } else if view_id.0 == "chrome.minimap" {
        CategoryContract {
            category: InteractionCategory::Drilldown,
            status: InteractionStatus::Open,
            note: "chrome.minimap has executable click-to-recenter proof via RecenterMap; drag recenter and screenshot proof remain open",
        }
    } else {
        not_proven(
            InteractionCategory::Drilldown,
            "no read-only drilldown target proof attached",
        )
    }
}

pub fn known_interaction_targets(view_id: ViewId) -> Vec<InteractionTargetContract> {
    let mut targets = Vec::new();
    targets.extend_from_slice(known_tab_switch_targets(view_id));
    targets.extend_from_slice(known_list_selection_targets(view_id));
    targets.extend_from_slice(known_drilldown_targets(view_id));
    targets
}

pub fn known_tab_switch_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "politics.politics" => &POLITICS_TAB_TARGETS,
        "military.army" => &MILITARY_ARMY_TAB_TARGETS,
        "designers.division" => &DESIGNERS_DIVISION_TAB_TARGETS,
        "intel.intel_agency" => &INTEL_AGENCY_TAB_TARGETS,
        "chrome.find" => &CHROME_FIND_TAB_TARGETS,
        _ => &[],
    }
}

pub fn known_list_selection_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "chrome.find" => &CHROME_FIND_ROW_TARGETS,
        "chrome.mapmodes" => &CHROME_MAPMODES_ROW_TARGETS,
        _ => &[],
    }
}

pub fn known_drilldown_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "chrome.find" => &CHROME_FIND_DRILLDOWN_TARGETS,
        "chrome.minimap" => &CHROME_MINIMAP_DRILLDOWN_TARGETS,
        _ => &[],
    }
}

const POLITICS_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable PoliticsView tests prove click switching, unavailable-tab inert handling, and selected-tab A->B->A persistence for this read-only politics slice";
const MILITARY_ARMY_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable ArmyView tests prove click switching, unavailable-tab inert handling, and selected-tab A->B->A persistence for this read-only roster slice";
const DESIGNERS_DIVISION_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable DivisionDesignerView tests prove Division/Army HQ click switching, non-HQ inert handling, and selected-tab A->B->A persistence for this read-only designer slice";
const INTEL_AGENCY_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable IntelAgencyView tests prove click switching, unavailable-tab inert handling, and selected-tab A->B->A persistence for this read-only intelligence slice";
const CHROME_FIND_TAB_NOTE: &str =
    "fixed widgets::tabs target; executable FindView tests prove click switching, inert-click handling, and selected-tab A->B->A persistence for this read-only catalog slice";
const CHROME_FIND_ROW_NOTE: &str =
    "fixed catalog row target; executable FindView tests prove read-only row selection/highlight, outside-row inert handling, and selected-row A->B->A persistence for this snapshot-derived catalog slice";
const CHROME_FIND_DRILLDOWN_NOTE: &str =
    "fixed catalog drill target; executable FindView tests prove state-backed rows emit read-only OpenView(core.state_panel, state_id); country drill, province drill, and map recenter remain open";
const CHROME_MINIMAP_DRILLDOWN_NOTE: &str =
    "fixed minimap navigation target; executable MinimapView tests prove click-to-recenter emits RecenterMap and letterbox/outside clicks do not fabricate map picks; drag recenter remains open";
const CHROME_MAPMODES_ROW_NOTE: &str =
    "fixed map-mode rail row target; executable MapModesView tests prove core-backed rows emit read-only SetMapMode commands while unavailable/data-proxy badges remain honest; Intel/non-core mode, active-state echo, legend hover, and screenshots remain open";

const POLITICS_TAB_TARGETS: [InteractionTargetContract; 4] = [
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "politics.politics.tab.government",
        label: "Government",
        status: InteractionStatus::Open,
        note: POLITICS_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "politics.politics.tab.national_spirits",
        label: "National Spirits",
        status: InteractionStatus::Open,
        note: POLITICS_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "politics.politics.tab.ideas_laws",
        label: "Ideas & Laws",
        status: InteractionStatus::Open,
        note: POLITICS_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "politics.politics.tab.autonomy",
        label: "Autonomy",
        status: InteractionStatus::Open,
        note: POLITICS_TAB_NOTE,
    },
];

const MILITARY_ARMY_TAB_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "military.army.tab.army",
        label: "Army",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "military.army.tab.navy",
        label: "Navy",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "military.army.tab.air",
        label: "Air",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_TAB_NOTE,
    },
];

const DESIGNERS_DIVISION_TAB_TARGETS: [InteractionTargetContract; 2] = [
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "designers.division.tab.normal",
        label: "Division",
        status: InteractionStatus::Open,
        note: DESIGNERS_DIVISION_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "designers.division.tab.hq",
        label: "Army HQ",
        status: InteractionStatus::Open,
        note: DESIGNERS_DIVISION_TAB_NOTE,
    },
];

const INTEL_AGENCY_TAB_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "intel.intel_agency.tab.agency",
        label: "Agency",
        status: InteractionStatus::Open,
        note: INTEL_AGENCY_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "intel.intel_agency.tab.operations",
        label: "Operations",
        status: InteractionStatus::Open,
        note: INTEL_AGENCY_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "intel.intel_agency.tab.cryptology",
        label: "Cryptology",
        status: InteractionStatus::Open,
        note: INTEL_AGENCY_TAB_NOTE,
    },
];

const CHROME_FIND_TAB_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "chrome.find.tab.states",
        label: "States",
        status: InteractionStatus::Open,
        note: CHROME_FIND_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "chrome.find.tab.countries",
        label: "Countries",
        status: InteractionStatus::Open,
        note: CHROME_FIND_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "chrome.find.tab.vps",
        label: "Victory Points",
        status: InteractionStatus::Open,
        note: CHROME_FIND_TAB_NOTE,
    },
];

const CHROME_FIND_ROW_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.find.row.state",
        label: "State row",
        status: InteractionStatus::Open,
        note: CHROME_FIND_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.find.row.country",
        label: "Country row",
        status: InteractionStatus::Open,
        note: CHROME_FIND_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.find.row.vp",
        label: "Victory Point row",
        status: InteractionStatus::Open,
        note: CHROME_FIND_ROW_NOTE,
    },
];

const CHROME_FIND_DRILLDOWN_TARGETS: [InteractionTargetContract; 2] = [
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.find.drill.state",
        label: "State row to state panel",
        status: InteractionStatus::Open,
        note: CHROME_FIND_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.find.drill.vp",
        label: "Victory Point row to state panel",
        status: InteractionStatus::Open,
        note: CHROME_FIND_DRILLDOWN_NOTE,
    },
];

const CHROME_MAPMODES_ROW_TARGETS: [InteractionTargetContract; 13] = [
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.political",
        label: "Political",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.control",
        label: "Control",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.resistance",
        label: "Resistance",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.compliance",
        label: "Compliance",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.victory_points",
        label: "Victory Points",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.buildings",
        label: "Buildings",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.resources",
        label: "Resources",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.weather",
        label: "Weather",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.front_lines",
        label: "Front Lines",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.faction",
        label: "Faction",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.air_superiority",
        label: "Air Superiority",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.naval",
        label: "Naval",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.supply",
        label: "Supply",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ROW_NOTE,
    },
];

const CHROME_MINIMAP_DRILLDOWN_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.minimap.drill.recenter",
        label: "Minimap click recenter",
        status: InteractionStatus::Open,
        note: CHROME_MINIMAP_DRILLDOWN_NOTE,
    }];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{BindContext, ViewDescriptor};
    use crate::host::{BoundView, HostPolicy, MenuRoute, ViewInstance, ZBand};
    use std::collections::BTreeSet;

    #[test]
    fn contract_inventory_covers_all_registered_views() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);

        assert_eq!(
            reg.len(),
            114,
            "the executable interaction contract is expected to cover the 114 registered native views"
        );
        assert_eq!(
            contracts.len(),
            reg.len(),
            "each registered view must have exactly one interaction contract"
        );

        let registry_ids: BTreeSet<_> = reg.iter_views().map(|d| d.id).collect();
        let mut contract_ids = BTreeSet::new();
        for contract in &contracts {
            assert!(
                contract_ids.insert(contract.view_id),
                "duplicate interaction contract for {}",
                contract.view_id
            );
            assert_complete_categories(contract);
        }

        assert_eq!(
            contract_ids, registry_ids,
            "registered views and interaction contracts must match exactly"
        );
    }

    #[test]
    fn contract_inventory_is_registry_derived() {
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&CONTRACT_TEST_MENU_VIEW);
        reg.insert_view(&CONTRACT_TEST_NON_MENU_VIEW);

        let contracts = interaction_contracts(&reg);
        let ids: Vec<_> = contracts.iter().map(|c| c.view_id.as_str()).collect();
        assert_eq!(ids, vec!["contract.menu", "contract.non_menu"]);

        for contract in &contracts {
            assert_complete_categories(contract);
        }
        assert_eq!(
            contracts[0]
                .category(InteractionCategory::OpenClose)
                .unwrap()
                .status,
            InteractionStatus::Open
        );
        assert_eq!(
            contracts[1]
                .category(InteractionCategory::ScreenshotProof)
                .unwrap()
                .status,
            InteractionStatus::NotProven
        );
    }

    #[test]
    fn no_registered_view_can_miss_an_explicit_category_status() {
        let reg = ViewRegistry::from_inventory();
        for contract in interaction_contracts(&reg) {
            for category in InteractionCategory::ALL {
                let status = contract.category(category).unwrap_or_else(|| {
                    panic!("{} missing {}", contract.view_id, category.as_str())
                });
                assert!(
                    !status.note.is_empty(),
                    "{} {} must explain its scaffold status",
                    contract.view_id,
                    category.as_str()
                );
            }
        }
    }

    #[test]
    fn real_registered_view_has_tab_switch_targets() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("chrome.find")).is_some(),
            "the tab-switch inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let find = contract_by_id(&contracts, ViewId("chrome.find"));
        let tab_status = find.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(tab_status.status, InteractionStatus::Open);
        assert!(
            tab_status.note.contains("executable click switching"),
            "chrome.find tab contract note must cite the executable proof"
        );

        let targets: Vec<_> = find.targets(InteractionCategory::TabSwitch).collect();
        assert_eq!(targets.len(), 3);
        assert_eq!(
            targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "chrome.find.tab.states",
                "chrome.find.tab.countries",
                "chrome.find.tab.vps",
            ]
        );
        assert_eq!(
            targets.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["States", "Countries", "Victory Points"]
        );
        assert!(
            targets.iter().all(|t| t.status == InteractionStatus::Open),
            "target rows remain open until the broader parity contract has a completed status"
        );
        assert!(
            targets
                .iter()
                .all(|t| t.note.contains("FindView tests prove")),
            "chrome.find target notes must cite click/scrub proof"
        );
    }

    #[test]
    fn politics_politics_tab_targets_cite_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("politics.politics")).is_some(),
            "the Politics tab-switch inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let politics = contract_by_id(&contracts, ViewId("politics.politics"));
        let tab_status = politics.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(tab_status.status, InteractionStatus::Open);
        assert!(
            tab_status.note.contains("executable click switching"),
            "politics.politics tab contract note must cite the executable proof"
        );

        let targets: Vec<_> = politics.targets(InteractionCategory::TabSwitch).collect();
        assert_eq!(targets.len(), 4);
        assert_eq!(
            targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "politics.politics.tab.government",
                "politics.politics.tab.national_spirits",
                "politics.politics.tab.ideas_laws",
                "politics.politics.tab.autonomy",
            ]
        );
        assert_eq!(
            targets.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["Government", "National Spirits", "Ideas & Laws", "Autonomy"]
        );
        assert!(
            targets
                .iter()
                .all(|t| t.note.contains("PoliticsView tests prove")),
            "politics.politics target notes must cite click/scrub proof"
        );
    }

    #[test]
    fn military_army_tab_targets_cite_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("military.army")).is_some(),
            "the Army tab-switch inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let army = contract_by_id(&contracts, ViewId("military.army"));
        let tab_status = army.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(tab_status.status, InteractionStatus::Open);
        assert!(
            tab_status.note.contains("executable click switching"),
            "military.army tab contract note must cite the executable proof"
        );

        let targets: Vec<_> = army.targets(InteractionCategory::TabSwitch).collect();
        assert_eq!(targets.len(), 3);
        assert_eq!(
            targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "military.army.tab.army",
                "military.army.tab.navy",
                "military.army.tab.air",
            ]
        );
        assert_eq!(
            targets.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["Army", "Navy", "Air"]
        );
        assert!(
            targets
                .iter()
                .all(|t| t.note.contains("ArmyView tests prove")),
            "military.army target notes must cite click/scrub proof"
        );
    }

    #[test]
    fn designers_division_tab_targets_cite_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("designers.division")).is_some(),
            "the Division Designer tab-switch inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let designer = contract_by_id(&contracts, ViewId("designers.division"));
        let tab_status = designer.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(tab_status.status, InteractionStatus::Open);
        assert!(
            tab_status
                .note
                .contains("executable read-only click switching"),
            "designers.division tab contract note must cite the executable proof"
        );

        let targets: Vec<_> = designer.targets(InteractionCategory::TabSwitch).collect();
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec!["designers.division.tab.normal", "designers.division.tab.hq"]
        );
        assert_eq!(
            targets.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["Division", "Army HQ"]
        );
        assert!(
            targets
                .iter()
                .all(|t| t.note.contains("DivisionDesignerView tests prove")),
            "designers.division target notes must cite click/scrub proof"
        );
    }

    #[test]
    fn intel_agency_tab_targets_cite_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("intel.intel_agency")).is_some(),
            "the Intel Agency tab-switch inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let agency = contract_by_id(&contracts, ViewId("intel.intel_agency"));
        let tab_status = agency.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(tab_status.status, InteractionStatus::Open);
        assert!(
            tab_status.note.contains("executable click switching"),
            "intel.intel_agency tab contract note must cite the executable proof"
        );

        let targets: Vec<_> = agency.targets(InteractionCategory::TabSwitch).collect();
        assert_eq!(targets.len(), 3);
        assert_eq!(
            targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "intel.intel_agency.tab.agency",
                "intel.intel_agency.tab.operations",
                "intel.intel_agency.tab.cryptology",
            ]
        );
        assert_eq!(
            targets.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["Agency", "Operations", "Cryptology"]
        );
        assert!(
            targets
                .iter()
                .all(|t| t.note.contains("IntelAgencyView tests prove")),
            "intel.intel_agency target notes must cite click/scrub proof"
        );
    }

    #[test]
    fn chrome_find_row_selection_and_drilldown_stay_partial_open() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let find = contract_by_id(&contracts, ViewId("chrome.find"));

        let selection = find.category(InteractionCategory::ListSelection).unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("row selection/highlight proof"),
            "chrome.find list-selection contract must cite the executable row-selection proof"
        );
        assert!(
            selection.note.contains("remain open"),
            "chrome.find row-selection proof must not close global find/search behavior"
        );

        let drilldown = find.category(InteractionCategory::Drilldown).unwrap();
        assert_eq!(drilldown.status, InteractionStatus::Open);
        assert!(
            drilldown.note.contains("OpenView(core.state_panel"),
            "chrome.find drilldown contract must cite the state-panel command proof"
        );
        assert!(
            drilldown.note.contains("remain open"),
            "chrome.find drilldown proof must not claim complete drilldown/recenter parity"
        );

        let row_targets: Vec<_> = find.targets(InteractionCategory::ListSelection).collect();
        assert_eq!(row_targets.len(), 3);
        assert_eq!(
            row_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "chrome.find.row.state",
                "chrome.find.row.country",
                "chrome.find.row.vp",
            ]
        );
        assert!(
            row_targets
                .iter()
                .all(|t| t.note.contains("FindView tests prove")),
            "chrome.find row targets must cite executable row-selection proof"
        );

        let drill_targets: Vec<_> = find.targets(InteractionCategory::Drilldown).collect();
        assert_eq!(drill_targets.len(), 2);
        assert_eq!(
            drill_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec!["chrome.find.drill.state", "chrome.find.drill.vp"]
        );
        assert!(
            drill_targets
                .iter()
                .all(|t| t.note.contains("OpenView(core.state_panel")),
            "chrome.find drill targets must cite executable state-panel proof"
        );
    }

    #[test]
    fn chrome_minimap_recenter_drilldown_stays_partial_open() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let minimap = contract_by_id(&contracts, ViewId("chrome.minimap"));

        let drilldown = minimap.category(InteractionCategory::Drilldown).unwrap();
        assert_eq!(drilldown.status, InteractionStatus::Open);
        assert!(
            drilldown.note.contains("click-to-recenter proof"),
            "chrome.minimap drilldown contract must cite the executable recenter proof"
        );
        assert!(
            drilldown.note.contains("drag recenter"),
            "chrome.minimap click proof must not claim drag recenter parity"
        );

        let drill_targets: Vec<_> = minimap.targets(InteractionCategory::Drilldown).collect();
        assert_eq!(drill_targets.len(), 1);
        assert_eq!(drill_targets[0].id, "chrome.minimap.drill.recenter");
        assert!(
            drill_targets[0]
                .note
                .contains("MinimapView tests prove click-to-recenter"),
            "chrome.minimap drill target must cite executable recenter proof"
        );
    }

    #[test]
    fn chrome_mapmodes_core_row_selection_stays_partial_open() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let mapmodes = contract_by_id(&contracts, ViewId("chrome.mapmodes"));

        let selection = mapmodes
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection
                .note
                .contains("existing render::modes::MapMode variants"),
            "chrome.mapmodes selection note must limit proof to core-backed rows"
        );
        assert!(
            selection.note.contains("Intel/non-core mode"),
            "chrome.mapmodes selection note must keep the unsupported Intel row open"
        );
        assert!(
            selection
                .note
                .contains("broader map/chrome parity remain open"),
            "chrome.mapmodes selection note must not overclaim global parity"
        );

        let row_targets: Vec<_> = mapmodes
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 13);
        assert!(
            row_targets.iter().all(|t| t.note.contains("SetMapMode")),
            "chrome.mapmodes row targets must cite the SetMapMode proof"
        );
        assert!(
            row_targets.iter().all(|t| !t.id.contains("intel")),
            "Intel has no render::modes::MapMode target and must not be listed as implemented"
        );
    }

    #[test]
    fn coverage_report_locks_current_registry_category_and_target_counts() {
        let reg = ViewRegistry::from_inventory();
        let report = interaction_coverage_report(&reg);

        assert_eq!(report.registered_view_count, 114);
        assert_eq!(report.contract_count, 114);
        assert_eq!(
            report.category_row_count,
            114 * INTERACTION_CATEGORY_COUNT,
            "the report must keep nine category rows per registered view"
        );
        assert_eq!(report.category_status_counts.open, 123);
        assert_eq!(report.category_status_counts.not_proven, 903);
        assert_eq!(report.missing_not_proven_category_count, 903);

        assert_eq!(
            report.target_row_count, 34,
            "current target rows are 15 tab rows, 16 list-selection rows, two chrome.find drill rows, and one chrome.minimap recenter row"
        );
        assert_eq!(report.target_status_counts.open, 34);
        assert_eq!(report.target_status_counts.not_proven, 0);
        assert_eq!(
            report.target_rows_with_executable_proof_notes, 34,
            "partial executable-proof notes are accounted separately from implemented status"
        );
    }

    #[test]
    fn coverage_report_breaks_down_per_category_counts() {
        let reg = ViewRegistry::from_inventory();
        let report = interaction_coverage_report(&reg);

        assert_eq!(report.per_category.len(), INTERACTION_CATEGORY_COUNT);
        for category in InteractionCategory::ALL {
            let row = report.category(category).unwrap();
            assert_eq!(
                row.category_status_counts.total(),
                114,
                "{} must account for every registered view",
                category.as_str()
            );
        }

        assert_category_report(
            &report,
            InteractionCategory::OpenClose,
            InteractionStatusCounts {
                open: 114,
                not_proven: 0,
            },
            0,
            InteractionStatusCounts::default(),
            0,
        );
        assert_category_report(
            &report,
            InteractionCategory::TabSwitch,
            InteractionStatusCounts {
                open: 5,
                not_proven: 109,
            },
            15,
            InteractionStatusCounts {
                open: 15,
                not_proven: 0,
            },
            15,
        );
        assert_category_report(
            &report,
            InteractionCategory::ListSelection,
            InteractionStatusCounts {
                open: 2,
                not_proven: 112,
            },
            16,
            InteractionStatusCounts {
                open: 16,
                not_proven: 0,
            },
            16,
        );
        assert_category_report(
            &report,
            InteractionCategory::Scroll,
            InteractionStatusCounts {
                open: 0,
                not_proven: 114,
            },
            0,
            InteractionStatusCounts::default(),
            0,
        );
        assert_category_report(
            &report,
            InteractionCategory::HoverTooltip,
            InteractionStatusCounts {
                open: 0,
                not_proven: 114,
            },
            0,
            InteractionStatusCounts::default(),
            0,
        );
        assert_category_report(
            &report,
            InteractionCategory::Drilldown,
            InteractionStatusCounts {
                open: 2,
                not_proven: 112,
            },
            3,
            InteractionStatusCounts {
                open: 3,
                not_proven: 0,
            },
            3,
        );
        assert_category_report(
            &report,
            InteractionCategory::DisabledMutation,
            InteractionStatusCounts {
                open: 0,
                not_proven: 114,
            },
            0,
            InteractionStatusCounts::default(),
            0,
        );
        assert_category_report(
            &report,
            InteractionCategory::ScrubState,
            InteractionStatusCounts {
                open: 0,
                not_proven: 114,
            },
            0,
            InteractionStatusCounts::default(),
            0,
        );
        assert_category_report(
            &report,
            InteractionCategory::ScreenshotProof,
            InteractionStatusCounts {
                open: 0,
                not_proven: 114,
            },
            0,
            InteractionStatusCounts::default(),
            0,
        );
    }

    #[test]
    fn coverage_report_does_not_overclaim_broad_parity() {
        let reg = ViewRegistry::from_inventory();
        let report = interaction_coverage_report(&reg);

        assert_eq!(
            report.implemented_or_proven_target_count, None,
            "Open/NotProven statuses cannot represent completed target implementation"
        );
        assert_eq!(
            report.intentionally_inert_mutation_count, None,
            "inert mutation controls are not first-class target rows yet"
        );
        assert!(
            report.missing_not_proven_category_count > 0,
            "global interaction parity must remain open while categories are not proven"
        );
        assert_eq!(
            report
                .category(InteractionCategory::DisabledMutation)
                .unwrap()
                .category_status_counts
                .not_proven,
            114,
            "the report must not convert generic inert/no-op behavior into mutation parity"
        );
        assert_eq!(
            report
                .category(InteractionCategory::ScreenshotProof)
                .unwrap()
                .category_status_counts
                .not_proven,
            114,
            "screenshot parity must stay open until --shot-view outputs are captured and reviewed for every view"
        );
    }

    #[test]
    fn every_recorded_target_has_stable_id_label_status() {
        let reg = ViewRegistry::from_inventory();
        let mut count = 0usize;
        for contract in interaction_contracts(&reg) {
            for target in &contract.targets {
                count += 1;
                assert!(
                    target.id.starts_with(contract.view_id.as_str()),
                    "{} target id {} must be namespaced by the view id",
                    contract.view_id,
                    target.id
                );
                assert!(
                    target_kind_matches_category(target),
                    "{} target id {} must include the stable kind for {:?}",
                    contract.view_id,
                    target.id,
                    target.category
                );
                assert!(
                    !target.label.trim().is_empty(),
                    "{} {} must have a stable label",
                    contract.view_id,
                    target.id
                );
                assert!(
                    !target.status.as_str().is_empty(),
                    "{} {} must have an explicit status",
                    contract.view_id,
                    target.id
                );
                assert!(
                    !target.note.trim().is_empty(),
                    "{} {} must explain its scaffold status",
                    contract.view_id,
                    target.id
                );
            }
        }
        assert!(
            count > 0,
            "at least one executable target inventory must be present"
        );
    }

    fn assert_category_report(
        report: &InteractionCoverageReport,
        category: InteractionCategory,
        category_status_counts: InteractionStatusCounts,
        target_count: usize,
        target_status_counts: InteractionStatusCounts,
        target_rows_with_executable_proof_notes: usize,
    ) {
        let row = report.category(category).unwrap();
        assert_eq!(row.category_status_counts, category_status_counts);
        assert_eq!(row.target_count, target_count);
        assert_eq!(row.target_status_counts, target_status_counts);
        assert_eq!(
            row.target_rows_with_executable_proof_notes,
            target_rows_with_executable_proof_notes
        );
    }

    fn target_kind_matches_category(target: &InteractionTargetContract) -> bool {
        match target.category {
            InteractionCategory::TabSwitch => target.id.contains(".tab."),
            InteractionCategory::ListSelection => target.id.contains(".row."),
            InteractionCategory::Drilldown => target.id.contains(".drill."),
            _ => false,
        }
    }

    #[test]
    fn registered_views_without_known_tab_targets_stay_explicitly_tracked() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let map = contract_by_id(&contracts, ViewId("core.map"));

        assert!(map.targets(InteractionCategory::TabSwitch).next().is_none());
        let status = map.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(status.status, InteractionStatus::NotProven);
        assert!(
            status
                .note
                .contains("no executable tab/subtab target inventory"),
            "views without known tab targets must remain visible as open accounting gaps"
        );
    }

    fn assert_complete_categories(contract: &ViewInteractionContract) {
        let categories: Vec<_> = contract.categories.iter().map(|c| c.category).collect();
        assert_eq!(
            categories,
            InteractionCategory::ALL,
            "{} must explicitly list every interaction category",
            contract.view_id
        );
    }

    fn contract_by_id(
        contracts: &[ViewInteractionContract],
        view_id: ViewId,
    ) -> &ViewInteractionContract {
        contracts
            .iter()
            .find(|c| c.view_id == view_id)
            .unwrap_or_else(|| panic!("missing interaction contract for {view_id}"))
    }

    struct ContractTestView;

    impl ViewInstance for ContractTestView {
        fn view_id(&self) -> ViewId {
            ViewId("contract.test_instance")
        }

        fn policy(&self) -> HostPolicy {
            HostPolicy::window(ZBand::Panel)
        }

        fn bind(&self, _ctx: &BindContext) -> BoundView {
            panic!("contract tests never bind the dummy view")
        }

        crate::impl_view_any!();
    }

    fn contract_test_open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
        Box::new(ContractTestView)
    }

    static CONTRACT_TEST_MENU_VIEW: ViewDescriptor = ViewDescriptor {
        id: ViewId("contract.menu"),
        gui_root: None,
        required_domains: &[],
        route: Some(MenuRoute {
            cluster: MenuCluster::Core,
            label: "Contract Menu",
            order: 1,
        }),
        shortcut: None,
        open: contract_test_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    static CONTRACT_TEST_NON_MENU_VIEW: ViewDescriptor = ViewDescriptor {
        id: ViewId("contract.non_menu"),
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: contract_test_open,
        policy: HostPolicy::window(ZBand::Panel),
    };
}
