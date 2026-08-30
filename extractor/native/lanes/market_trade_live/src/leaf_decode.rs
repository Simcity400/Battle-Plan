// @leaf-source-authority crate=d43_grammar root=extractor/native/d43_grammar files=8 sha256=1c09d4e7cfb0eb8d9f6f15f471b36197031c998521e6a715747308ea59134748
// @d43-payload-reader-region begin
use super::*;

pub(super) const LANE_DECODE_CONTEXT: d43_grammar::decode::DecodeContext =
    d43_grammar::decode::DecodeContext::new(
        d43_grammar::decode::D43AcceptedFamilies::LANE_ALL,
        MAX_STRATEGIC_RESOURCE_COUNT,
        STRATEGIC_RESOURCE_IDENTITY_STRIDE,
        MARKET_TRADE_MAX_TAG_DB_ID,
        MAX_STRATEGIC_RESOURCE_TOKEN_LEN,
        MARKET_TRADE_MAX_CONTRACT_ID,
        MARKET_TRADE_MAX_VARIANT_ID,
        compact_contract::build_profile::MAX_MARKET_TRADE_CLASSIC_DEAL_ORDINAL,
        MARKET_TRADE_MAX_AMOUNT_RAW,
    );

pub(super) fn project_decoded_payload(
    offset: usize,
    report: &d43_grammar::decode::DecodeReport,
) -> MarketTradePayloadSummary {
    let mut summary = project_document(report);
    for finding in &report.findings {
        if !finding_was_reached_by_lane(report, finding) {
            continue;
        }
        if let Some(message) = format_finding(report, finding) {
            push_unique(
                &mut summary.parse_errors,
                format!("offset {offset}: {message}"),
            );
        }
    }
    summary
}

