use super::{
    ComposedPlan, ComposedSubWindow, DEFAULT_VIEWPORT, GameWindowError, GameWindowRender,
    GameWindowRequest, GameWindowTooltip, Prepared, RenderParams, Service, WindowCacheKey,
    compose::{
        collect_template_assets, disable_by_name, find_direct, hide_by_name, set_frame_by_name,
        set_sprite_by_name, set_text_by_name,
    },
    blank,
    equipment_designer::{RenderContext, compute_read_hour},
    layout_window, missing_window_refusal,
};
use content_reader_gui::gui::native::{self, Element};
use content_reader_localisation::localisation::Localisation;
use content_reader_military::intelligence::AgencyUpgrades;
use dashboard_base::source::Work;
use dashboard_core::config::LoadContext;
use dashboard_intelligence::products::intelligence::{
    agencies::{self, Agency, Upgrades},
    agency_content::{self, Shown},
    espionage::{self, OperationUnderWay},
    operative_content::{self, Art, OperativeContent},
};
use dashboard_text::catalogue::View;
use replay_data::rows::{TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::atomic::AtomicBool,
};
use viewer_native_gui::{
    native_effects::Effect,
    native_gui::{self, AlphaMask, AnimationRecord, Assets, Holder, Layout, Resources, SpriteInfo, Texture, TextureFile, lifecycle::Instance},
    native_text::Font,
};

#[cfg(test)]
mod tests;

const WINDOW_NAME: &str = "countryintelligenceagencyview";
const BRANCH_TEMPLATE: &str = "upgrade_button";
const PIP_TEMPLATE: &str = "agency_upgrade_researched_icon";
const OPERATION_TEMPLATE: &str = "operation_view_entry";
const OPERATION_TARGET_FLAG: &str = "container_operation_info/btn_target_flag";
const TARGET_FLAG_LAYOUT_INDEX: i32 = i32::MIN;
const UNBOUND_TARGET_FLAG: &str = "No flag atlas binding is available for this target country.";
const NOT_RECORDED_VALUE: &str = "n/a";
const FULL_PREPARATION: i32 = 100_000;

const REQUIRED_TABLES: &[TableId] = &[
    TableId::IntelligenceAgencies,
    TableId::AgencyUpgradeLevels,
    TableId::Operations,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
];

const EMBLEM: &str = "Agency emblem";
const EMBLEM_ASSET: &str = "the emblem chosen for this agency";
const SPYMASTER: &str = "Spymaster";
const SPYMASTER_ASSET: &str = "the spymaster badge";
const NOT_IN_RECORDING: &str = "not recorded in this recording";
const ARRESTED: &str = "Arrested operatives";
const DEAD: &str = "Dead operatives";
const COUNT_NOT_RECORDED: &str = "Not recorded: this count of the agency's operatives is not recorded.";
const RECRUITING: &str = "Operative recruitment";
const RECRUITING_NOT_RECORDED: &str = "Not recorded: whether an operative is being recruited, and its progress, are not recorded.";
const UPGRADING: &str = "Upgrade in progress";
const UPGRADING_NOT_RECORDED: &str = "Not recorded: the recording holds the agency's upgrade progress but not which upgrade it is, so the bar has no fill and no remaining days.";
const NETWORK_NOT_RECORDED: &str = "Not recorded: the network strength an operation needs is not recorded.";
const CRYPTOLOGY_DISABLED: &str = "This view is unavailable in the agency window because its cipher thresholds are unavailable. The Decryption widget shows each cipher's progress.";
const NO_RUNNING_OPERATIONS: &str = "No running operations recorded at this hour.";
const STARTABLE_NOT_RECORDED: &str = "Only the agency's running operations are recorded: the operations it could start are not, so they are not listed.";

