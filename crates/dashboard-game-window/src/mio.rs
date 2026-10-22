//! The policy chooser of a military-industrial organisation.
//!
//! The window and its row are the content's templates: [`ROOT`] holds the list box [`LIST`],
//! which holds one [`ROW`] window per policy. Their names, and the names of the row's controls,
//! are the binding contract; the content decides everything they draw.
//!
//! Rows are the policies the organisation allows (`IndustrialOrganisationDetails.allowed_policies`),
//! each joined to its instance in the country's policy roster (`IndustrialOrganisationPolicies`).
//! A policy's `visible` and `available` conditions are evaluated in the organisation's scope
//! at the hour, by the shared trigger interpreter: a policy whose visibility does not hold has
//! no row, and a row whose availability does not hold is muted as the app mutes rows it cannot
//! act on (the game marks it with its controls' disabled effect, which the dashboard does not
//! run). A condition the recording cannot decide never guesses: the policy is withheld by name,
//! or its row left unmuted with the reason on hover, and both are counted. The game's display
//! order is not established: rows keep the recorded order of the organisation's allowed
//! policies, and the reply says so.
//!
//! A row's `name` is the country's stored policy name (already localised text) and its `icon`
//! the policy's authored icon. The attach cost the row shows, its debugging AI value and the
//! frames that mark a selected, unavailable or cooling-down policy have no established binding:
//! they are named unbound slots, the cost left as construction leaves it (empty) and the AI
//! value, whose authored text is a placeholder, hidden. What the recording does hold of each
//! row (its key, base cost, cooldown and whether it is selected) is shown on hover.

use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

use dashboard_base::{Source, catalogue::Catalogue, metadata::check_cancel};
use dashboard_text::triggers::{RECORDED_TABLES, Scope, Scopes, inputs::capture};
use installed_content_core::{
    ResolvedLayers, industrial_organisations::policies::Catalog as Policies, parser::Item,
};
use installed_content_localisation::localisation::Localisation;
use replay::container::SeekResult;
use replay_data::rows::{
    IndustrialOrganisationDetailsRow, IndustrialOrganisationPolicy, TableId, tables,
};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::map::native_gui::{Assets, SpriteSetter, assets_in};

use crate::{
    compose::{self, BoundList, Hover, Row},
    products::game_window::{Bound, SlotState, WindowSlot},
    resources::WindowResources,
    scene,
    session::Session,
};

/// The policy chooser's root window.
pub const ROOT: &str = "industrial_organisation_policy_window";
/// The row window the chooser's list holds, one per policy.
pub const ROW: &str = "policy_item";
/// The list box below [`ROOT`] that holds the rows.
pub const LIST: &str = "policy_list_window/policy_list";
/// The row's controls.
const ROW_ICON: &str = "icon";
const ROW_NAME: &str = "name";
/// A control whose authored text is a placeholder and whose binding is not established.
const ROW_AI_VALUE: &str = "ai_value";
/// The organisation's selected policy when none is selected: the constructor's sentinel.
const UNSELECTED_POLICY: u32 = 0x4c17;

/// The tables one hour of the chooser reads. Its conditions are evaluated on them first; the
/// tables an undecided condition read beyond them are read only then ([`gates`]).
const TABLES: [TableId; 4] = [
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
    TableId::LexicalTokens,
];

/// One row as the recording supplies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRow {
    pub token: Vec<u8>,
    /// The country's stored name: localised text.
    pub name: Vec<u8>,
    /// The country's stored base attach cost.
    pub cost: i32,
    pub days: i32,
    pub selected: bool,
}

/// What the hour records of one organisation's chooser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    pub rows: Vec<PolicyRow>,
    /// Allowed policies the country roster does not hold, by token: they have no row.
    pub unlisted: Vec<Vec<u8>>,
    pub read_hour: Option<i32>,
    pub cooldown_hour: Option<i32>,
}

/// The rows of `details`, joined to `roster`, with each token spelled by `spell`.
pub fn rows(
    details: &IndustrialOrganisationDetailsRow,
    roster: &[IndustrialOrganisationPolicy],
    spell: impl Fn(u32) -> Result<Vec<u8>, String>,
) -> Result<(Vec<PolicyRow>, Vec<Vec<u8>>), String> {
    let mut rows = Vec::with_capacity(details.allowed_policies.len());
    let mut unlisted = Vec::new();
    for &token in &details.allowed_policies {
        let spelled = spell(token)?;
        match roster.iter().find(|policy| policy.token == token) {
            Some(policy) => rows.push(PolicyRow {
                token: spelled,
                name: policy.name.clone(),
                cost: policy.cost,
                days: policy.days,
                selected: details.state.policy != UNSELECTED_POLICY
                    && details.state.policy == token,
            }),
            None => unlisted.push(spelled),
        }
    }
    Ok((rows, unlisted))
}

