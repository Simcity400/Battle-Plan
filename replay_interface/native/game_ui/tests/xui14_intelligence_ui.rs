use game_ui::compose::Rect;
use game_ui::dynamic_collection::{
    CollectionItemTrace, CollectionTraceAuthority, DynamicCollectionTrace, MaterializedSlotTrace,
};
use game_ui::intelligence_ui::*;

fn collection(frame: u64, name: &str, ids: &[&str]) -> DynamicCollectionTrace {
    DynamicCollectionTrace {
        authority: CollectionTraceAuthority::ExactA729Runtime,
        collection_instance_id: name.into(),
        frame_hour: frame,
        source_items: ids
            .iter()
            .enumerate()
            .map(|(i, id)| CollectionItemTrace {
                stable_item_id: (*id).into(),
                source_ordinal: i as u32,
                included: true,
                filter_reason: None,
            })
            .collect(),
        ordered_item_ids: ids.iter().rev().map(|id| (*id).into()).collect(),
        group_boundaries: vec![],
        row_template_id: format!("{name}_row"),
        materialized_slots: ids
            .iter()
            .enumerate()
            .map(|(i, id)| MaterializedSlotTrace {
                slot_id: format!("{name}-slot-{i}"),
                bound_item_id: (*id).into(),
                row_template_id: format!("{name}_row"),
                rect: Rect::new(0, i as i32 * 20, 100, 20),
            })
            .collect(),
        clip_rect: Rect::new(0, 0, 100, 100),
        content_extent_pixels: (100, ids.len() as i32 * 20),
        viewport_extent_pixels: (100, 100),
        scroll_min_fixed: 0,
        scroll_max_fixed: FIXED_ONE,
        scroll_current_fixed: 0,
        scroll_target_fixed: 0,
    }
}
fn measured<T>(
    frame: u64,
    name: &str,
    rows: Vec<T>,
    ids: &[&str],
) -> ProducerValue<MeasuredList<T>> {
    ProducerValue::Measured(MeasuredList {
        rows,
        collection: collection(frame, name, ids),
    })
}
fn visible() -> Vec<VisibilityGate> {
    IntelligenceSurface::ALL
        .into_iter()
        .map(|surface| VisibilityGate {
            surface,
            disposition: VisibilityDisposition::Visible,
            reason: None,
        })
        .collect()
}