#[derive(Deserialize)]
struct Target {
    country: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pip {
    pub token: String,
    pub name: Shown,
    pub levels: usize,
    pub level: Result<i32, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Branch {
    pub token: String,
    pub name: Shown,
    pub pips: Vec<Pip>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Operation {
    token: Option<String>,
    name: Shown,
    sprite: Result<String, String>,
    target: Option<String>,
    required: u32,
    duration_days: i32,
    preparing: bool,
}

#[derive(Clone, Debug, Serialize)]
struct AgencyView {
    agency: Agency,
    branches: Result<Vec<Branch>, String>,
    operations: Result<Vec<Operation>, String>,
    upgrade_progress: String,
    read_hour: i32,
    warnings: Vec<String>,
}

pub fn pip_frame(frames: i32, levels: usize, level: i32) -> Option<i32> {
    let levels = i32::try_from(levels).ok().filter(|levels| *levels > 0)?;
    let first = levels.checked_mul(levels + 1)? / 2;
    (first + levels <= frames).then(|| first + level.clamp(0, levels))
}

pub fn branches(
    upgrades: &AgencyUpgrades,
    recorded: Result<&Upgrades, &str>,
    branch_name: impl Fn(&str) -> Shown,
    upgrade_name: impl Fn(&str, &str) -> Shown,
) -> Vec<Branch> {
    let unplaced = recorded.map(|held| {
        let unidentified = held.items.iter().filter(|item| item.branch.is_none() || item.token.is_none()).count();
        (held, unidentified, held.unavailable.as_deref())
    });
    upgrades
        .branches
        .iter()
        .map(|(branch, tokens)| Branch {
            token: branch.clone(),
            name: branch_name(branch),
            pips: tokens
                .iter()
                .map(|token| {
                    let levels = upgrades.branch_definitions.get(&(branch.clone(), token.clone())).map_or(0, |definition| definition.levels);
                    let level = match unplaced {
                        Err(why) => Err(format!("Not recorded: acquired agency upgrades are not recorded at this hour ({why}).")),
                        Ok((held, unidentified, unavailable)) => {
                            let found = held.items.iter().find(|item| {
                                item.branch.as_deref().is_some_and(|kept| kept.eq_ignore_ascii_case(branch))
                                    && item.token.as_deref().is_some_and(|kept| kept.eq_ignore_ascii_case(token))
                                    && (unavailable.is_none() || item.name.is_some())
                            });
                            match (found, unidentified, unavailable) {
                                (Some(item), ..) => Ok(item.level),
                                (None, 0, None) => Ok(0),
                                (None, _, Some(why)) => Err(format!("Not recorded: {why}, so whether this upgrade is among them is unknown.")),
                                (None, count, None) => Err(format!(
                                    "Not recorded: {count} acquired upgrade(s) of this agency could not be identified, so whether this one is among them is unknown."
                                )),
                            }
                        }
                    };
                    Pip { token: token.clone(), name: upgrade_name(branch, token), levels, level }
                })
                .collect(),
        })
        .collect()
}

fn operation(operation: &OperationUnderWay, content: Result<&OperativeContent, &str>) -> Operation {
    let (name, sprite) = match (operation.operation.as_deref(), content) {
        (None, _) => {
            let why = "Not recorded: the operation's definition token has no recorded text.".to_owned();
            (Err(why.clone()), Err(why))
        }
        (Some(_), Err(why)) => (Err(why.to_owned()), Err(why.to_owned())),
        (Some(token), Ok(content)) => {
            let (name, art) = content.operation_type(token);
            let sprite = art.and_then(|art| match art {
                Art::Sprite(sprite) => Ok(sprite),
                other => Err(format!("Operation {token} is drawn with {other:?}, not an interface sprite")),
            });
            (name, sprite)
        }
    };
    Operation {
        token: operation.operation.clone(),
        name,
        sprite,
        target: operation.target.tag.clone(),
        required: operation.state.assigned_slots,
        duration_days: operation.state.duration_days,
        preparing: operation.state.preparation_ratio.is_some_and(|ratio| ratio < FULL_PREPARATION),
    }
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
    let frame = service.frame_at(source, &request, admitted_gen, branch_id, REQUIRED_TABLES)?;
    let load = LoadContext { content: source.admit_resolved(Work::Ahead) };
    let content = agency_content::content(source, &load);
    match &content {
        Ok(content) => extra_warnings.extend(content.warnings.iter().cloned()),
        Err(why) => extra_warnings.push(why.clone()),
    }
    let mut population_warnings = Vec::new();
    let (listed, upgrades_missing) = agencies::agencies(&frame, true, content.as_deref().map_err(String::as_str), &mut population_warnings)
        .map_err(|error| GameWindowError::refused(format!("The intelligence agency window cannot be shown at hour {}: {error}", request.hour)))?;
    let agency = listed
        .into_iter()
        .find(|agency| agency.country.id == target.country)
        .ok_or_else(|| GameWindowError::refused(format!("No intelligence agency is recorded for country {} at hour {}.", target.country, request.hour)))?;
    if !agency.created {
        return Err(GameWindowError::refused(format!(
            "{} has not created its intelligence agency at hour {}.",
            agency.country.tag.as_deref().unwrap_or("This country"),
            request.hour
        )));
    }
    let row = frame
        .complete::<tables::IntelligenceAgencies>()
        .map_err(|error| GameWindowError::refused(format!("Intelligence agencies are not recorded at hour {}: {error}", request.hour)))?
        .iter()
        .find(|row| row.country == target.country)
        .ok_or_else(|| GameWindowError::refused(format!("No intelligence agency is recorded for country {} at hour {}.", target.country, request.hour)))?;
    let upgrade_progress = content_reader_parser::fixed5::fixed5_to_string(row.upgrade_progress_raw, 2);

    let branches = match &content {
        Err(why) => Err(format!("Not recorded: the agency upgrade definitions could not be read from the recorded mod stack ({why}).")),
        Ok(content) => content.upgrades().map(|upgrades| {
            let recorded = agency.upgrades.as_ref().ok_or_else(|| upgrades_missing.clone().unwrap_or_else(|| "acquired upgrades are not recorded".to_owned()));
            for item in agency.upgrades.iter().flat_map(|held| &held.items) {
                if let (Some(branch), Some(token)) = (item.branch.as_deref(), item.token.as_deref())
                    && upgrades.branch_upgrade(branch, token).is_ok_and(|found| found.is_none())
                {
                    extra_warnings.push(format!("Acquired upgrade {branch}/{token} is not defined in the recorded mod stack, so the branches do not show it."));
                }
            }
            branches(
                upgrades,
                recorded.as_ref().map(|held| *held).map_err(String::as_str),
                |branch| content.branch_name(branch),
                |branch, token| content.upgrade_name(branch, token),
            )
        }),
    };

    let operatives = operative_content::content(source, &load);
    let operations = espionage::under_way(&frame, |tags, row| tags.slot(row.owner).is_some() && tags.slot(row.owner) == tags.slot(target.country))
        .map(|(running, warnings)| {
            extra_warnings.extend(warnings);
            running.iter().map(|running| operation(running, operatives.as_deref().map_err(String::as_str))).collect()
        })
        .map_err(|error| format!("Not recorded: the agency's operations are not recorded at this hour ({error})."));

    let samples: Vec<(TableId, i32)> = REQUIRED_TABLES
        .iter()
        .filter_map(|&id| frame.tables[id.index()].sampled_at().map(|hour| (id, hour)))
        .collect();
    let (read_hour, cadence_warning) = compute_read_hour(request.hour, &samples);
    extra_warnings.extend(cadence_warning);

    let view = AgencyView { agency, branches, operations, upgrade_progress, read_hour, warnings: extra_warnings };
    let key = WindowCacheKey {
        source_id: request.source_id.clone(),
        revision: request.revision.clone(),
        game_dir: resolve.game_dir.clone(),
        user_dir: resolve.user_dir.clone(),
        visual_revision: visual_revision.to_owned(),
        language: language.to_owned(),
        force_pow2_textures: decode.force_pow2_textures,
        art_stack: layers.art_stack_key(),
        window: WINDOW_NAME.to_owned(),
    };
    render_view(service, layers, key, view, request.scale, &request.scroll, admitted_gen, cancel)
}

fn fingerprint(view: &AgencyView) -> Option<[u8; 32]> {
    use sha2::{Digest, Sha256};
    serde_json::to_vec(view).ok().map(|bytes| Sha256::digest(bytes).into())
}

#[expect(clippy::too_many_arguments, reason = "the window's one render entry passes the host's request parts through unchanged")]
fn render_view(
    service: &Service,
    layers: &content_reader_layers::ResolvedLayers,
    key: WindowCacheKey,
    view: AgencyView,
    scale: f32,
    scroll: &BTreeMap<String, f32>,
    admitted_gen: (u64, u64),
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
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
            extra_warnings: view.warnings.clone(),
            admitted_gen: Some(admitted_gen),
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
            for sprite in view.operations.iter().flatten().filter_map(|operation| operation.sprite.as_deref().ok()) {
                if shared.registry.sprite(sprite.as_bytes()).is_some() {
                    assets.sprites.insert(sprite.as_bytes().to_vec());
                }
            }
            Ok(Prepared { assets, state: (), fingerprint: print })
        },
        |root, _, _, localisation, instance, tooltips, ()| configure_main(root, localisation, instance, tooltips, &view),
        |plan, main_layout, root, guis, resources, localisation, ()| {
            let sources = Sources { guis, resources, localisation };
            header_text(plan, root, &view);
            compose_branches(plan, main_layout, root, &sources, &view)?;
            agents_text(plan, &view);
            compose_operations(plan, main_layout, root, &sources, &view)?;
            plan.text("crypto_tab_button", &crypto_tab_title(root, localisation), Ok(CRYPTOLOGY_DISABLED.to_owned()));
            plan.title = Some(view.agency.name.clone().unwrap_or_else(|| NOT_RECORDED_VALUE.to_owned()));
            plan.read_hour = Some(view.read_hour);
            Ok(())
        },
    )
}

fn shown_or(value: &Shown) -> String {
    value.clone().unwrap_or_else(|_| NOT_RECORDED_VALUE.to_owned())
}

fn not_recorded(title: &str, why: impl Into<String>) -> GameWindowTooltip {
    GameWindowTooltip { title: title.to_owned(), body: Some(why.into()), detail: None }
}

fn tooltip_at(root: &Element, path: &str, tip: GameWindowTooltip, tooltips: &mut BTreeMap<String, GameWindowTooltip>) {
    if let Some(element) = find_direct(root, path) {
        tooltips.insert(element.path.clone(), tip);
    }
}

fn crypto_tab_title(root: &Element, localisation: &Localisation) -> String {
    find_direct(root, "crypto_tab_button")
        .and_then(|element| match &element.fields {
            native::Fields::Button(button) => dashboard_text::catalogue::localised_bytes(localisation, &button.text, View::Plain).ok(),
            _ => None,
        })
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Unavailable agency tab".to_owned())
}

fn name_of(agency: &Agency) -> Shown {
    agency.name.clone().ok_or_else(|| agency.name_missing.clone().unwrap_or_else(|| "Not recorded: the agency's name".to_owned()))
}

fn defense_of(agency: &Agency) -> Shown {
    agency.defense_text.clone().ok_or_else(|| format!("Not shown: the counter-intelligence line could not be rendered from the recorded content (raw value {}).", agency.defense_raw))
}

fn operatives_of(agency: &Agency) -> String {
    format!("{}/{}", agency.active_count, agency.maximum)
}

fn configure_main(
    root: &Element,
    localisation: &Localisation,
    instance: &mut Instance,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
    view: &AgencyView,
) {
    let agency = &view.agency;
    let name = name_of(agency);
    set_text_by_name(instance, root, "name_logo/agency_name", shown_or(&name));
    if let Err(why) = &name {
        tooltip_at(root, "name_logo/Background", not_recorded(NOT_RECORDED_VALUE, why.clone()), tooltips);
    }
    let defense = defense_of(agency);
    set_text_by_name(instance, root, "name_logo/agency_defense_level", shown_or(&defense));
    if let Err(why) = &defense {
        tooltip_at(root, "name_logo/counter_intelligence_icon", not_recorded(NOT_RECORDED_VALUE, why.clone()), tooltips);
    }
    for name in ["spymaster_title", "spymaster_button", "faction_button", "spymaster_button_name", "spymaster_button_cost", "spymaster_flag"] {
        hide_by_name(instance, root, &format!("name_logo/spymaster/{name}"));
    }
    for name in ["before_creation_panel", "agency_branches/upgrades_before_agency", "agency_agents/operatives_before_agency", "agency_branches/ai_factory_allocation", "agency_crypto"] {
        hide_by_name(instance, root, name);
    }

    set_text_by_name(instance, root, "agency_agents/total_operative_text", operatives_of(agency));
    for (text, icon) in [("arrested_operative_text", "arrested_operative_icon"), ("dead_operative_text", "dead_operative_icon")] {
        set_text_by_name(instance, root, &format!("agency_agents/{text}"), NOT_RECORDED_VALUE.to_owned());
        tooltip_at(root, &format!("agency_agents/{icon}"), not_recorded(NOT_RECORDED_VALUE, COUNT_NOT_RECORDED), tooltips);
    }
    for (bar, description, why) in [
        ("upgrade_progressbar", format!("{} {}", view.upgrade_progress, NOT_RECORDED_VALUE), UPGRADING_NOT_RECORDED),
        ("operatives_progressbar", NOT_RECORDED_VALUE.to_owned(), RECRUITING_NOT_RECORDED),
    ] {
        for part in ["progressbar_content", "progressbar_days", "cancel_upgrade"] {
            hide_by_name(instance, root, &format!("agency_agents/{bar}/{part}"));
        }
        set_text_by_name(instance, root, &format!("agency_agents/{bar}/progressbar_desc"), description);
        tooltip_at(root, &format!("agency_agents/{bar}/progressbar_frame"), not_recorded(NOT_RECORDED_VALUE, why), tooltips);
    }

    hide_by_name(instance, root, "agency_operations/show_unavailable_operations");
    hide_by_name(instance, root, "agency_operations/show_unavailable_operations_label");
    tooltip_at(root, "agency_operations/operations_header", not_recorded(NOT_RECORDED_VALUE, STARTABLE_NOT_RECORDED), tooltips);
    let not_active = "agency_operations/operations_grid_container/operations_not_active";
    match &view.operations {
        Ok(running) if !running.is_empty() => hide_by_name(instance, root, not_active),
        Ok(_) => {
            set_text_by_name(instance, root, not_active, NO_RUNNING_OPERATIONS.to_owned());
            tooltip_at(root, not_active, not_recorded(NO_RUNNING_OPERATIONS, STARTABLE_NOT_RECORDED), tooltips);
        }
        Err(why) => {
            set_text_by_name(instance, root, not_active, NOT_RECORDED_VALUE.to_owned());
            tooltip_at(root, not_active, not_recorded(NOT_RECORDED_VALUE, why.clone()), tooltips);
        }
    }
    disable_by_name(instance, root, "crypto_tab_button");
    tooltip_at(root, "crypto_tab_button", not_recorded(&crypto_tab_title(root, localisation), CRYPTOLOGY_DISABLED), tooltips);
}

fn header_text(plan: &mut ComposedPlan, root: &Element, view: &AgencyView) {
    let agency = &view.agency;
    plan.text("name_logo/agency_name", "Agency", name_of(agency));
    plan.text("name_logo/agency_defense_level", "Counter-intelligence", defense_of(agency));
    for (path, item, asset) in [("name_logo/agency_logo", EMBLEM, EMBLEM_ASSET), ("name_logo/spymaster/spymaster_icon", SPYMASTER, SPYMASTER_ASSET)] {
        let at = find_direct(root, path).map_or_else(|| path.to_owned(), |element| element.path.clone());
        plan.placeholder(&at, None, item, asset, NOT_IN_RECORDING);
    }
}

fn agents_text(plan: &mut ComposedPlan, view: &AgencyView) {
    plan.text("agency_agents/total_operative_text", "Operatives (active / maximum)", Ok(operatives_of(&view.agency)));
    plan.text("agency_agents/arrested_operative_text", ARRESTED, Err(COUNT_NOT_RECORDED.to_owned()));
    plan.text("agency_agents/dead_operative_text", DEAD, Err(COUNT_NOT_RECORDED.to_owned()));
    plan.text("agency_agents/upgrade_progressbar/progressbar_desc", "Upgrade progress (points as recorded)", Ok(view.upgrade_progress.clone()));
    plan.text("agency_agents/upgrade_progressbar/progressbar_desc", UPGRADING, Err(UPGRADING_NOT_RECORDED.to_owned()));
    plan.text("agency_agents/operatives_progressbar/progressbar_desc", RECRUITING, Err(RECRUITING_NOT_RECORDED.to_owned()));
}

struct Sources<'a> {
    guis: &'a native::Guis,
    resources: &'a dyn Resources,
    localisation: &'a Localisation,
}