fn spelling(frame: &DecodedFrame, token: u32) -> Result<Vec<u8>, String> {
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|why| why.to_string())?;
    tokens
        .binary_search_by_key(&token, |row| row.token)
        .map(|index| tokens[index].literal.clone())
        .map_err(|_| format!("LexicalTokens does not spell token {token}"))
}

/// The organisation's chooser inputs at the frame's hour.
pub fn inputs(frame: &DecodedFrame, organisation: u64) -> Result<Inputs, String> {
    let organisations = frame
        .complete::<tables::IndustrialOrganisations>()
        .map_err(|why| why.to_string())?;
    let row = organisations
        .binary_search_by_key(&organisation, |row| row.organisation)
        .map(|index| &organisations[index])
        .map_err(|_| "the organisation is not recorded at this hour".to_owned())?;
    let details = frame
        .complete::<tables::IndustrialOrganisationDetails>()
        .map_err(|why| why.to_string())?;
    let details = details
        .binary_search_by_key(&organisation, |details| details.organisation)
        .map(|index| &details[index])
        .ok()
        .filter(|details| details.country == row.country)
        .ok_or_else(|| "the organisation's details are not recorded at this hour".to_owned())?;
    let rosters = frame
        .complete::<tables::IndustrialOrganisationPolicies>()
        .map_err(|why| why.to_string())?;
    let roster = rosters
        .binary_search_by_key(&details.country, |roster| roster.country)
        .map(|index| &rosters[index])
        .map_err(|_| "the country's policy roster is not recorded at this hour".to_owned())?;
    let (rows, unlisted) = rows(details, &roster.policies, |token| spelling(frame, token))?;
    Ok(Inputs {
        rows,
        unlisted,
        read_hour: frame.tables[TableId::IndustrialOrganisationDetails.index()].sampled_at(),
        cooldown_hour: details.state.cooldown_hour,
    })
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(installed_content_core::parser::c_string(bytes)).into_owned()
}

/// The policies' authored definitions and what reading them noted, read once per stack against
/// the stack's shared content names ([`ResolvedLayers::key_names`]).
#[derive(Default)]
#[allow(clippy::type_complexity)]
struct PolicyDefinitions(std::sync::OnceLock<Result<Arc<(Policies, Vec<String>)>, String>>);

/// Reads the policies' definitions ahead of the first window over `layers`.
pub fn warm(layers: &ResolvedLayers) {
    let _ = definitions(layers);
}

fn definitions(layers: &ResolvedLayers) -> Result<Arc<(Policies, Vec<String>)>, String> {
    layers
        .derived::<PolicyDefinitions>()
        .0
        .get_or_init(|| {
            let mut notes = Vec::new();
            layers
                .key_names()
                .and_then(|names| {
                    installed_content_core::industrial_organisations::read_policies(
                        layers, &names, &mut notes,
                    )
                })
                .map(|policies| Arc::new((policies, notes)))
                .map_err(|why| format!("the policies' content does not read: {why}"))
        })
        .clone()
}

/// What a row draws, from its recorded state and its definition.
#[derive(Clone)]
pub struct Shown {
    pub policy: PolicyRow,
    /// The stored name as the viewer may show it, or why it may not.
    pub name: Result<String, String>,
    pub icon: Result<Vec<u8>, String>,
    /// Whether its availability condition holds, or why that is unknown.
    pub available: Result<bool, String>,
}

