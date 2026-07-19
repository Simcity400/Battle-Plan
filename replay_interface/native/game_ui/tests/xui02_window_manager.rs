use game_ui::definition_admission::{
    frozen_playset_files, load_frozen_definition_catalog, RejectedWindowDefinition,
};
use game_ui::window_manager_law::{
    apply_runtime_activation_frame, register_dormant_window_manager,
    register_effective_dormant_window_manager, resolvable_activation_sources, ReplayFocusTarget,
    ReplayOwnedWindowManager, ReplayPanelActivation, ReplayTraversalDevice,
    ReplayTraversalDirection, ReplayWindowManagerRejection, ResolvableActivationClass,
    RuntimeActivationDependency, RuntimeWindowManagerState, WindowManagerRejection,
    XUI02_DEPENDENCY_FILES, XUI02_DEPENDENCY_WINDOWS, XUI02_WINDOW_DEFINITIONS,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn catalog() -> game_ui::definition_admission::GuiDefinitionCatalog {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
    let rows = frozen_playset_files(&root).expect("frozen XUI01 playset");
    load_frozen_definition_catalog(rows).expect("XUI01 definition catalog")
}

fn fabricated_state(window_id: &str) -> RuntimeWindowManagerState {
    RuntimeWindowManagerState {
        window_id: window_id.to_string(),
        producer_packet_id: "XUI02_TEST".to_string(),
        producer_manifest_sha256: "a".repeat(64),
        producer_record: "test.controller_frame[0]".to_string(),
        open: true,
        visible: true,
        composed: true,
        modal_owner: Some("fabricated_modal".to_string()),
        stack_ordinal: Some(0),
        focus_owner: Some("fabricated_focus".to_string()),
        restore_focus_to: Some("fabricated_restore".to_string()),
        tooltip_owner: Some("fabricated_tooltip".to_string()),
        tooltip_instance_record: Some("A26.test.tooltip[0]".to_string()),
        variant_envelope_record: Some("A32.test.variant[0]".to_string()),
        keyboard_traversal: vec!["fabricated_keyboard_target".to_string()],
        controller_traversal: vec!["fabricated_controller_target".to_string()],
    }
}

fn open_state(window_id: &str, stack_ordinal: u32) -> RuntimeWindowManagerState {
    RuntimeWindowManagerState {
        window_id: window_id.to_string(),
        producer_packet_id: "XUI02_TEST".to_string(),
        producer_manifest_sha256: "a".repeat(64),
        producer_record: format!("controller_windows[{window_id}]"),
        open: true,
        visible: true,
        composed: true,
        modal_owner: None,
        stack_ordinal: Some(stack_ordinal),
        focus_owner: None,
        restore_focus_to: None,
        tooltip_owner: None,
        tooltip_instance_record: None,
        variant_envelope_record: None,
        keyboard_traversal: Vec::new(),
        controller_traversal: Vec::new(),
    }
}

fn replay_activation(window_id: &str, prefix: &str) -> ReplayPanelActivation {
    ReplayPanelActivation {
        window_id: window_id.to_string(),
        modal_owner: None,
        initial_focus: Some(format!("{prefix}.first")),
        keyboard_traversal: vec![
            format!("{prefix}.first"),
            format!("{prefix}.second"),
            format!("{prefix}.third"),
        ],
        controller_traversal: vec![
            format!("{prefix}.third"),
            format!("{prefix}.second"),
            format!("{prefix}.first"),
        ],
        tooltip_targets: vec![format!("{prefix}.hover")],
    }
}

#[test]
fn exact_full_definition_domain_registers_dormant_with_complete_coverage() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    assert_eq!(registry.windows.len(), XUI02_WINDOW_DEFINITIONS);
    assert_eq!(registry.dependency_window_count(), XUI02_DEPENDENCY_WINDOWS);
    assert_eq!(registry.dependency_file_count(), XUI02_DEPENDENCY_FILES);
    assert_eq!(
        registry
            .windows
            .iter()
            .map(|row| row.window_id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        XUI02_WINDOW_DEFINITIONS
    );
    assert!(registry.windows.iter().all(|row| {
        !row.open
            && !row.visible
            && !row.composed
            && row.modal_owner.is_none()
            && row.stack_ordinal.is_none()
            && row.focus_owner.is_none()
            && row.restore_focus_to.is_none()
            && row.tooltip_owner.is_none()
            && row.keyboard_traversal.is_empty()
            && row.controller_traversal.is_empty()
    }));
}

#[test]
fn namespace_drift_fails_closed() {
    let mut catalog = catalog();
    catalog.files.pop();
    assert_eq!(
        register_dormant_window_manager(&catalog, &[]),
        Err(WindowManagerRejection::NamespaceDrift)
    );
}

#[test]
fn admitted_effective_namespace_registers_without_frozen_vanilla_counts() {
    let mut effective = catalog();
    let removed = effective.admitted_windows.pop().expect("window row");
    effective
        .files
        .iter_mut()
        .find(|row| row.source_file == removed.source_file)
        .expect("owning file row")
        .admitted_windows -= 1;
    let registry = register_effective_dormant_window_manager(&effective, &[])
        .expect("manifest-authenticated effective namespace");
    assert_eq!(registry.windows.len(), XUI02_WINDOW_DEFINITIONS - 1);
    let manager = ReplayOwnedWindowManager::new(&registry).expect("replay-owned manager");
    assert_eq!(manager.panel_count(), XUI02_WINDOW_DEFINITIONS - 1);
}

#[test]
fn any_definition_rejection_fails_closed() {
    let mut catalog = catalog();
    catalog.rejected_windows.push(RejectedWindowDefinition {
        window_id: "fabricated#00001".to_string(),
        source_file: "fabricated.gui".to_string(),
        ordinal: 1,
        element_type: "unknownType".to_string(),
        name: "fabricated".to_string(),
        hierarchy_path: "fabricated".to_string(),
        unsupported: BTreeSet::from(["fabricated".to_string()]),
    });
    assert_eq!(
        register_dormant_window_manager(&catalog, &[]),
        Err(WindowManagerRejection::DefinitionRejectionsPresent)
    );
}

#[test]
fn unknown_runtime_state_window_fails_closed() {
    let catalog = catalog();
    let state = fabricated_state("interface/unknown.gui#99999");
    assert_eq!(
        register_dormant_window_manager(&catalog, &[state]),
        Err(WindowManagerRejection::UnknownManagerStateWindow(
            "interface/unknown.gui#99999".to_string()
        ))
    );
}

#[test]
fn known_but_unowned_runtime_state_fails_closed_with_every_field_populated() {
    let catalog = catalog();
    let window_id = catalog.admitted_windows[0].window_id.clone();
    let state = fabricated_state(&window_id);
    assert_eq!(
        register_dormant_window_manager(&catalog, &[state]),
        Err(WindowManagerRejection::RuntimeStateRequiresTypedOwnerAdmission(window_id))
    );
}

#[test]
fn every_currently_resolvable_activation_source_expands_exactly() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let sources = resolvable_activation_sources(&registry).expect("activation sources");
    assert!(!sources.is_empty());
    assert_eq!(
        sources
            .iter()
            .map(|source| source.window_id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        sources.len()
    );
    let states = sources
        .iter()
        .enumerate()
        .map(|(ordinal, source)| open_state(&source.window_id, ordinal as u32))
        .collect::<Vec<_>>();
    let frame = apply_runtime_activation_frame(&registry, &states).expect("activation frame");
    assert_eq!(
        frame.resolved.len() + frame.blocked.len(),
        XUI02_WINDOW_DEFINITIONS
    );
    let direct = frame
        .resolved
        .iter()
        .filter(|row| !row.inherited_from_source)
        .count();
    let inherited = frame
        .resolved
        .iter()
        .filter(|row| row.inherited_from_source)
        .count();
    assert_eq!(direct, sources.len());
    assert_eq!(direct + inherited, frame.resolved.len());
    assert!(frame.resolved.iter().all(|row| {
        row.open && row.visible && row.composed && !row.producer_record.trim().is_empty()
    }));
    let mut dependencies = [0usize; 5];
    for row in &frame.blocked {
        match &row.dependency {
            RuntimeActivationDependency::AwaitingResolvableControllerFrame(_) => {
                dependencies[0] += 1
            }
            RuntimeActivationDependency::AwaitingDataProducer { .. } => dependencies[1] += 1,
            RuntimeActivationDependency::DownstreamOwners(_) => dependencies[2] += 1,
            RuntimeActivationDependency::StaticHelperHasNoRuntimeProducer => dependencies[3] += 1,
            RuntimeActivationDependency::NoRuntimeProducer => dependencies[4] += 1,
        }
    }
    assert_eq!(dependencies[0], 0);
    assert_eq!(dependencies[1], 0);
    assert_eq!(dependencies[2..].iter().sum::<usize>(), frame.blocked.len());
}

#[test]
fn complete_manager_facets_survive_on_source_and_not_on_inherited_children() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let sources = resolvable_activation_sources(&registry).expect("activation sources");
    let mut owner = open_state(&sources[0].window_id, 1);
    let mut modal = open_state(&sources[1].window_id, 2);
    modal.modal_owner = Some(owner.window_id.clone());
    modal.focus_owner = Some("focus.widget".to_string());
    modal.restore_focus_to = Some("restore.widget".to_string());
    modal.tooltip_owner = Some("tooltip.widget".to_string());
    modal.tooltip_instance_record = Some("A26.tooltip_instances[0]".to_string());
    modal.variant_envelope_record = Some("A32.variants[0]".to_string());
    modal.keyboard_traversal = vec!["key.first".to_string(), "key.second".to_string()];
    modal.controller_traversal = vec![
        "controller.first".to_string(),
        "controller.second".to_string(),
    ];
    owner.producer_record = "controller.owner".to_string();
    modal.producer_record = "controller.modal".to_string();
    let frame =
        apply_runtime_activation_frame(&registry, &[owner, modal.clone()]).expect("manager frame");
    let direct = frame
        .resolved
        .iter()
        .find(|row| row.window_id == modal.window_id)
        .expect("direct modal source");
    assert_eq!(direct.modal_owner, modal.modal_owner);
    assert_eq!(direct.focus_owner, modal.focus_owner);
    assert_eq!(direct.restore_focus_to, modal.restore_focus_to);
    assert_eq!(direct.tooltip_owner, modal.tooltip_owner);
    assert_eq!(
        direct.tooltip_instance_record,
        modal.tooltip_instance_record
    );
    assert_eq!(
        direct.variant_envelope_record,
        modal.variant_envelope_record
    );
    assert_eq!(direct.keyboard_traversal, modal.keyboard_traversal);
    assert_eq!(direct.controller_traversal, modal.controller_traversal);
    for inherited in frame.resolved.iter().filter(|row| {
        row.activation_source_window_id == modal.window_id && row.inherited_from_source
    }) {
        assert!(inherited.modal_owner.is_none());
        assert!(inherited.focus_owner.is_none());
        assert!(inherited.restore_focus_to.is_none());
        assert!(inherited.tooltip_owner.is_none());
        assert!(inherited.tooltip_instance_record.is_none());
        assert!(inherited.variant_envelope_record.is_none());
        assert!(inherited.keyboard_traversal.is_empty());
        assert!(inherited.controller_traversal.is_empty());
    }
}

