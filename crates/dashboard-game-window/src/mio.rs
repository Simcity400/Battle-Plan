//! The policy chooser of a military-industrial organisation.
//!
//! The window and its row are the content's templates: [`ROOT`] holds the list box [`LIST`],
//! which holds one [`ROW`] window per policy. Their names, and the names of the row's controls,
//! are the binding contract; the content decides everything they draw.
//!
//! Rows are the policies the organisation allows (`IndustrialOrganisationDetails.allowed_policies`),
//! each joined to its instance in its owner's policy roster (`IndustrialOrganisationPolicies`).
//! The organisation's and its policies' conditions are evaluated at the hour by the shared
//! trigger interpreter in the scopes the game gives them ([`EventScopes::organisation`]): a policy
//! whose visibility does not hold has no row. A condition the recording cannot decide never guesses: the policy is withheld by
//! name, counted. The game's display order is not established: rows keep the recorded order of
//! the organisation's allowed policies, and the reply says so.
//!
//! A row draws as the game draws it ([`ROW_BACKGROUND`] and [`ROW_ICON`] are its state): the
//! background takes frame 2 for the organisation's attached policy and 1 for the others, and the
//! background and icon draw their Disable technique when the row's action is blocked
//! ([`row_blocked`]): by the organisation's own `available` or `visible`, a remaining cooldown,
//! or for an unattached policy its own `available` or the owner's political power for its attach
//! quote. A failing part blocks the row whatever the others. The quote scales the stored base
//! cost by the organisation's own modifiers and its owner's recorded effective policy-cost
//! modifier ([`PolicyCost`]); a row whose gate the recording does not decide (that modifier not
//! recorded, say) has its background and icon hidden, counted, rather than drawn in a guessed
//! state. `name` is the country's stored policy name (already localised text); the icon follows
//! the game's lookup ([`policy_icon`]). An unattached row with a positive quote shows it through
//! [`COST_KEY`]; an attached row, or a quote of zero or less, hides the cost, and an unknown
//! quote hides it, counted. The AI value is hidden, as the game hides it with its AI view off.
//! What the recording holds of each row (its key, base cost, cooldown and whether it is
//! attached) is shown on hover.

use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

use dashboard_base::{
    Source,
    catalogue::Catalogue,
    metadata::check_cancel,
    products::industrial_organisations::{
        PolicyCost, affordable, both, owner_country, owner_roster, policy_cost, shows_attach_cost,
    },
};
use dashboard_text::{
    scripted_text::EventScopes,
    triggers::{Evaluator, RECORDED_TABLES, inputs::capture},
};
use installed_content_core::{
    ResolvedLayers, industrial_organisations::policies::Catalog as Policies, parser::Item,
};
use installed_content_localisation::localisation::{Argument, Localisation};
use replay::container::SeekResult;
use replay_data::rows::{
    IndustrialOrganisationDetailsRow, IndustrialOrganisationPolicy, IndustrialOrganisationRow,
    TableId, tables,
};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::map::native_gui::{Assets, SpriteSetter, assets_in};

use crate::{
    compose::{self, BoundList, Hover, Row},
    products::game_window::{Bound, SlotState, WindowSlot},
    resources::WindowResources,
    session::Session,
};

/// The policy chooser's root window.
pub const ROOT: &str = "industrial_organisation_policy_window";
/// The row window the chooser's list holds, one per policy.
pub const ROW: &str = "policy_item";
/// The list box below [`ROOT`] that holds the rows.
pub const LIST: &str = "policy_list_window/policy_list";
/// The row's controls.
const ROW_BACKGROUND: &str = "background_button";
const ROW_ICON: &str = "icon";
const ROW_NAME: &str = "name";
const ROW_COST: &str = "attach_cost";
/// Shown only with the game's AI view on.
const ROW_AI_VALUE: &str = "ai_value";
/// The controls the binding names.
const ROW_CONTROLS: [&str; 5] = [ROW_BACKGROUND, ROW_ICON, ROW_NAME, ROW_COST, ROW_AI_VALUE];
/// The background's frames for an attached policy and for the others.
const ATTACHED_FRAME: i32 = 2;
const UNATTACHED_FRAME: i32 = 1;
/// The icon a policy with no icon of its own and no token-derived sprite shows.
const UNKNOWN_ICON: &[u8] = b"GFX_idea_unknown";
/// The organisation's selected policy when none is selected: the constructor's sentinel.
const UNSELECTED_POLICY: u32 = 0x4c17;
/// The text an unattached row's cost shows, its `VALUE` the attach quote.
const COST_KEY: &str = "INDUSTRIAL_ORG_POLICY_PP_COST";