struct TargetFlagResources<'a>(&'a dyn Resources);

impl Resources for TargetFlagResources<'_> {
    fn physical_viewport(&self) -> Option<[i32; 2]> { self.0.physical_viewport() }
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> { self.0.sprite(name) }
    fn sprite_info(&self, name: &[u8]) -> Option<Cow<'_, SpriteInfo>> { self.0.sprite_info(name) }
    fn portrait_texture(&self, name: &[u8]) -> Option<Texture> { self.0.portrait_texture(name) }
    fn sprite_alpha(&self, name: &[u8]) -> Option<AlphaMask> { self.0.sprite_alpha(name) }
    fn unloaded_sprite(&self, name: &[u8]) -> bool { self.0.unloaded_sprite(name) }
    fn font(&self, name: &[u8]) -> Option<&Font> { self.0.font(name) }
    fn localisation(&self) -> &Localisation { self.0.localisation() }
    fn text_colors(&self) -> &native::fonts::TextColors { self.0.text_colors() }
    fn text_effect(&self) -> Result<Effect, String> { self.0.text_effect() }
    fn effect(&self, file: &str) -> Result<Effect, String> { self.0.effect(file) }

    fn shield_flag(&self, small: bool, index: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        if index == TARGET_FLAG_LAYOUT_INDEX {
            return Ok(None);
        }
        self.0.shield_flag(small, index)
    }

    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> { self.0.text_flag(tag) }
    fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> { self.0.resolved_text_flag(key) }
    fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> { self.0.resolved_text_flag_key(text) }
    fn gui_type(&self, name: &[u8]) -> Option<&Element> { self.0.gui_type(name) }
    fn texture_file(&self, path: &[u8]) -> Option<TextureFile> { self.0.texture_file(path) }
    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<AnimationRecord>> { self.0.sprite_animations(sprite) }
    fn session_text_sprite(&self, name: &[u8]) -> bool { self.0.session_text_sprite(name) }
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

