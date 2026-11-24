use super::{
    ComposedPlan, ComposedSubWindow, DEFAULT_VIEWPORT, GameWindowError, GameWindowRegion,
    GameWindowRender, GameWindowRequest, GameWindowTooltip, Prepared, RenderParams, Service,
    WindowCacheKey,
    compose::{
        collect_template_assets, find_direct, hide_by_name, loc_or, set_frame_by_name,
        set_text_by_name, spell,
    },
    equipment_designer::{RenderContext, compute_read_hour, witnessed_rows, witnessed_single_key},
    layout_window, missing_window_refusal,
};
use content_reader_gui::gui::native::{self, Element};
use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::{Argument, Localisation};
use content_reader_military::intelligence::{self, AgencyUpgrades, Intelligence};
use content_reader_parser::parser::c_string;
use dashboard_text::{catalogue::View, metadata::link_slot};
use replay::campaign::SeekResult;
use replay_data::rows::{AgencyUpgradeLevelRow, CountryLinkRow, CountryRow, LexicalTokenRow, TableId, tables};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use viewer_native_gui::native_gui::{self, Assets, Holder, Layout, Resources, lifecycle::Instance};

#[cfg(test)]
mod tests;

const WINDOW_NAME: &str = "countryintelligenceagencyview";
const BRANCH_TEMPLATE: &str = "upgrade_button";
const PIP_TEMPLATE: &str = "agency_upgrade_researched_icon";
const OPERATION_TEMPLATE: &str = "operation_view_entry";
const NOT_RECORDED_VALUE: &str = "n/a";
const AUTHENTICATED_IDENTITY: u8 = 0;
const PIP_FRAME_LEVELS: usize = 4;

const REQUIRED_TABLES: &[TableId] = &[
    TableId::IntelligenceAgencies,
    TableId::AgencyUpgradeLevels,
    TableId::Operations,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
];