#[test]
fn scripted_gui_source_without_frame_retains_exact_a38_dependency() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let source = resolvable_activation_sources(&registry)
        .expect("activation sources")
        .into_iter()
        .find(|row| row.class == ResolvableActivationClass::ScriptedGuiPredicateRoot)
        .expect("scripted GUI activation source");
    let frame = apply_runtime_activation_frame(&registry, &[]).expect("empty activation frame");
    let blocked = frame
        .blocked
        .iter()
        .find(|row| row.window_id == source.window_id)
        .expect("blocked scripted GUI source");
    assert_eq!(
        blocked.dependency,
        RuntimeActivationDependency::AwaitingDataProducer {
            activation_class: "SCRIPTED_GUI_PREDICATE_ROOT".to_string(),
            dossiers: vec!["A38_SCRIPTED_GUI_EVALUATION_INPUTS".to_string()],
        }
    );
}

#[test]
fn unresolved_source_invalid_state_and_modal_owner_all_reject() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let sources = resolvable_activation_sources(&registry).expect("activation sources");
    let source_ids = sources
        .iter()
        .map(|row| row.window_id.as_str())
        .collect::<BTreeSet<_>>();
    let unresolved = registry
        .windows
        .iter()
        .find(|row| !source_ids.contains(row.window_id.as_str()))
        .expect("unresolved row");
    assert_eq!(
        apply_runtime_activation_frame(&registry, &[open_state(&unresolved.window_id, 0)]),
        Err(WindowManagerRejection::UnresolvableActivationSource(
            unresolved.window_id.clone()
        ))
    );

    let mut invalid = open_state(&sources[0].window_id, 0);
    invalid.composed = false;
    assert_eq!(
        apply_runtime_activation_frame(&registry, &[invalid]),
        Err(WindowManagerRejection::InvalidRuntimeState(
            sources[0].window_id.clone()
        ))
    );

    let mut invalid_manifest = open_state(&sources[0].window_id, 0);
    invalid_manifest.producer_manifest_sha256 = "ABC".to_string();
    assert_eq!(
        apply_runtime_activation_frame(&registry, &[invalid_manifest]),
        Err(WindowManagerRejection::InvalidRuntimeState(
            sources[0].window_id.clone()
        ))
    );

    let mut missing_a26 = open_state(&sources[0].window_id, 0);
    missing_a26.tooltip_owner = Some("tooltip.widget".to_string());
    assert_eq!(
        apply_runtime_activation_frame(&registry, &[missing_a26]),
        Err(WindowManagerRejection::InvalidRuntimeState(
            sources[0].window_id.clone()
        ))
    );

    let mut missing_a32 = open_state(&sources[0].window_id, 0);
    missing_a32.focus_owner = Some("focus.widget".to_string());
    assert_eq!(
        apply_runtime_activation_frame(&registry, &[missing_a32]),
        Err(WindowManagerRejection::InvalidRuntimeState(
            sources[0].window_id.clone()
        ))
    );

    let mut invalid_modal = open_state(&sources[0].window_id, 0);
    invalid_modal.modal_owner = Some(sources[1].window_id.clone());
    assert_eq!(
        apply_runtime_activation_frame(&registry, &[invalid_modal]),
        Err(WindowManagerRejection::InvalidModalOwner(
            sources[0].window_id.clone()
        ))
    );
}