fn holder_rect(holder: &Holder) -> [f32; 4] {
    match holder.clip {
        Some([left, top, right, bottom]) => [left as f32, top as f32, (right - left).max(0) as f32, (bottom - top).max(0) as f32],
        None => [holder.origin[0], holder.origin[1], holder.content[0] as f32, holder.content[1] as f32],
    }
}

fn template<'a>(guis: &'a native::Guis, name: &str) -> Result<&'a Element, GameWindowError> {
    guis.template(name.as_bytes()).ok_or_else(|| GameWindowError::refused(missing_window_refusal(name)))
}

fn pip_frames(pip: &Element, resources: &dyn Resources) -> Result<i32, String> {
    let icon = find_direct(pip, "upgrade_icon").ok_or_else(|| format!("{PIP_TEMPLATE} has no upgrade_icon"))?;
    let native::Fields::Button(button) = &icon.fields else {
        return Err(format!("{PIP_TEMPLATE}/upgrade_icon is not a button"));
    };
    let info = resources
        .sprite_info(&button.sprite)
        .ok_or_else(|| format!("sprite {} is not in the installed interface", String::from_utf8_lossy(&button.sprite)))?;
    match &info.missing {
        Some(why) => Err(format!("sprite {}: {why}", String::from_utf8_lossy(&button.sprite))),
        None => Ok(info.frames),
    }
}

