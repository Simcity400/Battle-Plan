//! Shipping XUI18 authored compositor and viewer-owned lifecycle.

use crate::assets::AssetResolver;
use crate::compose::{ComposeOptions, ComposeOutput, Compositor, UiAssetCache};
use crate::decision_chrome::DecisionAvailability;
use crate::gfx::SpriteRegistry;
use crate::gui::GuiElement;
use crate::gui_scene_util::{
    find_named_element, parse_layout_set, prefix_element_names, set_position,
};
use crate::loc::Localisation;
use crate::politics_state::PoliticsTooltipPayloads;
use crate::power_balance::{apply_power_balance_to_compose_options, PowerBalancePresentation};
use crate::tooltip_compositor::{
    authored_tooltip_shell_name, AdmittedTooltipSource, RowAvailability, TooltipLineKind,
    TooltipOwner, TooltipPayload, TooltipRow,
};
use crate::{GameUiError, Result};

pub const XUI18_GUI_SOURCE: &str = "interface/powerbalanceview.gui";
pub const XUI18_ROOT: &str = "powerbalanceview";
pub const XUI18_DECISION_GUI_SOURCE: &str = "interface/countrydecisionview.gui";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerBalanceInputRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PowerBalanceInputRegion {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PowerBalanceViewerState {
    pub panel_open: bool,
}

impl PowerBalanceViewerState {
    pub fn open(&mut self) {
        self.panel_open = true;
    }

    pub fn close(&mut self) {
        self.panel_open = false;
    }

    pub fn reset_for_seek_or_country_change(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredPowerBalanceComposeOutput {
    pub output: ComposeOutput,
    pub authored_root: String,
    pub source_record_id: String,
    pub decision_record_ids: Vec<String>,
    pub panel_region: PowerBalanceInputRegion,
    pub close_region: PowerBalanceInputRegion,
    pub marker_top_left: [i32; 2],
    pub range_splitter_top_left: Vec<[i32; 2]>,
    pub tooltip_targets: Vec<PowerBalanceTooltipHitTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceTooltipHitTarget {
    pub rect: PowerBalanceInputRegion,
    pub payloads: PoliticsTooltipPayloads,
}

impl PowerBalanceTooltipHitTarget {
    pub fn contains(&self, point: (i32, i32)) -> bool {
        self.rect.contains(point.0, point.1)
    }
}

pub fn compose_power_balance_surface(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    cache: &mut UiAssetCache,
    panel: &PowerBalancePresentation,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<AuthoredPowerBalanceComposeOutput> {
    let resolved = resolver.resolve_file(XUI18_GUI_SOURCE)?;
    let layouts = parse_layout_set(&resolved.path, &resolved.rel_path)?;
    if layouts.stats.recovery_count != 0 || layouts.stats.decode_loss_count != 0 {
        return Err(GameUiError::parse(
            "XUI18 effective powerbalanceview.gui is not losslessly parsed",
        ));
    }
    let mut root = find_named_element(&layouts, XUI18_GUI_SOURCE, XUI18_ROOT)?;
    let range_template = find_named_element(&layouts, XUI18_GUI_SOURCE, "range_bar")?;

    let decision_resolved = resolver.resolve_file(XUI18_DECISION_GUI_SOURCE)?;
    let decision_layouts = parse_layout_set(&decision_resolved.path, &decision_resolved.rel_path)?;
    if decision_layouts.stats.recovery_count != 0 || decision_layouts.stats.decode_loss_count != 0 {
        return Err(GameUiError::parse(
            "XUI18 effective countrydecisionview.gui is not losslessly parsed",
        ));
    }
    let decision_grid = find_named_mut(&mut root, "decision_grid")
        .ok_or_else(|| GameUiError::missing("XUI18 authored decision_grid is absent"))?;
    decision_grid.children.clear();

    let mut options =
        ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
    apply_power_balance_to_compose_options(panel, &mut options);
    let mut decision_record_ids = Vec::with_capacity(panel.decisions.len());
    for (index, decision) in panel.decisions.iter().enumerate() {
        let mut card = find_named_element(
            &decision_layouts,
            XUI18_DECISION_GUI_SOURCE,
            &decision.card.template,
        )?;
        let prefix = format!("xui18_decision_{index}_");
        prefix_element_names(&mut card, &prefix);
        let name = format!("{prefix}name_text");
        let icon = format!("{prefix}icon");
        let cost = format!("{prefix}cost_and_timer_text");
        let timer = format!("{prefix}timer_text");
        let target_flag = format!("{prefix}target_flag");
        let target_frame = format!("{prefix}target_flag_frame");
        // The authored templates contain controller-owned positive/negative
        // progress bars which the generic compositor otherwise paints at its
        // 100% default. D74P4 carries no decision-progress authority, so both
        // bars stay hidden until a future exact carrier explicitly owns them.
        hide_unauthenticated_decision_progress(&mut options, &prefix);
        options
            .text_overrides
            .insert(name, decision.resolved_name.clone());
        options
            .sprite_name_overrides
            .insert(icon, decision.card.icon_sprite.clone());
        match decision.card.cost_and_timer_text.as_ref() {
            Some(text) => {
                options.text_overrides.insert(cost.clone(), text.clone());
            }
            None => {
                options.hidden_elements.insert(cost.clone());
            }
        }
        match decision.card.timer_text.as_ref() {
            Some(text) => {
                options.text_overrides.insert(timer.clone(), text.clone());
            }
            None => {
                options.hidden_elements.insert(timer.clone());
            }
        }
        match decision.target_flag_sprite.as_ref() {
            Some(sprite) => {
                options
                    .sprite_name_overrides
                    .insert(target_flag, sprite.clone());
            }
            None => {
                options.hidden_elements.insert(target_flag);
                options.hidden_elements.insert(target_frame);
            }
        }
        if matches!(
            decision.card.availability,
            Some(DecisionAvailability::Unavailable | DecisionAvailability::OnCooldown)
        ) {
            options
                .disabled_button_elements
                .insert(format!("{prefix}btn_select"));
        }
        decision_record_ids.push(decision.source_record_id.clone());
        decision_grid.children.push(card);
    }

    let mut compositor = Compositor::with_cache(resolver, registry, cache);
    // The engine positions the marker and dynamically cloned range splitters
    // against the effective bar widget. Derive that geometry from the exact
    // winning GUI and selected sprite instead of reusing vanilla pixel
    // literals: mods may move or resize any of these authored assets.
    options.position_overrides.remove("position_marker");
    let mut range_probe = range_template.clone();
    range_probe.name = "xui18_range_geometry_probe".into();
    root.children.push(range_probe);
    let preflight = compositor.compose(&root, &options);
    let root_region = unique_region(&preflight, XUI18_ROOT)?;
    let bar_region = unique_region(&preflight, "power_balance_value")?;
    let marker_authored = unique_region(&preflight, "position_marker")?;
    let splitter_authored = unique_region(&preflight, "xui18_range_geometry_probe")?;
    root.children
        .retain(|child| child.name != "xui18_range_geometry_probe");
    let (marker_top_left, range_splitter_top_left) = derive_dynamic_positions(
        root_region,
        bar_region,
        marker_authored,
        splitter_authored,
        panel.normalized_value,
        panel
            .range_boundaries
            .iter()
            .map(|boundary| boundary.normalized),
    );
    options
        .position_overrides
        .insert("position_marker".into(), marker_top_left);
    for (index, top_left) in range_splitter_top_left.iter().copied().enumerate() {
        let mut splitter = range_template.clone();
        splitter.name = format!("xui18_range_{index}");
        set_position(&mut splitter, top_left[0], top_left[1]);
        root.children.push(splitter);
    }
    let output = compositor.compose(&root, &options);
    let panel_region = unique_region(&output, XUI18_ROOT)?;
    let close_region = unique_region(&output, "close")?;
    let tooltip_targets = power_balance_tooltip_targets(resolver, &output, panel)?;
    Ok(AuthoredPowerBalanceComposeOutput {
        output,
        authored_root: XUI18_ROOT.to_string(),
        source_record_id: panel.source_record_id.clone(),
        decision_record_ids,
        panel_region,
        close_region,
        marker_top_left,
        range_splitter_top_left,
        tooltip_targets,
    })
}

fn hide_unauthenticated_decision_progress(options: &mut ComposeOptions, prefix: &str) {
    options
        .hidden_elements
        .insert(format!("{prefix}btn_progress_good"));
    options
        .hidden_elements
        .insert(format!("{prefix}btn_progress_bad"));
}

fn power_balance_tooltip_targets(
    resolver: &AssetResolver,
    output: &ComposeOutput,
    panel: &PowerBalancePresentation,
) -> Result<Vec<PowerBalanceTooltipHitTarget>> {
    let tooltip_root_name = authored_tooltip_shell_name(resolver)?;
    let mut targets = Vec::new();
    let mut push = |element_name: &str,
                    rows: Vec<(&str, TooltipLineKind, &str, RowAvailability)>|
     -> Result<()> {
        let (rect, child_path) = unique_region_and_path(output, element_name)?;
        let tooltip_rows = rows
            .into_iter()
            .enumerate()
            .map(
                |(ordinal, (field, line_kind, text, availability))| TooltipRow {
                    source_ordinal: ordinal as u32,
                    line_kind,
                    source_identity: format!("{}:{field}", panel.source_record_id),
                    evaluation_context: panel
                        .provenance
                        .presentation_semantic_receipt_sha256
                        .clone(),
                    availability,
                    text: text.to_string(),
                    admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                        packet_id: "XUI18".into(),
                        producer_identity: panel.source_record_id.clone(),
                        field: field.into(),
                        manifest_sha256: panel.provenance.effective_content_manifest_sha256.clone(),
                    }],
                },
            )
            .collect();
        let payload = TooltipPayload {
            owner: TooltipOwner {
                source_file: XUI18_GUI_SOURCE.into(),
                root_window: XUI18_ROOT.into(),
                element_path: child_path,
                engine_instance_token: format!(
                    "xui18:{}:{}:{}:{element_name}",
                    panel.provenance.effective_content_manifest_sha256,
                    panel.country_tag,
                    panel.source_record_id,
                ),
            },
            tooltip_root_name: tooltip_root_name.clone(),
            rows: tooltip_rows,
        };
        payload.validate()?;
        targets.push(PowerBalanceTooltipHitTarget {
            rect,
            payloads: PoliticsTooltipPayloads {
                primary: payload,
                delayed: None,
            },
        });
        Ok(())
    };

    push(
        "power_balance_name",
        vec![(
            "balance_tooltip",
            TooltipLineKind::Description,
            panel.resolved_text.balance_tooltip.as_str(),
            RowAvailability::Resolved,
        )],
    )?;
    push(
        "left_power_icon",
        vec![(
            "left_side_name",
            TooltipLineKind::Header,
            panel.resolved_text.left_side_name.as_str(),
            RowAvailability::Resolved,
        )],
    )?;
    push(
        "right_power_icon",
        vec![(
            "right_side_name",
            TooltipLineKind::Header,
            panel.resolved_text.right_side_name.as_str(),
            RowAvailability::Resolved,
        )],
    )?;
    if let Some(trend) = panel.trend.as_ref() {
        push(
            "position_marker",
            vec![(
                "trend",
                TooltipLineKind::Value,
                trend.resolved_text.as_str(),
                RowAvailability::Resolved,
            )],
        )?;
    }
    push(
        "active_range_name",
        vec![
            (
                "active_range_tooltip",
                TooltipLineKind::Description,
                panel.resolved_text.active_range_tooltip.as_str(),
                RowAvailability::Resolved,
            ),
            (
                "power_balance_modifier_desc",
                TooltipLineKind::Breakdown,
                panel.power_balance_modifier_desc.as_str(),
                if panel.power_balance_modifier_desc.is_empty() {
                    RowAvailability::MeasuredEmpty
                } else {
                    RowAvailability::Resolved
                },
            ),
        ],
    )?;
    Ok(targets)
}

fn derive_dynamic_positions(
    root: PowerBalanceInputRegion,
    bar: PowerBalanceInputRegion,
    marker_authored: PowerBalanceInputRegion,
    splitter_authored: PowerBalanceInputRegion,
    marker_normalized: f32,
    splitter_normalized: impl IntoIterator<Item = f32>,
) -> ([i32; 2], Vec<[i32; 2]>) {
    let bar_local = [bar.x.saturating_sub(root.x), bar.y.saturating_sub(root.y)];
    let marker_offset = [
        marker_authored.x.saturating_sub(root.x),
        marker_authored.y.saturating_sub(root.y),
    ];
    let splitter_offset = [
        splitter_authored.x.saturating_sub(root.x),
        splitter_authored.y.saturating_sub(root.y),
    ];
    let place = |normalized: f32, offset: [i32; 2]| {
        [
            bar_local[0]
                .saturating_add((f64::from(normalized) * f64::from(bar.width)).round() as i32)
                .saturating_add(offset[0]),
            bar_local[1].saturating_add(offset[1]),
        ]
    };
    (
        place(marker_normalized, marker_offset),
        splitter_normalized
            .into_iter()
            .map(|normalized| place(normalized, splitter_offset))
            .collect(),
    )
}

fn unique_region(output: &ComposeOutput, name: &str) -> Result<PowerBalanceInputRegion> {
    unique_region_and_path(output, name).map(|(region, _)| region)
}

fn unique_region_and_path(
    output: &ComposeOutput,
    name: &str,
) -> Result<(PowerBalanceInputRegion, Vec<u32>)> {
    let hits = output
        .debug_rects
        .iter()
        .filter(|rect| rect.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    let [rect] = hits.as_slice() else {
        return Err(GameUiError::parse(format!(
            "XUI18 composed element {name:?} is not unique"
        )));
    };
    if rect.w <= 0 || rect.h <= 0 {
        return Err(GameUiError::parse(format!(
            "XUI18 composed element {name:?} has invalid geometry"
        )));
    }
    Ok((
        PowerBalanceInputRegion {
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
        },
        rect.child_path.clone(),
    ))
}

fn find_named_mut<'a>(root: &'a mut GuiElement, name: &str) -> Option<&'a mut GuiElement> {
    if root.name.eq_ignore_ascii_case(name) {
        return Some(root);
    }
    root.children
        .iter_mut()
        .find_map(|child| find_named_mut(child, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_geometry_uses_effective_authored_bar_and_offsets() {
        let region = |x, y, width, height| PowerBalanceInputRegion {
            x,
            y,
            width,
            height,
        };
        let (marker, splitters) = derive_dynamic_positions(
            region(500, 20, 700, 600),
            region(625, 170, 512, 20),
            region(483, 11, 24, 28),
            region(491, 14, 12, 8),
            0.25,
            [0.0, 0.5, 1.0],
        );
        assert_eq!(marker, [236, 141]);
        assert_eq!(splitters, [[116, 144], [372, 144], [628, 144]]);
    }

    #[test]
    fn input_regions_are_half_open() {
        let region = PowerBalanceInputRegion {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        assert!(region.contains(10, 20));
        assert!(region.contains(39, 59));
        assert!(!region.contains(40, 59));
        assert!(!region.contains(39, 60));
    }

    #[test]
    fn decision_progress_chrome_is_hidden_without_exact_controller_authority() {
        let mut options = ComposeOptions::new(1920, 1080);
        hide_unauthenticated_decision_progress(&mut options, "xui18_decision_4_");
        assert!(options
            .hidden_elements
            .contains("xui18_decision_4_btn_progress_good"));
        assert!(options
            .hidden_elements
            .contains("xui18_decision_4_btn_progress_bad"));
        assert!(!options
            .progress_overrides
            .contains_key("xui18_decision_4_btn_progress_good"));
        assert!(!options
            .progress_overrides
            .contains_key("xui18_decision_4_btn_progress_bad"));
    }
}
