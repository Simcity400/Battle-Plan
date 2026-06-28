//! Executable interaction-contract accounting for registered replay views.
//!
//! This is deliberately conservative: a row in this inventory does not claim that the
//! interaction works. It records that every registered view has an explicit status for
//! each interaction-parity category, so missing per-view proof cannot hide outside the
//! code path the app already uses to collect views.

use crate::host::registry::{GuiSymbolRef, MenuCluster, ViewDescriptor, ViewId, ViewRegistry};

pub const INTERACTION_CATEGORY_COUNT: usize = 10;

/// The view-level interaction categories tracked for product-parity work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InteractionCategory {
    OpenClose,
    TabSwitch,
    ListSelection,
    TextInput,
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
        InteractionCategory::TextInput,
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
            InteractionCategory::TextInput => "text_input",
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
    /// First-class disabled-mutation target rows whose notes cite executable no-op
    /// proof. `None` means no intentionally inert mutation target has proof yet.
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

    let mut intentionally_inert_mutation_count = 0usize;

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
            if target.category == InteractionCategory::DisabledMutation && has_proof_note {
                intentionally_inert_mutation_count += 1;
            }
        }
    }

    if intentionally_inert_mutation_count > 0 {
        report.intentionally_inert_mutation_count = Some(intentionally_inert_mutation_count);
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
            text_input_contract(d.id),
            scroll_contract(d.id),
            hover_tooltip_contract(d.id),
            drilldown_contract(d.id),
            disabled_mutation_contract(d.id),
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
    if view_id.0 == "politics.politics" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "politics.politics has executable row selection/highlight proof for stable-keyed Government/National Spirits/Ideas & Laws/Autonomy rows with duplicate/unkeyed-row fail-closed handling; detail drilldowns, mutation controls, and broader politics row parity remain open",
        }
    } else if view_id.0 == "chrome.find" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "chrome.find has executable row selection/highlight proof for filtered snapshot-derived catalog rows; localized names, recenter selection, and other row selections remain open",
        }
    } else if view_id.0 == "military.army" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "military.army has executable row selection/highlight proof for keyed Army/Navy/Air roster rows; detail-pane drilldowns, other military row lists, and broader roster parity remain open",
        }
    } else if view_id.0 == "military.theatre_selector" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "military.theatre_selector has executable row selection/highlight proof for keyed theatre rows; group/member drilldowns, alerts, pin/priority UI state, and broader theatre parity remain open",
        }
    } else if view_id.0 == "chrome.mapmodes" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "chrome.mapmodes has executable click-to-switch proof for the real numbered .gui rail slots and the real mapmodes_all popup path for Supply/Intel; custom slot configuration/defaults, legend hover, screenshots, and broader map/chrome parity remain open",
        }
    } else if view_id.0 == "core.focus_tree" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "core.focus_tree has executable focus-node click selection proof keyed by exact focus id, with drag-pan conflict protection, missing-selected fail-closed handling, selected-node A->B->A scrub proof, and a first factual selected-detail panel/digest slice for id, state, cost, current progress days, prereq group count, and position fidelity; full the game national_focus_detail_view parity, filter/shortcut controls, focus mutation controls, and broader focus-tree parity remain open",
        }
    } else if view_id.0 == "logistics.unit_view" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "logistics.unit_view has executable measured-div_id row selection proof that updates the read-only equipment pane and fails closed when the selected division is missing; ship rows, column sorts, and broader unit-detail parity remain open",
        }
    } else if view_id.0 == "production.production" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "production.production has executable production-line row selection proof keyed by measured live id or save line_id with duplicate/unkeyed-row fail-closed handling; line detail drilldowns, filters, mutation controls, and broader production parity remain open",
        }
    } else if view_id.0 == "production.construction" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "production.construction has executable construction-queue row selection proof keyed by measured live line_id or save template/state/created identity with scrub-safe missing-selection handling; building details, build-picker actions, market/debug controls, and broader construction parity remain open",
        }
    } else if view_id.0 == "production.decisions" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "production.decisions has executable timed-decision row selection proof keyed by measured decision/state identity with scrub-safe missing-selection handling; decision detail drilldowns, one-shot availability, and mutating decision buttons remain open",
        }
    } else if view_id.0 == "research.research" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "research.research has executable active-slot row selection proof keyed by the exact in-progress tech token, with empty/outside slot inert behavior and missing-selected fail-closed handling; slot detail drilldowns, ETA/progress parity, and mutating research controls remain open",
        }
    } else if view_id.0 == "research.tech_tree" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "research.tech_tree has executable tech-node click selection proof keyed by the exact catalog tech id, with drag-pan conflict protection, missing-selected fail-closed handling, selected-node A->B->A scrub proof, and a first factual selected-detail panel/digest slice for id, folder, state, and research cost; full the game detail-window parity, research-start mutation controls, and broader tech-tree parity remain open",
        }
    } else if view_id.0 == "research.doctrine_tree" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "research.doctrine_tree has executable doctrine-node click selection proof keyed by the exact catalog doctrine id, with drag-pan conflict protection, missing-selected fail-closed handling, selected-node A->B->A scrub proof, and a first factual selected-detail panel/digest slice for id, folder, exact-when-fed/unfed-badged state, and XOR-link count; full the game detail-window parity, doctrine tooltip parity, and broader doctrine-tree parity remain open",
        }
    } else if view_id.0 == "leaders.army_trait_tree" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "leaders.army_trait_tree has executable click selection proof for current N3 owned trait-token nodes only, with unavailable no-leader/no-traits glyphs inert, drag movement canceling click selection, missing-selected fail-closed handling, selected-node A->B->A scrub proof, and a factual selected-detail panel/digest slice for trait id, owned state, branch, leader name, leader key, and derived static DEF catalog rows for scope/cost/prereq/mutual-exclusion/modifier/source metadata with null fields badged unavailable; full static trait catalog graph, disabled/pickable states, real trait icons/effects, leader-roster driven selection, and broader trait-tree parity remain open",
        }
    } else if view_id.0 == "leaders.navy_trait_tree" {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "leaders.navy_trait_tree has executable click selection proof for current N3 owned trait-token nodes only, with exact navy branch detail rows, derived static DEF catalog rows, and inert unavailable glyph behavior shared with the army trait tree; full static trait catalog graph, disabled/pickable states, real trait icons/effects, leader-roster driven selection, and broader trait-tree parity remain open",
        }
    } else if matches!(
        view_id.0,
        "designers.tank" | "designers.plane" | "designers.equipment"
    ) {
        CategoryContract {
            category: InteractionCategory::ListSelection,
            status: InteractionStatus::Open,
            note: "component designer views have executable module-slot click selection proof for tank, plane, and equipment designers with selected-slot scrub/missing-restored no-fallback handling; module-slot detail parity, selector drilldowns, disabled visuals, screenshots, and broader component-designer parity remain open",
        }
    } else {
        not_proven(
            InteractionCategory::ListSelection,
            "no executable row/list selection target inventory or detail-update proof attached",
        )
    }
}

fn text_input_contract(view_id: ViewId) -> CategoryContract {
    if view_id.0 == "chrome.find" {
        CategoryContract {
            category: InteractionCategory::TextInput,
            status: InteractionStatus::Open,
            note: "chrome.find has executable replay-local query proof for focusing the real to_find_text edit box, typing/backspace, token filtering over honest catalog labels/keys, scroll reset/clamp, and query A->B->A persistence; localized names, country/province drill, and full text-field parity remain open",
        }
    } else {
        not_proven(
            InteractionCategory::TextInput,
            "no executable text-entry/edit-box focus or filtering proof attached",
        )
    }
}

fn scroll_contract(view_id: ViewId) -> CategoryContract {
    if view_id.0 == "core.state_panel" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "core.state_panel has executable wheel-scroll and scrollbar-thumb drag proof for the real state-building grid geometry: visible building rows change and clamp, captured thumb drags keep scrolling until pointer-up, track clicks are consumed without scroll/row/map leakage, outside-grid wheel input is inert, and scroll is scrub-clamped when the selected state's building rows shrink; exact scrollbar art, province/state drilldown links, resources-grid scroll, and broader state-panel parity remain open",
        }
    } else if view_id.0 == "politics.politics" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "politics.politics has executable wheel-scroll and scrollbar-thumb drag proof for long National Spirits and Ideas & Laws active-tab rows: visible rows change and clamp, captured thumb drags keep scrolling outside the list until pointer-up, track clicks do not select underlying rows, per-tab scroll state stays isolated, scrolled clicks select the correct backing row, and scroll+selection survive A->B->A scrub; Government/Autonomy long-list scroll, exact scrollbar art, politics modal lists, and broader politics scroll surfaces remain open",
        }
    } else if view_id.0 == "chrome.find" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "chrome.find has executable wheel-scroll and scrollbar-thumb drag proof for the results catalog: visible rows change and clamp, captured thumb drags keep scrolling outside the list until pointer-up, scrolled clicks select the correct backing row, and scroll offset survives A->B->A scrub; exact scrollbar art, screenshots, and other scroll surfaces remain open",
        }
    } else if view_id.0 == "military.army" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "military.army has executable wheel-scroll and scrollbar-thumb drag proof for Army/Navy/Air roster rows: visible rows change and clamp, captured thumb drags keep scrolling outside the list until pointer-up, track clicks do not select underlying rows, scrolled clicks select the correct backing row, per-tab scroll state stays isolated, and scroll offset survives A->B->A scrub; exact scrollbar art and other military scroll surfaces remain open",
        }
    } else if view_id.0 == "military.theatre_selector" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "military.theatre_selector has executable wheel-scroll and scrollbar-thumb drag proof for theatre rows: visible rows change and clamp, captured thumb drags keep scrolling outside the list until pointer-up, scrolled clicks select the correct backing theatre row, and scroll+selection survive A->B->A scrub; scrollbar art, group/member rows, and other theatre scroll surfaces remain open",
        }
    } else if view_id.0 == "logistics.unit_view" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "logistics.unit_view has executable wheel-scroll and scrollbar-thumb drag proof for measured-div_id division rows and the selected-division equipment pane: visible rows change and clamp, captured thumb drags keep scrolling outside the pane until pointer-up, track clicks are inert and do not select underlying rows, scrolled division clicks select the correct backing division, equipment scroll resets on explicit selection changes, stale captures clear on rebind, and scroll offsets survive A->B->A scrub; exact scrollbar art and broader unit-detail scroll surfaces remain open",
        }
    } else if view_id.0 == "production.production" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "production.production has executable wheel-scroll and scrollbar-thumb drag proof for production-line rows: visible rows change and clamp, captured thumb drags keep scrolling outside the list until pointer-up, scrolled clicks select the correct backing production key, and scroll+selection survive A->B->A scrub; scrollbar art, filters, and mutating line controls remain open",
        }
    } else if view_id.0 == "production.construction" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "production.construction has executable wheel-scroll and scrollbar-thumb drag proof for construction-queue rows: visible rows change and clamp, captured thumb drags keep scrolling outside the grid until pointer-up, scrolled clicks select the correct backing construction key, and scroll+selection survive A->B->A scrub; scrollbar art, build-picker actions, market/debug controls, and broader construction parity remain open",
        }
    } else if view_id.0 == "production.decisions" {
        CategoryContract {
            category: InteractionCategory::Scroll,
            status: InteractionStatus::Open,
            note: "production.decisions has executable wheel-scroll and scrollbar-thumb drag proof for active timed-decision rows: visible rows change and clamp, captured thumb drags keep scrolling outside the grid until pointer-up, scrolled clicks select the correct backing decision key, and scroll+selection survive A->B->A scrub; scrollbar art, decision detail drilldowns, one-shot availability, and mutating decision buttons remain open",
        }
    } else {
        not_proven(
            InteractionCategory::Scroll,
            "no executable wheel/drag scroll proof attached for this view",
        )
    }
}

fn hover_tooltip_contract(view_id: ViewId) -> CategoryContract {
    match view_id.0 {
        "core.map" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "core.map has executable factual province-hover tooltip proof for exposed map picks; full game-style map tooltip frames, per-mode legend hover, and side-by-side screenshot proof remain open",
        },
        "core.state_panel" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "core.state_panel has executable factual hover-tooltip proof for state owner flags, state-building rows, and resource rows, including scroll-aware building hover and stale-hover fail-closed rebind behavior; full the game tooltip frames, hover chrome, drilldowns, screenshots, and side-by-side proof remain open",
        },
        "core.topbar" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "core.topbar has executable factual hover-tooltip proof for threat_button/threat_value_button and faction_button over synced real .gui rects, including missing/null no-fabrication badges and hover-clear behavior; full the game tooltip frames, hover chrome, localization expansion, screenshots, and broader topbar tooltip parity remain open",
        },
        "chrome.mapmodes" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "chrome.mapmodes has executable hover tooltip proof for real rail rows, including fidelity/source text and hover-clear behavior; all-modes popup rows share the same factual tooltip path, while legend hover, game-style tooltip frame parity, and broader map tooltip coverage remain open",
        },
        "research.tech_tree" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "research.tech_tree has executable factual tech-node hover tooltip proof for id, folder, state, and research cost with missing/null cost badged unavailable; full the game tooltip frames, prerequisite/domain-specific composers, localization expansion, and side-by-side proof remain open",
        },
        "research.doctrine_tree" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "research.doctrine_tree has executable factual doctrine-node hover tooltip proof for id, folder, exact-when-fed state, unfed-badged parser-unfed state, and full static XOR-link count; full the game tooltip frames, prerequisite/domain-specific composers, localization expansion, and side-by-side proof remain open",
        },
        "leaders.army_trait_tree" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "leaders.army_trait_tree has executable factual hover tooltip proof for current N3 owned trait-token nodes only, including derived static DEF catalog rows for scope/cost/prereq/mutual-exclusion/modifier/source metadata, inert no-leader glyph hover, and hover-clear proof; full the game trait tooltip frame, localized trait name/icon/effect text, prerequisite text/effect expansion, disabled/pickable states, and side-by-side proof remain open",
        },
        "leaders.navy_trait_tree" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "leaders.navy_trait_tree shares the executable owned trait-token hover tooltip path with the army tree and proves factual navy trait/leader rows plus derived static DEF catalog rows; full the game trait tooltip frame, localized trait name/icon/effect text, prerequisite text/effect expansion, disabled/pickable states, and side-by-side proof remain open",
        },
        "designers.tank" | "designers.plane" | "designers.equipment" => CategoryContract {
            category: InteractionCategory::HoverTooltip,
            status: InteractionStatus::Open,
            note: "component designer views have executable passive module-slot hover inspection proof over the shared module-slot grid; full the game module tooltip frames, localized/effect-expanded stat text, hover chrome, screenshots, and broader component tooltip parity remain open",
        },
        _ => not_proven(
            InteractionCategory::HoverTooltip,
            "no per-view hover/tooltip target proof attached",
        ),
    }
}

fn drilldown_contract(view_id: ViewId) -> CategoryContract {
    if view_id.0 == "chrome.find" {
        CategoryContract {
            category: InteractionCategory::Drilldown,
            status: InteractionStatus::Open,
            note: "chrome.find state and Victory Point rows emit read-only OpenView(core.state_panel, state_id), and Find/Goto Capital buttons emit read-only RecenterMap from measured state/capital geometry; country detail panels, province-level VP drill, and full find parity remain open",
        }
    } else if view_id.0 == "core.topbar" {
        CategoryContract {
            category: InteractionCategory::Drilldown,
            status: InteractionStatus::Open,
            note: "core.topbar has executable read-only OpenView proof for synced real .gui navigation and world-tension button rects, including threat_button/threat_value_button -> intel.world_tension_popup; faction_button and dismissed_alerts_button are separately proven as disabled/no-op controls; hover frames, screenshots, and broader topbar parity remain open",
        }
    } else if view_id.0 == "chrome.minimap" {
        CategoryContract {
            category: InteractionCategory::Drilldown,
            status: InteractionStatus::Open,
            note: "chrome.minimap has executable click-and-drag recenter proof via RecenterMap; screenshot proof remains open",
        }
    } else {
        not_proven(
            InteractionCategory::Drilldown,
            "no read-only drilldown target proof attached",
        )
    }
}