fn project_document(report: &d43_grammar::decode::DecodeReport) -> MarketTradePayloadSummary {
    use d43_grammar::{CoreWord, DetailFlag, FlowKind, FlowVersion, HeaderPreview};

    let mut summary = MarketTradePayloadSummary::default();
    if let Some(header) = report.document.core_header {
        summary.elapsed_wall_ms = header.elapsed_wall_ms;
        summary.row_count = header.word(CoreWord::RowCount);
        summary.changed_rows = header.word(CoreWord::ChangedRows);
        summary.status_flags = header.word(CoreWord::StatusFlags);
        summary.observed_contract_rows = header.word(CoreWord::ObservedContractRows);
        summary.emitted_contract_rows = header.word(CoreWord::EmittedContractRows);
        summary.omitted_candidate_rows = summary
            .observed_contract_rows
            .saturating_sub(summary.emitted_contract_rows);
        summary.buyer_ptr_valid_rows = header.word(CoreWord::BuyerPtrValidRows);
        summary.variant_valid_rows = header.word(CoreWord::VariantValidRows);
        summary.amount_valid_rows = header.word(CoreWord::AmountValidRows);
        summary.partial_field_rows = header.word(CoreWord::PartialFieldRows);
        summary.scale_raw = header.word(CoreWord::ScaleRaw);
    }
    summary.rows = report
        .document
        .core_rows
        .iter()
        .map(|item| MarketTradeRow {
            contract_id: item.row.contract_id,
            contract_type: item.row.contract_type,
            flags: item.row.flags,
            seller_tag_db_id: item.row.seller_tag_db_id,
            buyer_tag_db_id: item.row.buyer_tag_db_id,
            equipment_variant_id: item.row.equipment_variant_id,
            amount_raw: item.row.amount_raw,
            equipment_subbuf_status: item.row.equipment_subbuf_status,
            delivery_subbuf_status: item.row.delivery_subbuf_status,
            buyer_ptr_status: item.row.buyer_ptr_status,
        })
        .collect();

    summary.residual_trailer_present = family_was_reached(report, d43_grammar::Trailer::Residual);
    if summary.residual_trailer_present {
        if let Some(header) = report.document.residual_header {
            summary.residual_badge_rows = header.sized.row_count as u32;
            summary.residual_source_blocked_rows = header.source_blocked_rows;
            summary.residual_schema_blocked_rows = header.schema_blocked_rows;
            summary.residual_owned_elsewhere_rows = header.owned_elsewhere_rows;
            summary.residual_shared_patch_blocked_rows = header.shared_patch_blocked_rows;
        } else if let Some(HeaderPreview::Residual(words)) =
            family_header_preview(report, d43_grammar::Trailer::Residual)
        {
            summary.residual_badge_rows = words.sized.row_count;
            summary.residual_source_blocked_rows = words.source_blocked_rows;
            summary.residual_schema_blocked_rows = words.schema_blocked_rows;
            summary.residual_owned_elsewhere_rows = words.owned_elsewhere_rows;
            summary.residual_shared_patch_blocked_rows = words.shared_patch_blocked_rows;
        }
    }
    if family_body_was_admitted(report, d43_grammar::Trailer::Residual) {
        summary.residual_badges = report
            .document
            .residual_rows
            .iter()
            .map(|item| MarketTradeResidualBadgeRow {
                gap_id: item.row.gap_id,
                status: item.row.status,
                evidence_mask: item.row.evidence_mask,
                current_support: item.row.current_support,
                current_domain_id: item.row.current_domain_id,
                proposed_record_kind: item.row.proposed_record_kind,
                current_row_len: item.row.current_row_len,
                source_ref_id: item.row.source_ref_id,
                owner_boundary: item.row.owner_boundary,
                root_status: item.row.root_status,
                proof_status: item.row.proof_status,
            })
            .collect();
    }

    summary.detail_trailer_present = family_was_reached(report, d43_grammar::Trailer::Detail);
    if summary.detail_trailer_present {
        if let Some(header) = report.document.detail_header {
            summary.detail_rows = header.sized.row_count as u32;
            summary.detail_price_valid_rows = header.price_valid_rows;
            summary.detail_speed_valid_rows = header.speed_valid_rows;
            summary.detail_delivery_route_valid_rows = header.delivery_route_valid_rows;
            summary.detail_progress_valid_rows = header.progress_valid_rows;
        } else if let Some(HeaderPreview::Detail(words)) =
            family_header_preview(report, d43_grammar::Trailer::Detail)
        {
            summary.detail_rows = words.sized.row_count;
            summary.detail_price_valid_rows = words.price_valid_rows;
            summary.detail_speed_valid_rows = words.speed_valid_rows;
            summary.detail_delivery_route_valid_rows = words.delivery_route_valid_rows;
            summary.detail_progress_valid_rows = words.progress_valid_rows;
        }
    }
    let detail_rows = if family_body_was_admitted(report, d43_grammar::Trailer::Detail) {
        report.document.detail_rows.as_slice()
    } else {
        &[]
    };
    for item in detail_rows {
        let row = item.row;
        summary.detail_completed_valid_rows = summary
            .detail_completed_valid_rows
            .saturating_add(u32::from(DetailFlag::CompletedValid.is_set(row.flags)));
        summary.detail_days_valid_rows = summary
            .detail_days_valid_rows
            .saturating_add(u32::from(DetailFlag::DaysValid.is_set(row.flags)));
        summary.detail_delivery_country_valid_rows = summary
            .detail_delivery_country_valid_rows
            .saturating_add(u32::from(
                DetailFlag::DeliveryCountryValid.is_set(row.flags),
            ));
        summary.detail_convoys_valid_rows = summary
            .detail_convoys_valid_rows
            .saturating_add(u32::from(DetailFlag::ConvoysValid.is_set(row.flags)));
        summary.detail_efficiency_valid_rows = summary
            .detail_efficiency_valid_rows
            .saturating_add(u32::from(DetailFlag::EfficiencyValid.is_set(row.flags)));
        summary.detail_contracts.push(PurchaseContractDetailRow {
            contract_id: row.contract_id,
            flags: row.flags,
            price_raw: row.price_raw,
            amount_raw: row.amount_raw,
            speed_raw: row.speed_raw,
            completed_raw: row.completed_raw,
            days: row.days,
            delivery_country_tag_db_id: row.delivery_country_tag_db_id,
            convoys_assigned: row.convoys_assigned,
            convoys_total: row.convoys_total,
            efficiency_raw: row.efficiency_raw,
            factory_cic_progress_raw: row.factory_cic_progress_raw,
            equipment_transfer_progress_raw: row.equipment_transfer_progress_raw,
        });
    }

    summary.lend_lease_trailer_present =
        family_was_reached(report, d43_grammar::Trailer::LendLease);
    if summary.lend_lease_trailer_present {
        if let Some(header) = report.document.lend_lease_header {
            summary.lend_lease_mapping_rows = header.sized.row_count as u32;
            summary.lend_lease_outgoing_rows = header.outgoing_rows;
            summary.lend_lease_incoming_rows = header.incoming_rows;
            summary.lend_lease_amount_valid_rows = header.amount_valid_rows;
            summary.lend_lease_price_valid_rows = header.price_valid_rows;
        } else if let Some(HeaderPreview::LendLease(words)) =
            family_header_preview(report, d43_grammar::Trailer::LendLease)
        {
            summary.lend_lease_mapping_rows = words.sized.row_count;
            summary.lend_lease_outgoing_rows = words.outgoing_rows;
            summary.lend_lease_incoming_rows = words.incoming_rows;
            summary.lend_lease_amount_valid_rows = words.amount_valid_rows;
            summary.lend_lease_price_valid_rows = words.price_valid_rows;
        }
    }
    if family_body_was_admitted(report, d43_grammar::Trailer::LendLease) {
        summary.lend_lease_mappings = report
            .document
            .lend_lease_rows
            .iter()
            .map(|item| LendLeaseMappingRow {
                contract_id: item.row.contract_id,
                perspective_tag_db_id: item.row.perspective_tag_db_id,
                counterpart_tag_db_id: item.row.counterpart_tag_db_id,
                direction: item.row.direction,
                equipment_variant_id: item.row.equipment_variant_id,
                flags: item.row.flags,
                amount_raw: item.row.amount_raw,
                price_raw: item.row.price_raw,
                source_record_kind: item.row.source_record_kind,
            })
            .collect();
    }

    summary.resource_flow_trailer_present =
        family_was_reached(report, d43_grammar::Trailer::ResourceFlow);
    if summary.resource_flow_trailer_present {
        if let Some(header) = report.document.flow_header {
            summary.resource_flow_rows = header.row_count as u32;
            summary.resource_flow_country_rows = if header.version == FlowVersion::Legacy {
                0
            } else {
                header.country_count_or_exported as u32
            };
            summary.resource_flow_scale_raw = header.scale_raw;
            summary.resource_flow_resource_mask = header.mask_or_count;
            summary.resource_flow_resource_count = header.resource_count;
            summary.resource_flow_kind_mask = header.kind_mask;
        } else if let Some(HeaderPreview::ResourceFlow(words)) =
            family_header_preview(report, d43_grammar::Trailer::ResourceFlow)
        {
            summary.resource_flow_rows = words.sized.row_count;
            summary.resource_flow_country_rows = if words.sized.prefix.version == 1 {
                0
            } else {
                words.country_count_or_exported
            };
            summary.resource_flow_scale_raw = words.scale_raw;
            summary.resource_flow_resource_mask = words.mask_or_count;
            summary.resource_flow_kind_mask = words.kind_mask;
            summary.resource_flow_resource_count = match words.sized.prefix.version {
                1 | 2 => MARKET_TRADE_FIXED7_RESOURCE_COUNT,
                3 if (1..=MAX_STRATEGIC_RESOURCE_COUNT).contains(&words.mask_or_count) => {
                    words.mask_or_count
                }
                _ => 0,
            };
        }
    }
    let flow_rows = if family_body_was_admitted(report, d43_grammar::Trailer::ResourceFlow) {
        report.document.flow_rows.as_slice()
    } else {
        &[]
    };
    for item in flow_rows {
        if item.row.flow_kind == FlowKind::Exported.wire() {
            summary.resource_flow_exported_rows =
                summary.resource_flow_exported_rows.saturating_add(1);
        }
        summary.resource_flow_rows_decoded.push(ResourceFlowRow {
            tag_db_id: item.row.tag_db_id,
            flow_kind: item.row.flow_kind,
            resource_mask_or_count: item.row.mask_or_count,
            values_raw: report.document.flow_values[item.values.clone()].to_vec(),
        });
    }

    summary.factory_header_trailer_present =
        family_was_reached(report, d43_grammar::Trailer::FactoryHeader);
    if summary.factory_header_trailer_present {
        if let Some(header) = report.document.factory_header {
            summary.factory_header_rows = header.sized.row_count as u32;
        } else if let Some(HeaderPreview::FactoryHeader(words)) =
            family_header_preview(report, d43_grammar::Trailer::FactoryHeader)
        {
            summary.factory_header_rows = words.sized.row_count;
        }
    }
    if family_body_was_admitted(report, d43_grammar::Trailer::FactoryHeader) {
        summary.factory_headers = report
            .document
            .factory_rows
            .iter()
            .map(|item| FactoryHeaderRow {
                tag_db_id: item.row.tag_db_id,
                total: item.row.total,
                used: item.row.used,
                available: item.row.available,
            })
            .collect();
    }

    summary.resource_row_trailer_present =
        family_was_reached(report, d43_grammar::Trailer::ResourceRow);
    if summary.resource_row_trailer_present {
        if let Some(header) = report.document.resource_header {
            summary.resource_rows = header.row_count as u32;
            summary.resource_row_country_count = header.country_count as u32;
            summary.resource_row_resource_count = header.resource_count;
        } else if let Some(HeaderPreview::ResourceRow(words)) =
            family_header_preview(report, d43_grammar::Trailer::ResourceRow)
        {
            summary.resource_rows = words.sized.row_count;
            summary.resource_row_country_count = words.country_count;
            summary.resource_row_resource_count = words.resource_count;
        }
    }
    if family_body_was_admitted(report, d43_grammar::Trailer::ResourceRow) {
        summary.resource_tokens = report
            .document
            .resource_identities
            .iter()
            .filter(|identity| identity_was_admitted(report, identity.index))
            .filter_map(d43_grammar::decode::identity_token)
            .map(str::to_owned)
            .collect();
        summary.resource_view_rows = report
            .document
            .resource_rows
            .iter()
            .map(|item| ResourceViewRow {
                tag_db_id: item.row.tag_db_id,
                resource_index: item.row.resource_index,
                visible_order: item.row.visible_order,
                needed_type: item.row.needed_type,
                balance: item.row.balance,
                produced: item.row.produced,
                imported: item.row.imported,
                exported: item.row.exported,
                special_project: item.row.special_project,
                production_line: item.row.production_line,
                needed: item.row.needed,
                needed_current: item.row.needed_current,
                needed_max: item.row.needed_max,
                flags: item.row.flags,
                descriptor_db_index: item.row.descriptor_db_index,
            })
            .collect();
    }

    summary.classic_deal_trailer_present =
        family_was_reached(report, d43_grammar::Trailer::ClassicDeal);
    if summary.classic_deal_trailer_present {
        if let Some(header) = report.document.deal_header {
            summary.classic_deal_rows = header.row_count as u32;
            summary.classic_deal_country_count = header.country_count as u32;
            summary.classic_deal_incomplete_owner_count = header.incomplete_count as u32;
            if family_body_was_admitted(report, d43_grammar::Trailer::ClassicDeal) {
                summary.classic_deal_country_witnesses = report
                    .document
                    .deal_witnesses
                    .iter()
                    .map(|item| item.row)
                    .collect();
                summary.classic_deal_incomplete_owners = report
                    .document
                    .deal_incomplete_owners
                    .iter()
                    .map(|item| item.row)
                    .collect();
                summary.classic_deals = report
                    .document
                    .deal_rows
                    .iter()
                    .map(|item| ClassicDealRow {
                        owner_tag_db_id: item.row.owner_tag_db_id,
                        counterpart_tag_db_id: item.row.counterpart_tag_db_id,
                        resource_index: item.row.resource_index,
                        visible_order: item.row.visible_order,
                        direction: item.row.direction,
                        route_kind: item.row.route_kind,
                        flags: item.row.flags,
                        route_status_flags: (header.version != d43_grammar::DealVersion::Legacy)
                            .then_some(item.row.route_status_or_reserved),
                        identity: ClassicDealIdentity {
                            exporter_tag_db_id: item.row.identity_exporter_tag_db_id,
                            importer_tag_db_id: item.row.identity_importer_tag_db_id,
                            resource_index: item.row.resource_index,
                            ordinal: item.row.identity_ordinal,
                        },
                        requested_raw: item.row.requested_raw,
                        delivered_raw: item.row.delivered_raw,
                        available_export_raw: item.row.available_export_raw,
                        factory_cost_raw: item.row.factory_cost_raw,
                        efficiency_raw: item.row.efficiency_raw,
                        civilian_factories: item.row.civilian_factories,
                        convoys_assigned: item.row.convoys_assigned,
                        convoys_required: item.row.convoys_required,
                    })
                    .collect();
            }
        } else if let Some(HeaderPreview::ClassicDeal(words)) =
            family_header_preview(report, d43_grammar::Trailer::ClassicDeal)
        {
            summary.classic_deal_rows = words.sized.row_count;
            summary.classic_deal_country_count = words.country_count;
            summary.classic_deal_incomplete_owner_count = if matches!(
                words.sized.prefix.version,
                MARKET_TRADE_CLASSIC_DEAL_PER_OWNER_SCHEMA_VERSION
                    | MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION
            ) {
                words.incomplete_count
            } else {
                0
            };
        }
    }

    summary.activation_trailer_present =
        family_was_reached(report, d43_grammar::Trailer::Activation);
    if family_body_was_admitted(report, d43_grammar::Trailer::Activation) {
        if let Some(header) = report.document.activation_header {
            let active = header.flags & MARKET_TRADE_ACTIVATION_FLAG_ACTIVE != 0;
            summary.activation = Some(MarketTradeActivation {
                active,
                country_tag_db_id: active.then_some(header.country_tag_db_id),
                observation_sequence: header.observation_sequence,
            });
        }
    }

    summary.stockpile_trailer_present = family_was_reached(report, d43_grammar::Trailer::Stockpile);
    if summary.stockpile_trailer_present {
        if let Some(header) = report.document.stockpile_header {
            summary.stockpile_country_rows = header.country_count as u32;
            summary.stockpile_item_rows = header.item_count as u32;
            summary.stockpile_expected_country_count = header.expected_country_count as u32;
        } else if let Some(HeaderPreview::Stockpile(words)) =
            family_header_preview(report, d43_grammar::Trailer::Stockpile)
        {
            summary.stockpile_country_rows = words.country_count;
            summary.stockpile_item_rows = words.item_count;
            summary.stockpile_expected_country_count = words.expected_country_count;
        }
    }
    if family_body_was_admitted(report, d43_grammar::Trailer::Stockpile) {
        summary.stockpile_countries = report
            .document
            .stockpile_countries
            .iter()
            .map(|item| MarketStockpileCountryRow {
                tag_db_id: item.row.tag_db_id,
                item_first: item.row.item_first as u32,
                item_count: item.row.item_count as u32,
            })
            .collect();
    }
    if stockpile_items_were_admitted(report) {
        summary.stockpile_items = report
            .document
            .stockpile_items
            .iter()
            .map(|item| MarketStockpileItemRow {
                equipment_variant_id: item.row.equipment_variant_id,
                equipment_type: item.row.variant_type,
                ordinal: item.row.ordinal,
                amount_raw: item.row.amount_raw,
            })
            .collect();
    }
    summary
}