/// The rows the chooser shows, and the policies withheld because whether they are visible is
/// unknown, each with the reason. `gate` evaluates a condition in the organisation's scope; an
/// empty condition holds without it.
pub fn shown(
    rows: Vec<PolicyRow>,
    policies: Result<&Policies, String>,
    gate: impl Fn(&[Item]) -> Result<bool, String>,
    stored: impl Fn(&[u8]) -> Result<String, String>,
) -> (Vec<Shown>, Vec<(String, String)>) {
    let condition = |items: &[Item]| {
        if items.is_empty() {
            Ok(true)
        } else {
            gate(items)
        }
    };
    let mut withheld = Vec::new();
    let mut shown = Vec::with_capacity(rows.len());
    for policy in rows {
        let definition = match &policies {
            Ok(policies) => policies.policy(&policy.token),
            Err(why) => Err(why.clone()),
        };
        let definition = match definition {
            Ok(definition) => definition,
            // Without its definition neither condition is known.
            Err(why) => {
                withheld.push((lossy(&policy.token), why));
                continue;
            }
        };
        match condition(&definition.visible) {
            Ok(true) => {}
            // The game shows no row for it.
            Ok(false) => continue,
            Err(why) => {
                withheld.push((lossy(&policy.token), why));
                continue;
            }
        }
        let available = condition(&definition.available);
        let icon = if definition.icon.is_empty() {
            Err(format!(
                "policy {} names no icon, and the game's icon fallback is not established",
                lossy(&policy.token)
            ))
        } else {
            Ok(definition.icon.clone())
        };
        let name = stored(installed_content_core::parser::c_string(&policy.name));
        shown.push(Shown {
            policy,
            name,
            icon,
            available,
        });
    }
    (shown, withheld)
}

/// What the app explains on a row's hover: the recording's own facts, codes included.
pub fn hover(shown: &Shown) -> Hover {
    let policy = &shown.policy;
    let mut details = vec![format!("Key: {}", lossy(&policy.token))];
    details.push(format!("Base attach cost (recorded): {}", policy.cost));
    details.push(format!(
        "Cooldown after attaching (recorded): {} days",
        policy.days
    ));
    if policy.selected {
        details.push("Attached to this organisation".to_owned());
    }
    match &shown.available {
        Ok(true) => {}
        Ok(false) => {
            details.push("Not available: its availability condition does not hold".to_owned())
        }
        Err(why) => details.push(format!("Availability unknown: {why}")),
    }
    let title = shown
        .name
        .as_deref()
        .ok()
        .and_then(|name| dashboard_base::catalogue::plain_text(name).ok())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| lossy(&policy.token));
    Hover { title, details }
}

/// The rows a frame's conditions show ([`shown`]), and whether a condition was left undecided
/// having read tables the frame does not hold.
enum Gates {
    /// Every condition decided, or left undecided for no table the frame lacks.
    Decided((Vec<Shown>, Vec<(String, String)>)),
    /// Some conditions undecided, but reading more tables cannot decide them.
    Undecided((Vec<Shown>, Vec<(String, String)>)),
    /// Undecided conditions read these tables, which the frame does not hold.
    Read(Vec<TableId>),
}

/// Evaluates every policy's conditions in the organisation's scope on `frame`. A decided
/// condition is decided whatever tables the frame lacks (an unread table only ever leaves a
/// condition undecided), so the tables an undecided one read, by what its inputs declare, are
/// the only ones worth reading the hour again for; an input that declares nothing asks for every
/// table the interpreter reads ([`RECORDED_TABLES`]).
fn gates(
    catalogue: &Catalogue,
    frame: &DecodedFrame,
    selected: &[TableId],
    organisation: u64,
    rows: &[PolicyRow],
    policies: &Result<&Policies, String>,
) -> Gates {
    let world = catalogue
        .scripted_content()
        .and_then(|content| content.recorded_world(frame));
    let scope = Scopes::of(Scope::IndustrialOrganisation(organisation));
    let undecided = std::cell::Cell::new(false);
    let gate = |items: &[Item]| -> Result<bool, String> {
        let decided = world
            .as_ref()
            .map_err(Clone::clone)?
            .evaluate(items, scope)
            .decided();
        undecided.set(undecided.get() || decided.is_err());
        decided
    };
    let stored = |bytes: &[u8]| {
        dashboard_text::scripted_text::stored_text(
            &catalogue.localisation,
            &catalogue.recorded_language,
            bytes,
            "the policy's stored name",
        )
    };
    let (gated, read) = capture(frame, || {
        shown(rows.to_vec(), policies.clone(), gate, stored)
    });
    if !undecided.get() {
        return Gates::Decided(gated);
    }
    let mut more = Vec::new();
    for key in read.keys() {
        match key.dependence() {
            Some(reads) => more.extend(reads.into_iter().map(|read| read.table)),
            None => more.extend_from_slice(RECORDED_TABLES),
        }
    }
    more.sort_unstable();
    more.dedup();
    more.retain(|table| !selected.contains(table));
    if more.is_empty() {
        Gates::Undecided(gated)
    } else {
        Gates::Read(more)
    }
}