fn compose_branches(
    plan: &mut ComposedPlan,
    main_layout: &Layout,
    root: &Element,
    sources: &Sources<'_>,
    view: &AgencyView,
) -> Result<(), GameWindowError> {
    let Sources { guis, resources, .. } = *sources;
    let grid = find_direct(root, "agency_branches/upgrades_list")
        .ok_or_else(|| GameWindowError::refused(format!("Window '{WINDOW_NAME}' has no 'agency_branches/upgrades_list'.")))?;
    let Some(holder) = holder_for(main_layout, grid) else {
        plan.warnings.push(format!("{}: the upgrade branches are not laid out by the installed interface.", grid.path));
        return Ok(());
    };
    let branches = match &view.branches {
        Ok(branches) => branches,
        Err(why) => {
            plan.placeholder(&grid.path, Some(holder_rect(holder)), "Agency upgrades", "the upgrade branches", why);
            return Ok(());
        }
    };
    let entry = template(guis, BRANCH_TEMPLATE)?;
    let pip = template(guis, PIP_TEMPLATE)?;
    let frames = pip_frames(pip, resources);
    let pip_grid = find_direct(entry, "researched_icons");
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let grid_fields = grid.grid().ok_or_else(|| GameWindowError::refused(format!("{} is not a grid.", grid.path)))?;
    let (_, extent, _) = native_gui::grid_geometry(grid, grid_fields, holder.content)
        .map_err(GameWindowError::failed)?;
    let laid = native_gui::finalize_grid(grid, holder.content, viewport, &vec![entry; branches.len()], holder.position)
        .map_err(|refusal| GameWindowError::failed(refusal.to_string()))?;
    let places = laid.places();
    let slots = native_gui::grid_slots(grid, holder.content, branches.len(), holder.position)
        .map_err(GameWindowError::failed)?;
    for (index, branch) in branches.iter().enumerate() {
        let title = shown_or(&branch.name);
        plan.text(&format!("branch:{index}"), "Branch", branch.name.clone());
        for (pip_index, pip) in branch.pips.iter().enumerate() {
            let name = pip.name.as_deref().unwrap_or(pip.token.as_str());
            plan.text(&format!("branch:{index}:pip:{pip_index}"), name,
                pip.level.as_ref().map(|level| format!("{level} / {}", pip.levels)).map_err(Clone::clone));
        }
        let Some((&place, _)) = places.get(index).zip(slots.get(index)).filter(|(_, slot)| {
            slot.iter().zip(laid.origin).zip(extent).all(|((&coordinate, origin), limit)| {
                (0..limit).contains(&coordinate.wrapping_sub(origin))
            })
        }) else {
            plan.text(&format!("branch:{index}/display"), &title,
                Ok("Shown in the game's separate upgrades window, which BattlePlan does not have.".to_owned()));
            continue;
        };
        let layout = layout_window(entry, resources, viewport, |instance| {
            set_text_by_name(instance, entry, "agency_branches_title", title.clone());
            hide_by_name(instance, entry, "opportunities");
        })?;
        let lines: Vec<String> = branch
            .pips
            .iter()
            .map(|pip| match &pip.level {
                Ok(level) => format!("{}: {level} / {}", shown_or(&pip.name), pip.levels),
                Err(_) => format!("{}: {NOT_RECORDED_VALUE}", shown_or(&pip.name)),
            })
            .collect();
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
            plan.warnings.push(format!("{BRANCH_TEMPLATE}: the upgrade markers are not laid out by the installed interface."));
            continue;
        };
        let pip_places = grid_places(pip_grid, pip_holder, &vec![pip; branch.pips.len()])?;
        for (pip_index, (pip_state, pip_place)) in branch.pips.iter().zip(pip_places).enumerate() {
            let name = shown_or(&pip_state.name);
            let path = format!("branch:{index}:pip:{pip_index}");
            let frame = match (&pip_state.level, &frames) {
                (Err(why), _) => Err(why.clone()),
                (Ok(_), Err(why)) => Err(format!("Not shown: the level marker's art is unavailable ({why}).")),
                (Ok(level), Ok(frames)) => pip_frame(*frames, pip_state.levels, *level).ok_or_else(|| {
                    format!(
                        "Not shown: this upgrade has {} levels and the installed level marker has no frames for that count; its recorded level is {level}.",
                        pip_state.levels
                    )
                }),
            };
            let mut pip_layout = layout_window(pip, resources, viewport, |instance| {
                hide_by_name(instance, pip, "agency_upgrade_anim");
                if let Ok(frame) = &frame {
                    set_frame_by_name(instance, pip, "upgrade_icon", *frame);
                }
            })?;
            let marker = find_direct(pip, "upgrade_icon");
            let unknown = match (&frame, marker) {
                (Err(_), Some(marker)) => blank(&mut pip_layout, &marker.path),
                _ => None,
            };
            let inner = pip_holder.place(pip_place, &pip_layout);
            let parts = holder.place_parts(place, &inner);
            let regions = holder.place_regions(place, &pip_holder.place_regions(pip_place, &pip_layout.regions));
            let mut overrides = BTreeMap::new();
            if let (Ok(_), Some(button)) = (&frame, marker) {
                let body = match &pip_state.level {
                    Ok(level) => format!("{level} / {}", pip_state.levels),
                    Err(why) => why.clone(),
                };
                overrides.insert(button.path.clone(), GameWindowTooltip { title: name.clone(), body: Some(body), detail: Some(pip_state.token.clone()) });
            }
            if let Err(why) = &frame {
                let rect = unknown.map(|region| holder.place_regions(place, &pip_holder.place_regions(pip_place, &[region]))[0].rect);
                plan.placeholder(&path, rect, &format!("{name} level marker"), "its level marker", why);
                if let Some(text) = plan.text.last_mut() {
                    (text.value, text.missing) = (Some(why.clone()), None);
                }
            }
            plan.sub_windows.push(ComposedSubWindow {
                before_part: holder.before,
                parts,
                regions,
                diagnostics: pip_layout.diagnostics,
                root_window: PIP_TEMPLATE.to_owned(),
                path_prefix: format!("{path}/"),
                close_paths: Vec::new(),
                tooltip_overrides: overrides,
            });
        }
    }
    Ok(())
}