fn family_start(
    report: &d43_grammar::decode::DecodeReport,
    family: d43_grammar::Trailer,
) -> Option<usize> {
    use d43_grammar::Trailer;
    let decoded = match family {
        Trailer::Detail => report.document.detail_header.map(|header| header.sized.at),
        Trailer::ResourceFlow => report.document.flow_header.map(|header| header.at),
        Trailer::FactoryHeader => report.document.factory_header.map(|header| header.sized.at),
        Trailer::ResourceRow => report.document.resource_header.map(|header| header.at),
        Trailer::ClassicDeal => report.document.deal_header.map(|header| header.at),
        Trailer::Activation => report.document.activation_header.map(|header| header.at),
        Trailer::Stockpile => report.document.stockpile_header.map(|header| header.at),
        Trailer::Residual => report
            .document
            .residual_header
            .map(|header| header.sized.at),
        Trailer::LendLease => report
            .document
            .lend_lease_header
            .map(|header| header.sized.at),
    };
    decoded.or_else(|| {
        report
            .findings
            .iter()
            .filter(|finding| finding.family == Some(family))
            .filter_map(|finding| finding.header_preview.map(|preview| preview.prefix().at))
            .min()
    })
}

fn family_header_preview(
    report: &d43_grammar::decode::DecodeReport,
    family: d43_grammar::Trailer,
) -> Option<d43_grammar::HeaderPreview> {
    report
        .findings
        .iter()
        .filter(|finding| finding.family == Some(family))
        .find_map(|finding| finding.header_preview)
}

fn identity_was_admitted(report: &d43_grammar::decode::DecodeReport, index: usize) -> bool {
    use d43_grammar::decode::D43DecodeRule;
    !report.findings.iter().any(|finding| {
        finding.family == Some(d43_grammar::Trailer::ResourceRow)
            && finding.row_or_slot == Some(index)
            && matches!(
                finding.rule,
                D43DecodeRule::D43IdentityLength
                    | D43DecodeRule::D43IdentityPadding
                    | D43DecodeRule::D43IdentityUtf8
                    | D43DecodeRule::D43IdentityCharset
            )
    })
}

fn first_header_refusal_start(report: &d43_grammar::decode::DecodeReport) -> Option<usize> {
    use d43_grammar::decode::{D43DecodeBlock, D43RuleFatality};
    report
        .findings
        .iter()
        .filter(|finding| {
            finding.fatality == D43RuleFatality::Fatal
                && finding.block == D43DecodeBlock::D43TrailerHeader
                && format_finding(report, finding).is_some()
        })
        .filter_map(|finding| {
            finding
                .family
                .and_then(|family| family_start(report, family))
        })
        .min()
}