#[test]
fn replay_owned_manager_covers_every_admitted_window_dormant() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    assert_eq!(manager.panel_count(), XUI02_WINDOW_DEFINITIONS);
    assert!(manager.composed_stack().is_empty());
    assert!(manager.focus().is_none());
    assert!(manager.tooltip().is_none());
    assert_eq!(manager.input_window_id(), None);
    assert!(registry.windows.iter().all(|row| {
        manager.panel(&row.window_id).is_some_and(|state| {
            !state.open
                && !state.visible
                && !state.composed
                && state.stack_ordinal.is_none()
                && state.modal_owner.is_none()
                && state.focused_widget.is_none()
                && state.keyboard_traversal.is_empty()
                && state.controller_traversal.is_empty()
                && state.tooltip_targets.is_empty()
        })
    }));
}

#[test]
fn replay_open_close_and_bring_to_front_keep_contiguous_z_order() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let ids = registry
        .windows
        .iter()
        .take(3)
        .map(|row| row.window_id.clone())
        .collect::<Vec<_>>();
    let mut manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    manager.open(replay_activation(&ids[0], "a")).unwrap();
    manager.open(replay_activation(&ids[1], "b")).unwrap();
    manager.open(replay_activation(&ids[2], "c")).unwrap();
    assert_eq!(manager.composed_stack(), ids.as_slice());
    assert_eq!(manager.panel(&ids[0]).unwrap().stack_ordinal, Some(0));
    assert_eq!(manager.panel(&ids[1]).unwrap().stack_ordinal, Some(1));
    assert_eq!(manager.panel(&ids[2]).unwrap().stack_ordinal, Some(2));
    assert_eq!(manager.input_window_id(), Some(ids[2].as_str()));

    manager.bring_to_front(&ids[0]).unwrap();
    assert_eq!(
        manager.composed_stack(),
        &[ids[1].clone(), ids[2].clone(), ids[0].clone()]
    );
    assert_eq!(manager.panel(&ids[1]).unwrap().stack_ordinal, Some(0));
    assert_eq!(manager.panel(&ids[2]).unwrap().stack_ordinal, Some(1));
    assert_eq!(manager.panel(&ids[0]).unwrap().stack_ordinal, Some(2));
    assert_eq!(manager.focus().unwrap().widget_id, "a.first");

    manager.close(&ids[2]).unwrap();
    let closed = manager.panel(&ids[2]).unwrap();
    assert!(!closed.open && !closed.visible && !closed.composed);
    assert_eq!(closed.stack_ordinal, None);
    assert_eq!(manager.composed_stack(), &[ids[1].clone(), ids[0].clone()]);
    assert_eq!(manager.panel(&ids[0]).unwrap().stack_ordinal, Some(1));
}