/// The tables one hour of the chooser reads. Its conditions are evaluated on them first; the
/// tables an undecided condition read beyond them are read only then ([`gates`]).
const TABLES: [TableId; 8] = [
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    // The quote's per-source modifier eligibility and clamp.
    TableId::ModifierDefinitions,
    TableId::SimulationFeatures,
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
    /// The tag of the country the policy instance belongs to, or why it is unknown.
    pub tag: Result<String, String>,
}

/// What the hour records of one organisation's chooser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// The organisation's script key and its owner's canonical slot, or why it is unknown.
    pub token: Vec<u8>,
    pub owner: Result<u32, String>,
    /// The remaining policy cooldown in days, as the game counts it ([`remaining_days`]).
    pub cooldown_days: i32,
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
    tag: impl Fn(u32) -> Result<String, String>,
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
                tag: tag(policy.country),
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

/// The organisation's identity and details rows at the frame's hour.
fn organisation_rows(
    frame: &DecodedFrame,
    organisation: u64,
) -> Result<(&IndustrialOrganisationRow, &IndustrialOrganisationDetailsRow), String> {
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
    Ok((row, details))
}

/// The organisation's chooser inputs at the frame's hour.
pub fn inputs(frame: &DecodedFrame, organisation: u64) -> Result<Inputs, String> {
    let (row, details) = organisation_rows(frame, organisation)?;
    // The game reads the candidates and the attached policy from the owner's roster, through
    // the owner's country (its raw reference can be an alias).
    let roster = owner_roster(frame, row)
        .map_err(|_| "the owner's policy roster is not recorded at this hour".to_owned())?;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|why| why.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let tags = dashboard_text::variant_names::Tags::new(&countries, &links, None);
    let (rows, unlisted) = rows(
        details,
        &roster.policies,
        |token| spelling(frame, token),
        |country| {
            tags.tag_of(country)
                .ok_or_else(|| format!("the policy's country {country} has no recorded tag"))
        },
    )?;
    let owner = dashboard_text::metadata::link_slot(&links, row.owner).ok_or_else(|| {
        format!(
            "the organisation's owner {} has no recorded country",
            row.owner
        )
    });
    Ok(Inputs {
        token: spelling(frame, row.token)?,
        owner,
        cooldown_days: remaining_days(details.state.cooldown_hour, frame.hour),
        rows,
        unlisted,
        read_hour: frame.tables[TableId::IndustrialOrganisationDetails.index()].sampled_at(),
        cooldown_hour: details.state.cooldown_hour,
    })
}

/// The remaining policy cooldown in days: the difference of the calendar days of the stored end
/// hour and of `hour` (none for no end), with the game's signed 32-bit arithmetic. The whole
/// end day is already off cooldown.
pub fn remaining_days(end: Option<i32>, hour: i32) -> i32 {
    let day = |hour: i32| hour.wrapping_sub(engine_native::cdate::HOUR_EPOCH) / 24;
    end.map_or(0, |end| day(end).wrapping_sub(day(hour)))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(installed_content_core::parser::c_string(bytes)).into_owned()
}

/// The policies' authored definitions and what reading them noted, read once per stack against
/// the stack's shared content names ([`ResolvedLayers::key_names`]).
#[derive(Default)]
#[allow(clippy::type_complexity)]
struct PolicyDefinitions(std::sync::OnceLock<Result<Arc<(Policies, Vec<String>)>, String>>);

/// The organisations' trait trees, read once per stack.
#[derive(Default)]
struct Trees(std::sync::OnceLock<Result<Arc<installed_content_core::mio_traits::Catalog>, String>>);

fn trees(layers: &ResolvedLayers) -> Result<Arc<installed_content_core::mio_traits::Catalog>, String> {
    layers
        .derived::<Trees>()
        .0
        .get_or_init(|| {
            installed_content_core::mio_traits::read(layers)
                .map(Arc::new)
                .map_err(|why| format!("the organisations' content does not read: {why}"))
        })
        .clone()
}