fn exact() -> IntelligenceUiTrace {
    let frame = 900;
    IntelligenceUiTrace {
        authority: IntelligenceTraceAuthority::ExactA729ExtractorController,
        frame_hour: frame,
        producer_record_identity: "a30:900:GER:GER:1".into(),
        pov_country: "GER".into(),
        owner_country: "GER".into(),
        agency: ProducerValue::Measured(AgencyState {
            agency_instance_id: "agency:GER".into(),
            owner_country: "GER".into(),
            name_text: "Abwehr".into(),
            icon_sprite: "GFX_intelligence_agency_logo_ger".into(),
            created: true,
            upgrade_progress_fixed: 25_000,
            current_upgrade_id: Some("upgrade_crypto".into()),
            max_operatives: 3,
            usable_slots: 2,
            defense_raw: 500_000,
            defense_denominator_raw: 100_000,
        }),
        upgrades: measured(
            frame,
            "upgrades",
            vec![AgencyUpgradeRow {
                stable_item_id: "upgrade:crypto".into(),
                upgrade_id: "upgrade_crypto".into(),
                branch_id: "cryptology".into(),
                level: 1,
                icon_sprite: "GFX_upgrade_crypto".into(),
            }],
            &["upgrade:crypto"],
        ),
        operatives: measured(
            frame,
            "operatives",
            vec![
                OperativeUiRow {
                    stable_item_id: "operative:7".into(),
                    operative_id: "7".into(),
                    owner_country: "GER".into(),
                    name_text: "Alpha".into(),
                    portrait_sprite: "GFX_portrait_alpha".into(),
                    skill: 2,
                    experience_fixed: 30_000,
                    traits: vec!["operative_linguist".into()],
                    nationalities: vec!["GER".into()],
                    captured: false,
                    captured_by: None,
                    recruitable_pool_member: false,
                    assignment: OperativeAssignment::Operation {
                        operation_id: "op:9".into(),
                    },
                },
                OperativeUiRow {
                    stable_item_id: "operative:8".into(),
                    operative_id: "8".into(),
                    owner_country: "GER".into(),
                    name_text: "Beta".into(),
                    portrait_sprite: "GFX_portrait_beta".into(),
                    skill: 1,
                    experience_fixed: 10_000,
                    traits: vec![],
                    nationalities: vec!["GER".into()],
                    captured: false,
                    captured_by: None,
                    recruitable_pool_member: false,
                    assignment: OperativeAssignment::Network {
                        network_id: "net:1".into(),
                    },
                },
            ],
            &["operative:7", "operative:8"],
        ),
        operations: measured(
            frame,
            "operations",
            vec![OperationUiRow {
                stable_item_id: "operation:9".into(),
                operation_id: "op:9".into(),
                definition_id: "operation_capture_cipher".into(),
                owner_country: "GER".into(),
                target: OperationTarget::Country("ENG".into()),
                status: OperationStatus::Running,
                phase_index: 0,
                phase_count: 3,
                progress_fixed: 40_000,
                start_hour: 850,
                duration_hours: 720,
                assigned_operative_ids: vec!["7".into()],
            }],
            &["operation:9"],
        ),
        ciphers: measured(
            frame,
            "ciphers",
            vec![CipherUiRow {
                stable_item_id: "cipher:ENG".into(),
                owner_country: "GER".into(),
                target_country: "ENG".into(),
                decryption_points_raw: 960_000,
                progress_denominator_raw: 1_600_000,
                progress_fixed: 60_000,
                decrypting: true,
                cipher_broken: false,
            }],
            &["cipher:ENG"],
        ),
        intel_levels: measured(
            frame,
            "intel",
            vec![IntelLevelUiRow {
                stable_item_id: "intel:ENG".into(),
                viewer_country: "GER".into(),
                target_country: "ENG".into(),
                civilian_fixed: 20_000,
                army_fixed: 30_000,
                navy_fixed: 40_000,
                air_fixed: 50_000,
            }],
            &["intel:ENG"],
        ),
        networks: measured(
            frame,
            "networks",
            vec![IntelNetworkUiRow {
                stable_item_id: "network:1".into(),
                network_id: "net:1".into(),
                owner_country: "GER".into(),
                target_country: "ENG".into(),
                operative_ids: vec!["8".into()],
                state_ids: vec![1, 4, 9],
                strength_fixed: 75_000,
                national_coverage_fixed: 20_000,
            }],
            &["network:1"],
        ),
        visibility: visible(),
    }
}

#[test]
fn exact_trace_admits_and_preserves_controller_order() {
    let trace = exact();
    let admitted = admit_intelligence_ui(&trace).expect("exact XUI14 trace");
    assert_eq!(
        admitted
            .ordered_operatives()
            .unwrap()
            .iter()
            .map(|r| r.operative_id.as_str())
            .collect::<Vec<_>>(),
        vec!["8", "7"]
    );
    assert_eq!(
        admitted.ordered_operations().unwrap()[0].operation_id,
        "op:9"
    );
    assert_eq!(
        admitted.visibility(IntelligenceSurface::Networks),
        VisibilityDisposition::Visible
    );
}

#[test]
fn collection_order_or_cross_reference_drift_fails_closed() {
    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.operatives {
        v.collection.ordered_item_ids.pop();
    }
    assert!(
        admit_intelligence_ui(&trace)
            .unwrap_err()
            .message
            .contains("XUI14_FAIL_CLOSED")
            || admit_intelligence_ui(&trace)
                .unwrap_err()
                .message
                .contains("XUI03_FAIL_CLOSED")
    );
    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.operations {
        v.rows[0].assigned_operative_ids[0] = "missing".into();
    }
    assert!(admit_intelligence_ui(&trace)
        .unwrap_err()
        .message
        .contains("XUI14_FAIL_CLOSED"));
}

#[test]
fn network_and_visibility_gates_reject_unproven_state() {
    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.networks {
        v.rows[0].state_ids = vec![4, 1];
    }
    assert!(admit_intelligence_ui(&trace)
        .unwrap_err()
        .message
        .contains("XUI14_FAIL_CLOSED"));
    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.networks {
        v.rows[0].operative_ids.push("7".into());
    }
    assert!(admit_intelligence_ui(&trace)
        .unwrap_err()
        .message
        .contains("XUI14_FAIL_CLOSED"));
    let mut trace = exact();
    trace
        .visibility
        .retain(|g| g.surface != IntelligenceSurface::Ciphers);
    assert!(admit_intelligence_ui(&trace)
        .unwrap_err()
        .message
        .contains("XUI14_FAIL_CLOSED"));
}