#[test]
fn replay_modal_owns_input_and_close_restores_prior_focus() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let owner_id = registry.windows[0].window_id.clone();
    let modal_id = registry.windows[1].window_id.clone();
    let background_id = registry.windows[2].window_id.clone();
    let mut manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    manager.open(replay_activation(&owner_id, "owner")).unwrap();
    manager.focus_widget(&owner_id, "owner.second").unwrap();
    let mut modal = replay_activation(&modal_id, "modal");
    modal.modal_owner = Some(owner_id.clone());
    manager.open(modal).unwrap();
    assert_eq!(manager.input_window_id(), Some(modal_id.as_str()));
    assert_eq!(
        manager.panel(&modal_id).unwrap().modal_owner.as_deref(),
        Some(owner_id.as_str())
    );
    assert_eq!(
        manager.bring_to_front(&owner_id),
        Err(ReplayWindowManagerRejection::ModalInputBlocked(
            owner_id.clone()
        ))
    );
    assert_eq!(
        manager.open(replay_activation(&background_id, "background")),
        Err(ReplayWindowManagerRejection::ModalInputBlocked(
            background_id
        ))
    );
    assert_eq!(
        manager.close(&owner_id),
        Err(ReplayWindowManagerRejection::ModalInputBlocked(
            owner_id.clone()
        ))
    );

    manager.close(&modal_id).unwrap();
    assert_eq!(manager.input_window_id(), Some(owner_id.as_str()));
    assert_eq!(
        manager.focus(),
        Some(&ReplayFocusTarget {
            window_id: owner_id,
            widget_id: "owner.second".to_string(),
        })
    );
}