fn disabled_mutation_contract(view_id: ViewId) -> CategoryContract {
    match view_id.0 {
        "logistics.ingame_menu" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "logistics.ingame_menu has executable click-no-op proof for the replay-disabled save/load/resign/quit ingame-menu verbs; per-button leaf routing, disabled visual treatment, save-list filesystem browsing, and all other mutation controls remain open",
        },
        "logistics.event_window" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "logistics.event_window has executable click-no-op proof for displayed static event-definition preview option actions and inside-root modal chrome clicks; fired-event option history, popup-window buttons, disabled visual treatment, and broader event modal/mutation parity remain open",
        },
        "logistics.popup_window" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "logistics.popup_window has executable click-no-op proof for the displayed generic popup-library action area as a replay-modal disabled no-op; per-popup leaf routing, disabled visual treatment, scripted popup text, and broader popup mutation parity remain open",
        },
        "production.construction" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "production.construction has executable click-no-op proof for the top count-container factory add/sub buttons and visible queue priority/amount/close buttons; build-picker actions, market/debug controls, disabled visual treatment, and broader construction mutation parity remain open",
        },
        "production.production" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "production.production has executable click-no-op proof for visible add-line buttons, production-line close buttons, factory add/sub buttons, priority/amount buttons, and row manufacturer/equipment/convert/deploy/name-list controls; variant/obsolete equipment-info controls, disabled visual treatment, and broader production mutation parity remain open",
        },
        "production.decisions" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "production.decisions has executable click-no-op proof for visible timed-decision btn_select action buttons; one-shot decision availability, decision detail drilldowns, disabled visual treatment, and broader decisions mutation parity remain open",
        },
        "leaders.army_trait_tree" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "leaders.army_trait_tree has executable click-no-op proof for the rendered add_trait_0 through add_trait_3 trait-pick slots plus traits_tabs and preferred_tactic_tiling_container as replay-disabled no-ops; trait-pick availability, disabled visual treatment, button-level tab/wiki controls, static catalog pickable/disabled nodes, and broader trait-tree mutation parity remain open",
        },
        "leaders.navy_trait_tree" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "leaders.navy_trait_tree has executable click-no-op proof for the rendered add_trait_0 through add_trait_3 trait-pick slots plus traits_tabs as replay-disabled no-ops; trait-pick availability, disabled visual treatment, button-level tab/wiki controls, static catalog pickable/disabled nodes, and broader trait-tree mutation parity remain open",
        },
        "core.state_panel" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "core.state_panel has executable click-no-op proof for the visible occupation-law, garrison-template, and scorched-earth state controls as replay-disabled no-ops; shared-slot building remove buttons remain unrendered/unclaimed until the native shared-slot row factory exists; disabled visual treatment, province/state drilldowns, and broader state-panel mutation parity remain open",
        },
        "core.topbar" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "core.topbar has executable click-no-op proof for visible pause/date, speed-control, lobby/menu/help/achievements, playlist/music controls, dismissed_alerts_button, and faction_button as replay-disabled/no-op controls; threat_button/threat_value_button are separately proven read-only OpenView controls; disabled visual treatment, screenshots, and broader topbar mutation parity remain open",
        },
        "designers.tank" | "designers.plane" | "designers.equipment" => CategoryContract {
            category: InteractionCategory::DisabledMutation,
            status: InteractionStatus::Open,
            note: "component designer views have executable click-no-op proof for the shared non-slot designer chrome/control area as replay-disabled no-op input; upgrade sliders, module add/remove, save/duplicate/production controls, disabled visual treatment, screenshots, and broader component-designer mutation parity remain open",
        },
        _ => not_proven(
            InteractionCategory::DisabledMutation,
            "no mutating-control disabled/inert target inventory or click-no-op proof attached",
        ),
    }
}

pub fn known_interaction_targets(view_id: ViewId) -> Vec<InteractionTargetContract> {
    let mut targets = Vec::new();
    targets.extend_from_slice(known_tab_switch_targets(view_id));
    targets.extend_from_slice(known_list_selection_targets(view_id));
    targets.extend_from_slice(known_text_input_targets(view_id));
    targets.extend_from_slice(known_scroll_targets(view_id));
    targets.extend_from_slice(known_hover_tooltip_targets(view_id));
    targets.extend_from_slice(known_drilldown_targets(view_id));
    targets.extend_from_slice(known_disabled_mutation_targets(view_id));
    targets
}

pub fn known_tab_switch_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "politics.politics" => &POLITICS_TAB_TARGETS,
        "military.army" => &MILITARY_ARMY_TAB_TARGETS,
        "designers.division" => &DESIGNERS_DIVISION_TAB_TARGETS,
        "intel.intel_agency" => &INTEL_AGENCY_TAB_TARGETS,
        "chrome.find" => &CHROME_FIND_TAB_TARGETS,
        "logistics.unit_view" => &LOGISTICS_UNIT_VIEW_TAB_TARGETS,
        _ => &[],
    }
}

pub fn known_list_selection_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "politics.politics" => &POLITICS_ROW_TARGETS,
        "chrome.find" => &CHROME_FIND_ROW_TARGETS,
        "military.army" => &MILITARY_ARMY_ROW_TARGETS,
        "military.theatre_selector" => &MILITARY_THEATRE_SELECTOR_ROW_TARGETS,
        "chrome.mapmodes" => &CHROME_MAPMODES_ROW_TARGETS,
        "core.focus_tree" => &CORE_FOCUS_TREE_ROW_TARGETS,
        "logistics.unit_view" => &LOGISTICS_UNIT_VIEW_ROW_TARGETS,
        "production.production" => &PRODUCTION_PRODUCTION_ROW_TARGETS,
        "production.construction" => &PRODUCTION_CONSTRUCTION_ROW_TARGETS,
        "production.decisions" => &PRODUCTION_DECISIONS_ROW_TARGETS,
        "research.research" => &RESEARCH_RESEARCH_ROW_TARGETS,
        "research.tech_tree" => &RESEARCH_TECH_TREE_ROW_TARGETS,
        "research.doctrine_tree" => &RESEARCH_DOCTRINE_TREE_ROW_TARGETS,
        "leaders.army_trait_tree" => &LEADERS_ARMY_TRAIT_TREE_ROW_TARGETS,
        "leaders.navy_trait_tree" => &LEADERS_NAVY_TRAIT_TREE_ROW_TARGETS,
        "designers.tank" => &DESIGNERS_TANK_ROW_TARGETS,
        "designers.plane" => &DESIGNERS_PLANE_ROW_TARGETS,
        "designers.equipment" => &DESIGNERS_EQUIPMENT_ROW_TARGETS,
        _ => &[],
    }
}

pub fn known_text_input_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "chrome.find" => &CHROME_FIND_TEXT_TARGETS,
        _ => &[],
    }
}

pub fn known_scroll_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "core.state_panel" => &CORE_STATE_PANEL_SCROLL_TARGETS,
        "politics.politics" => &POLITICS_SCROLL_TARGETS,
        "chrome.find" => &CHROME_FIND_SCROLL_TARGETS,
        "military.army" => &MILITARY_ARMY_SCROLL_TARGETS,
        "military.theatre_selector" => &MILITARY_THEATRE_SELECTOR_SCROLL_TARGETS,
        "logistics.unit_view" => &LOGISTICS_UNIT_VIEW_SCROLL_TARGETS,
        "production.production" => &PRODUCTION_PRODUCTION_SCROLL_TARGETS,
        "production.construction" => &PRODUCTION_CONSTRUCTION_SCROLL_TARGETS,
        "production.decisions" => &PRODUCTION_DECISIONS_SCROLL_TARGETS,
        _ => &[],
    }
}

pub fn known_hover_tooltip_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "core.map" => &CORE_MAP_HOVER_TARGETS,
        "core.state_panel" => &CORE_STATE_PANEL_HOVER_TARGETS,
        "core.topbar" => &CORE_TOPBAR_HOVER_TARGETS,
        "chrome.mapmodes" => &CHROME_MAPMODES_HOVER_TARGETS,
        "research.tech_tree" => &RESEARCH_TECH_TREE_HOVER_TARGETS,
        "research.doctrine_tree" => &RESEARCH_DOCTRINE_TREE_HOVER_TARGETS,
        "leaders.army_trait_tree" => &LEADERS_ARMY_TRAIT_TREE_HOVER_TARGETS,
        "leaders.navy_trait_tree" => &LEADERS_NAVY_TRAIT_TREE_HOVER_TARGETS,
        "designers.tank" => &DESIGNERS_TANK_HOVER_TARGETS,
        "designers.plane" => &DESIGNERS_PLANE_HOVER_TARGETS,
        "designers.equipment" => &DESIGNERS_EQUIPMENT_HOVER_TARGETS,
        _ => &[],
    }
}

pub fn known_drilldown_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "chrome.find" => &CHROME_FIND_DRILLDOWN_TARGETS,
        "core.topbar" => &CORE_TOPBAR_DRILLDOWN_TARGETS,
        "chrome.minimap" => &CHROME_MINIMAP_DRILLDOWN_TARGETS,
        _ => &[],
    }
}

pub fn known_disabled_mutation_targets(view_id: ViewId) -> &'static [InteractionTargetContract] {
    match view_id.0 {
        "logistics.ingame_menu" => &LOGISTICS_INGAME_MENU_DISABLED_MUTATION_TARGETS,
        "logistics.event_window" => &LOGISTICS_EVENT_WINDOW_DISABLED_MUTATION_TARGETS,
        "logistics.popup_window" => &LOGISTICS_POPUP_WINDOW_DISABLED_MUTATION_TARGETS,
        "production.production" => &PRODUCTION_PRODUCTION_DISABLED_MUTATION_TARGETS,
        "production.construction" => &PRODUCTION_CONSTRUCTION_DISABLED_MUTATION_TARGETS,
        "production.decisions" => &PRODUCTION_DECISIONS_DISABLED_MUTATION_TARGETS,
        "leaders.army_trait_tree" => &LEADERS_ARMY_TRAIT_TREE_DISABLED_MUTATION_TARGETS,
        "leaders.navy_trait_tree" => &LEADERS_NAVY_TRAIT_TREE_DISABLED_MUTATION_TARGETS,
        "core.state_panel" => &CORE_STATE_PANEL_DISABLED_MUTATION_TARGETS,
        "core.topbar" => &CORE_TOPBAR_DISABLED_MUTATION_TARGETS,
        "designers.tank" => &DESIGNERS_TANK_DISABLED_MUTATION_TARGETS,
        "designers.plane" => &DESIGNERS_PLANE_DISABLED_MUTATION_TARGETS,
        "designers.equipment" => &DESIGNERS_EQUIPMENT_DISABLED_MUTATION_TARGETS,
        _ => &[],
    }
}

const POLITICS_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable PoliticsView tests prove click switching, unavailable-tab inert handling, and selected-tab A->B->A persistence for this read-only politics slice";
const POLITICS_ROW_NOTE: &str =
    "dynamic politics row target; executable PoliticsView tests prove stable-keyed read-only row selection/highlight for Government/National Spirits/Ideas & Laws/Autonomy rows, duplicate/unkeyed-row inert behavior, scrolled backing-row selection, and selected-row A->B->A persistence without carrying B-only rows; detail drilldowns, mutation controls, and broader politics row parity remain open";
const POLITICS_SCROLL_NOTE: &str =
    "dynamic politics row scroll target; executable PoliticsView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible rows for long National Spirits and Ideas & Laws lists, captured thumb drags continue outside the list until pointer-up, track clicks do not select underlying rows, outside-list wheel input is inert, per-tab scroll state stays isolated, scrolled clicks select the correct backing row, and scroll+selection survives A->B->A without carrying B-only rows; Government/Autonomy long-list scroll, exact scrollbar art, politics modal lists, and broader politics scroll surfaces remain open";
const MILITARY_ARMY_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable ArmyView tests prove click switching, unavailable-tab inert handling, and selected-tab A->B->A persistence for this read-only roster slice";
const MILITARY_ARMY_ROW_NOTE: &str =
    "dynamic roster row target; executable ArmyView tests prove keyed read-only row selection/highlight, scrolled backing-row selection, and selected-row A->B->A persistence for this Army/Navy/Air roster slice";
const MILITARY_ARMY_SCROLL_NOTE: &str =
    "dynamic roster scroll target; executable ArmyView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible rows across Army/Navy/Air rosters, captured thumb drags continue outside the list until pointer-up, track clicks do not select underlying rows, outside-list wheel input is inert, per-tab scroll state stays isolated, scrolled clicks select the correct backing row, and scroll offset survives A->B->A without carrying B-only rows; exact scrollbar art and other military scroll surfaces remain open";
const MILITARY_THEATRE_SELECTOR_ROW_NOTE: &str =
    "dynamic theatre row target; executable TheatreSelectorView tests prove measured theatre id read-only row selection/highlight, scrolled backing-row selection, A->B->A persistence from current-hour rows, unkeyed-row inert behavior, and missing selected-row fail-closed proof; group/member drilldowns, alerts, pin/priority UI state, and broader theatre parity remain open";
const MILITARY_THEATRE_SELECTOR_SCROLL_NOTE: &str =
    "dynamic theatre list scroll target; executable TheatreSelectorView tests prove wheel scroll changes/clamps visible measured theatre rows, scrollbar-thumb drag changes/clamps rows and continues outside the list until pointer-up, scrollbar track clicks do not select underlying rows, scrolled clicks select the correct backing theatre row, scroll+selection survive A->B->A without carrying B-only rows, and outside-list wheel input is inert; scrollbar art, group/member rows, and broader theatre scroll surfaces remain open";
const DESIGNERS_DIVISION_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable DivisionDesignerView tests prove Division/Army HQ click switching, non-HQ inert handling, and selected-tab A->B->A persistence for this read-only designer slice";
const DESIGNERS_COMPONENT_SLOT_ROW_NOTE: &str =
    "dynamic component module-slot target; executable component designer tests prove read-only module-slot click selection for tank, plane, and equipment designers with no emitted commands, selected-slot digest visibility, repeated-click no-redraw behavior, and selected-slot save->missing->save scrub restoration without falling back to another slot; module-slot detail parity, selector drilldowns, disabled visuals, screenshots, and broader component-designer parity remain open";
const DESIGNERS_COMPONENT_SLOT_HOVER_NOTE: &str =
    "dynamic component module-slot hover target; executable component designer tests prove passive hover inspection over the shared module-slot grid with no emitted commands, redraw only when hover changes, digest-visible hovered slot state, and repeat-hover no-redraw behavior; this is factual inspection, not full the game module tooltip frame/effect text parity, and screenshots remain open";
const DESIGNERS_COMPONENT_DISABLED_MUTATION_NOTE: &str =
    "component designer non-slot control target; executable component designer tests prove clicks inside the shared non-slot designer chrome/control area are consumed as replay-disabled no-op pointer-down/up input with no emitted commands, no redraw request, and unchanged bind digest; upgrade sliders, module add/remove, save/duplicate/production controls, disabled visual treatment, screenshots, and broader component-designer mutation parity remain open";
const INTEL_AGENCY_TAB_NOTE: &str =
    "dynamic widgets::tabs target; executable IntelAgencyView tests prove click switching, unavailable-tab inert handling, and selected-tab A->B->A persistence for this read-only intelligence slice";
const CHROME_FIND_TAB_NOTE: &str =
    "fixed widgets::tabs target; executable FindView tests prove click switching, inert-click handling, and selected-tab A->B->A persistence for this read-only catalog slice";
const LOGISTICS_UNIT_VIEW_TAB_NOTE: &str =
    "fixed unit stats-panel tab target; executable UnitView tests prove Stats/History/Template click switching, unchanged-tab inert redraw handling, selected-tab A->B->A persistence, and no fabricated equipment body on non-stats tabs; template/history data binding and broader unit-detail parity remain open";
const CHROME_FIND_ROW_NOTE: &str =
    "fixed catalog row target; executable FindView tests prove read-only row selection/highlight, outside-row inert handling, and selected-row A->B->A persistence for this snapshot-derived catalog slice";
const CHROME_FIND_TEXT_NOTE: &str =
    "fixed edit-box target; executable FindView tests prove replay-local focus, typed character/backspace handling, token filtering over honest catalog labels/keys, scroll reset/clamp, and query A->B->A persistence; localized names and full text-field parity remain open";
const CHROME_FIND_SCROLL_NOTE: &str =
    "fixed catalog scroll target; executable FindView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible rows, captured thumb drags continue outside the list until pointer-up, track clicks do not select or drill underlying rows, scrolled clicks select the correct backing row, and scroll offset survives A->B->A without carrying B-only rows; exact scrollbar art and screenshot proof remain open";
const CORE_STATE_PANEL_SCROLL_NOTE: &str =
    "fixed state-building grid scroll target; executable StatePanelView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible building rows using the real state_building_entries geometry, captured thumb drags continue until pointer-up, track clicks are inert and do not scroll or leak to row/map actions, outside-grid wheel input is inert, and scrub rebind clamps/clears stale drag when row count shrinks; exact scrollbar art, resources-grid scroll, province/state drilldown links, and broader state-panel parity remain open";
const CORE_STATE_PANEL_HOVER_NOTE: &str =
    "fixed state-panel hover target; executable StatePanelView tests prove passive factual tooltips for owner flags, scrolled building rows, and resource rows, with hover-clear behavior and stale-row rebind clearing instead of falling back to fabricated rows; full the game tooltip frames, hover chrome, province/state drilldown links, screenshots, and side-by-side proof remain open";
const CORE_STATE_PANEL_DISABLED_MUTATION_NOTE: &str =
    "fixed state-panel mutation target; executable StatePanelView tests prove select_law_button, template_garrison_button, and scorched_earth_state_button are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged bind digest while neighboring panel space remains unclaimed by the view-specific handler; shared-slot building remove buttons are separately proven unrendered/unclaimed because the native renderer still instantiates state_building_entry rows, not the shared-slot row factory; disabled visual treatment, province/state drilldowns, and broader state-panel mutation parity remain open";
const CHROME_FIND_STATE_PANEL_DRILLDOWN_NOTE: &str =
    "fixed catalog drill target; executable FindView tests prove state-backed rows emit read-only OpenView(core.state_panel, state_id); country detail panels, province-level VP drill, and full find parity remain open";
const CHROME_FIND_RECENTER_NOTE: &str =
    "fixed find navigation target; executable FindView tests prove Find/Goto Capital buttons emit read-only RecenterMap from selected or first filtered catalog rows and measured capital VP geometry; exact localized/province-level find parity remains open";
const CHROME_MINIMAP_DRILLDOWN_NOTE: &str =
    "fixed minimap navigation target; executable MinimapView tests prove click-and-drag recenter emits RecenterMap, pointer-up clears drag capture state, and letterbox/outside clicks do not fabricate map picks; screenshot proof remains open";