#[test]
fn invalid_operation_target_fails_closed() {
    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.operations {
        v.rows[0].target = OperationTarget::State(0);
    }
    assert!(admit_intelligence_ui(&trace)
        .unwrap_err()
        .message
        .contains("XUI14_FAIL_CLOSED"));

    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.operatives {
        v.rows[0].experience_fixed = FIXED_ONE + 1;
    }
    assert!(admit_intelligence_ui(&trace)
        .unwrap_err()
        .message
        .contains("XUI14_FAIL_CLOSED"));
}

#[test]
fn unobtained_family_is_explicit_and_never_fabricated() {
    let mut trace = exact();
    trace.intel_levels = ProducerValue::Unobtained {
        reason: "owner-target intel percentages unavailable".into(),
    };
    let gate = trace
        .visibility
        .iter_mut()
        .find(|g| g.surface == IntelligenceSurface::IntelLedger)
        .unwrap();
    gate.disposition = VisibilityDisposition::Unavailable;
    gate.reason = Some("missing exact extractor carrier".into());
    let admitted =
        admit_intelligence_ui(&trace).expect("explicit unavailable family is admitted fail-closed");
    assert!(admitted.ordered_intel_levels().unwrap().is_empty());
    assert_eq!(
        admitted.visibility(IntelligenceSurface::IntelLedger),
        VisibilityDisposition::Unavailable
    );
}