#[test]
fn replay_keyboard_and_controller_traversal_are_exact_and_wrap() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let window_id = registry.windows[0].window_id.clone();
    let mut manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    manager.open(replay_activation(&window_id, "nav")).unwrap();
    assert_eq!(manager.focus().unwrap().widget_id, "nav.first");
    assert_eq!(
        manager
            .traverse(
                ReplayTraversalDevice::Keyboard,
                ReplayTraversalDirection::Forward
            )
            .unwrap()
            .widget_id,
        "nav.second"
    );
    assert_eq!(
        manager
            .traverse(
                ReplayTraversalDevice::Keyboard,
                ReplayTraversalDirection::Backward
            )
            .unwrap()
            .widget_id,
        "nav.first"
    );
    assert_eq!(
        manager
            .traverse(
                ReplayTraversalDevice::Keyboard,
                ReplayTraversalDirection::Backward
            )
            .unwrap()
            .widget_id,
        "nav.third"
    );
    assert_eq!(
        manager
            .traverse(
                ReplayTraversalDevice::Controller,
                ReplayTraversalDirection::Forward
            )
            .unwrap()
            .widget_id,
        "nav.second"
    );
    assert_eq!(
        manager
            .traverse(
                ReplayTraversalDevice::Controller,
                ReplayTraversalDirection::Forward
            )
            .unwrap()
            .widget_id,
        "nav.first"
    );
}