const CORE_TOPBAR_DRILLDOWN_NOTE: &str =
    "fixed topbar navigation/world-tension target; executable TopbarView tests prove synced real .gui button rects emit read-only OpenView commands, including threat_button/threat_value_button -> intel.world_tension_popup, leave the topbar render digest unchanged, and keep empty-space clicks falling through; faction_button and dismissed_alerts_button are separately proven as disabled/no-op controls, while hover frames, screenshots, and broader topbar parity remain open";
const CORE_TOPBAR_HOVER_NOTE: &str =
    "fixed topbar hover target; executable TopbarView tests prove threat_button/threat_value_button hover emits a factual World tension tooltip with aggregate unavailable and measured threat-ledger count when fed, missing ledgers badged unavailable instead of fabricated zero, and faction_button hover emits factual country/faction name/leader rows with missing/null faction fields badged unavailable and hover-clear behavior; full the game tooltip frames, hover chrome, localization expansion, screenshots, and broader topbar tooltip parity remain open";
const CORE_TOPBAR_DISABLED_MUTATION_NOTE: &str =
    "fixed topbar time-control target; executable TopbarView tests prove pause_button_date, pause_button_date_paused, button_speeddown, button_speedstep1, button_speedstep2, button_speedstep3, button_speedstep4, button_speedstep5, and button_speedup are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged topbar digest; shell tests prove higher-z panels prevent hidden topbar time-control targets from arming; disabled visual treatment, screenshots, and broader topbar mutation parity remain open";
const CORE_TOPBAR_DISABLED_CHROME_MUTATION_NOTE: &str =
    "fixed topbar lobby/menu/music chrome target; executable TopbarView tests prove reopen_ingame_lobby, menu_button, help_button, achievements_button, playlist_button, music_pause_button, music_next_button, and musicplayer_button are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged topbar digest; shell tests prove higher-z panels prevent hidden topbar menu/music targets from arming; disabled visual treatment, screenshots, and broader topbar mutation parity remain open";
const CORE_TOPBAR_ALERT_FACTION_NOOP_NOTE: &str =
    "fixed topbar alert/faction no-op target; executable TopbarView tests prove dismissed_alerts_button and faction_button are resolved from synced real .gui rects, consume pointer-down/up as replay-disabled/no-op controls with no emitted commands, no redraw request, and unchanged topbar digest; a missing alert ledger stays inert instead of opening fabricated data, faction inspection is limited to factual hover rows, disabled visual treatment, screenshots, and broader topbar mutation parity remain open";
const CHROME_MAPMODES_ROW_NOTE: &str =
    "fixed map-mode rail row target; executable MapModesView tests prove real laid-out mapmode_button_* slots emit read-only SetMapMode commands while unavailable/data-proxy badges remain honest; custom slot configuration/defaults, legend hover, and screenshots remain open";
const CHROME_MAPMODES_ALL_NOTE: &str =
    "real mapmodes_all popup target; executable MapModesView tests prove the extracted all-modes button opens WindowWithAllMapModes and the popup Supply/Intel rows emit read-only SetMapMode commands while unavailable/data-proxy badges remain honest; custom slot configuration/defaults, legend hover, and screenshots remain open";
const CHROME_MAPMODES_HOVER_NOTE: &str =
    "fixed map-mode hover target; executable MapModesView tests prove real row hover emits a factual tooltip with active state, fidelity/source text, and clear-on-exit behavior; full the game tooltip frame/legend parity remain open";
const CORE_MAP_HOVER_NOTE: &str =
    "bottom-layer map hover target; executable MapView tests prove exposed-map CPU raster picks emit factual province/state/owner tooltip rows, missing land metadata stays unavailable, and broader the game map tooltip/frame/legend parity remains open";
const CORE_FOCUS_TREE_ROW_NOTE: &str =
    "dynamic focus-node target; executable FocusShell tests prove exact focus-id read-only node selection/highlight, drag-pan conflict protection, missing-selected fail-closed proof without node fallback, selected-node A->B->A persistence while rebuilding current focus state, and a factual selected-detail panel/digest slice for id/state/cost/current progress-days/prereq-group count/position fidelity with missing cost/progress never fabricated as zero; full the game national_focus_detail_view parity, filter/shortcut controls, focus mutation controls, and broader focus-tree parity remain open";
const RESEARCH_TECH_TREE_HOVER_NOTE: &str =
    "tech-tree hover target; executable TechTreeView tests prove factual no-fabrication tooltip rows for tech id, folder, state, and research cost, with missing/null cost unavailable instead of zero; full the game frame/prerequisite/localized/comparison tooltip parity remains open";
const RESEARCH_DOCTRINE_TREE_HOVER_NOTE: &str =
    "doctrine-tree hover target; executable DoctrineTreeView tests prove factual no-fabrication tooltip rows for doctrine id, folder, exact-when-fed state, unfed-badged parser-unfed state, and full static XOR-link count including grand-doctrine groups; full the game frame/prerequisite/localized/comparison tooltip parity remains open";
const LOGISTICS_UNIT_VIEW_ROW_NOTE: &str =
    "dynamic division row target; executable UnitView tests prove measured div_id read-only row selection updates the equipment pane, survives A->B->A by rebuilding current-hour equipment, rejects missing div_id/index identity, and fails closed when the selected division is missing or foreign-owned; ship rows, column sorts, and broader unit-detail parity remain open";
const LOGISTICS_UNIT_VIEW_DIVISION_SCROLL_NOTE: &str =
    "dynamic division-list scroll target; executable UnitView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible measured div_id rows, captured thumb drags continue outside the list until pointer-up, track clicks do not select underlying rows, stale drag captures clear on rebind when geometry disappears, scrolled clicks select the correct backing division, scroll+selection survive A->B->A without carrying B-only rows, and outside-list wheel input is inert; exact scrollbar art, ship rows, column sorts, and broader unit-detail parity remain open";
const LOGISTICS_UNIT_VIEW_EQUIPMENT_SCROLL_NOTE: &str =
    "dynamic selected-division equipment-pane scroll target; executable UnitView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible equipment rows, captured thumb drags continue outside the pane until pointer-up, track clicks are inert, equipment scrolling does not move the division list, resets on explicit selected-division changes, survives A->B->A without carrying B-only rows, and fails closed with eq_rows=0/eq_visible=[] when the selected division is missing; exact scrollbar art, ship rows, column sorts, and broader unit-detail parity remain open";
const PRODUCTION_PRODUCTION_ROW_NOTE: &str =
    "dynamic production-line row target; executable ProductionView tests prove measured live id and save line_id read-only row selection for military/naval lines, scrolled backing-row selection, A->B->A persistence from current-hour rows, unkeyed/duplicate-row inert behavior, and missing selected-row fail-closed proof; line detail drilldowns, filters, mutating line controls, and side-by-side visual parity remain open";
const PRODUCTION_PRODUCTION_SCROLL_NOTE: &str =
    "dynamic production-line scroll target; executable ProductionView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible line rows, captured thumb drags continue outside the list until pointer-up, track clicks do not select underlying rows, scrolled clicks select the correct backing production key, scroll+selection survive A->B->A without carrying B-only rows, and outside-grid wheel input is inert; scrollbar art, filters, mutating line controls, and side-by-side visual parity remain open";
const PRODUCTION_CONSTRUCTION_ROW_NOTE: &str =
    "dynamic construction-queue row target; executable ConstructionView tests prove measured live line_id and save template/state/created read-only row selection, scrolled backing-row selection, A->B->A persistence from current-hour rows, unkeyed-row inert behavior, and missing selected-row fail-closed proof; building details, build-picker actions, market/debug controls, and broader construction parity remain open";
const PRODUCTION_CONSTRUCTION_SCROLL_NOTE: &str =
    "dynamic construction-queue scroll target; executable ConstructionView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible queue rows, captured thumb drags continue outside the grid until pointer-up, track clicks do not select underlying rows, scrolled clicks select the correct backing construction key, scroll+selection survive A->B->A without carrying B-only rows, and outside-grid wheel input is inert; scrollbar art, build-picker actions, market/debug controls, and broader construction parity remain open";
const PRODUCTION_DECISIONS_ROW_NOTE: &str =
    "dynamic timed-decision row target; executable DecisionsView tests prove measured decision/state read-only row selection, duplicate decision id safety, A->B->A scrub persistence from current-hour rows, unkeyed-row inert behavior, and missing selected-row fail-closed proof; decision detail drilldowns, one-shot availability, and mutating decision buttons remain open";
const PRODUCTION_DECISIONS_SCROLL_NOTE: &str =
    "dynamic timed-decision list scroll target; executable DecisionsView tests prove wheel scroll and scrollbar-thumb drag change/clamp visible rows, captured thumb drags continue outside the grid until pointer-up, track clicks do not select underlying rows, scrolled clicks select the correct backing decision key, scroll+selection survive A->B->A without carrying B-only rows, and outside-grid wheel input is inert; scrollbar art, decision detail drilldowns, one-shot availability, and mutating decision buttons remain open";
const RESEARCH_RESEARCH_ROW_NOTE: &str =
    "dynamic active-research slot row target; executable ResearchView tests prove exact-tech-token read-only slot selection/highlight, empty/outside slot inert behavior, missing-selected fail-closed proof without row fallback, and selected-slot A->B->A persistence without carrying B-only rows; ETA/progress parity, tech-tree drilldowns, and mutating research controls remain open";
const RESEARCH_TECH_TREE_ROW_NOTE: &str =
    "dynamic tech-tree node target; executable TechTreeView tests prove exact catalog tech-id read-only node selection/highlight, drag-pan conflict protection, missing-selected fail-closed proof without node fallback, selected-node A->B->A persistence while rebuilding current research state, and a factual selected-detail panel/digest slice for id/folder/state/research cost with unavailable cost never fabricated as zero; full the game detail-window parity, research-start mutation controls, and broader tech-tree parity remain open";
const RESEARCH_DOCTRINE_TREE_ROW_NOTE: &str =
    "dynamic doctrine-tree node target; executable DoctrineTreeView tests prove exact catalog doctrine-id read-only node selection/highlight, drag-pan conflict protection, missing-selected fail-closed proof without node fallback, selected-node A->B->A persistence while rebuilding current doctrine state when exact doctrine data is fed, and a factual selected-detail panel/digest slice for id/folder/exact-when-fed-or-unfed-badged-state/XOR-link count without fabricating unresearched; full the game detail-window parity, doctrine tooltip parity, and broader doctrine-tree parity remain open";
const LEADERS_ARMY_TRAIT_TREE_ROW_NOTE: &str =
    "dynamic N3 owned-trait node target; executable ArmyTraitTreeView tests prove exact trait-token read-only node selection/highlight, inert no-leader/no-traits unavailable glyphs, drag movement canceling click selection, missing-selected fail-closed proof without node fallback, selected-node A->B->A persistence while rebuilding current leader trait data, and a factual selected-detail panel/digest slice for trait id/owned state/branch/leader name/leader key plus derived static DEF catalog scope/cost/prereq/mutual-exclusion/modifier/source rows with null trait_type unavailable; full static trait catalog graph, disabled/pickable states, icons/effect text, leader-roster driven selection, and broader trait-tree parity remain open";
const LEADERS_NAVY_TRAIT_TREE_ROW_NOTE: &str =
    "dynamic N3 owned-trait node target; executable NavyTraitTreeView tests prove exact admiral trait-token read-only node selection/highlight, factual navy branch/leader detail rows, and derived static DEF catalog rows, sharing the inert unavailable glyph and fail-closed selection model with the army trait tree; full static trait catalog graph, disabled/pickable states, icons/effect text, leader-roster driven selection, and broader trait-tree parity remain open";
const LEADERS_ARMY_TRAIT_TREE_HOVER_NOTE: &str =
    "dynamic N3 owned-trait hover target; executable ArmyTraitTreeView tests prove factual passive tooltip rows for trait id, owned state, branch, leader name, leader key, and derived static DEF catalog scope/cost/prereq/mutual-exclusion/modifier/source rows, plus inert no-leader glyph hover and clear-on-exit behavior; full the game trait tooltip frame, localized name/icon/effect text, prerequisite text/effect expansion, disabled/pickable states, and side-by-side proof remain open";
const LEADERS_NAVY_TRAIT_TREE_HOVER_NOTE: &str =
    "dynamic N3 owned-trait hover target; executable NavyTraitTreeView tests prove the same factual tooltip composer for admiral trait tokens, exact navy branch rows, and derived static DEF catalog rows; full the game trait tooltip frame, localized name/icon/effect text, prerequisite text/effect expansion, disabled/pickable states, and side-by-side proof remain open";
const LEADERS_ARMY_TRAIT_TREE_DISABLED_MUTATION_NOTE: &str =
    "fixed trait-tree add-trait/control target; executable ArmyTraitTreeView tests prove add_trait_0, add_trait_1, add_trait_2, add_trait_3, traits_tabs, and preferred_tactic_tiling_container are resolved from the rendered real .gui roots and consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected trait/detail bind digest; trait-pick availability, disabled visual treatment, button-level tab/wiki controls, static catalog pickable/disabled nodes, and broader trait-tree mutation parity remain open";
const LEADERS_NAVY_TRAIT_TREE_DISABLED_MUTATION_NOTE: &str =
    "fixed trait-tree add-trait/control target; executable NavyTraitTreeView tests prove add_trait_0, add_trait_1, add_trait_2, add_trait_3, and traits_tabs are resolved from the rendered real .gui roots and consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected trait/detail bind digest; trait-pick availability, disabled visual treatment, button-level tab/wiki controls, static catalog pickable/disabled nodes, and broader trait-tree mutation parity remain open";
const LOGISTICS_INGAME_MENU_DISABLED_MUTATION_NOTE: &str =
    "fixed ingame-menu verb target; executable IngameMenuView/WindowHost tests prove replay-disabled Save/Load/Resign/Quit click streams are consumed as no-op modal input with no emitted commands, no redraw request, and unchanged bind digest; per-button leaf routing, disabled visual treatment, save-list filesystem browsing, and broader mutation parity remain open";
const LOGISTICS_EVENT_WINDOW_DISABLED_MUTATION_NOTE: &str =
    "dynamic event-definition preview option/chrome target; executable EventWindowView/WindowHost tests prove displayed static DEF preview option actions and inside-root outside-option chrome clicks are consumed as replay-disabled/modal no-op clicks with no emitted commands, no redraw request, and unchanged bind digest; off-root clicks are not claimed by EventWindowView-specific routing and are blocked only by the host modal policy; fired-event option history, popup-window buttons, disabled visual treatment, minimize/leader leaf routing, and broader event modal parity remain open";
const LOGISTICS_POPUP_WINDOW_DISABLED_MUTATION_NOTE: &str =
    "generic popup-library action target; executable PopupWindowView/WindowHost tests prove the displayed first popup's real accept-button area is consumed as replay-modal disabled no-op input with no emitted commands, no redraw request, and unchanged bind digest; per-root popup button leaf routing, disabled visual treatment, scripted popup text, and broader popup mutation parity remain open";
const PRODUCTION_CONSTRUCTION_DISABLED_MUTATION_NOTE: &str =
    "top count-container factory assignment target; executable ConstructionView tests prove factories_sub_1x/factories_sub_5x/factories_sub_10x and factories_add_1x/factories_add_5x/factories_add_10x collapse to two physical add/sub hit areas that are consumed as replay-disabled no-op clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring count-container clicks remain unclaimed; build-picker actions, market/debug controls, disabled visual treatment, and broader construction mutation parity remain open";
const PRODUCTION_CONSTRUCTION_QUEUE_MUTATION_NOTE: &str =
    "visible construction-queue row target; executable ConstructionView tests prove production_building_line_entry increase_priority/decrease_priority, increase_amount/decrease_amount, and close_production_line controls are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring row-body clicks still select the backing construction row; build-picker actions, market/debug controls, disabled visual treatment, and broader construction mutation parity remain open";
const PRODUCTION_PRODUCTION_CLOSE_MUTATION_NOTE: &str =
    "visible production-line close target; executable ProductionView tests prove close_production_line pointer-down/up on a visible scrolled production row is consumed as a replay-disabled no-op with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring row-body clicks still select the backing row; variant/obsolete equipment-info controls, disabled visual treatment, and broader production mutation parity remain open";
const PRODUCTION_PRODUCTION_FACTORY_MUTATION_NOTE: &str =
    "visible production-line factory assignment target; production.json now preserves countryproductionlineview.gui @prod_* defines so factories_sub_1x/factories_sub_5x/factories_sub_10x and factories_add_1x/factories_add_5x/factories_add_10x resolve to exact row coordinates; executable ProductionView tests prove those physical add/sub hit areas are consumed as replay-disabled no-op clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring row-body clicks still select the backing row; variant/obsolete equipment-info controls, disabled visual treatment, and broader production mutation parity remain open";
const PRODUCTION_PRODUCTION_PRIORITY_AMOUNT_MUTATION_NOTE: &str =
    "visible production-line priority and amount target; executable ProductionView tests prove increase_priority/decrease_priority plus military btn_x10/btn_x5 and naval increase_amount/decrease_amount row controls are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring row-body clicks still select the backing row; variant/obsolete equipment-info controls, disabled visual treatment, and broader production mutation parity remain open";