#[derive(Deserialize)]
struct Target {
    country: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipState {
    Level(i32),
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pip {
    pub upgrade: String,
    pub levels: usize,
    pub state: PipState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub token: String,
    pub pips: Vec<Pip>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningOperation {
    pub token: Option<String>,
    pub target: Option<String>,
    pub preparing: bool,
    pub duration_days: i32,
}

#[derive(Clone, Debug)]
pub struct AgencyView {
    pub tag: Option<String>,
    pub name: Result<String, String>,
    pub defense_raw: i64,
    pub active: i32,
    pub maximum: i32,
    pub branches: Result<Vec<Branch>, String>,
    pub operations: Result<Vec<RunningOperation>, String>,
    pub read_hour: i32,
    pub warnings: Vec<String>,
}

pub fn pip_frame(levels: usize, level: i32) -> Option<i32> {
    if !(1..=PIP_FRAME_LEVELS).contains(&levels) {
        return None;
    }
    let first = (levels * (levels + 1) / 2) as i32;
    Some(first + level.clamp(0, levels as i32))
}

pub struct RecordedLevel {
    pub branch: Option<String>,
    pub upgrade: Option<String>,
    pub level: i32,
    pub authenticated: bool,
}

pub fn branches(
    upgrades: &AgencyUpgrades,
    recorded: Result<&[RecordedLevel], String>,
) -> Vec<Branch> {
    let recorded = recorded.map(|rows| {
        let unplaced = rows
            .iter()
            .filter(|row| !row.authenticated || row.branch.is_none() || row.upgrade.is_none())
            .count();
        (rows, unplaced)
    });
    upgrades
        .branches
        .iter()
        .map(|(branch, tokens)| Branch {
            token: branch.clone(),
            pips: tokens
                .iter()
                .map(|token| {
                    let levels = upgrades
                        .branch_definitions
                        .get(&(branch.clone(), token.clone()))
                        .map_or(0, |definition| definition.levels);
                    let state = match &recorded {
                        Err(why) => PipState::Unknown(format!("Not recorded: acquired agency upgrades are not recorded at this hour ({why}).")),
                        Ok((rows, unplaced)) => {
                            let found = rows.iter().find(|row| {
                                row.authenticated
                                    && row.branch.as_deref().is_some_and(|kept| kept.eq_ignore_ascii_case(branch))
                                    && row.upgrade.as_deref().is_some_and(|kept| kept.eq_ignore_ascii_case(token))
                            });
                            match (found, unplaced) {
                                (Some(row), _) => PipState::Level(row.level),
                                (None, 0) => PipState::Level(0),
                                (None, count) => PipState::Unknown(format!(
                                    "Not recorded: {count} acquired upgrade(s) of this agency could not be identified, so whether this one is among them is unknown."
                                )),
                            }
                        }
                    };
                    Pip { upgrade: token.clone(), levels, state }
                })
                .collect(),
        })
        .collect()
}

fn cached_upgrades(layers: &ResolvedLayers) -> Result<Arc<AgencyUpgrades>, String> {
    #[derive(Default)]
    struct Cached(content_reader_layers::content::diagnostics::Once<Result<Arc<AgencyUpgrades>, String>>);
    layers
        .derived::<Cached>()
        .0
        .get_or_init(|| {
            intelligence::read_agency_upgrades(layers)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
}

fn cached_intelligence(layers: &ResolvedLayers) -> Result<Arc<Intelligence>, String> {
    #[derive(Default)]
    struct Cached(content_reader_layers::content::diagnostics::Once<Result<Arc<Intelligence>, String>>);
    layers
        .derived::<Cached>()
        .0
        .get_or_init(|| intelligence::read(layers).map(Arc::new).map_err(|error| error.to_string()))
        .clone()
}

fn country_tag(countries: &[CountryRow], links: &[CountryLinkRow], raw: u32) -> Option<String> {
    let slot = link_slot(links, raw)?;
    let country = countries.iter().find(|entry| entry.canonical_slot == slot)?;
    std::str::from_utf8(&country.tag)
        .ok()
        .filter(|tag| !tag.is_empty() && !tag.contains('\0'))
        .map(str::to_owned)
}

fn shown(localisation: &Localisation, key: &str, warnings: &mut Vec<String>) -> String {
    if localisation.get(key).is_none() {
        warnings.push(format!("Localisation key {key} is not in the recorded mod stack: the key is shown."));
    }
    loc_or(localisation, key)
}

fn not_recorded(title: &str, why: impl Into<String>) -> GameWindowTooltip {
    GameWindowTooltip { title: title.to_owned(), body: Some(why.into()), detail: None }
}

pub fn render(
    service: &Service,
    ctx: RenderContext<'_>,
    request: GameWindowRequest,
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
    let RenderContext {
        source,
        content: _,
        layers,
        resolve,
        visual_revision,
        language,
        decode,
        admitted_gen,
        mut extra_warnings,
    } = ctx;
    let target: Target = serde_json::from_value(request.target.clone())
        .map_err(|error| GameWindowError::refused(format!("Invalid intelligence agency target: {error}")))?;
    let branch_id: u64 = request
        .branch_id
        .parse()
        .map_err(|_| GameWindowError::refused(format!("Invalid branch id '{}'.", request.branch_id)))?;
    let frame = match source
        .replay_ref()
        .seek_selected(branch_id, request.hour, Some(REQUIRED_TABLES))
        .map_err(|error| GameWindowError::failed(format!("Could not seek recording to hour {}: {error}", request.hour)))?
    {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => {
            return Err(GameWindowError::refused(format!(
                "Hour {} is not recorded on this branch: {reason}",
                request.hour
            )));
        }
    };
    let agency = witnessed_single_key::<tables::IntelligenceAgencies, _>(&frame, target.country, |row| row.country)
        .map_err(|error| GameWindowError::refused(format!("Intelligence agencies are not recorded at this hour: {error}")))?
        .ok_or_else(|| GameWindowError::refused(format!("No intelligence agency is recorded for country {} at hour {}.", target.country, request.hour)))?;
    if agency.created == 0 {
        return Err(GameWindowError::refused(format!(
            "Country {}'s intelligence agency is not created at hour {}.",
            target.country, request.hour
        )));
    }
    let mut read_tables = vec![TableId::IntelligenceAgencies];
    let lexical: &[LexicalTokenRow] = match witnessed_rows::<tables::LexicalTokens>(&frame) {
        Ok((rows, _)) => {
            read_tables.push(TableId::LexicalTokens);
            rows
        }
        Err(why) => {
            extra_warnings.push(format!("Lexical tokens are not recorded at this hour ({why}): upgrade and operation names are unknown."));
            &[]
        }
    };
    let identity = match (frame.complete::<tables::Countries>(), frame.complete::<tables::CountryLinks>()) {
        (Ok(countries), Ok(links)) => {
            read_tables.extend([TableId::Countries, TableId::CountryLinks]);
            Some((countries, links))
        }
        (countries, links) => {
            let why = countries.err().or(links.err()).map(|error| error.to_string()).unwrap_or_default();
            extra_warnings.push(format!("Countries are not recorded at this hour ({why}): the agency's country and operation targets are unknown."));
            None
        }
    };
    let tag = identity.and_then(|(countries, links)| country_tag(countries, links, target.country));
    let name = std::str::from_utf8(c_string(&agency.name))
        .ok()
        .and_then(|name| dashboard_base::catalogue::plain_text(name).ok())
        .ok_or_else(|| "Not recorded: the stored agency name has bytes that are not text.".to_owned());

    let branches = match cached_upgrades(layers) {
        Err(why) => Err(format!("Not recorded: the agency upgrade definitions could not be read from the recorded mod stack ({why}).")),
        Ok(upgrades) => {
            extra_warnings.extend(upgrades.warnings.iter().cloned());
            let recorded = frame.complete::<tables::AgencyUpgradeLevels>().map_err(|error| error.to_string());
            if recorded.is_ok() {
                read_tables.push(TableId::AgencyUpgradeLevels);
            }
            let rows: Result<Vec<RecordedLevel>, String> = recorded.map(|rows| {
                rows.iter()
                    .filter(|row: &&AgencyUpgradeLevelRow| row.country == target.country)
                    .map(|row| RecordedLevel {
                        branch: row.branch_token.and_then(|token| spell(lexical, token)),
                        upgrade: row.upgrade_token.and_then(|token| spell(lexical, token)),
                        level: row.level,
                        authenticated: row.identity_status == AUTHENTICATED_IDENTITY,
                    })
                    .collect()
            });
            let laid = branches(&upgrades, rows.as_deref().map_err(Clone::clone));
            if let Ok(rows) = &rows {
                for row in rows.iter().filter(|row| row.authenticated) {
                    if let (Some(branch), Some(upgrade)) = (row.branch.as_deref(), row.upgrade.as_deref())
                        && !upgrades.branches.iter().any(|(kept, tokens)| kept.eq_ignore_ascii_case(branch) && tokens.iter().any(|token| token.eq_ignore_ascii_case(upgrade)))
                    {
                        extra_warnings.push(format!("Acquired upgrade {branch}/{upgrade} is not defined in the recorded mod stack, so the branches do not show it."));
                    }
                }
            }
            Ok(laid)
        }
    };

    let operations = match frame.complete::<tables::Operations>() {
        Ok(rows) => {
            read_tables.push(TableId::Operations);
            Ok(rows
                .iter()
                .filter(|row| row.owner == target.country && row.terminal == 0)
                .map(|row| RunningOperation {
                    token: spell(lexical, row.definition_token),
                    target: identity.and_then(|(countries, links)| country_tag(countries, links, row.target)),
                    preparing: row.preparation_current < row.preparation_required,
                    duration_days: row.duration_days,
                })
                .collect())
        }
        Err(error) => Err(format!("Not recorded: the agency's operations are not recorded at this hour ({error}).")),
    };

    let samples: Vec<(TableId, i32)> = read_tables
        .iter()
        .filter_map(|&id| frame.tables[id.index()].sampled_at().map(|hour| (id, hour)))
        .collect();
    let (read_hour, cadence_warning) = compute_read_hour(request.hour, &samples);
    extra_warnings.extend(cadence_warning);

    let view = AgencyView {
        tag,
        name,
        defense_raw: agency.defense_raw,
        active: agency.active_count,
        maximum: agency.unknown_f0 as i32,
        branches,
        operations,
        read_hour,
        warnings: extra_warnings,
    };
    let key = WindowCacheKey {
        source_id: request.source_id,
        revision: request.revision,
        game_dir: resolve.game_dir.clone(),
        user_dir: resolve.user_dir.clone(),
        visual_revision: visual_revision.to_owned(),
        language: language.to_owned(),
        force_pow2_textures: decode.force_pow2_textures,
        art_stack: layers.art_stack_key(),
        window: WINDOW_NAME.to_owned(),
    };
    render_view(service, layers, key, view, request.scale, &request.scroll, Some(admitted_gen), cancel)
}

fn fingerprint(view: &AgencyView) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("{view:?}").as_bytes()).into()
}

pub fn render_view(
    service: &Service,
    layers: &ResolvedLayers,
    key: WindowCacheKey,
    view: AgencyView,
    scale: f32,
    scroll: &BTreeMap<String, f32>,
    admitted_gen: Option<(u64, u64)>,
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
    let operations_content = cached_intelligence(layers);
    let mut warnings = view.warnings.clone();
    if let Err(why) = &operations_content {
        warnings.push(format!("Operation definitions could not be read from the recorded mod stack ({why}): operation names and pictures are unknown."));
    }
    let print = fingerprint(&view);
    service.render_composed_from_layers(
        key,
        layers,
        RenderParams {
            window_name: WINDOW_NAME,
            scale,
            read_hour: Some(view.read_hour),
            art_from_current_install: true,
            close_paths: &["close_button"],
            tooltip_overrides: &BTreeMap::new(),
            extra_warnings: warnings,
            admitted_gen,
            scroll,
        },
        cancel,
        |shared| {
            let mut assets = Assets::default();
            collect_template_assets(
                shared.guis.as_ref(),
                shared.localisation.as_ref(),
                &[BRANCH_TEMPLATE, PIP_TEMPLATE, OPERATION_TEMPLATE],
                &mut assets,
            );
            if let (Ok(content), Ok(operations)) = (&operations_content, &view.operations) {
                for operation in operations {
                    if let Some(sprite) = operation
                        .token
                        .as_deref()
                        .and_then(|token| content.operation(token).ok().flatten())
                        .map(|definition| definition.sprite.as_str())
                        .filter(|sprite| !sprite.is_empty() && shared.registry.sprite(sprite.as_bytes()).is_some())
                    {
                        assets.sprites.insert(sprite.as_bytes().to_vec());
                    }
                }
            }
            Ok(Prepared { assets, state: (), fingerprint: Some(print) })
        },
        |root, _, _, localisation, instance, tooltips, ()| {
            configure_main(root, localisation, instance, tooltips, &view);
        },
        |plan, main_layout, root, guis, resources, localisation, ()| {
            let sources = Sources { guis, resources, localisation };
            compose_branches(plan, main_layout, root, &sources, &view)?;
            compose_operations(plan, main_layout, root, &sources, &view, operations_content.as_deref().ok())?;
            plan.title = Some(view.name.clone().unwrap_or_else(|_| NOT_RECORDED_VALUE.to_owned()));
            plan.read_hour = Some(view.read_hour);
            Ok(())
        },
    )
}

fn tooltip_at(root: &Element, path: &str, tip: GameWindowTooltip, tooltips: &mut BTreeMap<String, GameWindowTooltip>) {
    if let Some(element) = find_direct(root, path) {
        tooltips.insert(element.path.clone(), tip);
    }
}

fn configure_main(
    root: &Element,
    localisation: &Localisation,
    instance: &mut Instance,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
    view: &AgencyView,
) {
    let mut ignored = Vec::new();
    match &view.name {
        Ok(name) => set_text_by_name(instance, root, "name_logo/agency_name", name.clone()),
        Err(why) => {
            set_text_by_name(instance, root, "name_logo/agency_name", NOT_RECORDED_VALUE.to_owned());
            tooltip_at(root, "name_logo/Background", not_recorded(NOT_RECORDED_VALUE, why.clone()), tooltips);
        }
    }
    let defense = dashboard_text::catalogue::localised_with(
        localisation,
        "AGENCY_DEFENSE_LEVEL",
        &[("LEVEL", Argument::Fixed5(view.defense_raw))],
        View::Plain,
    )
    .unwrap_or_else(|_| shown(localisation, "AGENCY_DEFENSE_LEVEL", &mut ignored));
    set_text_by_name(instance, root, "name_logo/agency_defense_level", defense);

    hide_by_name(instance, root, "name_logo/agency_logo");
    tooltip_at(root, "name_logo/agency_logo_bg", not_recorded(NOT_RECORDED_VALUE, "Not recorded: the emblem chosen for the agency is not recorded."), tooltips);
    for name in ["spymaster_title", "spymaster_button", "faction_button", "spymaster_button_name", "spymaster_button_cost", "spymaster_flag"] {
        hide_by_name(instance, root, &format!("name_logo/spymaster/{name}"));
    }
    tooltip_at(root, "name_logo/spymaster/spymaster_icon", not_recorded(NOT_RECORDED_VALUE, "Not recorded: whether this country is its faction's spymaster is not recorded."), tooltips);

    for name in ["before_creation_panel", "agency_branches/upgrades_before_agency", "agency_agents/operatives_before_agency", "agency_branches/ai_factory_allocation", "agency_crypto"] {
        hide_by_name(instance, root, name);
    }

    set_text_by_name(instance, root, "agency_agents/total_operative_text", format!("{}/{}", view.active, view.maximum));
    for (text, icon) in [("arrested_operative_text", "arrested_operative_icon"), ("dead_operative_text", "dead_operative_icon")] {
        set_text_by_name(instance, root, &format!("agency_agents/{text}"), NOT_RECORDED_VALUE.to_owned());
        tooltip_at(root, &format!("agency_agents/{icon}"), not_recorded(NOT_RECORDED_VALUE, "Not recorded: this count of the agency's operatives is not recorded."), tooltips);
    }
    for (bar, why) in [
        ("upgrade_progressbar", "Not recorded: which upgrade the agency is working on, and its progress, are not recorded."),
        ("operatives_progressbar", "Not recorded: whether an operative is being recruited, and its progress, are not recorded."),
    ] {
        hide_by_name(instance, root, &format!("agency_agents/{bar}/progressbar_content"));
        hide_by_name(instance, root, &format!("agency_agents/{bar}/progressbar_days"));
        hide_by_name(instance, root, &format!("agency_agents/{bar}/cancel_upgrade"));
        set_text_by_name(instance, root, &format!("agency_agents/{bar}/progressbar_desc"), NOT_RECORDED_VALUE.to_owned());
        tooltip_at(root, &format!("agency_agents/{bar}/progressbar_frame"), not_recorded(NOT_RECORDED_VALUE, why), tooltips);
    }

    hide_by_name(instance, root, "agency_operations/show_unavailable_operations");
    hide_by_name(instance, root, "agency_operations/show_unavailable_operations_label");
    tooltip_at(root, "agency_operations/operations_header", not_recorded(NOT_RECORDED_VALUE, "Only the agency's running operations are recorded: the operations it could start are not, so they are not listed."), tooltips);
    match &view.operations {
        Ok(running) if !running.is_empty() => hide_by_name(instance, root, "agency_operations/operations_grid_container/operations_not_active"),
        Ok(_) => {
            set_text_by_name(instance, root, "agency_operations/operations_grid_container/operations_not_active", NOT_RECORDED_VALUE.to_owned());
            tooltip_at(root, "agency_operations/operations_grid_container/operations_not_active", not_recorded(NOT_RECORDED_VALUE, "The agency runs no operation at this hour; the operations it could start are not recorded."), tooltips);
        }
        Err(why) => {
            set_text_by_name(instance, root, "agency_operations/operations_grid_container/operations_not_active", NOT_RECORDED_VALUE.to_owned());
            tooltip_at(root, "agency_operations/operations_grid_container/operations_not_active", not_recorded(NOT_RECORDED_VALUE, why.clone()), tooltips);
        }
    }
    tooltip_at(root, "crypto_tab_button", not_recorded(NOT_RECORDED_VALUE, "Not shown: the cryptology view is not drawn in this window; the Decryption widget shows each cipher's progress."), tooltips);
}

struct Sources<'a> {
    guis: &'a native::Guis,
    resources: &'a dyn Resources,
    localisation: &'a Localisation,
}

fn holder_for<'a>(layout: &'a Layout, element: &Element) -> Option<&'a Holder> {
    layout.holders.iter().find(|holder| holder.path == element.path)
}