#[test]
fn replay_tooltip_owner_is_input_scoped_and_cleared_by_transfer() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let first = registry.windows[0].window_id.clone();
    let second = registry.windows[1].window_id.clone();
    let mut manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    manager.open(replay_activation(&first, "tip_a")).unwrap();
    manager
        .set_tooltip_owner(&first, "tip_a.hover", "A26.tooltip_instances[0]")
        .unwrap();
    assert_eq!(manager.tooltip().unwrap().window_id, first);
    manager.focus_widget(&first, "tip_a.second").unwrap();
    assert!(manager.tooltip().is_none());
    manager
        .set_tooltip_owner(&first, "tip_a.hover", "A26.tooltip_instances[1]")
        .unwrap();
    manager.open(replay_activation(&second, "tip_b")).unwrap();
    assert!(manager.tooltip().is_none());
    assert_eq!(
        manager.set_tooltip_owner(&first, "stale", "A26.tooltip_instances[2]"),
        Err(ReplayWindowManagerRejection::InvalidTooltipOwner(first))
    );
    manager
        .set_tooltip_owner(&second, "tip_b.hover", "A26.tooltip_instances[3]")
        .unwrap();
    manager.close(&second).unwrap();
    assert!(manager.tooltip().is_none());
}

#[test]
fn replay_tooltip_owner_requires_an_explicit_panel_target() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let id = registry.windows[0].window_id.clone();
    let mut manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    manager.open(replay_activation(&id, "tip")).unwrap();
    assert_eq!(
        manager.set_tooltip_owner(&id, "tip.unadmitted", "A26.tooltip_instances[0]"),
        Err(ReplayWindowManagerRejection::InvalidTooltipOwner(
            id.clone()
        ))
    );
    manager
        .set_tooltip_owner(&id, "tip.hover", "A26.tooltip_instances[0]")
        .unwrap();
    assert_eq!(manager.tooltip().unwrap().widget_id, "tip.hover");
}

#[test]
fn replay_manager_rejects_unknown_duplicate_and_invalid_activation() {
    let catalog = catalog();
    let registry = register_dormant_window_manager(&catalog, &[]).expect("XUI02 registry");
    let id = registry.windows[0].window_id.clone();
    let mut manager = ReplayOwnedWindowManager::new(&registry).expect("replay manager");
    assert_eq!(
        manager.open(replay_activation("missing#0", "missing")),
        Err(ReplayWindowManagerRejection::UnknownWindow(
            "missing#0".to_string()
        ))
    );
    let mut invalid = replay_activation(&id, "bad");
    invalid.keyboard_traversal.push("bad.first".to_string());
    assert_eq!(
        manager.open(invalid),
        Err(ReplayWindowManagerRejection::InvalidTraversal(id.clone()))
    );
    manager.open(replay_activation(&id, "valid")).unwrap();
    assert_eq!(
        manager.open(replay_activation(&id, "again")),
        Err(ReplayWindowManagerRejection::AlreadyOpen(id.clone()))
    );
    manager.close(&id).unwrap();
    assert_eq!(
        manager.close(&id),
        Err(ReplayWindowManagerRejection::AlreadyClosed(id))
    );
}