const PRODUCTION_PRODUCTION_ADD_LINE_MUTATION_NOTE: &str =
    "visible root add-line target; executable ProductionView tests prove infantry_button/tank_button/aircraft_button/naval_button/naval_repair_button are resolved from real production.json offsets plus measured 81x41 button frames and consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/visible rows/bind digest while empty space between aircraft and naval groups remains unclaimed; variant/obsolete equipment-info controls, disabled visual treatment, and broader production mutation parity remain open";
const PRODUCTION_PRODUCTION_LINE_CONTROLS_MUTATION_NOTE: &str =
    "visible production-line manufacturer/equipment/convert/deploy/name-list target; executable ProductionView tests prove production_military_line_entry attach_manufacturer/change_manufacturer/equipment_button/expand/convert and production_naval_line_entry attach_manufacturer/change_manufacturer/equipment_button/expand/carrier_air_wing_composition/deploy_location/name_list_btn controls resolve from real template positions and are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring equipment and name-list row-body clicks still select the backing row; variant/obsolete equipment-info controls, unrendered refit-template controls, disabled visual treatment, and broader production mutation parity remain open";
const PRODUCTION_DECISIONS_SELECT_MUTATION_NOTE: &str =
    "visible timed-decision action target; executable DecisionsView tests prove timed_decision_item btn_select controls are consumed as replay-disabled no-op pointer-down/up clicks with no emitted commands, no redraw request, and unchanged selected row/scroll/bind digest while neighboring row-body clicks still select the backing timed decision and scrolled visible rows remain correctly resolved; one-shot decision availability, disabled visual treatment, and broader decisions mutation parity remain open";

const LEADERS_ARMY_TRAIT_TREE_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "leaders.army_trait_tree.mutation.add_trait_slots",
        label: "Add-trait slots",
        status: InteractionStatus::Open,
        note: LEADERS_ARMY_TRAIT_TREE_DISABLED_MUTATION_NOTE,
    }];
const LEADERS_NAVY_TRAIT_TREE_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "leaders.navy_trait_tree.mutation.add_trait_slots",
        label: "Add-trait slots",
        status: InteractionStatus::Open,
        note: LEADERS_NAVY_TRAIT_TREE_DISABLED_MUTATION_NOTE,
    }];
const LOGISTICS_INGAME_MENU_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "logistics.ingame_menu.mutation.verbs",
        label: "Save/Load/Resign/Quit verbs",
        status: InteractionStatus::Open,
        note: LOGISTICS_INGAME_MENU_DISABLED_MUTATION_NOTE,
    }];
const CORE_STATE_PANEL_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "core.state_panel.mutation.occupation_controls",
        label: "Occupation law/template and scorched-earth controls",
        status: InteractionStatus::Open,
        note: CORE_STATE_PANEL_DISABLED_MUTATION_NOTE,
    }];
const CORE_TOPBAR_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "core.topbar.mutation.time_controls",
        label: "Pause/date and speed controls",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DISABLED_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "core.topbar.mutation.lobby_menu_music",
        label: "Lobby/menu/help/achievements and music controls",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DISABLED_CHROME_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "core.topbar.mutation.alerts_faction_noops",
        label: "Dismissed alerts and faction no-op controls",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_ALERT_FACTION_NOOP_NOTE,
    },
];
const LOGISTICS_EVENT_WINDOW_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "logistics.event_window.mutation.definition_options",
        label: "Definition preview option actions",
        status: InteractionStatus::Open,
        note: LOGISTICS_EVENT_WINDOW_DISABLED_MUTATION_NOTE,
    }];
const LOGISTICS_POPUP_WINDOW_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "logistics.popup_window.mutation.displayed_action",
        label: "Displayed popup action",
        status: InteractionStatus::Open,
        note: LOGISTICS_POPUP_WINDOW_DISABLED_MUTATION_NOTE,
    }];
const PRODUCTION_CONSTRUCTION_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 2] = [
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.construction.mutation.count_container_factories",
        label: "Top factory add/sub buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_CONSTRUCTION_DISABLED_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.construction.mutation.queue_controls",
        label: "Visible queue priority/amount/close buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_CONSTRUCTION_QUEUE_MUTATION_NOTE,
    },
];
const PRODUCTION_PRODUCTION_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 5] = [
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.production.mutation.add_line",
        label: "Visible add-line buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_ADD_LINE_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.production.mutation.close_line",
        label: "Visible line close buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_CLOSE_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.production.mutation.factory_count",
        label: "Visible factory add/sub buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_FACTORY_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.production.mutation.priority_amount",
        label: "Visible priority/amount buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_PRIORITY_AMOUNT_MUTATION_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.production.mutation.line_controls",
        label: "Visible line manufacturer/equipment/deploy controls",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_LINE_CONTROLS_MUTATION_NOTE,
    },
];

const PRODUCTION_DECISIONS_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "production.decisions.mutation.timed_select",
        label: "Timed decision select buttons",
        status: InteractionStatus::Open,
        note: PRODUCTION_DECISIONS_SELECT_MUTATION_NOTE,
    }];

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

const POLITICS_ROW_TARGETS: [InteractionTargetContract; 4] = [
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "politics.politics.row.government",
        label: "Government row",
        status: InteractionStatus::Open,
        note: POLITICS_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "politics.politics.row.national_spirits",
        label: "National Spirits row",
        status: InteractionStatus::Open,
        note: POLITICS_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "politics.politics.row.ideas_laws",
        label: "Ideas & Laws row",
        status: InteractionStatus::Open,
        note: POLITICS_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "politics.politics.row.autonomy",
        label: "Autonomy row",
        status: InteractionStatus::Open,
        note: POLITICS_ROW_NOTE,
    },
];

const POLITICS_SCROLL_TARGETS: [InteractionTargetContract; 2] = [
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "politics.politics.scroll.national_spirits",
        label: "National Spirits wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: POLITICS_SCROLL_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "politics.politics.scroll.ideas_laws",
        label: "Ideas & Laws wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: POLITICS_SCROLL_NOTE,
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

const MILITARY_ARMY_ROW_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "military.army.row.army",
        label: "Army roster row",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "military.army.row.navy",
        label: "Navy roster row",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_ROW_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "military.army.row.air",
        label: "Air roster row",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_ROW_NOTE,
    },
];

const MILITARY_ARMY_SCROLL_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "military.army.scroll.army",
        label: "Army roster wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_SCROLL_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "military.army.scroll.navy",
        label: "Navy roster wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_SCROLL_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "military.army.scroll.air",
        label: "Air roster wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: MILITARY_ARMY_SCROLL_NOTE,
    },
];

const MILITARY_THEATRE_SELECTOR_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "military.theatre_selector.row.theatre",
        label: "Theatre selector row",
        status: InteractionStatus::Open,
        note: MILITARY_THEATRE_SELECTOR_ROW_NOTE,
    }];

const MILITARY_THEATRE_SELECTOR_SCROLL_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "military.theatre_selector.scroll.theatres",
        label: "Theatre selector list scroll",
        status: InteractionStatus::Open,
        note: MILITARY_THEATRE_SELECTOR_SCROLL_NOTE,
    }];

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

const DESIGNERS_TANK_ROW_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::ListSelection,
    id: "designers.tank.row.module_slot",
    label: "Tank module slot selection",
    status: InteractionStatus::Open,
    note: DESIGNERS_COMPONENT_SLOT_ROW_NOTE,
}];
const DESIGNERS_PLANE_ROW_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::ListSelection,
    id: "designers.plane.row.module_slot",
    label: "Plane module slot selection",
    status: InteractionStatus::Open,
    note: DESIGNERS_COMPONENT_SLOT_ROW_NOTE,
}];
const DESIGNERS_EQUIPMENT_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "designers.equipment.row.module_slot",
        label: "Equipment module slot selection",
        status: InteractionStatus::Open,
        note: DESIGNERS_COMPONENT_SLOT_ROW_NOTE,
    }];

const DESIGNERS_TANK_HOVER_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::HoverTooltip,
    id: "designers.tank.hover.module_slot",
    label: "Tank module slot hover inspection",
    status: InteractionStatus::Open,
    note: DESIGNERS_COMPONENT_SLOT_HOVER_NOTE,
}];
const DESIGNERS_PLANE_HOVER_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::HoverTooltip,
    id: "designers.plane.hover.module_slot",
    label: "Plane module slot hover inspection",
    status: InteractionStatus::Open,
    note: DESIGNERS_COMPONENT_SLOT_HOVER_NOTE,
}];
const DESIGNERS_EQUIPMENT_HOVER_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "designers.equipment.hover.module_slot",
        label: "Equipment module slot hover inspection",
        status: InteractionStatus::Open,
        note: DESIGNERS_COMPONENT_SLOT_HOVER_NOTE,
    }];

const DESIGNERS_TANK_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "designers.tank.mutation.component_controls",
        label: "Tank designer non-slot controls",
        status: InteractionStatus::Open,
        note: DESIGNERS_COMPONENT_DISABLED_MUTATION_NOTE,
    }];
const DESIGNERS_PLANE_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "designers.plane.mutation.component_controls",
        label: "Plane designer non-slot controls",
        status: InteractionStatus::Open,
        note: DESIGNERS_COMPONENT_DISABLED_MUTATION_NOTE,
    }];
const DESIGNERS_EQUIPMENT_DISABLED_MUTATION_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::DisabledMutation,
        id: "designers.equipment.mutation.component_controls",
        label: "Equipment designer non-slot controls",
        status: InteractionStatus::Open,
        note: DESIGNERS_COMPONENT_DISABLED_MUTATION_NOTE,
    }];

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

const CHROME_FIND_TEXT_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::TextInput,
    id: "chrome.find.text.query",
    label: "Find query text box",
    status: InteractionStatus::Open,
    note: CHROME_FIND_TEXT_NOTE,
}];

const LOGISTICS_UNIT_VIEW_TAB_TARGETS: [InteractionTargetContract; 3] = [
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "logistics.unit_view.tab.stats",
        label: "Stats",
        status: InteractionStatus::Open,
        note: LOGISTICS_UNIT_VIEW_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "logistics.unit_view.tab.history",
        label: "History",
        status: InteractionStatus::Open,
        note: LOGISTICS_UNIT_VIEW_TAB_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::TabSwitch,
        id: "logistics.unit_view.tab.template",
        label: "Template",
        status: InteractionStatus::Open,
        note: LOGISTICS_UNIT_VIEW_TAB_NOTE,
    },
];

const CHROME_FIND_SCROLL_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::Scroll,
    id: "chrome.find.scroll.results",
    label: "Results scroll",
    status: InteractionStatus::Open,
    note: CHROME_FIND_SCROLL_NOTE,
}];

const CORE_STATE_PANEL_SCROLL_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "core.state_panel.scroll.buildings",
        label: "Building grid wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: CORE_STATE_PANEL_SCROLL_NOTE,
    }];

const CORE_STATE_PANEL_HOVER_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "core.state_panel.hover.factual_rows",
        label: "State panel factual hover tooltips",
        status: InteractionStatus::Open,
        note: CORE_STATE_PANEL_HOVER_NOTE,
    }];

const CORE_TOPBAR_HOVER_TARGETS: [InteractionTargetContract; 2] = [
    InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "core.topbar.hover.threat",
        label: "World tension hover tooltip",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_HOVER_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "core.topbar.hover.faction",
        label: "Faction hover tooltip",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_HOVER_NOTE,
    },
];

const LOGISTICS_UNIT_VIEW_SCROLL_TARGETS: [InteractionTargetContract; 2] = [
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "logistics.unit_view.scroll.divisions",
        label: "Division list wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: LOGISTICS_UNIT_VIEW_DIVISION_SCROLL_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "logistics.unit_view.scroll.equipment",
        label: "Equipment pane wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: LOGISTICS_UNIT_VIEW_EQUIPMENT_SCROLL_NOTE,
    },
];

const PRODUCTION_PRODUCTION_SCROLL_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "production.production.scroll.lines",
        label: "Production line wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_SCROLL_NOTE,
    }];

const PRODUCTION_CONSTRUCTION_SCROLL_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "production.construction.scroll.queue",
        label: "Construction queue wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: PRODUCTION_CONSTRUCTION_SCROLL_NOTE,
    }];

const PRODUCTION_DECISIONS_SCROLL_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Scroll,
        id: "production.decisions.scroll.timed_decisions",
        label: "Timed decision list wheel/thumb scroll",
        status: InteractionStatus::Open,
        note: PRODUCTION_DECISIONS_SCROLL_NOTE,
    }];

const RESEARCH_RESEARCH_ROW_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::ListSelection,
    id: "research.research.row.active_slot",
    label: "Active research slot row",
    status: InteractionStatus::Open,
    note: RESEARCH_RESEARCH_ROW_NOTE,
}];

const RESEARCH_TECH_TREE_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "research.tech_tree.row.tech_node",
        label: "Tech tree node",
        status: InteractionStatus::Open,
        note: RESEARCH_TECH_TREE_ROW_NOTE,
    }];

const RESEARCH_DOCTRINE_TREE_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "research.doctrine_tree.row.doctrine_node",
        label: "Doctrine tree node",
        status: InteractionStatus::Open,
        note: RESEARCH_DOCTRINE_TREE_ROW_NOTE,
    }];

const CORE_FOCUS_TREE_ROW_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::ListSelection,
    id: "core.focus_tree.row.focus_node",
    label: "Focus tree node",
    status: InteractionStatus::Open,
    note: CORE_FOCUS_TREE_ROW_NOTE,
}];

const LEADERS_ARMY_TRAIT_TREE_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "leaders.army_trait_tree.row.owned_trait_node",
        label: "Army owned trait node",
        status: InteractionStatus::Open,
        note: LEADERS_ARMY_TRAIT_TREE_ROW_NOTE,
    }];

const LEADERS_NAVY_TRAIT_TREE_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "leaders.navy_trait_tree.row.owned_trait_node",
        label: "Navy owned trait node",
        status: InteractionStatus::Open,
        note: LEADERS_NAVY_TRAIT_TREE_ROW_NOTE,
    }];

const CHROME_FIND_DRILLDOWN_TARGETS: [InteractionTargetContract; 4] = [
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.find.drill.state",
        label: "State row to state panel",
        status: InteractionStatus::Open,
        note: CHROME_FIND_STATE_PANEL_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.find.drill.vp",
        label: "Victory Point row to state panel",
        status: InteractionStatus::Open,
        note: CHROME_FIND_STATE_PANEL_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.find.drill.recenter_selected",
        label: "Find selected/first match on map",
        status: InteractionStatus::Open,
        note: CHROME_FIND_RECENTER_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.find.drill.goto_capital",
        label: "Goto capital on map",
        status: InteractionStatus::Open,
        note: CHROME_FIND_RECENTER_NOTE,
    },
];

const CORE_TOPBAR_DRILLDOWN_TARGETS: [InteractionTargetContract; 15] = [
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.threat_icon",
        label: "World tension icon button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.threat_value",
        label: "World tension value button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.decisions",
        label: "Decisions button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.intel_agency",
        label: "Intel agency button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.research",
        label: "Technology button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.diplomacy",
        label: "Diplomacy button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.trade",
        label: "Trade button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.construction",
        label: "Construction button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.production",
        label: "Production button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.deployment",
        label: "Deployment button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.logistics",
        label: "Logistics button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.officer_corps",
        label: "Officer corps button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.army",
        label: "Army button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.navy",
        label: "Navy button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "core.topbar.drill.air",
        label: "Air button",
        status: InteractionStatus::Open,
        note: CORE_TOPBAR_DRILLDOWN_NOTE,
    },
];

const CHROME_MAPMODES_ROW_TARGETS: [InteractionTargetContract; 15] = [
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
        id: "chrome.mapmodes.row.all_toggle",
        label: "All map modes",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ALL_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.all_supply",
        label: "Supply via all modes",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ALL_NOTE,
    },
    InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "chrome.mapmodes.row.all_intel",
        label: "Intel via all modes",
        status: InteractionStatus::Open,
        note: CHROME_MAPMODES_ALL_NOTE,
    },
];

const CHROME_MAPMODES_HOVER_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::HoverTooltip,
    id: "chrome.mapmodes.hover.mode_row",
    label: "Map mode row tooltip",
    status: InteractionStatus::Open,
    note: CHROME_MAPMODES_HOVER_NOTE,
}];

const RESEARCH_TECH_TREE_HOVER_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "research.tech_tree.hover.tech_node",
        label: "Tech node tooltip",
        status: InteractionStatus::Open,
        note: RESEARCH_TECH_TREE_HOVER_NOTE,
    }];

const RESEARCH_DOCTRINE_TREE_HOVER_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "research.doctrine_tree.hover.doctrine_node",
        label: "Doctrine node tooltip",
        status: InteractionStatus::Open,
        note: RESEARCH_DOCTRINE_TREE_HOVER_NOTE,
    }];

const LEADERS_ARMY_TRAIT_TREE_HOVER_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "leaders.army_trait_tree.hover.owned_trait_node",
        label: "Army owned trait tooltip",
        status: InteractionStatus::Open,
        note: LEADERS_ARMY_TRAIT_TREE_HOVER_NOTE,
    }];

const LEADERS_NAVY_TRAIT_TREE_HOVER_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::HoverTooltip,
        id: "leaders.navy_trait_tree.hover.owned_trait_node",
        label: "Navy owned trait tooltip",
        status: InteractionStatus::Open,
        note: LEADERS_NAVY_TRAIT_TREE_HOVER_NOTE,
    }];