fn grid_places(grid: &Element, holder: &Holder, items: &[&Element]) -> Result<Vec<[i32; 2]>, GameWindowError> {
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    native_gui::finalize_grid(grid, holder.content, viewport, items, holder.position)
        .map(|laid| laid.places())
        .map_err(|refusal| GameWindowError::failed(refusal.to_string()))
}

fn compose_branches(
    plan: &mut ComposedPlan,
    main_layout: &Layout,
    root: &Element,
    sources: &Sources<'_>,
    view: &AgencyView,
) -> Result<(), GameWindowError> {
    let Sources { guis, resources, localisation } = *sources;
    let grid = find_direct(root, "agency_branches/upgrades_list")
        .ok_or_else(|| GameWindowError::refused(format!("Window '{WINDOW_NAME}' has no 'agency_branches/upgrades_list'.")))?;
    let Some(holder) = holder_for(main_layout, grid) else { return Ok(()) };
    let branches = match &view.branches {
        Ok(branches) => branches,
        Err(why) => {
            plan.extra_regions.push(GameWindowRegion {
                path: grid.path.clone(),
                rect: holder_rect(holder),
                clip: None,
                disabled: true,
                close: false,
                passthrough: false,
                view: None,
                tooltip: Some(not_recorded(NOT_RECORDED_VALUE, why.clone())),
                alpha_mask: None,
                scroll: None,
            });
            return Ok(());
        }
    };
    let entry = guis.template(BRANCH_TEMPLATE.as_bytes()).ok_or_else(|| GameWindowError::refused(missing_window_refusal(BRANCH_TEMPLATE)))?;
    let pip = guis.template(PIP_TEMPLATE.as_bytes()).ok_or_else(|| GameWindowError::refused(missing_window_refusal(PIP_TEMPLATE)))?;
    let pip_grid = find_direct(entry, "researched_icons");
    let places = grid_places(grid, holder, &vec![entry; branches.len()])?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    for (index, (branch, place)) in branches.iter().zip(places).enumerate() {
        let title = shown(localisation, &branch.token, &mut plan.warnings);
        let layout = layout_window(entry, resources, viewport, |instance| {
            set_text_by_name(instance, entry, "agency_branches_title", title.clone());
            hide_by_name(instance, entry, "opportunities");
        })?;
        let mut lines = Vec::new();
        for pip_state in &branch.pips {
            let name = shown(localisation, &pip_state.upgrade, &mut plan.warnings);
            lines.push(match &pip_state.state {
                PipState::Level(level) => format!("{name}: {level} / {}", pip_state.levels),
                PipState::Unknown(_) => format!("{name}: {NOT_RECORDED_VALUE}"),
            });
        }
        let mut overrides = BTreeMap::new();
        if let Some(button) = find_direct(entry, "background_button") {
            overrides.insert(button.path.clone(), GameWindowTooltip { title: title.clone(), body: Some(lines.join("\n")), detail: Some(branch.token.clone()) });
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place(place, &layout),
            regions: holder.place_regions(place, &layout.regions),
            diagnostics: layout.diagnostics.clone(),
            root_window: BRANCH_TEMPLATE.to_owned(),
            path_prefix: format!("branch:{index}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
        let (Some(pip_grid), Some(pip_holder)) = (pip_grid, pip_grid.and_then(|grid| holder_for(&layout, grid))) else {
            continue;
        };
        let pip_places = grid_places(pip_grid, pip_holder, &vec![pip; branch.pips.len()])?;
        for (pip_index, (pip_state, pip_place)) in branch.pips.iter().zip(pip_places).enumerate() {
            let name = shown(localisation, &pip_state.upgrade, &mut plan.warnings);
            let (frame, tip) = match &pip_state.state {
                PipState::Level(level) => match pip_frame(pip_state.levels, *level) {
                    Some(frame) => (Some(frame), GameWindowTooltip { title: name, body: Some(format!("{level} / {}", pip_state.levels)), detail: Some(pip_state.upgrade.clone()) }),
                    None => (None, not_recorded(&name, format!(
                        "Not shown: this upgrade has {} levels and the game's level icon has frames for one to {PIP_FRAME_LEVELS}; its recorded level is {level}.",
                        pip_state.levels
                    ))),
                },
                PipState::Unknown(why) => (None, not_recorded(&name, why.clone())),
            };
            let pip_layout = layout_window(pip, resources, viewport, |instance| {
                hide_by_name(instance, pip, "agency_upgrade_anim");
                match frame {
                    Some(frame) => set_frame_by_name(instance, pip, "upgrade_icon", frame),
                    None => hide_by_name(instance, pip, "upgrade_icon"),
                }
            })?;
            let inner = pip_holder.place(pip_place, &pip_layout);
            let parts = holder.place_parts(place, &inner);
            let regions = holder.place_regions(place, &pip_holder.place_regions(pip_place, &pip_layout.regions));
            let mut overrides = BTreeMap::new();
            if let Some(button) = find_direct(pip, "upgrade_icon") {
                overrides.insert(button.path.clone(), tip.clone());
            }
            if frame.is_none() {
                let origin = [
                    holder.origin[0] + place[0] as f32 + pip_holder.origin[0] + pip_place[0] as f32,
                    holder.origin[1] + place[1] as f32 + pip_holder.origin[1] + pip_place[1] as f32,
                ];
                let size = pip.container().map_or([0, 0], |container| [container.size.size[0].value, container.size.size[1].value]);
                plan.extra_regions.push(GameWindowRegion {
                    path: format!("branch:{index}:pip:{pip_index}"),
                    rect: [origin[0], origin[1], size[0] as f32, size[1] as f32],
                    clip: None,
                    disabled: true,
                    close: false,
                    passthrough: false,
                    view: None,
                    tooltip: Some(tip),
                    alpha_mask: None,
                    scroll: None,
                });
            }
            plan.sub_windows.push(ComposedSubWindow {
                before_part: holder.before,
                parts,
                regions,
                diagnostics: pip_layout.diagnostics,
                root_window: PIP_TEMPLATE.to_owned(),
                path_prefix: format!("branch:{index}:pip:{pip_index}/"),
                close_paths: Vec::new(),
                tooltip_overrides: overrides,
            });
        }
    }
    Ok(())
}

fn holder_rect(holder: &Holder) -> [f32; 4] {
    match holder.clip {
        Some([left, top, right, bottom]) => [left as f32, top as f32, (right - left).max(0) as f32, (bottom - top).max(0) as f32],
        None => [holder.origin[0], holder.origin[1], holder.content[0] as f32, holder.content[1] as f32],
    }
}

fn compose_operations(
    plan: &mut ComposedPlan,
    main_layout: &Layout,
    root: &Element,
    sources: &Sources<'_>,
    view: &AgencyView,
    content: Option<&Intelligence>,
) -> Result<(), GameWindowError> {
    let Sources { guis, resources, localisation } = *sources;
    let Ok(running) = &view.operations else { return Ok(()) };
    if running.is_empty() {
        return Ok(());
    }
    let grid = find_direct(root, "agency_operations/operations_grid_container/operations_grid")
        .ok_or_else(|| GameWindowError::refused(format!("Window '{WINDOW_NAME}' has no 'agency_operations/operations_grid_container/operations_grid'.")))?;
    let Some(holder) = holder_for(main_layout, grid) else { return Ok(()) };
    let entry = guis.template(OPERATION_TEMPLATE.as_bytes()).ok_or_else(|| GameWindowError::refused(missing_window_refusal(OPERATION_TEMPLATE)))?;
    let places = grid_places(grid, holder, &vec![entry; running.len()])?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    for (index, (operation, place)) in running.iter().zip(places).enumerate() {
        let definition = operation.token.as_deref().and_then(|token| content.and_then(|content| content.operation(token).ok().flatten()));
        let name = match (operation.token.as_deref(), definition) {
            (_, Some(definition)) => shown(localisation, &definition.name, &mut plan.warnings),
            (Some(token), None) => shown(localisation, token, &mut plan.warnings),
            (None, None) => NOT_RECORDED_VALUE.to_owned(),
        };
        let sprite = definition.map(|definition| definition.sprite.as_str()).filter(|sprite| !sprite.is_empty() && resources.sprite_info(sprite.as_bytes()).is_some());
        let status = shown(localisation, if operation.preparing { "OPERATION_VIEW_COLLECTING" } else { "OPERATION_VIEW_PROGRESS" }, &mut plan.warnings);
        let layout = layout_window(entry, resources, viewport, |instance| {
            set_text_by_name(instance, entry, "container_operation_info/operation_name_text", name.clone());
            for hidden in [
                "container_operation_info/operation_ai_score",
                "container_operation_info/operation_prepared_icon",
                "container_operation_info/operation_prepared_remaining_days_text",
                "container_operation_info/operation_completed_icon",
                "container_operation_info/operation_completed_warning",
                "container_operation_info/btn_target_flag",
                "container_operation_info/container_tokens",
            ] {
                hide_by_name(instance, entry, hidden);
            }
            match sprite {
                Some(sprite) => {
                    if let Some(icon) = find_direct(entry, "container_operation_info/operation_icon")
                        && let Ok(bound) = instance.control(entry, icon)
                    {
                        bound.sprite = Some(sprite.as_bytes().to_vec());
                    }
                }
                None => hide_by_name(instance, entry, "container_operation_info/operation_icon"),
            }
            set_text_by_name(instance, entry, "container_operation_info/container_operatives_required/operatives_required_text", NOT_RECORDED_VALUE.to_owned());
            set_text_by_name(instance, entry, "container_operation_info/container_network_strength/network_strength_text", NOT_RECORDED_VALUE.to_owned());
            set_text_by_name(instance, entry, "container_operation_info/operation_duration_text", operation.duration_days.to_string());
            set_text_by_name(instance, entry, "container_operation_info/container_launch_operation/btn_select_button", status.clone());
        })?;
        let mut overrides = BTreeMap::new();
        if let Some(background) = find_direct(entry, "operation_bg") {
            let target = operation.target.clone().unwrap_or_else(|| NOT_RECORDED_VALUE.to_owned());
            overrides.insert(background.path.clone(), GameWindowTooltip {
                title: name.clone(),
                body: Some(format!("{target}\n{}", "Not recorded: the operatives it needs and the network strength it uses.")),
                detail: operation.token.clone(),
            });
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place(place, &layout),
            regions: holder.place_regions(place, &layout.regions),
            diagnostics: layout.diagnostics,
            root_window: OPERATION_TEMPLATE.to_owned(),
            path_prefix: format!("operation:{index}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
    }
    Ok(())
}