fn family_was_reached(
    report: &d43_grammar::decode::DecodeReport,
    family: d43_grammar::Trailer,
) -> bool {
    let Some(start) = family_start(report, family) else {
        return report
            .findings
            .iter()
            .any(|finding| finding.family == Some(family));
    };
    first_header_refusal_start(report).is_none_or(|cutoff| start <= cutoff)
}

fn family_body_was_admitted(
    report: &d43_grammar::decode::DecodeReport,
    family: d43_grammar::Trailer,
) -> bool {
    let Some(start) = family_start(report, family) else {
        return false;
    };
    first_header_refusal_start(report).is_none_or(|cutoff| start < cutoff)
}

fn stockpile_has_country_finding(report: &d43_grammar::decode::DecodeReport) -> bool {
    report.findings.iter().any(|finding| {
        finding.family == Some(d43_grammar::Trailer::Stockpile)
            && finding.block == d43_grammar::decode::D43DecodeBlock::D43Countries
    })
}

fn stockpile_items_were_admitted(report: &d43_grammar::decode::DecodeReport) -> bool {
    family_body_was_admitted(report, d43_grammar::Trailer::Stockpile)
        && !stockpile_has_country_finding(report)
}

fn finding_was_reached_by_lane(
    report: &d43_grammar::decode::DecodeReport,
    finding: &d43_grammar::decode::Finding,
) -> bool {
    use d43_grammar::decode::D43DecodeBlock;
    if finding.family == Some(d43_grammar::Trailer::Stockpile)
        && finding.block == D43DecodeBlock::D43Items
        && stockpile_has_country_finding(report)
    {
        return false;
    }
    let (Some(family), Some(cutoff)) = (finding.family, first_header_refusal_start(report)) else {
        return true;
    };
    let Some(start) = family_start(report, family) else {
        return true;
    };
    start < cutoff
        || (start == cutoff
            && matches!(
                finding.block,
                D43DecodeBlock::D43TrailerMagic | D43DecodeBlock::D43TrailerHeader
            ))
}