const LOGISTICS_UNIT_VIEW_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "logistics.unit_view.row.division",
        label: "Division row",
        status: InteractionStatus::Open,
        note: LOGISTICS_UNIT_VIEW_ROW_NOTE,
    }];

const PRODUCTION_PRODUCTION_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "production.production.row.line",
        label: "Production line row",
        status: InteractionStatus::Open,
        note: PRODUCTION_PRODUCTION_ROW_NOTE,
    }];

const PRODUCTION_CONSTRUCTION_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "production.construction.row.queue",
        label: "Construction queue row",
        status: InteractionStatus::Open,
        note: PRODUCTION_CONSTRUCTION_ROW_NOTE,
    }];

const PRODUCTION_DECISIONS_ROW_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::ListSelection,
        id: "production.decisions.row.timed_decision",
        label: "Timed decision row",
        status: InteractionStatus::Open,
        note: PRODUCTION_DECISIONS_ROW_NOTE,
    }];

const CORE_MAP_HOVER_TARGETS: [InteractionTargetContract; 1] = [InteractionTargetContract {
    category: InteractionCategory::HoverTooltip,
    id: "core.map.hover.province",
    label: "Province hover tooltip",
    status: InteractionStatus::Open,
    note: CORE_MAP_HOVER_NOTE,
}];