/// Reads the policies' and organisations' definitions ahead of the first window over `layers`.
pub fn warm(layers: &ResolvedLayers) {
    let _ = definitions(layers);
    let _ = trees(layers);
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
    /// The definition's authored icon, empty when it names none.
    pub icon: Vec<u8>,
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
        let icon = definition.icon.clone();
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
    details.push("This row does not act in the replay.".to_owned());
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

/// What a frame's conditions decide: the rows they show ([`shown`]), the policies withheld,
/// whether the organisation's own conditions hold and what scales its attach quotes.
type Gated = (
    Vec<Shown>,
    Vec<(String, String)>,
    Result<bool, String>,
    Result<PolicyCost, String>,
);

/// A frame's conditions, and whether one was left undecided having read tables the frame does
/// not hold.
enum Gates {
    /// Every condition decided, or left undecided for no table the frame lacks.
    Decided(Gated),
    /// Some conditions undecided, but reading more tables cannot decide them.
    Undecided(Gated),
    /// Undecided conditions read these tables, which the frame does not hold.
    Read(Vec<TableId>),
}

/// Evaluates every policy's conditions in the organisation's scope on `frame`. A decided
/// condition is decided whatever tables the frame lacks (an unread table only ever leaves a
/// condition undecided), so the tables an undecided one read, by what its inputs declare, are
/// the only ones worth reading the hour again for; an input that declares nothing asks for every
/// table the interpreter reads ([`RECORDED_TABLES`]).
#[allow(clippy::too_many_arguments)]
fn gates(
    catalogue: &Catalogue,
    frame: &DecodedFrame,
    selected: &[TableId],
    organisation: u64,
    inputs: &Inputs,
    policies: &Result<&Policies, String>,
    trees: &Result<Arc<installed_content_core::mio_traits::Catalog>, String>,
) -> Gates {
    let world = catalogue
        .scripted_content()
        .and_then(|content| content.recorded_world(frame));
    // The game's organisation/owner scope pair; an unknown owner leaves FROM unknown.
    let graph = std::rc::Rc::new(std::cell::RefCell::new(EventScopes::organisation(
        organisation,
        inputs.owner.as_ref().ok().copied(),
    )));
    let evaluator = world
        .as_ref()
        .map(|world| Evaluator::new(world, None).with_event_scopes(graph))
        .map_err(Clone::clone);
    let undecided = std::cell::Cell::new(false);
    let gate = |items: &[Item]| -> Result<bool, String> {
        let evaluator = evaluator.as_ref().map_err(Clone::clone)?;
        let scope = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
        let decided = evaluator.evaluate(items, scope).decided();
        undecided.set(undecided.get() || decided.is_err());
        decided
    };
    // The organisation's `available`, then its `visible`; an empty body holds.
    let condition = |items: &[Item]| {
        if items.is_empty() {
            Ok(true)
        } else {
            gate(items)
        }
    };
    let organisation_gate = || -> Result<bool, String> {
        let name = lossy(&inputs.token);
        let trees = trees.as_ref().map_err(Clone::clone)?;
        let tree = trees
            .tree(&name)?
            .ok_or_else(|| format!("organisation {name} is not defined in the recorded content"))?;
        // The game checks `available`, then `visible`; either failing fails the gate.
        both(condition(tree.available), || condition(tree.visible))
    };
    let stored = |bytes: &[u8]| {
        dashboard_text::scripted_text::stored_text(
            &catalogue.localisation,
            &catalogue.recorded_language,
            bytes,
            "the policy's stored name",
        )
    };
    // The quote evaluates the organisation's conditions too, so it reads under the same capture.
    let cost = || -> Result<PolicyCost, String> {
        let world = world.as_ref().map_err(Clone::clone)?;
        let trees = trees.as_ref().map_err(Clone::clone)?;
        let policies = policies.as_ref().map_err(Clone::clone)?;
        let (row, details) = organisation_rows(frame, organisation)?;
        let cost = policy_cost(frame, row, details, trees, policies, world);
        undecided.set(undecided.get() || cost.is_err());
        cost
    };
    let (gated, read) = capture(frame, || {
        let (shown, withheld) = shown(inputs.rows.clone(), policies.clone(), gate, stored);
        (shown, withheld, organisation_gate(), cost())
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
/// policies withheld, the row gate's shared parts, when it was read and what the read warns of.
#[derive(Clone)]
pub struct Hour {
    pub shown: Vec<Shown>,
    pub withheld: Vec<(String, String)>,
    pub gate: Gate,
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
    let trees = trees(&catalogue.content.layers);
    let policies = definitions
        .as_deref()
        .map(|(policies, _)| policies)
        .map_err(Clone::clone);
    // A condition that read a table beyond those read has the hour read again with it; the
    // tables only grow, so this ends.
    let mut tables = TABLES.to_vec();
    let (shown, withheld, organisation_gate, cost) = loop {
        match gates(
            catalogue,
            &frame,
            &tables,
            organisation,
            &inputs,
            &policies,
            &trees,
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
        gate: Gate {
            organisation: organisation_gate,
            cooldown_days: inputs.cooldown_days,
            cost,
            political_power_raw: political_power(&frame, organisation),
        },
        read_hour: inputs.read_hour,
        warnings,
    }))
}

/// The owner's political power (fixed5) at the frame's hour, or why it is unknown.
fn political_power(frame: &DecodedFrame, organisation: u64) -> Result<i64, String> {
    let (row, _) = organisation_rows(frame, organisation)?;
    owner_country(frame, row)?
        .political_power_raw
        .ok_or_else(|| "the owner's political power is not recorded".to_owned())
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
        &read.gate,
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
    gate: &Gate,
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
    let declared = |name: &[u8]| session.gui.registry().sprite(name).is_some();
    let icons: Vec<Result<Vec<u8>, String>> = shown
        .iter()
        .map(|row| policy_icon(&row.icon, &row.policy, declared))
        .collect();
    let costs: Vec<Result<Option<String>, String>> = shown
        .iter()
        .map(|row| cost_text(row, gate, localisation))
        .collect();
    for ((row, icon), cost) in shown.iter().zip(&icons).zip(&costs) {
        if let Ok(name) = &row.name {
            assets.add_text(name);
        }
        if let Ok(icon) = icon {
            assets.sprites.insert(icon.clone());
        }
        if let Ok(Some(cost)) = cost {
            assets.add_text(cost);
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
    let controls: Result<Vec<_>, String> = ROW_CONTROLS
        .iter()
        .map(|control| compose::control(&row, control))
        .collect();
    let [
        background,
        icon_control,
        name_control,
        cost_control,
        ai_control,
    ] = match controls {
        Ok(controls) => <[_; 5]>::try_from(controls).expect("one per control"),
        Err(why) => return Ok(refuse(why)),
    };
    let mut rows = Vec::with_capacity(shown.len());
    let mut refused_icons = BTreeMap::<String, usize>::new();
    let mut refused_names = BTreeMap::<String, usize>::new();
    let mut undecided = BTreeMap::<String, usize>::new();
    let mut refused_costs = BTreeMap::<String, usize>::new();
    for ((shown, icon), cost) in shown.iter().zip(&icons).zip(&costs) {
        let mut instance = match compose::construct(&row, viewport, &resources) {
            Ok(instance) => instance,
            Err(why) => return Ok(refuse(why)),
        };
        let attached = shown.policy.selected;
        let blocked = row_blocked(shown, gate);
        let applied = (|| -> Result<(), String> {
            match &shown.name {
                Ok(name) => instance.set_text(&row, name_control, name, &resources)?,
                // The template's own text is a placeholder, not the policy's name.
                Err(why) => {
                    *refused_names.entry(why.clone()).or_default() += 1;
                    instance.hide(&row, name_control)?;
                }
            }
            let frame = if attached {
                ATTACHED_FRAME
            } else {
                UNATTACHED_FRAME
            };
            instance.set_frame(&row, background, frame)?;
            match icon {
                Ok(icon) => {
                    instance.set_sprite(&row, icon_control, icon, SpriteSetter::Icon)?;
                }
                Err(why) => {
                    *refused_icons.entry(why.clone()).or_default() += 1;
                    instance.hide(&row, icon_control)?;
                }
            }
            match &blocked {
                Ok(blocked) => {
                    instance.control(&row, background)?.enabled = Some(!blocked);
                    instance.control(&row, icon_control)?.enabled = Some(!blocked);
                }
                Err(why) => {
                    *undecided.entry(why.clone()).or_default() += 1;
                    instance.hide(&row, background)?;
                    instance.hide(&row, icon_control)?;
                }
            }
            match cost {
                Ok(Some(cost)) => instance.set_text(&row, cost_control, cost, &resources)?,
                // The game hides it on the attached policy and for a quote of zero or less.
                Ok(None) => instance.hide(&row, cost_control)?,
                Err(why) => {
                    *refused_costs.entry(why.clone()).or_default() += 1;
                    instance.hide(&row, cost_control)?;
                }
            }
            instance.hide(&row, ai_control)
        })();
        if let Err(why) = applied {
            return Ok(refuse(why));
        }
        rows.push(Row {
            instance,
            hover: Some(hover(shown)),
        });
    }
    let lists = [BoundList {
        list,
        template: &row,
        rows,
    }];
    let mut scene = compose::compose(
        &compose::Window {
            name: ROOT,
            root: &root,
            instance: &instance,
            lists: &lists,
            grids: &[],
            hovers: &[],
        },
        scroll,
        &resources,
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
    slots.push(counted_slot("row attach cost", &refused_costs));
    slots.push(counted_slot("row state", &undecided));
    slots.push(slot(
        "row AI value",
        SlotState::Bound,
        Some("hidden, as the game hides it with its AI view off".into()),
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
            for control in ROW_CONTROLS {
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

/// The parts of the row gate one organisation shares: whether its own conditions hold, its
/// remaining policy cooldown in days, what scales its attach quotes and its owner's political
/// power (fixed5), each or why it is unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gate {
    pub organisation: Result<bool, String>,
    pub cooldown_days: i32,
    pub cost: Result<PolicyCost, String>,
    pub political_power_raw: Result<i64, String>,
}

/// The attach quote of `shown` under `gate`, or why it is unknown.
fn quote(shown: &Shown, gate: &Gate) -> Result<i32, String> {
    gate.cost
        .as_ref()
        .map(|cost| cost.quote(shown.policy.cost))
        .map_err(|why| format!("the attach quote is unknown: {why}"))
}

/// Whether the row's action is blocked, as the game decides it: the organisation's own
/// conditions and a cooldown over (0 or fewer days left) for every row, and for an unattached
/// policy also the owner's political power for its attach quote and its `available` condition.
/// Any part that fails blocks it; a row no failing part blocks is undecided while a part is
/// unknown.
pub fn row_blocked(shown: &Shown, gate: &Gate) -> Result<bool, String> {
    let mut unknown = Vec::new();
    match &gate.organisation {
        Ok(false) => return Ok(true),
        Ok(true) => {}
        Err(why) => unknown.push(format!("the organisation's conditions are unknown: {why}")),
    }
    if gate.cooldown_days > 0 {
        return Ok(true);
    }
    if !shown.policy.selected {
        match &shown.available {
            Ok(false) => return Ok(true),
            Ok(true) => {}
            Err(why) => unknown.push(format!("availability unknown: {why}")),
        }
        match (quote(shown, gate), &gate.political_power_raw) {
            (Ok(quote), Ok(power)) => {
                if !affordable(*power, quote) {
                    return Ok(true);
                }
            }
            (Err(why), _) => unknown.push(why),
            (_, Err(why)) => unknown.push(format!("political power unknown: {why}")),
        }
    }
    if unknown.is_empty() {
        Ok(false)
    } else {
        Err(unknown.join("; "))
    }
}

/// The text a row's cost shows: none on the attached policy or for a quote of zero or less, as
/// the game hides it; otherwise [`COST_KEY`] with the quote as its `VALUE`. An unknown quote, or
/// text that does not localise, is refused.
fn cost_text(
    shown: &Shown,
    gate: &Gate,
    localisation: &Localisation,
) -> Result<Option<String>, String> {
    if shown.policy.selected {
        return Ok(None);
    }
    let quote = quote(shown, gate)?;
    if !shows_attach_cost(quote, false) {
        return Ok(None);
    }
    localisation
        .localise_with(COST_KEY.as_bytes(), &[("VALUE", Argument::Int32(quote))])
        .map(Some)
        .map_err(|why| format!("{COST_KEY}: {why:?}"))
}

/// The policy's icon as the game looks it up: its authored icon when it names one, else the
/// sprite named for the policy's country and token, then for its token, then
/// [`UNKNOWN_ICON`]. The token takes the `GFX_` prefix unless it already has it. The game keeps
/// the control's previous drawable when a name is not declared, which is not established, so
/// such an icon is refused.
fn policy_icon(
    authored: &[u8],
    policy: &PolicyRow,
    declared: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    let name = lossy(&policy.token);
    if !authored.is_empty() {
        return if declared(authored) {
            Ok(authored.to_vec())
        } else {
            Err(format!(
                "policy {name}'s icon {} is not declared",
                lossy(authored)
            ))
        };
    }
    let token = installed_content_core::parser::c_string(&policy.token);
    let prefix: &[u8] = if token.starts_with(b"GFX_") {
        b""
    } else {
        b"GFX_"
    };
    let tag = policy
        .tag
        .as_ref()
        .map_err(|why| format!("policy {name} names no icon and {why}"))?;
    let country = [prefix, tag.as_bytes(), b"_", token].concat();
    let generic = [prefix, token].concat();
    [country, generic, UNKNOWN_ICON.to_vec()]
        .into_iter()
        .find(|candidate| declared(candidate))
        .ok_or_else(|| {
            format!(
                "policy {name} names no icon, and neither its token's sprites nor {} are declared",
                lossy(UNKNOWN_ICON)
            )
        })
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

#[cfg(test)]
mod gate_tests {
    use super::*;
    use replay_data::rows::{
        CountryLinkRow, CountryRow, IndustrialOrganisationDetailsRow,
        IndustrialOrganisationPoliciesRow, IndustrialOrganisationRow, LexicalTokenRow, TableRows,
    };
    use replay_frame::frame::{Metrics, TableView};

    const ORGANISATION: u64 = 5;

    /// One organisation of the owner `SYN` (raw 10, slot 4) allowing `p_from` and `p_other`,
    /// at hour `hour`, its cooldown ending at `cooldown`.
    fn frame(hour: i32, cooldown: Option<i32>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
        };
        let mut set = |rows: TableRows| {
            let id = rows.table();
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        set(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: ORGANISATION,
                country: 10,
                owner: 10,
                token: 1,
                ..Default::default()
            },
        ]));
        let mut details = IndustrialOrganisationDetailsRow {
            organisation: ORGANISATION,
            country: 10,
            allowed_policies: vec![2, 3],
            ..Default::default()
        };
        details.state.policy = UNSELECTED_POLICY;
        details.state.cooldown_hour = cooldown;
        set(TableRows::IndustrialOrganisationDetails(vec![details]));
        let policy = |token| IndustrialOrganisationPolicy {
            token,
            cost: 10,
            days: 30,
            country: 10,
            name: Vec::new(),
            cached_score_raw: 0,
        };
        set(TableRows::IndustrialOrganisationPolicies(vec![
            IndustrialOrganisationPoliciesRow {
                country: 10,
                policies: vec![policy(2), policy(3)],
                cost_modifier_raw: Some(-20_000),
            },
        ]));
        set(TableRows::LexicalTokens(
            [(1, "syn_org"), (2, "p_from"), (3, "p_other")]
                .map(|(token, literal)| LexicalTokenRow {
                    token,
                    literal: literal.as_bytes().to_vec(),
                })
                .to_vec(),
        ));
        set(TableRows::Countries(vec![CountryRow {
            id: 10,
            canonical_slot: 4,
            tag: b"SYN".to_vec(),
            political_power_raw: Some(800_000),
            ..Default::default()
        }]));
        set(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 10,
            canonical_slot: 4,
        }]));
        // The owner exists: it owns a state.
        set(TableRows::CountryCounterInputs(vec![
            replay_data::rows::CountryCounterInputRow {
                country: 10,
                owned_state_count: 1,
                ..Default::default()
            },
        ]));
        frame
    }

    /// The organisation's and its policies' conditions are read in the native scopes: FROM is
    /// the owner, so a policy's `available` reading FROM is decided; the organisation's own
    /// `available` gates every row; a cooldown ending later the same day has passed.
    #[test]
    fn conditions_read_the_owner_through_from_and_the_cooldown_counts_calendar_days() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/00_defines.lua",
                "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
            ),
            (
                "common/military_industrial_organization/organizations/syn.txt",
                "syn_org = { available = { FROM = { tag = SYN } } }",
            ),
            (
                "common/military_industrial_organization/policies/syn.txt",
                "p_from = { available = { FROM = { tag = SYN } } } p_other = { available = { FROM = { NOT = { tag = SYN } } } }",
            ),
        ]);
        let layers = &catalogue.content.layers;
        let start = engine_native::cdate::HOUR_EPOCH + 1936 * 8760;
        let frame = frame(start + 5, Some(start + 23));
        let inputs = inputs(&frame, ORGANISATION).unwrap();
        assert_eq!(inputs.cooldown_days, 0);
        assert_eq!(inputs.owner, Ok(4));
        let definitions = definitions(layers).unwrap();
        let gated = gates(
            &catalogue,
            &frame,
            // The frame holds every table.
            &TableId::ALL,
            ORGANISATION,
            &inputs,
            &Ok(&definitions.0),
            &trees(layers),
        );
        let (shown, withheld, organisation, cost) = match gated {
            Gates::Decided(gated) => gated,
            Gates::Undecided((shown, _, organisation, _)) => panic!(
                "undecided: {organisation:?} {:?}",
                shown
                    .iter()
                    .map(|row| row.available.clone())
                    .collect::<Vec<_>>()
            ),
            Gates::Read(tables) => panic!("reads {tables:?}"),
        };
        assert!(withheld.is_empty(), "{withheld:?}");
        assert_eq!(organisation, Ok(true));
        assert_eq!(
            shown
                .iter()
                .map(|row| row.available.clone())
                .collect::<Vec<_>>(),
            [Ok(true), Ok(false)]
        );
        // The owner's -0.2 scales the base cost of 10 to a quote of 8.
        assert_eq!(
            cost,
            Ok(PolicyCost {
                local: 0,
                country: -20_000
            })
        );
        // `p_other` is blocked by its own condition; `p_from` by the political power for its
        // quote alone: 8 political power affords it exactly, a raw unit less does not.
        let mut gate = Gate {
            organisation,
            cooldown_days: inputs.cooldown_days,
            cost,
            political_power_raw: Ok(800_000),
        };
        assert_eq!(row_blocked(&shown[1], &gate), Ok(true));
        assert_eq!(row_blocked(&shown[0], &gate), Ok(false));
        gate.political_power_raw = Ok(799_999);
        assert_eq!(row_blocked(&shown[0], &gate), Ok(true));
    }

    /// The owner's raw reference can be an alias of its country: the roster, the political power
    /// and FROM all follow the country, and an unrecorded policy-cost modifier leaves an
    /// unattached row's quote unknown, never zero.
    #[test]
    fn the_quote_follows_the_owner_alias_and_refuses_an_unrecorded_modifier() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/00_defines.lua",
                "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
            ),
            (
                "common/military_industrial_organization/organizations/syn.txt",
                "syn_org = { available = { FROM = { tag = SYN } } }",
            ),
            (
                "common/military_industrial_organization/policies/syn.txt",
                "p_from = { } p_other = { }",
            ),
        ]);
        let layers = &catalogue.content.layers;
        let start = engine_native::cdate::HOUR_EPOCH + 1936 * 8760;
        let mut frame = frame(start, None);
        let set = |frame: &mut DecodedFrame, rows: TableRows| {
            let id = rows.table();
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        // The organisation names its owner by the alias 11 of the country 10.
        set(
            &mut frame,
            TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
                organisation: ORGANISATION,
                country: 10,
                owner: 11,
                token: 1,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::CountryLinks(
                [(10, 4), (11, 4)]
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .to_vec(),
            ),
        );
        let definitions = definitions(layers).unwrap();
        let read = |frame: &DecodedFrame| {
            let inputs = inputs(frame, ORGANISATION).unwrap();
            match gates(
                &catalogue,
                frame,
                &TableId::ALL,
                ORGANISATION,
                &inputs,
                &Ok(&definitions.0),
                &trees(layers),
            ) {
                Gates::Decided(gated) | Gates::Undecided(gated) => (inputs, gated),
                Gates::Read(tables) => panic!("reads {tables:?}"),
            }
        };
        let (inputs, (shown, _, organisation, cost)) = read(&frame);
        assert_eq!(inputs.owner, Ok(4));
        assert_eq!(organisation, Ok(true));
        let gate = Gate {
            organisation,
            cooldown_days: 0,
            cost,
            political_power_raw: political_power(&frame, ORGANISATION),
        };
        assert_eq!(gate.political_power_raw, Ok(800_000));
        assert_eq!(row_blocked(&shown[0], &gate), Ok(false));
        assert_eq!(quote(&shown[0], &gate), Ok(8));
        // No recorded modifier: the unattached rows are undecided, by name.
        set(
            &mut frame,
            TableRows::IndustrialOrganisationPolicies(vec![IndustrialOrganisationPoliciesRow {
                country: 10,
                policies: vec![
                    IndustrialOrganisationPolicy {
                        token: 2,
                        cost: 10,
                        days: 30,
                        country: 10,
                        name: Vec::new(),
                        cached_score_raw: 0,
                    },
                    IndustrialOrganisationPolicy {
                        token: 3,
                        cost: 10,
                        days: 30,
                        country: 10,
                        name: Vec::new(),
                        cached_score_raw: 0,
                    },
                ],
                cost_modifier_raw: None,
            }]),
        );
        let (_, (shown, _, organisation, cost)) = read(&frame);
        let gate = Gate {
            organisation,
            cooldown_days: 0,
            cost,
            political_power_raw: Ok(800_000),
        };
        let why = row_blocked(&shown[0], &gate).unwrap_err();
        assert!(why.contains("policy cost modifier is not recorded"), "{why}");
    }

    #[test]
    fn the_cooldown_is_a_difference_of_calendar_days() {
        let day = engine_native::cdate::HOUR_EPOCH + 1936 * 8760;
        assert_eq!(remaining_days(None, day), 0);
        assert_eq!(remaining_days(Some(day + 23), day), 0);
        assert_eq!(remaining_days(Some(day + 24), day + 23), 1);
        assert_eq!(remaining_days(Some(day + 48), day + 1), 2);
        assert_eq!(remaining_days(Some(day), day + 47), -1);
    }

    /// A failing organisation condition or an active cooldown blocks every row; an attached row
    /// needs neither its own `available` nor a quote; anything else unknown leaves it undecided.
    #[test]
    fn the_row_gate_is_decided_by_any_failing_part() {
        let row = |selected: bool, available: Result<bool, String>| Shown {
            policy: PolicyRow {
                token: b"p".to_vec(),
                name: Vec::new(),
                cost: 1,
                days: 1,
                selected,
                tag: Ok("SYN".into()),
            },
            name: Ok("p".into()),
            icon: Vec::new(),
            available,
        };
        let gate = |organisation: Result<bool, String>, cooldown_days| Gate {
            organisation,
            cooldown_days,
            cost: Err("not recorded".into()),
            political_power_raw: Ok(0),
        };
        let open = gate(Ok(true), 0);
        assert_eq!(row_blocked(&row(true, Ok(false)), &open), Ok(false));
        assert_eq!(row_blocked(&row(false, Ok(false)), &open), Ok(true));
        assert!(row_blocked(&row(false, Ok(true)), &open).is_err());
        // A known quote the power does not afford blocks; one it affords opens the row.
        let quoted = |power| Gate {
            cost: Ok(PolicyCost {
                local: 0,
                country: 0,
            }),
            political_power_raw: power,
            ..gate(Ok(true), 0)
        };
        assert_eq!(row_blocked(&row(false, Ok(true)), &quoted(Ok(99_999))), Ok(true));
        assert_eq!(row_blocked(&row(false, Ok(true)), &quoted(Ok(100_000))), Ok(false));
        assert!(row_blocked(&row(false, Ok(true)), &quoted(Err("unread".into()))).is_err());
        // An unaffordable quote decides even when availability is unknown.
        assert_eq!(row_blocked(&row(false, Err("unknown".into())), &quoted(Ok(0))), Ok(true));
        for selected in [true, false] {
            let blocked = |gate: &Gate| row_blocked(&row(selected, Ok(true)), gate);
            assert_eq!(blocked(&gate(Ok(false), 0)), Ok(true));
            assert_eq!(blocked(&gate(Ok(true), 3)), Ok(true));
            assert_eq!(blocked(&gate(Err("unknown".into()), 3)), Ok(true));
        }
        assert!(row_blocked(&row(true, Ok(true)), &gate(Err("unknown".into()), 0)).is_err());
    }

    /// A false organisation condition decides the gate even when the other is unknown, so an
    /// attached row whose cooldown is over draws Disable rather than being hidden.
    #[test]
    fn a_false_organisation_condition_decides_the_gate_whatever_the_other() {
        let unknown = || Err::<bool, String>("unknown".into());
        assert_eq!(both(unknown(), || Ok(false)), Ok(false));
        assert_eq!(both(Ok(false), || panic!("not evaluated")), Ok(false));
        assert_eq!(both(Ok(true), || Ok(false)), Ok(false));
        assert_eq!(both(Ok(true), || Ok(true)), Ok(true));
        assert!(both(unknown(), || Ok(true)).is_err());
        assert!(both(Ok(true), unknown).is_err());
        let gate = Gate {
            organisation: both(unknown(), || Ok(false)),
            cooldown_days: 0,
            cost: Err("not recorded".into()),
            political_power_raw: Err("not recorded".into()),
        };
        let attached = Shown {
            policy: PolicyRow {
                token: b"p".to_vec(),
                name: Vec::new(),
                cost: 1,
                days: 1,
                selected: true,
                tag: Ok("SYN".into()),
            },
            name: Ok("p".into()),
            icon: Vec::new(),
            available: Ok(true),
        };
        assert_eq!(row_blocked(&attached, &gate), Ok(true));
    }

    /// The icon is the authored one, else the country's, the token's, then the unknown idea's,
    /// with the token's prefix kept; an undeclared name is refused.
    #[test]
    fn the_icon_follows_the_native_lookup() {
        let policy = |token: &str| PolicyRow {
            token: token.as_bytes().to_vec(),
            name: Vec::new(),
            cost: 1,
            days: 1,
            selected: false,
            tag: Ok("SYN".into()),
        };
        let declared = |names: &'static [&'static str]| {
            move |name: &[u8]| names.iter().any(|known| known.as_bytes() == name)
        };
        let all = declared(&["GFX_authored", "GFX_SYN_p", "GFX_p", "GFX_idea_unknown"]);
        let icon = |authored: &[u8], token: &str, declared: &dyn Fn(&[u8]) -> bool| {
            policy_icon(authored, &policy(token), declared)
        };
        assert_eq!(
            icon(b"GFX_authored", "p", &all),
            Ok(b"GFX_authored".to_vec())
        );
        assert_eq!(icon(b"", "p", &all), Ok(b"GFX_SYN_p".to_vec()));
        let generic = declared(&["GFX_p", "GFX_idea_unknown"]);
        assert_eq!(icon(b"", "p", &generic), Ok(b"GFX_p".to_vec()));
        let prefixed = declared(&["GFX_q"]);
        assert_eq!(icon(b"", "GFX_q", &prefixed), Ok(b"GFX_q".to_vec()));
        let unknown = declared(&["GFX_idea_unknown"]);
        assert_eq!(icon(b"", "p", &unknown), Ok(b"GFX_idea_unknown".to_vec()));
        let none = declared(&[]);
        assert!(
            icon(b"GFX_authored", "p", &none)
                .unwrap_err()
                .contains("not declared")
        );
        assert!(
            icon(b"", "p", &none)
                .unwrap_err()
                .contains("GFX_idea_unknown")
        );
    }
}