fn operation_controls<'a>(entry: &'a Element, kind: native::Kind, name: &str) -> Vec<&'a Element> {
    let mut parents = vec![entry];
    let mut controls = Vec::new();
    while let Some(parent) = parents.pop() {
        parents.extend(&parent.children);
        controls.extend(parent.children.iter().filter(|child| child.kind == kind && child.is_named(name)));
    }
    controls
}

fn compose_operations(
    plan: &mut ComposedPlan,
    main_layout: &Layout,
    root: &Element,
    sources: &Sources<'_>,
    view: &AgencyView,
) -> Result<(), GameWindowError> {
    let Sources { guis, resources, localisation } = *sources;
    let running = match &view.operations {
        Err(why) => {
            plan.text("operations", "Running operations", Err(why.clone()));
            return Ok(());
        }
        Ok(running) if running.is_empty() => {
            plan.text("operations", "Running operations", Ok(format!("{NO_RUNNING_OPERATIONS} {STARTABLE_NOT_RECORDED}")));
            return Ok(());
        }
        Ok(running) => running,
    };
    plan.text("operations", "Running operations", Ok(format!("{}. {STARTABLE_NOT_RECORDED}", running.len())));
    let grid = find_direct(root, "agency_operations/operations_grid_container/operations_grid")
        .ok_or_else(|| GameWindowError::refused(format!("Window '{WINDOW_NAME}' has no 'agency_operations/operations_grid_container/operations_grid'.")))?;
    let Some(holder) = holder_for(main_layout, grid) else {
        plan.warnings.push(format!("{}: the running operations are not laid out by the installed interface.", grid.path));
        return Ok(());
    };
    let entry = template(guis, OPERATION_TEMPLATE)?;
    let places = grid_places(grid, holder, &vec![entry; running.len()])?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let shown_bottom = holder.clip.map(|[_, _, _, bottom]| bottom as f32);
    let mut below = 0usize;
    for (index, (operation, place)) in running.iter().zip(places).enumerate() {
        let path = format!("operation:{index}");
        let name = shown_or(&operation.name);
        let status_key = if operation.preparing { "OPERATION_VIEW_COLLECTING" } else { "OPERATION_VIEW_PROGRESS" };
        let status = dashboard_text::catalogue::localised_with(localisation, status_key, &[], View::Plain);
        let target = operation.target.clone().ok_or_else(|| "Not recorded: the target country's tag".to_owned());
        plan.text(&path, "Operation", operation.name.clone());
        plan.text(&format!("{path}/target"), "Target", target.clone());
        plan.text(&format!("{path}/operatives"), "Operatives required", Ok(operation.required.to_string()));
        plan.text(&format!("{path}/network"), "Network strength required", Err(NETWORK_NOT_RECORDED.to_owned()));
        plan.text(&format!("{path}/duration"), "Duration (days)", Ok(operation.duration_days.to_string()));
        plan.text(&format!("{path}/status"), "Status", status.clone());
        let sprite = operation.sprite.clone().and_then(|sprite| match resources.sprite_info(sprite.as_bytes()) {
            None => Err(format!("sprite {sprite} is not in the installed interface")),
            Some(info) => match &info.missing {
                Some(why) => Err(format!("sprite {sprite}: {why}")),
                None => Ok(sprite),
            },
        });
        let mut missing_controls = Vec::new();
        let mut layout = layout_window(entry, &TargetFlagResources(resources), viewport, |instance| {
            if let Some(flag) = find_direct(entry, OPERATION_TARGET_FLAG)
                && let Ok(bound) = instance.control(entry, flag)
            {
                bound.shield = Some(TARGET_FLAG_LAYOUT_INDEX);
            }
            set_text_by_name(instance, entry, "container_operation_info/operation_name_text", name.clone());
            for hidden in [
                "container_operation_info/operation_ai_score",
                "container_operation_info/operation_prepared_icon",
                "container_operation_info/operation_prepared_remaining_days_text",
                "container_operation_info/operation_completed_icon",
                "container_operation_info/operation_completed_warning",
            ] {
                hide_by_name(instance, entry, hidden);
            }
            if let Ok(sprite) = &sprite {
                set_sprite_by_name(instance, entry, "container_operation_info/operation_icon", sprite);
            }
            for (kind, control, label, value) in [
                (native::Kind::TextBox, "operatives_required_text", "Operatives required display", Some(operation.required.to_string())),
                (native::Kind::TextBox, "network_strength_text", "Network strength display", Some(NOT_RECORDED_VALUE.to_owned())),
                (native::Kind::TextBox, "operation_duration_text", "Duration display", Some(operation.duration_days.to_string())),
                (native::Kind::Button, "btn_select_button", "Status display", Some(shown_or(&status))),
                (native::Kind::Container, "container_tokens", "Operation tokens display", None),
            ] {
                let controls = operation_controls(entry, kind, control);
                let bound = match (value, controls.as_slice()) {
                    (Some(text), [element]) => instance.control(entry, element).map(|bound| bound.text = Some(text)),
                    (value, elements) => {
                        for element in elements { let _ = instance.hide(entry, element); }
                        value.map_or(Ok(()), |_| {
                            let state = if elements.is_empty() { "missing" } else { "ambiguous" };
                            Err(format!("Installed operation control '{control}' is {state}."))
                        })
                    }
                };
                if let Err(why) = bound { missing_controls.push((label, why)); }
            }
        })?;
        let mut stand_in = |element: &str, item: String, asset: &str, why: &str| {
            let rect = find_direct(entry, element)
                .and_then(|element| blank(&mut layout, &element.path))
                .map(|region| holder.place_regions(place, &[region])[0].rect);
            plan.placeholder(&format!("{path}/{element}"), rect, &item, asset, why);
        };
        if let Err(why) = &sprite {
            stand_in("container_operation_info/operation_icon", format!("{name} icon"), "the operation's picture", why);
        }
        let country = target.clone().unwrap_or_else(|_| "Unrecorded target country".to_owned());
        stand_in(OPERATION_TARGET_FLAG, country.clone(), "the target's flag", UNBOUND_TARGET_FLAG);
        if let Some(text) = plan.text.last_mut() {
            (text.label, text.value, text.missing) = (format!("Target flag for {country}"), Some(UNBOUND_TARGET_FLAG.to_owned()), None);
        }
        plan.warnings.push(format!("Could not draw the operation target flag for {country}: {UNBOUND_TARGET_FLAG}"));
        let mut overrides = BTreeMap::new();
        let mut body = format!("Target: {}\nTarget flag for {country}: {UNBOUND_TARGET_FLAG}\n{NETWORK_NOT_RECORDED}", target.unwrap_or_else(|why| why));
        for (label, why) in missing_controls {
            plan.text(&format!("{path}/{label}"), label, Ok(why.clone()));
            plan.warnings.push(format!("{name}: {why}"));
            body.push_str(&format!("\n{why}"));
        }
        if let Some(background) = find_direct(entry, "operation_bg") {
            overrides.insert(background.path.clone(), GameWindowTooltip {
                title: name.clone(),
                body: Some(body),
                detail: operation.token.clone(),
            });
        }
        if shown_bottom.is_some_and(|bottom| holder.origin[1] + place[1] as f32 >= bottom) {
            below += 1;
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place(place, &layout),
            regions: holder.place_regions(place, &layout.regions),
            diagnostics: layout.diagnostics,
            root_window: OPERATION_TEMPLATE.to_owned(),
            path_prefix: format!("{path}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
    }
    if below > 0 {
        plan.warnings.push(format!(
            "{below} running operation(s) lie below the visible part of the operations list, which does not scroll in this window yet; every one is listed in the window's text."
        ));
    }
    Ok(())
}