const CHROME_MINIMAP_DRILLDOWN_TARGETS: [InteractionTargetContract; 1] =
    [InteractionTargetContract {
        category: InteractionCategory::Drilldown,
        id: "chrome.minimap.drill.recenter",
        label: "Minimap click/drag recenter",
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

        let selection = politics
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("row selection/highlight proof"),
            "politics.politics row-selection contract must cite executable proof"
        );
        assert!(
            selection.note.contains("mutation controls"),
            "politics.politics row proof must keep mutating controls open"
        );
        let row_targets: Vec<_> = politics
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 4);
        assert_eq!(
            row_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "politics.politics.row.government",
                "politics.politics.row.national_spirits",
                "politics.politics.row.ideas_laws",
                "politics.politics.row.autonomy",
            ]
        );
        assert!(
            row_targets
                .iter()
                .all(|t| t.note.contains("PoliticsView tests prove stable-keyed")),
            "politics.politics row targets must cite stable-key row-selection proof"
        );
        assert!(
            row_targets
                .iter()
                .all(|t| t.note.contains("duplicate/unkeyed-row inert behavior")),
            "politics.politics row targets must cite duplicate/unkeyed fail-closed proof"
        );

        let scroll = politics.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "politics.politics scroll contract must cite executable wheel/thumb-scroll proof"
        );
        assert!(
            scroll
                .note
                .contains("broader politics scroll surfaces remain open"),
            "politics.politics scroll proof must keep broader politics scroll parity open"
        );
        let scroll_targets: Vec<_> = politics.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 2);
        assert_eq!(
            scroll_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "politics.politics.scroll.national_spirits",
                "politics.politics.scroll.ideas_laws",
            ]
        );
        assert!(
            scroll_targets.iter().all(|t| t
                .note
                .contains("PoliticsView tests prove wheel scroll and scrollbar-thumb drag")),
            "politics.politics scroll targets must cite executable wheel/thumb proof"
        );
        assert!(
            scroll_targets.iter().all(|t| t
                .note
                .contains("track clicks do not select underlying rows")),
            "politics.politics scroll targets must cite track-click inert proof"
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

        let selection = army.category(InteractionCategory::ListSelection).unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("row selection/highlight proof"),
            "military.army row-selection contract must cite executable proof"
        );
        assert!(
            selection.note.contains("detail-pane drilldowns"),
            "military.army row proof must keep drilldowns open"
        );
        let row_targets: Vec<_> = army.targets(InteractionCategory::ListSelection).collect();
        assert_eq!(row_targets.len(), 3);
        assert_eq!(
            row_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "military.army.row.army",
                "military.army.row.navy",
                "military.army.row.air",
            ]
        );
        assert!(
            row_targets
                .iter()
                .all(|t| t.note.contains("ArmyView tests prove keyed")),
            "military.army row targets must cite keyed row-selection proof"
        );

        let scroll = army.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "military.army scroll contract must cite executable wheel/thumb-scroll proof"
        );
        assert!(
            scroll.note.contains("exact scrollbar art"),
            "military.army scroll proof must keep exact scrollbar art open"
        );
        let scroll_targets: Vec<_> = army.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 3);
        assert_eq!(
            scroll_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "military.army.scroll.army",
                "military.army.scroll.navy",
                "military.army.scroll.air",
            ]
        );
        assert!(
            scroll_targets.iter().all(|t| t
                .note
                .contains("ArmyView tests prove wheel scroll and scrollbar-thumb drag")),
            "military.army scroll targets must cite executable wheel/thumb proof"
        );
    }

    #[test]
    fn military_theatre_selector_targets_cite_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("military.theatre_selector")).is_some(),
            "the Theatre Selector interaction inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let theatre = contract_by_id(&contracts, ViewId("military.theatre_selector"));

        let selection = theatre
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("row selection/highlight proof"),
            "military.theatre_selector row-selection contract must cite executable proof"
        );
        assert!(
            selection.note.contains("pin/priority UI state"),
            "military.theatre_selector row proof must keep UI-state parity open"
        );
        let row_targets: Vec<_> = theatre
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "military.theatre_selector.row.theatre");
        assert_eq!(row_targets[0].label, "Theatre selector row");
        assert!(
            row_targets[0]
                .note
                .contains("TheatreSelectorView tests prove measured theatre id"),
            "military.theatre_selector row target must cite measured-id proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing selected-row fail-closed proof"),
            "military.theatre_selector row target must cite missing-selection fail-closed proof"
        );

        let scroll = theatre.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "military.theatre_selector scroll contract must cite executable wheel and thumb-drag proof"
        );
        let scroll_targets: Vec<_> = theatre.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 1);
        assert_eq!(
            scroll_targets[0].id,
            "military.theatre_selector.scroll.theatres"
        );
        assert_eq!(scroll_targets[0].label, "Theatre selector list scroll");
        assert!(
            scroll_targets[0]
                .note
                .contains("outside-list wheel input is inert"),
            "military.theatre_selector scroll target must cite outside-list inert proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrollbar-thumb drag changes/clamps rows"),
            "military.theatre_selector scroll target must cite thumb-drag proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("broader theatre scroll surfaces remain open"),
            "military.theatre_selector scroll proof must keep broader theatre scroll parity open"
        );
    }

    #[test]
    fn logistics_unit_view_row_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("logistics.unit_view")).is_some(),
            "the UnitView row-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let unit = contract_by_id(&contracts, ViewId("logistics.unit_view"));

        let tab = unit.category(InteractionCategory::TabSwitch).unwrap();
        assert_eq!(tab.status, InteractionStatus::Open);
        assert!(
            tab.note.contains("known tab ids/labels are inventoried"),
            "logistics.unit_view tab category must be represented by target inventory"
        );
        let tab_targets: Vec<_> = unit.targets(InteractionCategory::TabSwitch).collect();
        assert_eq!(tab_targets.len(), 3);
        assert_eq!(
            tab_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "logistics.unit_view.tab.stats",
                "logistics.unit_view.tab.history",
                "logistics.unit_view.tab.template",
            ]
        );
        assert_eq!(
            tab_targets.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["Stats", "History", "Template"]
        );
        assert!(
            tab_targets.iter().all(|t| t
                .note
                .contains("UnitView tests prove Stats/History/Template")),
            "logistics.unit_view tab targets must cite executable UnitView proof"
        );
        assert!(
            tab_targets
                .iter()
                .all(|t| t.note.contains("broader unit-detail parity remain open")),
            "logistics.unit_view tab proof must keep broader detail parity open"
        );

        let selection = unit.category(InteractionCategory::ListSelection).unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection
                .note
                .contains("measured-div_id row selection proof"),
            "logistics.unit_view list-selection contract must cite measured div_id proof"
        );
        assert!(
            selection
                .note
                .contains("broader unit-detail parity remain open"),
            "logistics.unit_view proof must not close broader unit-detail parity"
        );

        let row_targets: Vec<_> = unit.targets(InteractionCategory::ListSelection).collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "logistics.unit_view.row.division");
        assert_eq!(row_targets[0].label, "Division row");
        assert!(
            row_targets[0]
                .note
                .contains("UnitView tests prove measured div_id"),
            "logistics.unit_view row target must cite executable UnitView proof"
        );
        assert!(
            row_targets[0].note.contains("fails closed"),
            "logistics.unit_view row target must cite missing-selection fail-closed proof"
        );

        let scroll = unit.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "logistics.unit_view scroll contract must cite executable wheel/thumb-scroll proof"
        );
        assert!(
            scroll.note.contains("exact scrollbar art"),
            "logistics.unit_view scroll proof must keep exact scrollbar art open"
        );
        assert!(
            scroll
                .note
                .contains("broader unit-detail scroll surfaces remain open"),
            "logistics.unit_view scroll proof must not close broader scroll parity"
        );
        let scroll_targets: Vec<_> = unit.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 2);
        assert_eq!(scroll_targets[0].id, "logistics.unit_view.scroll.divisions");
        assert_eq!(scroll_targets[0].label, "Division list wheel/thumb scroll");
        assert_eq!(scroll_targets[1].id, "logistics.unit_view.scroll.equipment");
        assert_eq!(scroll_targets[1].label, "Equipment pane wheel/thumb scroll");
        assert!(
            scroll_targets[0]
                .note
                .contains("UnitView tests prove wheel scroll and scrollbar-thumb drag"),
            "logistics.unit_view scroll target must cite executable UnitView proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("track clicks do not select underlying rows"),
            "logistics.unit_view division-scroll target must cite track-click inert proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("stale drag captures clear on rebind"),
            "logistics.unit_view division-scroll target must cite stale-capture clearing proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("outside-list wheel input is inert"),
            "logistics.unit_view scroll target must cite outside-list inert proof"
        );
        assert!(
            scroll_targets[1]
                .note
                .contains("wheel scroll and scrollbar-thumb drag"),
            "logistics.unit_view equipment-scroll target must cite wheel/thumb proof"
        );
        assert!(
            scroll_targets[1].note.contains("track clicks are inert"),
            "logistics.unit_view equipment-scroll target must cite track-click inert proof"
        );
        assert!(
            scroll_targets[1]
                .note
                .contains("selected-division equipment-pane scroll target"),
            "logistics.unit_view equipment-scroll target must identify the equipment pane"
        );
        assert!(
            scroll_targets[1]
                .note
                .contains("fails closed with eq_rows=0/eq_visible=[]"),
            "logistics.unit_view equipment-scroll target must cite missing-selection fail-closed proof"
        );
    }

    #[test]
    fn production_production_row_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("production.production")).is_some(),
            "the Production row-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let production = contract_by_id(&contracts, ViewId("production.production"));

        let selection = production
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection
                .note
                .contains("production-line row selection proof"),
            "production.production list-selection contract must cite production-line proof"
        );
        assert!(
            selection.note.contains("mutation controls"),
            "production.production proof must not close mutating line-control parity"
        );

        let row_targets: Vec<_> = production
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "production.production.row.line");
        assert_eq!(row_targets[0].label, "Production line row");
        assert!(
            row_targets[0]
                .note
                .contains("ProductionView tests prove measured live id"),
            "production.production row target must cite executable ProductionView proof"
        );
        assert!(
            row_targets[0].note.contains("save line_id"),
            "production.production row target must cite save fallback identity proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("unkeyed/duplicate-row inert behavior"),
            "production.production row target must cite duplicate/unkeyed fail-closed proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing selected-row fail-closed proof"),
            "production.production row target must cite missing-selection fail-closed proof"
        );

        let scroll = production.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "production.production scroll contract must cite executable wheel and thumb-drag proof"
        );
        assert!(
            scroll.note.contains("scrollbar art"),
            "production.production scroll proof must keep scrollbar art open"
        );

        let scroll_targets: Vec<_> = production.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 1);
        assert_eq!(scroll_targets[0].id, "production.production.scroll.lines");
        assert_eq!(
            scroll_targets[0].label,
            "Production line wheel/thumb scroll"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrolled clicks select the correct backing production key"),
            "production.production scroll target must cite scrolled backing-row proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrollbar-thumb drag change/clamp visible line rows"),
            "production.production scroll target must cite thumb-drag proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("track clicks do not select underlying rows"),
            "production.production scroll target must cite track-click inert proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("outside-grid wheel input is inert"),
            "production.production scroll target must cite outside-grid inert proof"
        );
    }

    #[test]
    fn production_construction_row_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("production.construction")).is_some(),
            "the Construction row-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let construction = contract_by_id(&contracts, ViewId("production.construction"));

        let selection = construction
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection
                .note
                .contains("construction-queue row selection proof"),
            "production.construction list-selection contract must cite construction-queue proof"
        );
        assert!(
            selection
                .note
                .contains("broader construction parity remain open"),
            "production.construction proof must not overclaim broad construction parity"
        );

        let row_targets: Vec<_> = construction
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "production.construction.row.queue");
        assert_eq!(row_targets[0].label, "Construction queue row");
        assert!(
            row_targets[0]
                .note
                .contains("ConstructionView tests prove measured live line_id"),
            "production.construction row target must cite executable ConstructionView proof"
        );
        assert!(
            row_targets[0].note.contains("save template/state/created"),
            "production.construction row target must cite save fallback identity proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing selected-row fail-closed proof"),
            "production.construction row target must cite missing-selection fail-closed proof"
        );

        let scroll = construction.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "production.construction scroll contract must cite executable wheel and thumb-drag proof"
        );
        assert!(
            scroll
                .note
                .contains("broader construction parity remain open"),
            "production.construction scroll proof must keep broader production/construction parity open"
        );

        let scroll_targets: Vec<_> = construction.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 1);
        assert_eq!(scroll_targets[0].id, "production.construction.scroll.queue");
        assert_eq!(
            scroll_targets[0].label,
            "Construction queue wheel/thumb scroll"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrollbar-thumb drag change/clamp visible queue rows"),
            "production.construction scroll target must cite thumb-drag proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("track clicks do not select underlying rows"),
            "production.construction scroll target must cite track-click inert proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrolled clicks select the correct backing construction key"),
            "production.construction scroll target must cite scrolled backing-row proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("outside-grid wheel input is inert"),
            "production.construction scroll target must cite outside-grid inert proof"
        );
    }

    #[test]
    fn production_decisions_row_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("production.decisions")).is_some(),
            "the Decisions row-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let decisions = contract_by_id(&contracts, ViewId("production.decisions"));

        let selection = decisions
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection
                .note
                .contains("timed-decision row selection proof"),
            "production.decisions list-selection contract must cite timed-decision proof"
        );
        assert!(
            selection
                .note
                .contains("mutating decision buttons remain open"),
            "production.decisions proof must not close mutating decision parity"
        );

        let row_targets: Vec<_> = decisions
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "production.decisions.row.timed_decision");
        assert_eq!(row_targets[0].label, "Timed decision row");
        assert!(
            row_targets[0]
                .note
                .contains("DecisionsView tests prove measured decision/state"),
            "production.decisions row target must cite executable DecisionsView proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing selected-row fail-closed proof"),
            "production.decisions row target must cite missing-selection fail-closed proof"
        );

        let scroll_targets: Vec<_> = decisions.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 1);
        assert_eq!(
            scroll_targets[0].id,
            "production.decisions.scroll.timed_decisions"
        );
        assert_eq!(
            scroll_targets[0].label,
            "Timed decision list wheel/thumb scroll"
        );
        assert!(
            decisions
                .category(InteractionCategory::Scroll)
                .unwrap()
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "production.decisions scroll contract must cite executable wheel and thumb-drag proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrollbar-thumb drag change/clamp visible rows"),
            "production.decisions scroll target must cite thumb-drag proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("track clicks do not select underlying rows"),
            "production.decisions scroll target must cite track-click inert proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("scrolled clicks select the correct backing decision key"),
            "production.decisions scroll target must cite scrolled backing-row proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("outside-grid wheel input is inert"),
            "production.decisions scroll target must cite outside-grid inert proof"
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
    fn component_designer_slot_hover_and_disabled_targets_cite_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        let cases = [
            (
                ViewId("designers.tank"),
                "designers.tank.row.module_slot",
                "designers.tank.hover.module_slot",
                "designers.tank.mutation.component_controls",
                "Tank module slot selection",
                "Tank module slot hover inspection",
                "Tank designer non-slot controls",
            ),
            (
                ViewId("designers.plane"),
                "designers.plane.row.module_slot",
                "designers.plane.hover.module_slot",
                "designers.plane.mutation.component_controls",
                "Plane module slot selection",
                "Plane module slot hover inspection",
                "Plane designer non-slot controls",
            ),
            (
                ViewId("designers.equipment"),
                "designers.equipment.row.module_slot",
                "designers.equipment.hover.module_slot",
                "designers.equipment.mutation.component_controls",
                "Equipment module slot selection",
                "Equipment module slot hover inspection",
                "Equipment designer non-slot controls",
            ),
        ];

        let contracts = interaction_contracts(&reg);
        for (view_id, row_id, hover_id, mutation_id, row_label, hover_label, mutation_label) in
            cases
        {
            assert!(
                reg.view(view_id).is_some(),
                "{view_id} component-designer inventory must be anchored to a real registered view"
            );
            let contract = contract_by_id(&contracts, view_id);

            let selection = contract
                .category(InteractionCategory::ListSelection)
                .unwrap();
            assert_eq!(selection.status, InteractionStatus::Open);
            assert!(
                selection.note.contains("module-slot click selection proof"),
                "{view_id} selection contract must cite component slot click proof"
            );
            assert!(
                selection
                    .note
                    .contains("broader component-designer parity remain open"),
                "{view_id} selection proof must not overclaim component-designer parity"
            );
            let row_targets: Vec<_> = contract
                .targets(InteractionCategory::ListSelection)
                .collect();
            assert_eq!(row_targets.len(), 1);
            assert_eq!(row_targets[0].id, row_id);
            assert_eq!(row_targets[0].label, row_label);
            assert!(
                row_targets[0].note.contains(
                    "component designer tests prove read-only module-slot click selection"
                ),
                "{view_id} row target must cite executable component-designer click proof"
            );
            assert!(
                row_targets[0]
                    .note
                    .contains("without falling back to another slot"),
                "{view_id} row target must cite missing-restored no-fallback proof"
            );

            let hover = contract
                .category(InteractionCategory::HoverTooltip)
                .unwrap();
            assert_eq!(hover.status, InteractionStatus::Open);
            assert!(
                hover
                    .note
                    .contains("passive module-slot hover inspection proof"),
                "{view_id} hover contract must cite component hover proof"
            );
            assert!(
                hover.note.contains("full the game module tooltip frames"),
                "{view_id} hover proof must not overclaim full tooltip parity"
            );
            let hover_targets: Vec<_> = contract
                .targets(InteractionCategory::HoverTooltip)
                .collect();
            assert_eq!(hover_targets.len(), 1);
            assert_eq!(hover_targets[0].id, hover_id);
            assert_eq!(hover_targets[0].label, hover_label);
            assert!(
                hover_targets[0]
                    .note
                    .contains("component designer tests prove passive hover inspection"),
                "{view_id} hover target must cite executable component hover proof"
            );
            assert!(
                hover_targets[0]
                    .note
                    .contains("not full the game module tooltip frame"),
                "{view_id} hover target must keep full tooltip parity open"
            );

            let disabled = contract
                .category(InteractionCategory::DisabledMutation)
                .unwrap();
            assert_eq!(disabled.status, InteractionStatus::Open);
            assert!(
                disabled
                    .note
                    .contains("shared non-slot designer chrome/control area"),
                "{view_id} disabled-mutation contract must cite the scoped control area"
            );
            assert!(
                disabled
                    .note
                    .contains("broader component-designer mutation parity remain open"),
                "{view_id} disabled-mutation proof must not overclaim mutation parity"
            );
            let mutation_targets: Vec<_> = contract
                .targets(InteractionCategory::DisabledMutation)
                .collect();
            assert_eq!(mutation_targets.len(), 1);
            assert_eq!(mutation_targets[0].id, mutation_id);
            assert_eq!(mutation_targets[0].label, mutation_label);
            assert!(
                mutation_targets[0]
                    .note
                    .contains("component designer tests prove clicks inside the shared non-slot"),
                "{view_id} disabled target must cite executable no-op proof"
            );
            assert!(
                mutation_targets[0].note.contains("unchanged bind digest")
                    && mutation_targets[0].note.contains("upgrade sliders"),
                "{view_id} disabled target must cite unchanged-output proof and remaining gaps"
            );
        }
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
            drilldown.note.contains("RecenterMap"),
            "chrome.find drilldown contract must cite the map recenter command proof"
        );
        assert!(
            drilldown.note.contains("remain open"),
            "chrome.find drilldown proof must not claim complete find/drill parity"
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

        let scroll = find.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "chrome.find scroll contract must cite the executable wheel/thumb proof"
        );
        let scroll_targets: Vec<_> = find.targets(InteractionCategory::Scroll).collect();
        assert_eq!(scroll_targets.len(), 1);
        assert_eq!(scroll_targets[0].id, "chrome.find.scroll.results");
        assert_eq!(scroll_targets[0].label, "Results scroll");
        assert!(
            scroll_targets[0]
                .note
                .contains("FindView tests prove wheel scroll and scrollbar-thumb drag"),
            "chrome.find scroll target must cite executable scroll proof"
        );
        assert!(
            scroll_targets[0]
                .note
                .contains("track clicks do not select or drill underlying rows"),
            "chrome.find scroll target must cite track-click inert proof"
        );

        let text = find.category(InteractionCategory::TextInput).unwrap();
        assert_eq!(text.status, InteractionStatus::Open);
        assert!(
            text.note.contains("replay-local query proof"),
            "chrome.find text-input contract must cite executable query proof"
        );
        assert!(
            text.note.contains("localized names"),
            "chrome.find text-input proof must keep localized-name parity open"
        );
        let text_targets: Vec<_> = find.targets(InteractionCategory::TextInput).collect();
        assert_eq!(text_targets.len(), 1);
        assert_eq!(text_targets[0].id, "chrome.find.text.query");
        assert!(
            text_targets[0]
                .note
                .contains("FindView tests prove replay-local focus"),
            "chrome.find text target must cite executable text-input proof"
        );

        let drill_targets: Vec<_> = find.targets(InteractionCategory::Drilldown).collect();
        assert_eq!(drill_targets.len(), 4);
        assert_eq!(
            drill_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "chrome.find.drill.state",
                "chrome.find.drill.vp",
                "chrome.find.drill.recenter_selected",
                "chrome.find.drill.goto_capital",
            ]
        );
        assert!(
            drill_targets[..2]
                .iter()
                .all(|t| t.note.contains("OpenView(core.state_panel")),
            "chrome.find drill targets must cite executable state-panel proof"
        );
        assert!(
            drill_targets[2..]
                .iter()
                .all(|t| t.note.contains("RecenterMap")),
            "chrome.find recenter targets must cite executable map recenter proof"
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
            drilldown.note.contains("click-and-drag recenter proof"),
            "chrome.minimap drilldown contract must cite the executable recenter proof"
        );
        assert!(
            drilldown.note.contains("screenshot proof remains open"),
            "chrome.minimap click/drag proof must not claim screenshot parity"
        );

        let drill_targets: Vec<_> = minimap.targets(InteractionCategory::Drilldown).collect();
        assert_eq!(drill_targets.len(), 1);
        assert_eq!(drill_targets[0].id, "chrome.minimap.drill.recenter");
        assert!(
            drill_targets[0]
                .note
                .contains("MinimapView tests prove click-and-drag recenter"),
            "chrome.minimap drill target must cite executable recenter proof"
        );
    }

    #[test]
    fn core_topbar_drilldown_targets_cite_executable_open_view_proof() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let topbar = contract_by_id(&contracts, ViewId("core.topbar"));

        let drilldown = topbar.category(InteractionCategory::Drilldown).unwrap();
        assert_eq!(drilldown.status, InteractionStatus::Open);
        assert!(
            drilldown.note.contains("OpenView proof"),
            "core.topbar drilldown contract must cite read-only OpenView proof"
        );
        assert!(
            drilldown.note.contains("broader topbar parity remain open"),
            "topbar navigation proof must not overclaim broader topbar parity"
        );

        let drill_targets: Vec<_> = topbar.targets(InteractionCategory::Drilldown).collect();
        assert_eq!(drill_targets.len(), 15);
        assert_eq!(
            drill_targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![
                "core.topbar.drill.threat_icon",
                "core.topbar.drill.threat_value",
                "core.topbar.drill.decisions",
                "core.topbar.drill.intel_agency",
                "core.topbar.drill.research",
                "core.topbar.drill.diplomacy",
                "core.topbar.drill.trade",
                "core.topbar.drill.construction",
                "core.topbar.drill.production",
                "core.topbar.drill.deployment",
                "core.topbar.drill.logistics",
                "core.topbar.drill.officer_corps",
                "core.topbar.drill.army",
                "core.topbar.drill.navy",
                "core.topbar.drill.air",
            ]
        );
        assert!(
            drill_targets
                .iter()
                .all(|t| t.note.contains("TopbarView tests prove")),
            "topbar drill targets must cite executable TopbarView proof"
        );
        assert!(
            drill_targets
                .iter()
                .all(|t| t.note.contains("empty-space clicks falling through")),
            "topbar target notes must preserve empty-space fallthrough proof"
        );
        assert!(
            drill_targets[..2]
                .iter()
                .all(|t| t.note.contains("intel.world_tension_popup")),
            "world tension drill targets must cite the exact read-only popup"
        );
    }

    #[test]
    fn core_topbar_hover_targets_cite_executable_no_fabrication_proof() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let topbar = contract_by_id(&contracts, ViewId("core.topbar"));

        let hover = topbar.category(InteractionCategory::HoverTooltip).unwrap();
        assert_eq!(hover.status, InteractionStatus::Open);
        assert!(
            hover.note.contains("threat_button")
                && hover.note.contains("threat_value_button")
                && hover.note.contains("faction_button"),
            "topbar hover contract must name the proven real .gui leaves"
        );
        assert!(
            hover.note.contains("no-fabrication") && hover.note.contains("hover-clear"),
            "topbar hover contract must cite no-fabrication and clear behavior"
        );

        let targets: Vec<_> = topbar.targets(InteractionCategory::HoverTooltip).collect();
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec!["core.topbar.hover.threat", "core.topbar.hover.faction"]
        );
        assert!(
            targets
                .iter()
                .all(|t| t.note.contains("TopbarView tests prove")),
            "topbar hover targets must cite executable TopbarView proof"
        );
        assert!(
            targets.iter().all(|t| t
                .note
                .contains("badged unavailable instead of fabricated zero")
                || t.note
                    .contains("missing/null faction fields badged unavailable")),
            "topbar hover targets must cite no-fabrication proof"
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
            selection.note.contains("real numbered .gui rail slots")
                && selection.note.contains("mapmodes_all popup path"),
            "chrome.mapmodes selection note must cite the real rail and popup geometry"
        );
        assert!(
            selection
                .note
                .contains("custom slot configuration/defaults"),
            "chrome.mapmodes selection note must keep unimplemented configuration behavior open"
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
        assert_eq!(row_targets.len(), 15);
        assert!(
            row_targets
                .iter()
                .all(|t| t.note.contains("MapModesView tests prove")),
            "chrome.mapmodes row targets must cite executable MapModesView proof"
        );
        assert!(
            row_targets.iter().all(
                |t| t.id != "chrome.mapmodes.row.supply" && t.id != "chrome.mapmodes.row.intel"
            ),
            "Supply/Intel must not be listed as real numbered .gui rail slots"
        );
        assert!(
            row_targets
                .iter()
                .any(|t| t.id == "chrome.mapmodes.row.all_toggle")
                && row_targets
                    .iter()
                    .any(|t| t.id == "chrome.mapmodes.row.all_supply")
                && row_targets
                    .iter()
                    .any(|t| t.id == "chrome.mapmodes.row.all_intel"),
            "the real all-modes popup targets must account for Supply/Intel mouse access"
        );

        let hover = mapmodes
            .category(InteractionCategory::HoverTooltip)
            .unwrap();
        assert_eq!(hover.status, InteractionStatus::Open);
        assert!(
            hover.note.contains("fidelity/source text"),
            "chrome.mapmodes hover note must cite the implemented tooltip readout"
        );
        assert!(
            hover
                .note
                .contains("broader map tooltip coverage remain open"),
            "chrome.mapmodes hover note must not overclaim tooltip parity"
        );

        let hover_targets: Vec<_> = mapmodes
            .targets(InteractionCategory::HoverTooltip)
            .collect();
        assert_eq!(hover_targets.len(), 1);
        assert_eq!(hover_targets[0].id, "chrome.mapmodes.hover.mode_row");
        assert!(
            hover_targets[0].note.contains("MapModesView tests prove"),
            "chrome.mapmodes hover target must cite executable tooltip proof"
        );
    }

    #[test]
    fn core_map_contract_tracks_factual_province_hover_tooltip_slice() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let map = contracts
            .iter()
            .find(|c| c.view_id.0 == "core.map")
            .expect("core.map contract");

        let hover = map.category(InteractionCategory::HoverTooltip).unwrap();
        assert_eq!(hover.status, InteractionStatus::Open);
        assert!(
            hover.note.contains("province-hover tooltip proof"),
            "core.map hover row must cite implemented province hover proof"
        );
        assert!(
            hover
                .note
                .contains("side-by-side screenshot proof remain open"),
            "core.map hover row must not overclaim visual parity"
        );

        let targets: Vec<_> = map.targets(InteractionCategory::HoverTooltip).collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "core.map.hover.province");
        assert!(
            targets[0].note.contains("MapView tests prove"),
            "core.map hover target must cite executable MapView proof"
        );
    }

    #[test]
    fn core_focus_tree_node_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("core.focus_tree")).is_some(),
            "the focus-tree node-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let focus = contract_by_id(&contracts, ViewId("core.focus_tree"));

        let selection = focus.category(InteractionCategory::ListSelection).unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("focus-node click selection proof"),
            "core.focus_tree list-selection contract must cite focus-node click proof"
        );
        assert!(
            selection
                .note
                .contains("selected-detail panel/digest slice"),
            "core.focus_tree selection proof must cite selected-detail proof"
        );
        assert!(
            selection
                .note
                .contains("full the game national_focus_detail_view parity"),
            "core.focus_tree selection proof must keep full game detail parity open"
        );

        let row_targets: Vec<_> = focus.targets(InteractionCategory::ListSelection).collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "core.focus_tree.row.focus_node");
        assert_eq!(row_targets[0].label, "Focus tree node");
        assert!(
            row_targets[0].note.contains("FocusShell tests prove"),
            "core.focus_tree row target must cite executable FocusShell proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing cost/progress never fabricated as zero"),
            "core.focus_tree row target must cite no-fabrication detail proof"
        );
    }

    #[test]
    fn leaders_trait_tree_node_selection_targets_cite_executable_proof_without_overclaiming() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("leaders.army_trait_tree")).is_some(),
            "the army trait-tree node-selection inventory must be anchored to a real registered view"
        );
        assert!(
            reg.view(ViewId("leaders.navy_trait_tree")).is_some(),
            "the navy trait-tree node-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let army = contract_by_id(&contracts, ViewId("leaders.army_trait_tree"));
        let navy = contract_by_id(&contracts, ViewId("leaders.navy_trait_tree"));

        let army_selection = army.category(InteractionCategory::ListSelection).unwrap();
        assert_eq!(army_selection.status, InteractionStatus::Open);
        assert!(
            army_selection.note.contains("owned trait-token nodes only"),
            "army trait-tree contract must limit the claim to surfaced owned trait tokens"
        );
        assert!(
            army_selection
                .note
                .contains("full static trait catalog graph"),
            "army trait-tree contract must keep full trait catalog parity open"
        );

        let army_targets: Vec<_> = army.targets(InteractionCategory::ListSelection).collect();
        assert_eq!(army_targets.len(), 1);
        assert_eq!(
            army_targets[0].id,
            "leaders.army_trait_tree.row.owned_trait_node"
        );
        assert!(
            army_targets[0]
                .note
                .contains("ArmyTraitTreeView tests prove exact trait-token"),
            "army trait-tree target must cite executable ArmyTraitTreeView proof"
        );
        assert!(
            army_targets[0]
                .note
                .contains("inert no-leader/no-traits unavailable glyphs"),
            "army trait-tree target must cite inert unavailable-glyph proof"
        );
        assert!(
            army_targets[0].note.contains("derived static DEF catalog"),
            "army trait-tree target must cite derived static DEF metadata proof"
        );
        assert!(
            army_targets[0].note.contains("null trait_type unavailable"),
            "army trait-tree target must cite no-fabrication proof for null DEF fields"
        );
        assert!(
            army_targets[0]
                .note
                .contains("full static trait catalog graph"),
            "army trait-tree target must not overclaim full trait catalog graph parity"
        );

        let navy_selection = navy.category(InteractionCategory::ListSelection).unwrap();
        assert_eq!(navy_selection.status, InteractionStatus::Open);
        assert!(
            navy_selection.note.contains("owned trait-token nodes only"),
            "navy trait-tree contract must limit the claim to surfaced owned trait tokens"
        );
        let navy_targets: Vec<_> = navy.targets(InteractionCategory::ListSelection).collect();
        assert_eq!(navy_targets.len(), 1);
        assert_eq!(
            navy_targets[0].id,
            "leaders.navy_trait_tree.row.owned_trait_node"
        );
        assert!(
            navy_targets[0]
                .note
                .contains("NavyTraitTreeView tests prove exact admiral trait-token"),
            "navy trait-tree target must cite executable NavyTraitTreeView proof"
        );
        assert!(
            navy_targets[0].note.contains("derived static DEF catalog"),
            "navy trait-tree target must cite derived static DEF metadata proof"
        );
        assert!(
            navy_targets[0]
                .note
                .contains("full static trait catalog graph"),
            "navy trait-tree target must not overclaim full trait catalog graph parity"
        );

        let army_hover = army.category(InteractionCategory::HoverTooltip).unwrap();
        assert_eq!(army_hover.status, InteractionStatus::Open);
        assert!(
            army_hover.note.contains("owned trait-token nodes only"),
            "army trait-tree hover contract must limit proof to surfaced owned traits"
        );
        assert!(
            army_hover.note.contains("full the game trait tooltip frame"),
            "army trait-tree hover contract must keep full tooltip parity open"
        );
        let army_hover_targets: Vec<_> = army.targets(InteractionCategory::HoverTooltip).collect();
        assert_eq!(army_hover_targets.len(), 1);
        assert_eq!(
            army_hover_targets[0].id,
            "leaders.army_trait_tree.hover.owned_trait_node"
        );
        assert!(
            army_hover_targets[0]
                .note
                .contains("ArmyTraitTreeView tests prove factual passive tooltip rows"),
            "army trait-tree hover target must cite executable tooltip proof"
        );
        assert!(
            army_hover_targets[0]
                .note
                .contains("inert no-leader glyph hover"),
            "army trait-tree hover target must cite inert unavailable-glyph hover proof"
        );
        assert!(
            army_hover_targets[0]
                .note
                .contains("derived static DEF catalog"),
            "army trait-tree hover target must cite static DEF metadata tooltip proof"
        );

        let navy_hover = navy.category(InteractionCategory::HoverTooltip).unwrap();
        assert_eq!(navy_hover.status, InteractionStatus::Open);
        let navy_hover_targets: Vec<_> = navy.targets(InteractionCategory::HoverTooltip).collect();
        assert_eq!(navy_hover_targets.len(), 1);
        assert_eq!(
            navy_hover_targets[0].id,
            "leaders.navy_trait_tree.hover.owned_trait_node"
        );
        assert!(
            navy_hover_targets[0]
                .note
                .contains("NavyTraitTreeView tests prove the same factual tooltip composer"),
            "navy trait-tree hover target must cite executable NavyTraitTreeView tooltip path"
        );
        assert!(
            navy_hover_targets[0]
                .note
                .contains("derived static DEF catalog"),
            "navy trait-tree hover target must cite static DEF metadata tooltip proof"
        );
        assert!(
            navy_hover_targets[0]
                .note
                .contains("full the game trait tooltip frame"),
            "navy trait-tree hover target must not overclaim full tooltip parity"
        );

        let army_disabled = army
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(army_disabled.status, InteractionStatus::Open);
        assert!(
            army_disabled
                .note
                .contains("add_trait_0 through add_trait_3"),
            "army trait-tree disabled-mutation contract must cite the scoped add-trait leaves"
        );
        assert!(
            army_disabled.note.contains("traits_tabs")
                && army_disabled
                    .note
                    .contains("preferred_tactic_tiling_container"),
            "army trait-tree disabled-mutation contract must cite the proven tactic/tab containers"
        );
        assert!(
            army_disabled
                .note
                .contains("broader trait-tree mutation parity remain open"),
            "army trait-tree disabled-mutation contract must not claim broad mutation parity"
        );
        let army_disabled_targets: Vec<_> = army
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(army_disabled_targets.len(), 1);
        assert_eq!(
            army_disabled_targets[0].id,
            "leaders.army_trait_tree.mutation.add_trait_slots"
        );
        assert!(
            army_disabled_targets[0]
                .note
                .contains("ArmyTraitTreeView tests prove add_trait_0"),
            "army trait-tree disabled target must cite executable no-op proof"
        );
        assert!(
            army_disabled_targets[0].note.contains("traits_tabs")
                && army_disabled_targets[0]
                    .note
                    .contains("preferred_tactic_tiling_container"),
            "army trait-tree disabled target must cite executable trait-control no-op proof"
        );
        assert!(
            army_disabled_targets[0]
                .note
                .contains("unchanged selected trait/detail bind digest"),
            "army trait-tree disabled target must cite unchanged digest proof"
        );

        let navy_disabled = navy
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(navy_disabled.status, InteractionStatus::Open);
        assert!(
            navy_disabled
                .note
                .contains("add_trait_0 through add_trait_3"),
            "navy trait-tree disabled-mutation contract must cite the scoped add-trait leaves"
        );
        assert!(
            navy_disabled.note.contains("traits_tabs")
                && navy_disabled.note.contains("button-level tab/wiki controls"),
            "navy trait-tree disabled-mutation contract must cite the proven tab container and leave button-level tab/wiki open"
        );
        let navy_disabled_targets: Vec<_> = navy
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(navy_disabled_targets.len(), 1);
        assert_eq!(
            navy_disabled_targets[0].id,
            "leaders.navy_trait_tree.mutation.add_trait_slots"
        );
        assert!(
            navy_disabled_targets[0]
                .note
                .contains("NavyTraitTreeView tests prove add_trait_0"),
            "navy trait-tree disabled target must cite executable no-op proof"
        );
        assert!(
            navy_disabled_targets[0].note.contains("traits_tabs")
                && navy_disabled_targets[0]
                    .note
                    .contains("button-level tab/wiki controls"),
            "navy trait-tree disabled target must cite executable trait-control no-op proof"
        );
        assert!(
            navy_disabled_targets[0]
                .note
                .contains("broader trait-tree mutation parity remain open"),
            "navy trait-tree disabled target must not overclaim broader mutation parity"
        );
    }

    #[test]
    fn research_research_slot_row_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("research.research")).is_some(),
            "the Research row-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let research = contract_by_id(&contracts, ViewId("research.research"));

        let selection = research
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("active-slot row selection proof"),
            "research.research list-selection contract must cite active-slot proof"
        );
        assert!(
            selection
                .note
                .contains("mutating research controls remain open"),
            "research.research proof must keep mutating research controls open"
        );

        let row_targets: Vec<_> = research
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "research.research.row.active_slot");
        assert_eq!(row_targets[0].label, "Active research slot row");
        assert!(
            row_targets[0]
                .note
                .contains("ResearchView tests prove exact-tech-token"),
            "research.research row target must cite executable ResearchView proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing-selected fail-closed proof"),
            "research.research row target must cite missing-selected fail-closed proof"
        );
    }

    #[test]
    fn research_tech_tree_node_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("research.tech_tree")).is_some(),
            "the tech-tree node-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let tech_tree = contract_by_id(&contracts, ViewId("research.tech_tree"));

        let selection = tech_tree
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection.note.contains("tech-node click selection proof"),
            "research.tech_tree list-selection contract must cite node-click proof"
        );
        assert!(
            selection
                .note
                .contains("selected-detail panel/digest slice"),
            "research.tech_tree selection proof must cite the selected-detail slice"
        );
        assert!(
            selection.note.contains("full the game detail-window parity"),
            "research.tech_tree selection proof must keep detail parity open"
        );

        let row_targets: Vec<_> = tech_tree
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(row_targets[0].id, "research.tech_tree.row.tech_node");
        assert_eq!(row_targets[0].label, "Tech tree node");
        assert!(
            row_targets[0]
                .note
                .contains("TechTreeView tests prove exact catalog tech-id"),
            "research.tech_tree row target must cite executable TechTreeView proof"
        );
        assert!(
            row_targets[0].note.contains("drag-pan conflict protection"),
            "research.tech_tree row target must cite drag/click conflict proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing-selected fail-closed proof"),
            "research.tech_tree row target must cite missing-selected fail-closed proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("selected-detail panel/digest slice"),
            "research.tech_tree row target must cite selected-detail proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("unavailable cost never fabricated as zero"),
            "research.tech_tree row target must cite no-fabrication detail proof"
        );
    }

    #[test]
    fn research_doctrine_tree_node_selection_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("research.doctrine_tree")).is_some(),
            "the doctrine-tree node-selection inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let doctrine_tree = contract_by_id(&contracts, ViewId("research.doctrine_tree"));

        let selection = doctrine_tree
            .category(InteractionCategory::ListSelection)
            .unwrap();
        assert_eq!(selection.status, InteractionStatus::Open);
        assert!(
            selection
                .note
                .contains("doctrine-node click selection proof"),
            "research.doctrine_tree list-selection contract must cite node-click proof"
        );
        assert!(
            selection
                .note
                .contains("selected-detail panel/digest slice"),
            "research.doctrine_tree selection proof must cite the selected-detail slice"
        );
        assert!(
            selection.note.contains("full the game detail-window parity"),
            "research.doctrine_tree selection proof must keep detail parity open"
        );

        let row_targets: Vec<_> = doctrine_tree
            .targets(InteractionCategory::ListSelection)
            .collect();
        assert_eq!(row_targets.len(), 1);
        assert_eq!(
            row_targets[0].id,
            "research.doctrine_tree.row.doctrine_node"
        );
        assert_eq!(row_targets[0].label, "Doctrine tree node");
        assert!(
            row_targets[0]
                .note
                .contains("DoctrineTreeView tests prove exact catalog doctrine-id"),
            "research.doctrine_tree row target must cite executable DoctrineTreeView proof"
        );
        assert!(
            row_targets[0].note.contains("drag-pan conflict protection"),
            "research.doctrine_tree row target must cite drag/click conflict proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("missing-selected fail-closed proof"),
            "research.doctrine_tree row target must cite missing-selected fail-closed proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("selected-detail panel/digest slice"),
            "research.doctrine_tree row target must cite selected-detail proof"
        );
        assert!(
            row_targets[0]
                .note
                .contains("without fabricating unresearched"),
            "research.doctrine_tree row target must cite doctrine no-fabrication detail proof"
        );
    }

    #[test]
    fn research_tech_tree_hover_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("research.tech_tree")).is_some(),
            "the tech-tree hover inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let tech_tree = contract_by_id(&contracts, ViewId("research.tech_tree"));
        let hover = tech_tree
            .category(InteractionCategory::HoverTooltip)
            .unwrap();
        assert_eq!(hover.status, InteractionStatus::Open);
        assert!(
            hover.note.contains("tech-node hover tooltip proof"),
            "research.tech_tree hover contract must cite executable tech-node proof"
        );
        assert!(
            hover.note.contains("full the game tooltip frames"),
            "research.tech_tree hover contract must not overclaim game tooltip parity"
        );

        let hover_targets: Vec<_> = tech_tree
            .targets(InteractionCategory::HoverTooltip)
            .collect();
        assert_eq!(hover_targets.len(), 1);
        assert_eq!(hover_targets[0].id, "research.tech_tree.hover.tech_node");
        assert!(
            hover_targets[0].note.contains("TechTreeView tests prove"),
            "research.tech_tree hover target must cite executable TechTreeView proof"
        );
        assert!(
            hover_targets[0]
                .note
                .contains("missing/null cost unavailable"),
            "research.tech_tree hover target must cite no-fabrication cost proof"
        );
    }

    #[test]
    fn research_doctrine_tree_hover_target_cites_executable_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("research.doctrine_tree")).is_some(),
            "the doctrine-tree hover inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let doctrine_tree = contract_by_id(&contracts, ViewId("research.doctrine_tree"));
        let hover = doctrine_tree
            .category(InteractionCategory::HoverTooltip)
            .unwrap();
        assert_eq!(hover.status, InteractionStatus::Open);
        assert!(
            hover.note.contains("doctrine-node hover tooltip proof"),
            "research.doctrine_tree hover contract must cite executable doctrine-node proof"
        );
        assert!(
            hover.note.contains("full the game tooltip frames"),
            "research.doctrine_tree hover contract must not overclaim game tooltip parity"
        );

        let hover_targets: Vec<_> = doctrine_tree
            .targets(InteractionCategory::HoverTooltip)
            .collect();
        assert_eq!(hover_targets.len(), 1);
        assert_eq!(
            hover_targets[0].id,
            "research.doctrine_tree.hover.doctrine_node"
        );
        assert!(
            hover_targets[0]
                .note
                .contains("DoctrineTreeView tests prove"),
            "research.doctrine_tree hover target must cite executable DoctrineTreeView proof"
        );
        assert!(
            hover_targets[0]
                .note
                .contains("unfed-badged parser-unfed state"),
            "research.doctrine_tree hover target must cite no-fabrication doctrine-state proof"
        );
    }

    #[test]
    fn ingame_menu_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("logistics.ingame_menu")).is_some(),
            "the ingame-menu disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let ingame_menu = contract_by_id(&contracts, ViewId("logistics.ingame_menu"));

        let disabled = ingame_menu
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled.note.contains("save/load/resign/quit"),
            "ingame-menu disabled-mutation row must stay scoped to documented menu verbs"
        );
        assert!(
            disabled
                .note
                .contains("all other mutation controls remain open"),
            "ingame-menu proof must not overclaim broad mutation parity"
        );

        let targets: Vec<_> = ingame_menu
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "logistics.ingame_menu.mutation.verbs");
        assert_eq!(targets[0].label, "Save/Load/Resign/Quit verbs");
        assert!(
            targets[0]
                .note
                .contains("IngameMenuView/WindowHost tests prove"),
            "ingame-menu target must cite executable click-no-op proof"
        );
        assert!(
            targets[0]
                .note
                .contains("broader mutation parity remain open"),
            "ingame-menu target must keep broader mutation parity open"
        );
    }

    #[test]
    fn core_state_panel_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("core.state_panel")).is_some(),
            "the state-panel disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let state_panel = contract_by_id(&contracts, ViewId("core.state_panel"));

        let disabled = state_panel
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled.note.contains("occupation-law")
                && disabled.note.contains("garrison-template")
                && disabled.note.contains("scorched-earth"),
            "state-panel disabled-mutation row must cite the proven visible controls"
        );
        assert!(
            disabled
                .note
                .contains("broader state-panel mutation parity remain open"),
            "state-panel proof must not overclaim broad mutation parity"
        );

        let targets: Vec<_> = state_panel
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].id,
            "core.state_panel.mutation.occupation_controls"
        );
        assert_eq!(
            targets[0].label,
            "Occupation law/template and scorched-earth controls"
        );
        assert!(
            targets[0].note.contains("StatePanelView tests prove"),
            "state-panel target must cite executable click-no-op proof"
        );
        assert!(
            targets[0].note.contains("select_law_button")
                && targets[0].note.contains("template_garrison_button")
                && targets[0].note.contains("scorched_earth_state_button"),
            "state-panel target must name the real GUI leaves"
        );
        assert!(
            targets[0]
                .note
                .contains("neighboring panel space remains unclaimed"),
            "state-panel target must cite non-broad-hit proof"
        );
        assert!(
            targets[0]
                .note
                .contains("shared-slot building remove buttons"),
            "state-panel target must leave remaining state-panel mutation controls open"
        );
        assert!(
            targets[0].note.contains("unrendered/unclaimed")
                && targets[0].note.contains("shared-slot row factory"),
            "state-panel target must not claim shared-slot remove no-op proof before those leaves render"
        );
    }

    #[test]
    fn core_topbar_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("core.topbar")).is_some(),
            "the topbar disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let topbar = contract_by_id(&contracts, ViewId("core.topbar"));

        let disabled = topbar
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled.note.contains("pause/date")
                && disabled.note.contains("speed-control")
                && disabled.note.contains("lobby/menu/help/achievements")
                && disabled.note.contains("playlist/music controls")
                && disabled.note.contains("dismissed_alerts_button")
                && disabled.note.contains("faction_button"),
            "topbar disabled-mutation row must cite the proven time, chrome, alert, and faction no-op clusters"
        );
        assert!(
            disabled.note.contains("threat_button/threat_value_button")
                && disabled.note.contains("read-only OpenView controls"),
            "topbar disabled-mutation row must route threat controls through drilldown, not no-op mutation"
        );
        assert!(
            disabled
                .note
                .contains("broader topbar mutation parity remain open"),
            "topbar proof must not overclaim broad mutation parity"
        );

        let targets: Vec<_> = topbar
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].id, "core.topbar.mutation.time_controls");
        assert_eq!(targets[0].label, "Pause/date and speed controls");
        assert!(
            targets[0].note.contains("TopbarView tests prove"),
            "topbar target must cite executable click-no-op proof"
        );
        assert!(
            targets[0].note.contains("pause_button_date")
                && targets[0].note.contains("button_speedstep5")
                && targets[0].note.contains("button_speedup"),
            "topbar target must name the real time-control GUI leaves"
        );
        assert!(
            targets[0]
                .note
                .contains("higher-z panels prevent hidden topbar"),
            "topbar target must cite hidden-target arming protection"
        );
        assert!(
            targets[0]
                .note
                .contains("broader topbar mutation parity remain open"),
            "topbar time target must leave broader mutation parity open"
        );

        assert_eq!(targets[1].id, "core.topbar.mutation.lobby_menu_music");
        assert_eq!(
            targets[1].label,
            "Lobby/menu/help/achievements and music controls"
        );
        assert!(
            targets[1].note.contains("TopbarView tests prove"),
            "topbar chrome target must cite executable click-no-op proof"
        );
        assert!(
            targets[1].note.contains("reopen_ingame_lobby")
                && targets[1].note.contains("menu_button")
                && targets[1].note.contains("achievements_button")
                && targets[1].note.contains("playlist_button")
                && targets[1].note.contains("music_pause_button")
                && targets[1].note.contains("music_next_button")
                && targets[1].note.contains("musicplayer_button"),
            "topbar chrome target must name the real menu/music GUI leaves"
        );
        assert!(
            targets[1]
                .note
                .contains("higher-z panels prevent hidden topbar"),
            "topbar chrome target must cite hidden-target arming protection"
        );
        assert!(
            targets[1]
                .note
                .contains("broader topbar mutation parity remain open"),
            "topbar chrome target must leave broader mutation parity open"
        );

        assert_eq!(targets[2].id, "core.topbar.mutation.alerts_faction_noops");
        assert_eq!(
            targets[2].label,
            "Dismissed alerts and faction no-op controls"
        );
        assert!(
            targets[2].note.contains("dismissed_alerts_button")
                && targets[2].note.contains("faction_button"),
            "topbar alert/faction target must name the real GUI leaves"
        );
        assert!(
            targets[2].note.contains("no emitted commands")
                && targets[2].note.contains("unchanged topbar digest"),
            "topbar alert/faction target must cite executable no-op proof"
        );
        assert!(
            targets[2].note.contains("missing alert ledger")
                && targets[2].note.contains("factual hover rows"),
            "topbar alert/faction target must cite fail-closed data behavior"
        );
    }

    #[test]
    fn event_window_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("logistics.event_window")).is_some(),
            "the event-window disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let event_window = contract_by_id(&contracts, ViewId("logistics.event_window"));

        let disabled = event_window
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled
                .note
                .contains("static event-definition preview option actions"),
            "event-window disabled-mutation row must stay scoped to displayed preview options"
        );
        assert!(
            disabled.note.contains("inside-root modal chrome clicks"),
            "event-window disabled-mutation row must cite the bounded modal chrome proof"
        );
        assert!(
            disabled
                .note
                .contains("broader event modal/mutation parity remain open"),
            "event-window proof must not overclaim broad event/popup mutation parity"
        );

        let targets: Vec<_> = event_window
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].id,
            "logistics.event_window.mutation.definition_options"
        );
        assert_eq!(targets[0].label, "Definition preview option actions");
        assert!(
            targets[0]
                .note
                .contains("EventWindowView/WindowHost tests prove"),
            "event-window target must cite executable click-no-op proof"
        );
        assert!(
            targets[0].note.contains("popup-window buttons"),
            "event-window target must leave popup button parity open"
        );
        assert!(
            targets[0]
                .note
                .contains("inside-root outside-option chrome clicks"),
            "event-window target must cite bounded inside-root modal chrome proof"
        );
        assert!(
            targets[0].note.contains("host modal policy"),
            "event-window target must distinguish off-root host-policy blocking from view-specific routing"
        );
        assert!(
            targets[0]
                .note
                .contains("broader event modal parity remain open"),
            "event-window target must not claim complete modal parity"
        );
    }

    #[test]
    fn popup_window_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("logistics.popup_window")).is_some(),
            "the popup-window disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let popup_window = contract_by_id(&contracts, ViewId("logistics.popup_window"));

        let disabled = popup_window
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled
                .note
                .contains("displayed generic popup-library action area"),
            "popup-window disabled-mutation row must stay scoped to the displayed popup action area"
        );
        assert!(
            disabled
                .note
                .contains("broader popup mutation parity remain open"),
            "popup-window proof must not overclaim broad popup mutation parity"
        );

        let targets: Vec<_> = popup_window
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].id,
            "logistics.popup_window.mutation.displayed_action"
        );
        assert_eq!(targets[0].label, "Displayed popup action");
        assert!(
            targets[0]
                .note
                .contains("PopupWindowView/WindowHost tests prove"),
            "popup-window target must cite executable click-no-op proof"
        );
        assert!(
            targets[0]
                .note
                .contains("per-root popup button leaf routing"),
            "popup-window target must leave per-root popup button parity open"
        );
    }

    #[test]
    fn production_construction_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("production.construction")).is_some(),
            "the construction disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let construction = contract_by_id(&contracts, ViewId("production.construction"));

        let disabled = construction
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled
                .note
                .contains("top count-container factory add/sub buttons and visible queue priority/amount/close buttons"),
            "construction disabled-mutation row must cite both proven construction mutation targets"
        );
        assert!(
            disabled
                .note
                .contains("broader construction mutation parity remain open"),
            "construction disabled-mutation proof must not overclaim broad mutation parity"
        );

        let targets: Vec<_> = construction
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0].id,
            "production.construction.mutation.count_container_factories"
        );
        assert_eq!(targets[0].label, "Top factory add/sub buttons");
        assert!(
            targets[0].note.contains("ConstructionView tests prove"),
            "construction target must cite executable click-no-op proof"
        );
        assert!(
            targets[0].note.contains("factories_sub_1x")
                && targets[0].note.contains("factories_add_10x"),
            "construction target must name the real top add/sub button variants"
        );
        assert!(
            targets[0]
                .note
                .contains("neighboring count-container clicks remain unclaimed"),
            "construction target must not claim broad count-container blocking"
        );
        assert!(
            targets[0].note.contains("build-picker actions"),
            "construction target must leave remaining construction mutation controls open"
        );
        assert_eq!(
            targets[1].id,
            "production.construction.mutation.queue_controls"
        );
        assert_eq!(
            targets[1].label,
            "Visible queue priority/amount/close buttons"
        );
        assert!(
            targets[1].note.contains("production_building_line_entry")
                && targets[1].note.contains("increase_priority")
                && targets[1].note.contains("increase_amount")
                && targets[1].note.contains("close_production_line"),
            "construction queue target must cite the real rendered row template leaves"
        );
        assert!(
            targets[1]
                .note
                .contains("neighboring row-body clicks still select"),
            "construction queue target must cite non-broad-hit proof"
        );
    }

    #[test]
    fn production_production_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("production.production")).is_some(),
            "the production disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let production = contract_by_id(&contracts, ViewId("production.production"));

        let disabled = production
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled
                .note
                .contains("visible add-line buttons, production-line close buttons, factory add/sub buttons, priority/amount buttons, and row manufacturer/equipment/convert/deploy/name-list controls"),
            "production disabled-mutation row must cite all five proven visible mutation targets"
        );
        assert!(
            disabled
                .note
                .contains("broader production mutation parity remain open"),
            "production disabled-mutation proof must not overclaim broad mutation parity"
        );

        let targets: Vec<_> = production
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 5);
        assert_eq!(targets[0].id, "production.production.mutation.add_line");
        assert_eq!(targets[0].label, "Visible add-line buttons");
        assert!(
            targets[0].note.contains("infantry_button")
                && targets[0].note.contains("tank_button")
                && targets[0].note.contains("aircraft_button")
                && targets[0].note.contains("naval_button")
                && targets[0].note.contains("naval_repair_button"),
            "add-line target must cite the real visible root button leaves"
        );
        assert!(
            targets[0].note.contains("81x41")
                && targets[0]
                    .note
                    .contains("empty space between aircraft and naval groups remains unclaimed"),
            "add-line target must cite measured frame geometry and non-broad-hit proof"
        );
        assert!(
            targets[0]
                .note
                .contains("variant/obsolete equipment-info controls"),
            "add-line target must leave remaining production mutation controls open"
        );
        assert_eq!(targets[1].id, "production.production.mutation.close_line");
        assert_eq!(targets[1].label, "Visible line close buttons");
        assert!(
            targets[1].note.contains("ProductionView tests prove"),
            "production target must cite executable click-no-op proof"
        );
        assert!(
            targets[1].note.contains("close_production_line")
                && targets[1]
                    .note
                    .contains("neighboring row-body clicks still select"),
            "production target must name the real close leaf and the non-broad-hit proof"
        );
        assert!(
            targets[1]
                .note
                .contains("variant/obsolete equipment-info controls"),
            "production target must leave remaining production mutation controls open"
        );
        assert_eq!(
            targets[2].id,
            "production.production.mutation.factory_count"
        );
        assert_eq!(targets[2].label, "Visible factory add/sub buttons");
        assert!(
            targets[2].note.contains("production.json now preserves")
                && targets[2].note.contains("@prod_* defines"),
            "factory target must cite the anchor-resolution proof"
        );
        assert!(
            targets[2].note.contains("factories_sub_1x")
                && targets[2].note.contains("factories_add_10x"),
            "factory target must name the real GUI leaves"
        );
        assert!(
            targets[2]
                .note
                .contains("neighboring row-body clicks still select")
                && targets[2]
                    .note
                    .contains("variant/obsolete equipment-info controls"),
            "factory target must cite non-broad-hit proof and leave remaining controls open"
        );
        assert_eq!(
            targets[3].id,
            "production.production.mutation.priority_amount"
        );
        assert_eq!(targets[3].label, "Visible priority/amount buttons");
        assert!(
            targets[3].note.contains("increase_priority")
                && targets[3].note.contains("btn_x10")
                && targets[3].note.contains("increase_amount")
                && targets[3]
                    .note
                    .contains("neighboring row-body clicks still select"),
            "priority/amount target must cite the real GUI leaves and non-broad-hit proof"
        );
        assert_eq!(
            targets[4].id,
            "production.production.mutation.line_controls"
        );
        assert_eq!(
            targets[4].label,
            "Visible line manufacturer/equipment/deploy controls"
        );
        assert!(
            targets[4].note.contains("production_military_line_entry")
                && targets[4].note.contains("attach_manufacturer")
                && targets[4].note.contains("equipment_button")
                && targets[4].note.contains("convert")
                && targets[4].note.contains("production_naval_line_entry")
                && targets[4].note.contains("carrier_air_wing_composition")
                && targets[4].note.contains("deploy_location")
                && targets[4].note.contains("name_list_btn"),
            "line-control target must cite the real military/naval template leaves"
        );
        assert!(
            targets[4]
                .note
                .contains("neighboring equipment and name-list row-body clicks still select")
                && targets[4]
                    .note
                    .contains("variant/obsolete equipment-info controls")
                && targets[4].note.contains("unrendered refit-template controls"),
            "line-control target must cite non-broad-hit proof and leave hidden production controls open"
        );
    }

    #[test]
    fn production_decisions_disabled_mutation_target_cites_executable_noop_proof() {
        let reg = ViewRegistry::from_inventory();
        assert!(
            reg.view(ViewId("production.decisions")).is_some(),
            "the decisions disabled-mutation inventory must be anchored to a real registered view"
        );
        let contracts = interaction_contracts(&reg);
        let decisions = contract_by_id(&contracts, ViewId("production.decisions"));

        let disabled = decisions
            .category(InteractionCategory::DisabledMutation)
            .unwrap();
        assert_eq!(disabled.status, InteractionStatus::Open);
        assert!(
            disabled
                .note
                .contains("visible timed-decision btn_select action buttons"),
            "decisions disabled-mutation row must cite the proven btn_select target"
        );
        assert!(
            disabled
                .note
                .contains("broader decisions mutation parity remain open"),
            "decisions disabled-mutation proof must not overclaim broad decisions parity"
        );

        let targets: Vec<_> = decisions
            .targets(InteractionCategory::DisabledMutation)
            .collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "production.decisions.mutation.timed_select");
        assert_eq!(targets[0].label, "Timed decision select buttons");
        assert!(
            targets[0].note.contains("DecisionsView tests prove")
                && targets[0].note.contains("btn_select"),
            "decisions target must cite executable btn_select no-op proof"
        );
        assert!(
            targets[0]
                .note
                .contains("neighboring row-body clicks still select")
                && targets[0]
                    .note
                    .contains("scrolled visible rows remain correctly resolved"),
            "decisions target must cite non-broad-hit and scrolled-row proof"
        );
        assert!(
            targets[0].note.contains("one-shot decision availability")
                && targets[0]
                    .note
                    .contains("broader decisions mutation parity"),
            "decisions target must leave remaining decision mutation controls open"
        );
    }

    #[test]
    fn core_state_panel_building_scroll_target_is_inventory_tracked() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let state_panel = contract_by_id(&contracts, ViewId("core.state_panel"));

        let scroll = state_panel.category(InteractionCategory::Scroll).unwrap();
        assert_eq!(scroll.status, InteractionStatus::Open);
        assert!(
            scroll
                .note
                .contains("wheel-scroll and scrollbar-thumb drag proof"),
            "state panel scroll contract must cite executable wheel/thumb proof"
        );
        assert!(
            scroll
                .note
                .contains("broader state-panel parity remain open"),
            "state panel scroll proof must keep broader state-panel parity open"
        );

        let targets: Vec<_> = state_panel.targets(InteractionCategory::Scroll).collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "core.state_panel.scroll.buildings");
        assert_eq!(targets[0].label, "Building grid wheel/thumb scroll");
        assert!(
            targets[0]
                .note
                .contains("StatePanelView tests prove wheel scroll"),
            "state panel scroll target must cite executable StatePanelView proof"
        );
        assert!(
            targets[0].note.contains("track clicks are inert")
                && targets[0]
                    .note
                    .contains("outside-grid wheel input is inert")
                && targets[0].note.contains("scrub rebind clamps"),
            "state panel scroll target must cite track, outside-wheel, and scrub-clamp proof"
        );
    }

    #[test]
    fn core_state_panel_hover_target_is_inventory_tracked() {
        let reg = ViewRegistry::from_inventory();
        let contracts = interaction_contracts(&reg);
        let state_panel = contract_by_id(&contracts, ViewId("core.state_panel"));

        let hover = state_panel
            .category(InteractionCategory::HoverTooltip)
            .unwrap();
        assert_eq!(hover.status, InteractionStatus::Open);
        assert!(
            hover.note.contains("state owner flags")
                && hover.note.contains("scroll-aware building hover")
                && hover.note.contains("stale-hover fail-closed"),
            "state panel hover contract must cite factual/no-fabrication proof"
        );
        assert!(
            hover.note.contains("full the game tooltip frames"),
            "state panel hover proof must not overclaim full tooltip parity"
        );

        let targets: Vec<_> = state_panel
            .targets(InteractionCategory::HoverTooltip)
            .collect();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "core.state_panel.hover.factual_rows");
        assert_eq!(targets[0].label, "State panel factual hover tooltips");
        assert!(
            targets[0]
                .note
                .contains("StatePanelView tests prove passive factual tooltips"),
            "state panel hover target must cite executable StatePanelView proof"
        );
        assert!(
            targets[0]
                .note
                .contains("stale-row rebind clearing instead of falling back"),
            "state panel hover target must cite stale-row no-fabrication proof"
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
            "the report must keep ten category rows per registered view"
        );
        assert_eq!(report.category_status_counts.open, 175);
        assert_eq!(report.category_status_counts.not_proven, 965);
        assert_eq!(report.missing_not_proven_category_count, 965);

        assert_eq!(
            report.target_row_count, 123,
            "current target rows are 18 tab rows, 39 list-selection rows, one text-input row, 13 scroll rows, twelve hover rows, 20 drill/navigation rows, and twenty disabled-mutation rows"
        );
        assert_eq!(report.target_status_counts.open, 123);
        assert_eq!(report.target_status_counts.not_proven, 0);
        assert_eq!(
            report.target_rows_with_executable_proof_notes, 123,
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
                open: 6,
                not_proven: 108,
            },
            18,
            InteractionStatusCounts {
                open: 18,
                not_proven: 0,
            },
            18,
        );
        assert_category_report(
            &report,
            InteractionCategory::ListSelection,
            InteractionStatusCounts {
                open: 18,
                not_proven: 96,
            },
            39,
            InteractionStatusCounts {
                open: 39,
                not_proven: 0,
            },
            39,
        );
        assert_category_report(
            &report,
            InteractionCategory::TextInput,
            InteractionStatusCounts {
                open: 1,
                not_proven: 113,
            },
            1,
            InteractionStatusCounts {
                open: 1,
                not_proven: 0,
            },
            1,
        );
        assert_category_report(
            &report,
            InteractionCategory::Scroll,
            InteractionStatusCounts {
                open: 9,
                not_proven: 105,
            },
            13,
            InteractionStatusCounts {
                open: 13,
                not_proven: 0,
            },
            13,
        );
        assert_category_report(
            &report,
            InteractionCategory::HoverTooltip,
            InteractionStatusCounts {
                open: 11,
                not_proven: 103,
            },
            12,
            InteractionStatusCounts {
                open: 12,
                not_proven: 0,
            },
            12,
        );
        assert_category_report(
            &report,
            InteractionCategory::Drilldown,
            InteractionStatusCounts {
                open: 3,
                not_proven: 111,
            },
            20,
            InteractionStatusCounts {
                open: 20,
                not_proven: 0,
            },
            20,
        );
        assert_category_report(
            &report,
            InteractionCategory::DisabledMutation,
            InteractionStatusCounts {
                open: 13,
                not_proven: 101,
            },
            20,
            InteractionStatusCounts {
                open: 20,
                not_proven: 0,
            },
            20,
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
            report.intentionally_inert_mutation_count,
            Some(20),
            "only the ingame-menu disabled verb target, state-panel occupation/scorched-earth controls target, topbar time-controls target, topbar lobby/menu/music chrome target, topbar dismissed-alert/faction no-op target, event preview option target, displayed popup action target, construction top factory add/sub target, construction visible queue-controls target, production visible add-line target, production visible close-line target, production visible factory add/sub target, production visible priority/amount target, production visible line-controls target, decisions timed btn_select target, army/navy trait-tree add-trait slot targets, and tank/plane/equipment component-designer non-slot control targets have first-class executable inert proof"
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
            101,
            "bounded ingame-menu, state-panel occupation/scorched-earth controls, topbar time controls, topbar lobby/menu/music chrome controls, topbar dismissed-alert/faction controls, event-preview, displayed-popup, construction top factory, construction queue-controls, production add-line, production close-line, production factory add/sub, production priority/amount, production line-controls, decisions timed btn_select, trait-tree add-trait slot, and component-designer non-slot no-op proofs must not convert generic inert/no-op behavior into mutation parity"
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
            InteractionCategory::TextInput => target.id.contains(".text."),
            InteractionCategory::Scroll => target.id.contains(".scroll."),
            InteractionCategory::HoverTooltip => target.id.contains(".hover."),
            InteractionCategory::Drilldown => target.id.contains(".drill."),
            InteractionCategory::DisabledMutation => target.id.contains(".mutation."),
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