#[test]
fn producer_identity_cipher_denominator_and_lifecycle_are_required() {
    let mut trace = exact();
    trace.producer_record_identity.clear();
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.ciphers {
        v.rows[0].progress_denominator_raw = 0;
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.operatives {
        v.rows[0].captured = true;
        v.rows[0].captured_by = Some("ENG".into());
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(v) = &mut trace.operatives {
        v.rows[0].recruitable_pool_member = true;
    }
    assert!(admit_intelligence_ui(&trace).is_err());
}

#[test]
fn a30_gh_01_agency_acceptance_gate_covers_every_semantic_leaf() {
    let mut trace = exact();
    if let ProducerValue::Measured(agency) = &mut trace.agency {
        agency.defense_denominator_raw = 0;
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.upgrades {
        rows.rows[0].branch_id.clear();
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(agency) = &mut trace.agency {
        agency.current_upgrade_id = Some("unjoined-upgrade".into());
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(agency) = &mut trace.agency {
        agency.usable_slots = agency.max_operatives + 1;
    }
    assert!(admit_intelligence_ui(&trace).is_err());
}

#[test]
fn a30_gh_02_operative_acceptance_gate_covers_lifecycle_and_identity() {
    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operatives {
        rows.rows[0].portrait_sprite.clear();
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operatives {
        rows.rows[0].traits.push("operative_linguist".into());
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operatives {
        rows.rows[0].captured = true;
        rows.rows[0].captured_by = Some("ENG".into());
        rows.rows[0].assignment = OperativeAssignment::Captured {
            captured_by: "USA".into(),
        };
    }
    assert!(admit_intelligence_ui(&trace).is_err());
}

#[test]
fn a30_gh_03_operation_acceptance_gate_covers_state_and_reciprocal_joins() {
    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operations {
        rows.rows[0].definition_id.clear();
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operations {
        rows.rows[0].phase_index = rows.rows[0].phase_count;
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operations {
        rows.rows[0].assigned_operative_ids.clear();
    }
    assert!(admit_intelligence_ui(&trace).is_err());
}

#[test]
fn a30_gh_04_cipher_intel_network_acceptance_gate_covers_exact_scales() {
    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.ciphers {
        rows.rows[0].decryption_points_raw = -1;
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.intel_levels {
        rows.rows[0].air_fixed = FIXED_ONE + 1;
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.networks {
        rows.rows[0].national_coverage_fixed = -1;
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    trace.pov_country = "SOV".into();
    if let ProducerValue::Measured(rows) = &mut trace.intel_levels {
        rows.rows[0].viewer_country = "SOV".into();
    }
    assert!(admit_intelligence_ui(&trace).is_ok());

    let mut trace = exact();
    trace.pov_country = "SOV".into();
    assert!(admit_intelligence_ui(&trace).is_err());
}

#[test]
fn a30_gh_05_controller_order_acceptance_gate_covers_visibility_and_materialization() {
    let mut trace = exact();
    trace.visibility[0].disposition = VisibilityDisposition::Hidden;
    trace.visibility[0].reason = None;
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operatives {
        rows.collection.materialized_slots[0].bound_item_id = "missing".into();
    }
    assert!(admit_intelligence_ui(&trace).is_err());

    let mut trace = exact();
    if let ProducerValue::Measured(rows) = &mut trace.operations {
        rows.collection.frame_hour += 1;
    }
    assert!(admit_intelligence_ui(&trace).is_err());
}

#[test]
fn panel_rendering_is_typed_and_uses_controller_order() {
    let trace = exact();
    let admitted = admit_intelligence_ui(&trace).expect("exact XUI14 trace");
    let panels = admitted.render_panels().expect("typed panel rendering");
    assert_eq!(
        panels.iter().map(|panel| panel.surface).collect::<Vec<_>>(),
        IntelligenceSurface::ALL
    );
    assert!(matches!(
        panels[0].content,
        Some(IntelligencePanelContent::Agency(_))
    ));
    match &panels[2].content {
        Some(IntelligencePanelContent::Operatives(rows)) => assert_eq!(
            rows.iter()
                .map(|row| row.operative_id.as_str())
                .collect::<Vec<_>>(),
            vec!["8", "7"]
        ),
        other => panic!("unexpected operative panel: {other:?}"),
    }
    assert!(panels
        .iter()
        .all(|panel| panel.disposition == VisibilityDisposition::Visible
            && panel.gate_reason.is_none()
            && panel.producer_reason.is_none()
            && panel.content.is_some()));
}

#[test]
fn hidden_panel_preserves_reason_and_cannot_leak_measured_rows() {
    let mut trace = exact();
    let gate = trace
        .visibility
        .iter_mut()
        .find(|gate| gate.surface == IntelligenceSurface::Ciphers)
        .unwrap();
    gate.disposition = VisibilityDisposition::Hidden;
    gate.reason = Some("controller selected another tab".into());
    let admitted = admit_intelligence_ui(&trace).expect("hidden measured surface");
    let panel = admitted
        .render_panels()
        .unwrap()
        .into_iter()
        .find(|panel| panel.surface == IntelligenceSurface::Ciphers)
        .unwrap();
    assert_eq!(panel.disposition, VisibilityDisposition::Hidden);
    assert_eq!(panel.gate_reason, Some("controller selected another tab"));
    assert_eq!(panel.producer_reason, None);
    assert_eq!(panel.content, None);
}

#[test]
fn unavailable_and_measured_empty_panels_remain_distinct() {
    let mut unavailable = exact();
    unavailable.intel_levels = ProducerValue::Unobtained {
        reason: "owner-target intel carrier absent".into(),
    };
    let gate = unavailable
        .visibility
        .iter_mut()
        .find(|gate| gate.surface == IntelligenceSurface::IntelLedger)
        .unwrap();
    gate.disposition = VisibilityDisposition::Unavailable;
    gate.reason = Some("external extractor dependency".into());
    let admitted = admit_intelligence_ui(&unavailable).expect("explicit unavailable intel");
    let panel = admitted
        .render_panels()
        .unwrap()
        .into_iter()
        .find(|panel| panel.surface == IntelligenceSurface::IntelLedger)
        .unwrap();
    assert_eq!(panel.disposition, VisibilityDisposition::Unavailable);
    assert_eq!(panel.gate_reason, Some("external extractor dependency"));
    assert_eq!(
        panel.producer_reason,
        Some("owner-target intel carrier absent")
    );
    assert_eq!(panel.content, None);

    let mut empty = exact();
    empty.intel_levels = measured(900, "intel", Vec::<IntelLevelUiRow>::new(), &[]);
    let admitted = admit_intelligence_ui(&empty).expect("measured-empty intel");
    let panel = admitted
        .render_panels()
        .unwrap()
        .into_iter()
        .find(|panel| panel.surface == IntelligenceSurface::IntelLedger)
        .unwrap();
    assert_eq!(panel.disposition, VisibilityDisposition::Visible);
    assert_eq!(panel.producer_reason, None);
    assert!(matches!(
        panel.content,
        Some(IntelligencePanelContent::IntelLedger(ref rows)) if rows.is_empty()
    ));
}