/// What one hour records of an organisation's chooser, its conditions evaluated: the rows, the
/// policies withheld, when it was read and what the read warns of.
#[derive(Clone)]
pub struct Hour {
    pub shown: Vec<Shown>,
    pub withheld: Vec<(String, String)>,
    pub read_hour: Option<i32>,
    pub warnings: Vec<String>,
}

/// The hours a source's choosers were last read at, by artwork session, branch, hour and
/// organisation: scrolling or reopening a window at an hour reads it once.
#[derive(Default)]
#[allow(clippy::type_complexity)]
struct Hours(std::collections::VecDeque<((String, u64, i32, u64), Arc<Hour>)>);

/// The hours [`Hours`] keeps.
const KEPT_HOURS: usize = 32;

/// The chooser's rows of `organisation` at `hour`, or why the hour gives none.
fn hour(
    source: &Source,
    catalogue: &Catalogue,
    branch: u64,
    hour: i32,
    organisation: u64,
    cancelled: &AtomicBool,
) -> io::Result<Result<Hour, String>> {
    let seek = |tables: &[TableId]| -> io::Result<Result<Box<DecodedFrame>, String>> {
        Ok(
            match source
                .replay_ref()
                .seek_selected_with(branch, hour, Some(tables), || check_cancel(cancelled))?
            {
                SeekResult::Frame(frame) => Ok(frame),
                SeekResult::Gap { reason, .. } => {
                    Err(format!("the hour is not recorded: {reason}"))
                }
            },
        )
    };
    let mut frame = match seek(&TABLES)? {
        Ok(frame) => frame,
        Err(why) => return Ok(Err(why)),
    };
    check_cancel(cancelled)?;
    let inputs = match inputs(&frame, organisation) {
        Ok(inputs) => inputs,
        Err(why) => return Ok(Err(why)),
    };
    let mut warnings = Vec::new();
    if !inputs.unlisted.is_empty() {
        warnings.push(format!(
            "{} allowed polic(ies) are not in the country's recorded roster and have no row: {}",
            inputs.unlisted.len(),
            inputs
                .unlisted
                .iter()
                .map(|token| lossy(token))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let definitions = definitions(&catalogue.content.layers);
    let policies = definitions
        .as_deref()
        .map(|(policies, _)| policies)
        .map_err(Clone::clone);
    // A condition that read a table beyond those read has the hour read again with it; the
    // tables only grow, so this ends.
    let mut tables = TABLES.to_vec();
    let (shown, withheld) = loop {
        match gates(
            catalogue,
            &frame,
            &tables,
            organisation,
            &inputs.rows,
            &policies,
        ) {
            Gates::Decided(gated) | Gates::Undecided(gated) => break gated,
            Gates::Read(more) => {
                tables.extend(more);
                frame = match seek(&tables)? {
                    Ok(frame) => frame,
                    Err(why) => return Ok(Err(why)),
                };
                check_cancel(cancelled)?;
            }
        }
    };
    Ok(Ok(Hour {
        shown,
        withheld,
        read_hour: inputs.read_hour,
        warnings,
    }))
}

/// The chooser of `organisation` at `hour`.
#[allow(clippy::too_many_arguments)]
pub fn bind(
    source: &Source,
    catalogue: &Catalogue,
    session: &std::sync::Mutex<Session>,
    branch: u64,
    at: i32,
    organisation: u64,
    scroll: &BTreeMap<String, i32>,
    cancelled: &AtomicBool,
) -> io::Result<Bound> {
    let key = (
        session
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .id
            .clone(),
        branch,
        at,
        organisation,
    );
    let hours = source.state::<Hours>();
    let kept = {
        let hours = hours.lock().unwrap_or_else(|error| error.into_inner());
        hours
            .0
            .iter()
            .find(|(kept, _)| *kept == key)
            .map(|(_, read)| Arc::clone(read))
    };
    let read = match kept {
        Some(read) => read,
        None => match hour(source, catalogue, branch, at, organisation, cancelled)? {
            Ok(read) => {
                let read = Arc::new(read);
                let mut hours = hours.lock().unwrap_or_else(|error| error.into_inner());
                if hours.0.len() == KEPT_HOURS {
                    hours.0.pop_front();
                }
                hours.0.push_back((key, Arc::clone(&read)));
                read
            }
            Err(why) => {
                return Ok(Bound {
                    title: None,
                    read_hour: None,
                    scene: Err(vec![format!("{ROOT}: {why}")]),
                    slots: Vec::new(),
                    warnings: Vec::new(),
                });
            }
        },
    };
    // Windows draw one at a time per session; the hour was read before it was held.
    // Native windows show the presentation's text.
    let localisation = catalogue.presentation_localisation()?;
    let mut session = session.lock().unwrap_or_else(|error| error.into_inner());
    check_cancel(cancelled)?;
    let mut bound = draw(
        &mut session,
        &catalogue.content.layers,
        &localisation,
        &read.shown,
        &read.withheld,
        scroll,
    )?;
    bound.read_hour = read.read_hour;
    bound.warnings.extend(read.warnings.iter().cloned());
    Ok(bound)
}

/// The chooser drawing `shown` rows, from content alone.
pub fn draw(
    session: &mut Session,
    layers: &ResolvedLayers,
    localisation: &Localisation,
    shown: &[Shown],
    withheld: &[(String, String)],
    scroll: &BTreeMap<String, i32>,
) -> io::Result<Bound> {
    let refuse = |why: String| Bound {
        title: None,
        read_hour: None,
        scene: Err(vec![format!("{ROOT}: {why}")]),
        slots: Vec::new(),
        warnings: Vec::new(),
    };
    let (Some(root), Some(row)) = (session.guis.template(ROOT), session.guis.template(ROW)) else {
        return Ok(refuse(format!(
            "the content declares no {ROOT} window with its {ROW} rows"
        )));
    };
    let (root, row) = (root.clone(), row.clone());
    let Some(list) = root.path_named(LIST) else {
        return Ok(refuse(format!("{ROOT} has no {LIST} list box")));
    };
    // What the rows bind: their names' inline icons and their policies' icons.
    let mut assets = Assets::default();
    {
        let guis = &session.guis;
        let gui_type = |name: &[u8]| guis.template(name);
        assets.extend(assets_in(&root, &gui_type, localisation));
        assets.extend(assets_in(&row, &gui_type, localisation));
    }
    for row in shown {
        if let Ok(name) = &row.name {
            assets.add_text(name);
        }
        if let Ok(icon) = &row.icon {
            assets.sprites.insert(icon.clone());
        }
    }
    session.load(layers, &assets)?;
    let resources = WindowResources {
        gui: &session.gui,
        guis: &session.guis,
        localisation,
    };
    let viewport = session.viewport;
    let instance = match compose::construct(&root, viewport, &resources) {
        Ok(instance) => instance,
        Err(why) => return Ok(refuse(why)),
    };
    let controls = compose::control(&row, ROW_ICON).and_then(|icon| {
        Ok((
            icon,
            compose::control(&row, ROW_NAME)?,
            compose::control(&row, ROW_AI_VALUE)?,
        ))
    });
    let (icon_control, name_control, ai_control) = match controls {
        Ok(controls) => controls,
        Err(why) => return Ok(refuse(why)),
    };
    let mut rows = Vec::with_capacity(shown.len());
    let mut refused_icons = BTreeMap::<String, usize>::new();
    let mut refused_names = BTreeMap::<String, usize>::new();
    for shown in shown {
        let mut instance = match compose::construct(&row, viewport, &resources) {
            Ok(instance) => instance,
            Err(why) => return Ok(refuse(why)),
        };
        let applied = (|| -> Result<(), String> {
            match &shown.name {
                Ok(name) => instance.set_text(&row, name_control, name, &resources)?,
                // The template's own text is a placeholder, not the policy's name.
                Err(why) => {
                    *refused_names.entry(why.clone()).or_default() += 1;
                    instance.hide(&row, name_control)?;
                }
            }
            match &shown.icon {
                Ok(icon) => instance.set_sprite(&row, icon_control, icon, SpriteSetter::Icon)?,
                Err(why) => *refused_icons.entry(why.clone()).or_default() += 1,
            }
            instance.hide(&row, ai_control)
        })();
        if let Err(why) = applied {
            return Ok(refuse(why));
        }
        rows.push(Row {
            instance,
            hover: Some(hover(shown)),
            muted: shown.available == Ok(false),
        });
    }
    let lists = [BoundList {
        list,
        template: &row,
        rows,
    }];
    let sizes = |id: usize| session.page_size(id);
    let mut scene = compose::compose(
        &compose::Window {
            name: ROOT,
            root: &root,
            instance: &instance,
            lists: &lists,
            grids: &[],
            muted: &[],
            hovers: &[],
        },
        scroll,
        &resources,
        &session.gui,
        &sizes,
    );
    if let Ok(scene) = &mut scene
        && !withheld.is_empty()
    {
        scene.notes.push(format!(
            "{ROOT} is drawn incompletely: {} polic(ies) are withheld because whether they are visible is unknown: {}",
            withheld.len(),
            withheld
                .iter()
                .map(|(token, why)| format!("{token} ({why})"))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let mut availability = scene::Counted::default();
    for row in shown {
        if let Err(why) = &row.available {
            availability.note(why.clone());
        }
    }
    let availability = availability.lines();
    let mut slots = vec![
        slot("rows", SlotState::Bound, None),
        if withheld.is_empty() {
            slot("row visibility", SlotState::Bound, None)
        } else {
            slot(
                "row visibility",
                SlotState::Refused,
                Some(format!(
                    "{} polic(ies) withheld: whether they are visible is unknown",
                    withheld.len()
                )),
            )
        },
        if availability.is_empty() {
            slot("row availability", SlotState::Bound, None)
        } else {
            slot(
                "row availability",
                SlotState::Refused,
                Some(format!(
                    "rows whose availability is unknown are not muted: {}",
                    availability.join("; ")
                )),
            )
        },
        slot(
            "row order",
            SlotState::Unbound,
            Some("rows keep the organisation's recorded order; the game's display order is not established".into()),
        ),
        counted_slot("row name", &refused_names),
    ];
    slots.push(if refused_icons.is_empty() {
        slot("row icon", SlotState::Bound, None)
    } else {
        slot(
            "row icon",
            SlotState::Refused,
            Some(
                refused_icons
                    .iter()
                    .map(|(why, count)| format!("{why} (count: {count})"))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        )
    });
    slots.push(slot(
        "row attach cost",
        SlotState::Unbound,
        Some("the cost the game shows (its modifiers and number format) is not established; the recorded base cost is shown on hover".into()),
    ));
    slots.push(slot(
        "row state",
        SlotState::Unbound,
        Some("which frames the game gives the attached, unavailable, cooling-down and unaffordable rows is not established; unavailable rows are muted as the app mutes rows".into()),
    ));
    slots.push(slot(
        "row AI value",
        SlotState::Unbound,
        Some(
            "hidden: its authored text is a placeholder and its binding is not established".into(),
        ),
    ));
    let title = root
        .path_named("title/title")
        .and_then(|title| title.text_box())
        .and_then(|text| localisation.localise(&text.text).ok())
        .and_then(|text| dashboard_base::catalogue::plain_text(&text).ok());
    Ok(Bound {
        title,
        read_hour: None,
        scene,
        slots,
        warnings: Vec::new(),
    })
}

/// What the content alone decides of the chooser: its templates and controls, and the
/// policies' authored definitions. Each refusal is one finding.
pub fn check(layers: &ResolvedLayers, guis: &installed_content_core::gui::native::Guis) -> Vec<String> {
    let mut refused = Vec::new();
    match (guis.template(ROOT), guis.template(ROW)) {
        (Some(root), Some(row)) => {
            if root.path_named(LIST).is_none() {
                refused.push(format!("{ROOT} has no {LIST} list box"));
            }
            for control in [ROW_ICON, ROW_NAME, ROW_AI_VALUE] {
                if let Err(why) = compose::control(row, control) {
                    refused.push(why);
                }
            }
        }
        _ => refused.push(format!(
            "the content declares no {ROOT} window with its {ROW} rows"
        )),
    }
    match definitions(layers) {
        Ok(definitions) => refused.extend(definitions.1.iter().cloned()),
        Err(why) => refused.push(why),
    }
    refused
}

/// A slot filled from the recording unless some of its values were refused, counted by reason.
fn counted_slot(name: &str, refused: &BTreeMap<String, usize>) -> WindowSlot {
    if refused.is_empty() {
        return slot(name, SlotState::Bound, None);
    }
    slot(
        name,
        SlotState::Refused,
        Some(
            refused
                .iter()
                .map(|(why, count)| format!("{why} (count: {count})"))
                .collect::<Vec<_>>()
                .join("; "),
        ),
    )
}

fn slot(name: &str, state: SlotState, detail: Option<String>) -> WindowSlot {
    WindowSlot {
        name: name.to_owned(),
        state,
        detail,
    }
}