fn format_finding(
    report: &d43_grammar::decode::DecodeReport,
    finding: &d43_grammar::decode::Finding,
) -> Option<String> {
    use d43_grammar::decode::{D43DecodeRule as Rule, D43LaneRule as LaneRule};
    let row = finding.row_or_slot.unwrap_or(0);
    if let Some(rule) = finding.lane_rule {
        return match rule {
            LaneRule::D43LaneResidualHeaderCountSum
            | LaneRule::D43LaneLendLeaseDirectionCountSum => None,
            LaneRule::D43LaneResidualGapIdZero => {
                Some(format!("residual badge row {row} has zero gap_id"))
            }
            LaneRule::D43LaneResidualStatusUnknown => Some(format!(
                "residual badge row {row} has unknown status {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualEvidenceEmpty => {
                Some(format!("residual badge row {row} has no evidence bits"))
            }
            LaneRule::D43LaneResidualCurrentSupportUnknown => Some(format!(
                "residual badge row {row} has unknown current_support {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualCurrentDomainUnknown => Some(format!(
                "residual badge row {row} has unknown current_domain_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualProposedRecordKindUnknown => Some(format!(
                "residual badge row {row} has unknown proposed_record_kind {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualCurrentRowLenRange => Some(format!(
                "residual badge row {row} has implausible current_row_len {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualSourceRefInvalid => Some(format!(
                "residual badge row {row} has invalid source_ref_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualOwnerBoundaryUnknown => Some(format!(
                "residual badge row {row} has unknown owner_boundary {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualRootStatusUnknown => Some(format!(
                "residual badge row {row} has unknown root_status {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualProofStatusUnknown => Some(format!(
                "residual badge row {row} has unknown proof_status {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualReserved => Some(format!(
                "residual badge row {row} reserved field is nonzero"
            )),
            LaneRule::D43LaneResidualDuplicateGapId => Some(format!(
                "residual badge row {row} duplicates gap_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneResidualSourceBlockedCount => Some(format!(
                "residual_source_blocked_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneResidualSchemaBlockedCount => Some(format!(
                "residual_schema_blocked_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneResidualOwnedElsewhereCount => Some(format!(
                "residual_owned_elsewhere_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneResidualSharedPatchBlockedCount => Some(format!(
                "residual_shared_patch_blocked_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneDetailSpeedRange => Some(format!(
                "detail row {row} invalid speed_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailCompletedRange => Some(format!(
                "detail row {row} invalid completed_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailDaysRange => Some(format!(
                "detail row {row} invalid days {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailConvoysAssignedRange => Some(format!(
                "detail row {row} invalid convoys_assigned {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailConvoysTotalRange => Some(format!(
                "detail row {row} invalid convoys_total {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailEfficiencyRange => Some(format!(
                "detail row {row} invalid efficiency_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailFactoryProgressRange => Some(format!(
                "detail row {row} invalid factory_cic_progress_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneDetailEquipmentProgressRange => Some(format!(
                "detail row {row} invalid equipment_transfer_progress_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneFlowCountryKindMask => Some(format!(
                "tag_db_id {} resource-flow kind mask {:#X} != {:#X}",
                finding.operand_a, finding.operand_b, finding.operand_c
            )),
            LaneRule::D43LaneFlowDecodedCountryCount => Some(format!(
                "decoded resource-flow country count {} != header {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneFlowLegacyExportedDecodedCount => Some(format!(
                "legacy resource_flow_exported_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneResourceUnknownFlags => {
                Some(format!("MTRR row {row} has invalid shape"))
            }
            LaneRule::D43LaneLendLeaseContractIdRange => Some(format!(
                "lend-lease row {row} invalid contract_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeasePerspectiveRange => Some(format!(
                "lend-lease row {row} invalid perspective_tag_db_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseCounterpartRange => Some(format!(
                "lend-lease row {row} invalid counterpart_tag_db_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseSameParty => Some(format!(
                "lend-lease row {row} perspective equals counterpart"
            )),
            LaneRule::D43LaneLendLeaseDirection => Some(format!(
                "lend-lease row {row} invalid direction {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseVariantRange => Some(format!(
                "lend-lease row {row} invalid equipment_variant_id {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseUnknownFlags => Some(format!(
                "lend-lease row {row} unknown flags 0x{:X}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseAmountRange => Some(format!(
                "detail row {row} invalid lend_lease.amount_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseAmountWithoutFlag => Some(format!(
                "detail row {row} has lend_lease.amount_raw without valid flag"
            )),
            LaneRule::D43LaneLendLeasePriceRange => Some(format!(
                "detail row {row} invalid lend_lease.price_raw {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeasePriceWithoutFlag => Some(format!(
                "detail row {row} has lend_lease.price_raw without valid flag"
            )),
            LaneRule::D43LaneLendLeaseSourceRecordKind => Some(format!(
                "lend-lease row {row} invalid source_record_kind {}",
                finding.operand_a
            )),
            LaneRule::D43LaneLendLeaseReserved => {
                Some(format!("lend-lease row {row} reserved field is nonzero"))
            }
            LaneRule::D43LaneLendLeaseDuplicateKey => Some(format!(
                "lend-lease row {row} duplicates contract/perspective/direction key"
            )),
            LaneRule::D43LaneLendLeaseContractHalfCount => Some(format!(
                "lend-lease contract_id {} has outgoing {} incoming {}, expected 1/1",
                finding.operand_a, finding.operand_b, finding.operand_c
            )),
            LaneRule::D43LaneLendLeaseOutgoingCount => Some(format!(
                "lend_lease_outgoing_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneLendLeaseIncomingCount => Some(format!(
                "lend_lease_incoming_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneLendLeaseAmountCount => Some(format!(
                "lend_lease_amount_valid_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
            LaneRule::D43LaneLendLeasePriceCount => Some(format!(
                "lend_lease_price_valid_rows header {} != decoded {}",
                finding.operand_a, finding.operand_b
            )),
        };
    }
    let message = match finding.rule {
        Rule::D43FindingsSuppressed => format!(
            "omitted {} additional findings for {:?}",
            finding.operand_a,
            finding
                .suppressed_rule
                .unwrap_or(Rule::D43FindingsSuppressed)
        ),
        Rule::D43DocumentCapacity => format!(
            "cannot allocate {} decoded core row(s) of {} byte(s) each",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43StructuralViolation => format_structural(finding),
        Rule::D43FamilyNotAccepted => format!(
            "{} trailer is not a family this decoder accepts",
            finding
                .family
                .map_or("unknown", d43_grammar::Trailer::label)
        ),
        Rule::D43CoreScale => format!("scale_raw {} != {}", finding.operand_a, finding.operand_b),
        Rule::D43CoreChangedRows => format!(
            "changed_rows {} > row_count {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43CoreEmittedRows => format!(
            "emitted_contract_rows {} != row_count {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43CoreObservedRows => format!(
            "observed_contract_rows {} < row_count {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43CoreHeaderCountBound => format!(
            "{} {} > row_count {}",
            core_word_name(finding.at),
            finding.operand_a,
            finding.operand_b
        ),
        Rule::D43CoreContractIdZero | Rule::D43CoreContractIdAuthorityRange => format!(
            "market trade row {row} invalid contract_id {}",
            finding.operand_a
        ),
        Rule::D43CoreDuplicateContractId => format!(
            "market trade row {row} duplicates contract_id {}",
            finding.operand_a
        ),
        Rule::D43CoreContractType => format!(
            "market trade row {row} contract_type {} != {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43CoreZeroParty => return None,
        Rule::D43CoreSellerAuthorityRange => format!(
            "market trade row {row} invalid seller_tag_db_id {}",
            finding.operand_a
        ),
        Rule::D43CoreBuyerAuthorityRange => format!(
            "market trade row {row} invalid buyer_tag_db_id {}",
            finding.operand_a
        ),
        Rule::D43CoreUnknownFlags => format!(
            "market trade row {row} unknown flags 0x{:X}",
            (finding.operand_a as u32) & !ROW_FLAGS_KNOWN
        ),
        Rule::D43CoreReserved => format!("market trade row {row} reserved field is nonzero"),
        Rule::D43CoreBuyerStatusRange => format!(
            "market trade row {row} invalid buyer_ptr_status {}",
            finding.operand_a
        ),
        Rule::D43CoreBuyerFlagStatus => {
            format!("market trade row {row} buyer pointer flag/status mismatch")
        }
        Rule::D43CoreEquipmentStatusRange => format!(
            "market trade row {row} invalid equipment_subbuf_status {}",
            finding.operand_a
        ),
        Rule::D43CoreDeliveryStatusRange => format!(
            "market trade row {row} invalid delivery_subbuf_status {}",
            finding.operand_a
        ),
        Rule::D43CoreEquipmentFlagStatus => {
            format!("market trade row {row} equipment_subbuf flag/status mismatch")
        }
        Rule::D43CoreDeliveryFlagStatus => {
            format!("market trade row {row} delivery_subbuf flag/status mismatch")
        }
        Rule::D43CoreVariantRange => format!(
            "market trade row {row} invalid equipment_variant_id {}",
            finding.operand_a
        ),
        Rule::D43CoreVariantWithoutEquipment => return None,
        Rule::D43CoreVariantWithoutFlag => {
            format!("market trade row {row} has variant id without valid flag")
        }
        Rule::D43CoreAmountRange => format!(
            "market trade row {row} invalid amount_raw {}",
            finding.operand_a
        ),
        Rule::D43CoreAmountWithoutDelivery => return None,
        Rule::D43CoreAmountWithoutFlag => {
            format!("market trade row {row} has amount without valid flag")
        }
        Rule::D43CoreComputedCountMismatch => format!(
            "{} header {} != decoded {}",
            core_word_name(finding.at),
            finding.operand_b,
            finding.operand_c
        ),
        Rule::D43CoreDuplicateContractPair => return None,
        Rule::D43DetailContractIdZero | Rule::D43DetailContractIdAuthorityRange => {
            format!("detail row {row} invalid contract_id {}", finding.operand_a)
        }
        Rule::D43DetailMissingCoreContract => return None,
        Rule::D43DetailDuplicateContract if finding.operand_a == 0 => return None,
        Rule::D43DetailDuplicateContract => format!(
            "detail row {row} duplicates contract_id {}",
            finding.operand_a
        ),
        Rule::D43DetailInvalidFlags if finding.operand_a == 0 => return None,
        Rule::D43DetailInvalidFlags => format!(
            "detail row {row} unknown flags 0x{:X}",
            (finding.operand_a as u32) & !DETAIL_FLAGS_KNOWN
        ),
        Rule::D43DetailReserved => format!("detail row {row} reserved field is nonzero"),
        Rule::D43DetailPresentValueRange
            if matches!(
                finding.value_field,
                Some(
                    d43_grammar::decode::D43ValueField::D43CompletedRaw
                        | d43_grammar::decode::D43ValueField::D43Days
                        | d43_grammar::decode::D43ValueField::D43EfficiencyRaw
                        | d43_grammar::decode::D43ValueField::D43FactoryCicProgressRaw
                        | d43_grammar::decode::D43ValueField::D43EquipmentTransferProgressRaw
                )
            ) =>
        {
            return None
        }
        Rule::D43DetailPresentValueRange => format!(
            "detail row {row} invalid {} {}",
            value_field_name(finding.value_field),
            finding.operand_a
        ),
        Rule::D43DetailAbsentValueNonzero => format!(
            "detail row {row} has {} without valid flag",
            value_field_name(finding.value_field)
        ),
        Rule::D43DetailDeliveryCountryRange => format!(
            "detail row {row} invalid delivery_country_tag_db_id {}",
            finding.operand_a
        ),
        Rule::D43DetailDeliveryCountryWithoutFlag => {
            format!("detail row {row} has delivery country without valid flag")
        }
        Rule::D43DetailConvoyRange => return None,
        Rule::D43DetailConvoyWithoutFlag => {
            format!("detail row {row} has convoy counts without valid flag")
        }
        Rule::D43DetailHeaderCountMismatch => {
            let name = [
                "detail_price_valid_rows",
                "detail_speed_valid_rows",
                "detail_delivery_route_valid_rows",
                "detail_progress_valid_rows",
            ]
            .get(finding.operand_a as usize)
            .copied()
            .unwrap_or("detail_count");
            format!(
                "{name} header {} != decoded {}",
                finding.operand_b, finding.operand_c
            )
        }
        Rule::D43SizedHeaderReserved0 => match finding.family {
            Some(d43_grammar::Trailer::Detail) => {
                "detail trailer reserved header field is nonzero".to_string()
            }
            Some(d43_grammar::Trailer::ResourceFlow) => {
                "resource-flow trailer reserved header field is nonzero".to_string()
            }
            Some(d43_grammar::Trailer::Residual) => {
                "residual trailer reserved header field is nonzero".to_string()
            }
            Some(d43_grammar::Trailer::LendLease) => {
                "lend-lease trailer reserved header field is nonzero".to_string()
            }
            Some(d43_grammar::Trailer::FactoryHeader) => factory_shape(report),
            Some(d43_grammar::Trailer::ResourceRow) => resource_shape(report),
            Some(d43_grammar::Trailer::ClassicDeal) => deal_shape(report),
            _ => "market trade trailer reserved header field is nonzero".to_string(),
        },
        Rule::D43FlowHeaderResourceCount => format!(
            "resource_flow_resource_count {} is outside 1..={}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowHeaderResourceMask => format!(
            "resource_flow_resource_mask {:#X} != {:#X}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowHeaderKindMask
            if report
                .document
                .flow_header
                .is_some_and(|header| header.version == d43_grammar::FlowVersion::Legacy) =>
        {
            "legacy resource-flow trailer reserved header field is nonzero".to_string()
        }
        Rule::D43FlowHeaderKindMask => format!(
            "resource_flow_kind_mask {:#X} != {:#X}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowHeaderRowCount
            if report
                .document
                .flow_header
                .is_some_and(|header| header.version == d43_grammar::FlowVersion::Legacy) =>
        {
            format!(
                "legacy resource_flow_exported_rows {} != row_count {}",
                finding.operand_b, finding.operand_a
            )
        }
        Rule::D43FlowHeaderRowCount => format!(
            "resource_flow_rows {} != country_count {} * 6",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowHeaderScale => format!(
            "resource_flow_scale_raw {} != {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowTagRange => format!(
            "resource-flow row {row} invalid tag_db_id {}",
            finding.operand_a
        ),
        Rule::D43FlowKind => format!(
            "resource-flow row {row} invalid flow_kind {}",
            finding.operand_a
        ),
        Rule::D43FlowRowMaskOrCount => format!(
            "resource-flow row {row} states {:#X}, header states {:#X}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowReserved => format!("resource-flow row {row} reserved field is nonzero"),
        Rule::D43FlowLegacyValuesAllZero => {
            format!("resource-flow row {row} has all-zero exported resources")
        }
        Rule::D43FlowDuplicateCountryKind => format!(
            "resource-flow row {row} duplicates tag_db_id {} flow_kind {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowDecodedCountryCount => format!(
            "decoded resource-flow country count {} != header {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43FlowCountryKindsIncomplete => return None,
        Rule::D43FactoryHeaderShape => factory_shape(report),
        Rule::D43FactoryTagRange => format!(
            "factory-header row {row} invalid tag_db_id {}",
            finding.operand_a
        ),
        Rule::D43FactoryDuplicateTag => format!(
            "factory-header row {row} duplicates tag_db_id {}",
            finding.operand_a
        ),
        Rule::D43IdentityLength => format!(
            "MTRR identity {row} token length {} is outside 1..={}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43IdentityPadding => format!("MTRR identity {row} has non-zero padding"),
        Rule::D43IdentityUtf8 => format!("MTRR identity {row} is not UTF-8"),
        Rule::D43IdentityCharset => format!("MTRR identity {row} is not a resource token"),
        Rule::D43IdentityDuplicateToken => {
            let token = report
                .document
                .resource_identities
                .get(row)
                .and_then(d43_grammar::decode::identity_token)
                .unwrap_or("<unreadable>");
            format!("MTRR identity {row} repeats token {token}")
        }
        Rule::D43ResourceHeaderShape => resource_shape(report),
        Rule::D43ResourceTagRange => {
            format!("MTRR row {row} invalid tag_db_id {}", finding.operand_a)
        }
        Rule::D43ResourceIndexRange
        | Rule::D43ResourceVisibleOrderRange
        | Rule::D43ResourceDescriptorZero
        | Rule::D43ResourceReserved => format!("MTRR row {row} has invalid shape"),
        Rule::D43ResourceIndexOrderMismatch => return None,
        Rule::D43ResourceUnavailableVersion => {
            format!("MTRR row {row} has inconsistent needed semantics")
        }
        Rule::D43ResourceUnavailableShape => {
            format!("MTRR row {row} has inconsistent unobserved-needed semantics")
        }
        Rule::D43ResourceOilShape => format!("MTRR row {row} has inconsistent needed semantics"),
        Rule::D43ResourceNonOilFlags
        | Rule::D43ResourceNonOilCurrentMax
        | Rule::D43ResourceNonOilNeededBalance
        | Rule::D43ResourceNonOilNeededType
        | Rule::D43ResourceNonOilSign => {
            format!("MTRR row {row} has inconsistent needed semantics")
        }
        Rule::D43ResourceDuplicateCountrySlot => {
            format!("MTRR row {row} duplicates country/resource key")
        }
        Rule::D43ResourceDescriptorChanged => return None,
        Rule::D43ResourceOilSlotCount => format!(
            "MTRR states {} oil slots, expected exactly 1",
            finding.operand_a
        ),
        Rule::D43ResourceOilRowCount => format!(
            "MTRR states {} oil rows for {} countries",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43ResourceOilWithoutCountry => {
            "MTRR states an oil row with no countries".to_string()
        }
        Rule::D43ResourceDescriptorCoverage => return None,
        Rule::D43ResourceCountryCount => format!(
            "MTRR decoded country count {} != header {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43ResourceCountryCoverage => format!(
            "MTRR country {} visible order is not complete",
            finding.operand_a
        ),
        Rule::D43DealHeaderShape => deal_shape(report),
        Rule::D43DealWitnessRange | Rule::D43DealWitnessDuplicate => {
            format!("MTRD witness {row} is invalid or duplicated")
        }
        Rule::D43DealIncompleteNotSubset | Rule::D43DealIncompleteNotAscending => {
            format!("MTRD incomplete owner {row} is not an ascending witness")
        }
        Rule::D43DealOwnerNotWitnessed
        | Rule::D43DealCounterpartNotWitnessed
        | Rule::D43DealSameParty
        | Rule::D43DealResourceIndexRange
        | Rule::D43DealDirection
        | Rule::D43DealRoute
        | Rule::D43DealFlags
        | Rule::D43DealLegacyReservedStatus
        | Rule::D43DealStatusBits
        | Rule::D43DealPackedReserved
        | Rule::D43DealPackedResourceEcho
        | Rule::D43DealIdentityOrdinal
        | Rule::D43DealIdentityParties
        | Rule::D43DealEfficiencyRange
        | Rule::D43DealReservedOrdinal => format!("MTRD row {row} has invalid shape"),
        Rule::D43DealRequestedScale => return None,
        Rule::D43DealDuplicateVisibleOrder => {
            format!("MTRD row {row} duplicates owner/visible_order")
        }
        Rule::D43DealHalfCount => format_deal_half_count(finding),
        Rule::D43DealHalfMismatch => format_deal_identity(finding, "reciprocal halves disagree"),
        Rule::D43DealVisibleOrderCoverage => format!(
            "MTRD owner {} visible order is not contiguous",
            finding.operand_a
        ),
        Rule::D43ActivationFlags
        | Rule::D43ActivationRootExact
        | Rule::D43ActivationActiveCountry
        | Rule::D43ActivationSequence
        | Rule::D43ActivationCountryRange
        | Rule::D43ActivationRootKind
        | Rule::D43ActivationOpenRva => "MTRA activation identity/shape is invalid".to_string(),
        Rule::D43StockpileScale
        | Rule::D43StockpileSegmentLength
        | Rule::D43StockpileCountryCount
        | Rule::D43StockpileExpectedCountryCount
        | Rule::D43StockpileGlobalFlags
        | Rule::D43StockpileReserved => stockpile_shape(report),
        Rule::D43StockpileCountryTagRange
        | Rule::D43StockpileCountryDuplicate
        | Rule::D43StockpileCountryFirst
        | Rule::D43StockpileCountryRange
        | Rule::D43StockpileCountryFlags
        | Rule::D43StockpileCountryReserved => {
            format!("MTS1 country row {row} has invalid tag/range/completeness")
        }
        Rule::D43StockpileCountryCoverage => format!(
            "MTS1 country ranges cover {} item(s), expected {}",
            finding.operand_a, finding.operand_b
        ),
        Rule::D43StockpileVariantRange
        | Rule::D43StockpileEquipmentType
        | Rule::D43StockpileOrdinal
        | Rule::D43StockpileItemReserved
        | Rule::D43StockpileAmountRange
        | Rule::D43StockpileDuplicateVariant => {
            format!("MTS1 item {row} has invalid identity/order/amount")
        }
        Rule::D43CrossResourceCountAgreement
        | Rule::D43CrossDealResourceSlot
        | Rule::D43CrossCountrySetAgreement => return None,
        Rule::D43CrossFlowV3NeedsIdentityResourceRows => return None,
        Rule::D43ResidualHeaderCounts
        | Rule::D43ResidualReserved
        | Rule::D43LendLeaseHeaderCounts
        | Rule::D43LendLeaseRowShape => return None,
    };
    Some(message)
}

fn push_unique(errors: &mut Vec<String>, message: String) {
    if !errors.iter().any(|existing| existing == &message) {
        errors.push(message);
    }
}

fn core_word_name(at: usize) -> &'static str {
    d43_grammar::CoreWord::ALL
        .iter()
        .copied()
        .find(|word| word.offset() == at)
        .map_or("count", d43_grammar::CoreWord::diagnostic_name)
}

fn value_field_name(field: Option<d43_grammar::decode::D43ValueField>) -> &'static str {
    use d43_grammar::decode::D43ValueField;
    match field {
        Some(D43ValueField::D43PriceRaw) => "price_raw",
        Some(D43ValueField::D43AmountRaw) => "amount_raw",
        Some(D43ValueField::D43SpeedRaw) => "speed_raw",
        Some(D43ValueField::D43CompletedRaw) => "completed_raw",
        Some(D43ValueField::D43Days) => "days",
        Some(D43ValueField::D43EfficiencyRaw) => "efficiency_raw",
        Some(D43ValueField::D43FactoryCicProgressRaw) => "factory_cic_progress_raw",
        Some(D43ValueField::D43EquipmentTransferProgressRaw) => "equipment_transfer_progress_raw",
        None => "value",
    }
}

fn lane_family_name(family: Option<d43_grammar::Trailer>) -> &'static str {
    match family {
        Some(d43_grammar::Trailer::Detail) => "detail",
        Some(d43_grammar::Trailer::ResourceFlow) => "resource-flow",
        Some(d43_grammar::Trailer::FactoryHeader) => "factory-header",
        Some(d43_grammar::Trailer::ResourceRow) => "resource-row",
        Some(d43_grammar::Trailer::ClassicDeal) => "classic-deal",
        Some(d43_grammar::Trailer::Activation) => "activation",
        Some(d43_grammar::Trailer::Stockpile) => "stockpile",
        Some(d43_grammar::Trailer::Residual) => "residual",
        Some(d43_grammar::Trailer::LendLease) => "lend-lease",
        None => "market trade",
    }
}

fn factory_shape(report: &d43_grammar::decode::DecodeReport) -> String {
    let Some(header) = report.document.factory_header else {
        return "invalid factory-header shape".to_string();
    };
    format!(
        "invalid factory-header shape version {} header_len {} row_len {} rows {} countries {}",
        header.sized.version.wire(),
        header.sized.header_len,
        header.sized.row_len,
        header.sized.row_count,
        header.country_count
    )
}

fn resource_shape(report: &d43_grammar::decode::DecodeReport) -> String {
    let Some(header) = report.document.resource_header else {
        return "invalid MTRR shape".to_string();
    };
    format!(
        "invalid MTRR shape version {} header_len {} row_len {} rows {} countries {} resources {} scale {}",
        header.version.wire(),
        header.header_len,
        header.row_len,
        header.row_count,
        header.country_count,
        header.resource_count,
        header.display_scale
    )
}

fn deal_shape(report: &d43_grammar::decode::DecodeReport) -> String {
    let Some(header) = report.document.deal_header else {
        return "invalid MTRD shape".to_string();
    };
    format!(
        "invalid MTRD shape version {} header_len {} row_len {} countries {} witnesses {} incomplete {} scale {}",
        header.version.wire(),
        header.header_len,
        header.row_len,
        header.country_count,
        header.witness_count,
        header.incomplete_count,
        header.scale_raw
    )
}

fn stockpile_shape(report: &d43_grammar::decode::DecodeReport) -> String {
    let Some(header) = report.document.stockpile_header else {
        return "invalid MTS1 header shape".to_string();
    };
    format!(
        "invalid MTS1 header shape version {} header_len {} country_row_len {} item_row_len {} segment_len {}/{} countries {}/{} scale {} flags {:#x} reserved {}",
        header.version.wire(),
        header.header_len,
        header.country_row_len,
        header.item_row_len,
        header.declared_segment_len,
        header.computed_segment_len(),
        header.country_count,
        header.expected_country_count,
        header.scale_raw,
        header.flags,
        header.reserved
    )
}

fn format_deal_identity(finding: &d43_grammar::decode::Finding, suffix: &str) -> String {
    let Some(identity) = finding.deal_identity else {
        return format!(
            "MTRD deal_id at row {} {suffix}",
            finding.row_or_slot.unwrap_or(0)
        );
    };
    format!(
        "MTRD deal_id {}->{} resource {} ordinal {} {suffix}",
        identity.exporter_tag_db_id,
        identity.importer_tag_db_id,
        identity.resource_index,
        identity.ordinal
    )
}

fn format_deal_half_count(finding: &d43_grammar::decode::Finding) -> String {
    let count = finding.operand_a;
    let Some(identity) = finding.deal_identity else {
        return format!("MTRD deal_id has {count} half/halves, expected 2");
    };
    format!(
        "MTRD deal_id {}->{} resource {} ordinal {} has {count} half/halves, expected 2",
        identity.exporter_tag_db_id,
        identity.importer_tag_db_id,
        identity.resource_index,
        identity.ordinal
    )
}

fn format_structural(finding: &d43_grammar::decode::Finding) -> String {
    use d43_grammar::{Section, Trailer, ViolationKind};
    let Some(violation) = finding.structural else {
        return "market trade structural violation".to_string();
    };
    match violation.kind {
        ViolationKind::Need {
            section,
            needed,
            available,
        } => match section {
            Section::CoreHeader | Section::CoreRows => format!(
                "market_trade_live payload is {available} byte(s), expected at least {needed}"
            ),
            Section::TrailerMagic => "optional trailer is too short to contain magic".to_string(),
            Section::TrailerHeader(trailer) => {
                let remaining = available.saturating_sub(violation.at);
                let local_needed = needed.saturating_sub(violation.at);
                match trailer {
                    Trailer::FactoryHeader => "factory-header trailer is too short".to_string(),
                    Trailer::ResourceRow => "MTRR trailer is too short".to_string(),
                    Trailer::ClassicDeal => "MTRD trailer is too short".to_string(),
                    Trailer::Activation => "MTRA activation trailer is too short".to_string(),
                    Trailer::Stockpile => "MTS1 trailer is too short".to_string(),
                    Trailer::Detail => format!(
                        "detail trailer is {remaining} byte(s), expected at least {local_needed}"
                    ),
                    Trailer::ResourceFlow => format!(
                        "resource-flow trailer is {remaining} byte(s), expected at least {local_needed}"
                    ),
                    Trailer::Residual => format!(
                        "residual trailer is {remaining} byte(s), expected at least {local_needed}"
                    ),
                    Trailer::LendLease => format!(
                        "lend-lease trailer is {remaining} byte(s), expected at least {local_needed}"
                    ),
                }
            }
            Section::TrailerBody(trailer) => {
                let trailer_at = finding
                    .header_preview
                    .map_or(violation.at, |preview| preview.prefix().at);
                let name = match trailer {
                    Trailer::ResourceRow => "MTRR",
                    Trailer::ClassicDeal => "MTRD",
                    Trailer::Activation => "MTRA activation",
                    Trailer::Stockpile => "MTS1",
                    _ => lane_family_name(Some(trailer)),
                };
                format!(
                    "{} trailer is {} byte(s), expected at least {}",
                    name,
                    available.saturating_sub(trailer_at),
                    needed.saturating_sub(trailer_at)
                )
            }
        },
        ViolationKind::UnknownTrailerMagic { .. } | ViolationKind::TrailingBytes { .. } => {
            "market trade optional trailer has unknown magic".to_string()
        }
        ViolationKind::DuplicateTrailer { trailer, .. } => match trailer {
            Trailer::Activation => "duplicate MTRA activation trailer".to_string(),
            Trailer::Stockpile => "duplicate MTS1 stockpile trailer".to_string(),
            _ => format!("duplicate {} trailer", trailer.label()),
        },
        ViolationKind::UnknownVersion { trailer, version } => match trailer {
            Trailer::ResourceFlow => {
                format!("unsupported resource-flow trailer version {version}")
            }
            Trailer::ResourceRow | Trailer::ClassicDeal | Trailer::FactoryHeader => {
                preview_shape(finding).unwrap_or_else(|| {
                    format!("invalid {} shape version {version}", trailer.label())
                })
            }
            Trailer::Activation => "MTRA activation identity/shape is invalid".to_string(),
            Trailer::Stockpile => {
                preview_shape(finding).unwrap_or_else(|| "invalid MTS1 header shape".to_string())
            }
            Trailer::Detail | Trailer::Residual | Trailer::LendLease => preview_shape(finding)
                .unwrap_or_else(|| {
                    format!(
                        "unsupported {} trailer version {version}",
                        lane_family_name(Some(trailer))
                    )
                }),
        },
        ViolationKind::HeaderLen {
            trailer,
            declared,
            expected,
        } => preview_shape(finding).unwrap_or_else(|| {
            format!(
                "{} trailer header_len {declared} != {expected}",
                lane_family_name(Some(trailer))
            )
        }),
        ViolationKind::RowLen {
            trailer,
            declared,
            expected,
        } => preview_shape(finding).unwrap_or_else(|| {
            format!(
                "{} trailer row_len {declared} != {expected}",
                lane_family_name(Some(trailer))
            )
        }),
        ViolationKind::ResourceCount {
            trailer,
            declared,
            max,
        } => {
            if trailer == Trailer::ResourceRow {
                preview_shape(finding).unwrap_or_else(|| {
                    format!("invalid MTRR shape resource_count {declared} is outside 1..={max}")
                })
            } else if trailer == Trailer::ResourceFlow {
                format!("resource_flow_resource_count {declared} is outside 1..={max}")
            } else {
                format!(
                    "{} resource_count {declared} is outside 1..={max}",
                    trailer.label()
                )
            }
        }
        ViolationKind::IdentityStride { trailer, stride } => preview_shape(finding)
            .unwrap_or_else(|| format!("{} identity stride {stride} is invalid", trailer.label())),
        ViolationKind::AbsentBlockDeclared { trailer, .. } => match trailer {
            Trailer::ResourceRow | Trailer::ClassicDeal => preview_shape(finding)
                .unwrap_or_else(|| format!("invalid {} shape", trailer.label())),
            _ => format!("invalid {} header shape", trailer.label()),
        },
        ViolationKind::OffsetOverflow { .. } => "market trade payload extent overflow".to_string(),
        ViolationKind::CallerDefect(_) => {
            "market trade optional trailer did not advance".to_string()
        }
    }
}
// @d43-payload-reader-region end

fn preview_shape(finding: &d43_grammar::decode::Finding) -> Option<String> {
    use d43_grammar::HeaderPreview;
    match finding.header_preview? {
        HeaderPreview::Detail(words) => Some(format!(
            "unsupported detail trailer version {} header_len {} row_len {}",
            words.sized.prefix.version, words.sized.prefix.header_len, words.sized.row_len
        )),
        HeaderPreview::Residual(words) => Some(format!(
            "unsupported residual trailer version {} header_len {} row_len {}",
            words.sized.prefix.version, words.sized.prefix.header_len, words.sized.row_len
        )),
        HeaderPreview::LendLease(words) => Some(format!(
            "unsupported lend-lease trailer version {} header_len {} row_len {}",
            words.sized.prefix.version, words.sized.prefix.header_len, words.sized.row_len
        )),
        HeaderPreview::ResourceFlow(words) => {
            let expected_row_len = match words.sized.prefix.version {
                3 => 16usize.saturating_add((words.mask_or_count as usize).saturating_mul(8)),
                _ => 72,
            };
            Some(format!(
                "resource-flow trailer version {} header_len {} row_len {} != {}",
                words.sized.prefix.version,
                words.sized.prefix.header_len,
                words.sized.row_len,
                expected_row_len
            ))
        }
        HeaderPreview::FactoryHeader(words) => Some(format!(
            "invalid factory-header shape version {} header_len {} row_len {} rows {} countries {}",
            words.sized.prefix.version,
            words.sized.prefix.header_len,
            words.sized.row_len,
            words.sized.row_count,
            words.country_count
        )),
        HeaderPreview::ResourceRow(words) => Some(format!(
            "invalid MTRR shape version {} header_len {} row_len {} rows {} countries {} resources {} scale {}",
            words.sized.prefix.version,
            words.sized.prefix.header_len,
            words.sized.row_len,
            words.sized.row_count,
            words.country_count,
            words.resource_count,
            words.display_scale
        )),
        HeaderPreview::ClassicDeal(words) => Some(format!(
            "invalid MTRD shape version {} header_len {} row_len {} countries {} witnesses {} incomplete {} scale {}",
            words.sized.prefix.version,
            words.sized.prefix.header_len,
            words.sized.row_len,
            words.country_count,
            words.witness_count,
            words.incomplete_count,
            words.scale_raw
        )),
        HeaderPreview::Activation(_) => Some("MTRA activation identity/shape is invalid".to_string()),
        HeaderPreview::Stockpile(words) => {
            let expected_len = (words.prefix.header_len as usize)
                .saturating_add(
                    (words.country_count as usize)
                        .saturating_mul(words.country_row_len as usize),
                )
                .saturating_add(
                    (words.item_count as usize).saturating_mul(words.item_row_len as usize),
                );
            Some(format!(
                "invalid MTS1 header shape version {} header_len {} country_row_len {} item_row_len {} segment_len {}/{} countries {}/{} scale {} flags {:#x} reserved {}",
                words.prefix.version,
                words.prefix.header_len,
                words.country_row_len,
                words.item_row_len,
                words.declared_segment_len,
                expected_len,
                words.country_count,
                words.expected_country_count,
                words.scale_raw,
                words.flags,
                words.reserved
            ))
        }
    }
}
