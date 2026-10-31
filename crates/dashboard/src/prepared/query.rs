//! Host-direct reads of the integrated slice's prepared pages.

mod served;

#[cfg(test)]
mod profile_tests;
pub(crate) use dashboard_base::prepared::query::progressive;

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod factions_tests;
#[cfg(test)]
mod forces_tests;
#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod stored_tests;

use crate::{
    Source,
    metadata::check_cancel,
    products::{ContentOptions, Product},
    transport::{MAX_REPLY_BYTES, ProductRequest, bounded_json},
};
use dashboard_base::prepared::{gates, published};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusPoint {
    branch_id: String,
    tag: String,
    hour: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusCompletions {
    branch_id: String,
    tag: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusUnavailable {
    branch_id: String,
    hour: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusStaticText {
    /// The database every token is looked up in.
    database: crate::FocusDatabase,
    tokens: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FocusPage {
    branch_id: String,
    tag: String,
    hour: i32,
    pages: crate::prepared_focus::Pages,
    page: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductionSelection {
    hour: i32,
    selection: crate::products::equipment_produced::ProducedParams,
    offset: String,
    limit: usize,
    #[serde(default)]
    ranked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reply<'a> {
    source_id: &'a str,
    revision: &'a str,
    product: &'a str,
    /// For a pending or partial reply ([`waits`]), the preparation status
    /// entries it waits for: `walk` for the hourly walk's coverage, or a
    /// family prepared on its own, listed under its name in the status's
    /// `families`. A client refreshes when one moves and its shown-reply interval
    /// has elapsed, or immediately when every gate completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<Vec<&'static str>>,
    /// The requested hours the families it reads will not answer, and why
    /// (`PreparedHole`): never pending, never shown as values.
    #[serde(skip_serializing_if = "Option::is_none")]
    holes: Option<&'a [super::coverage::Hole]>,
    /// Written after the rest, as the answer's text ([`served::Served::write`]).
    data: (),
}

struct CachedQuery {
    data: served::Answer,
    /// Whether `data` is pending or partial ([`waits`]).
    waits: bool,
    short: std::collections::BTreeSet<&'static str>,
    /// The holes the gates it read found in the hours it answers.
    holes: Vec<super::coverage::Hole>,
    summaries: super::series::SummaryCache,
    /// What computing `data` drew in the recording's style because the source's visual mods
    /// could not supply it, named on every reply served from it.
    fallbacks: std::collections::BTreeSet<String>,
    /// Whether computing `data` added to state of the whole session, which no key captures:
    /// the content's fallbacks or drawing diagnostics.
    session: bool,
    /// The lazy readers computing `data` used, whose warnings it names.
    readers: std::collections::BTreeSet<dashboard_text::catalogue::Reader>,
}

/// Whether the content bound for `content` holds a drawing diagnostic. Memoised artwork replays
/// none of its own, so only an empty set shows an answer drew none.
pub(crate) fn drawn(source: &Source, content: Option<&ContentOptions>) -> bool {
    content
        .and_then(|content| source.bound_content(&content.options))
        .is_some_and(|content| !content.layers.shader_warning_lines().is_empty())
}

#[cfg(test)]
thread_local! {
    /// Prepared answers this thread computed rather than took from a cache.
    pub(crate) static COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Run as each answer is computed under content, standing in for what its product notes.
    pub(crate) static COMPUTING: std::cell::Cell<Option<fn(&Source, &ContentOptions)>> =
        const { std::cell::Cell::new(None) };
    /// Every leader's careers this thread read rather than took from their slot ([`all_careers`]).
    pub(crate) static CAREERS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a complete answer kept on disk between sessions ([`stored_answer`]) keeps beside its
/// text ([`served::Answer::keep`]).
#[derive(Serialize, Deserialize)]
struct StoredQuery {
    fallbacks: std::collections::BTreeSet<String>,
    readers: std::collections::BTreeSet<dashboard_text::catalogue::Reader>,
}

/// A digest of `key`, which kept answers are filed under. The key stamps every file of the
/// content stack, tens of thousands, so it is serialized and hashed once per key, not per answer.
fn key_digest(key: &store::SourceKey) -> io::Result<String> {
    use sha2::Digest;
    static DIGESTS: std::sync::Mutex<Vec<(store::SourceKey, String)>> =
        std::sync::Mutex::new(Vec::new());
    let mut digests = DIGESTS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(at) = digests.iter().position(|(held, _)| held == key) {
        let found = digests.remove(at);
        let digest = found.1.clone();
        digests.insert(0, found);
        return Ok(digest);
    }
    let digest = sha2::Sha256::digest(serde_json::to_vec(key)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    digests.insert(0, (key.clone(), digest.clone()));
    // The keys of the recordings open at once.
    digests.truncate(8);
    Ok(digest)
}

/// Where `request`'s answer at `publication` is kept on disk: under the store's key, every page
/// published to any of the recording's branches and the executable the engine's tables were read
/// from, so any later publication, Clear, content or game change misses it, whatever it reads.
fn stored_answer(
    source: &Source,
    request: &ProductRequest,
    content: Option<&ContentOptions>,
    published: &gates::Published,
    key: &store::SourceKey,
    publication: &str,
) -> io::Result<Option<crate::reply_cache::Entry>> {
    let Some(content) = content else {
        return Ok(None);
    };
    // Art drawn from a visual mod joins the files every reply names as shown from mods, which an
    // answer served from disk would leave out: only the recording's own presentation keeps any.
    if !dashboard_base::catalogue::reads_recorded(content) {
        return Ok(None);
    }
    let branches = source.with_metadata(|metadata| {
        metadata
            .branches
            .iter()
            .filter_map(|branch| branch.id.parse::<u64>().ok())
            .collect::<Vec<_>>()
    });
    let mut pages = Vec::new();
    for branch in branches {
        pages.extend(super::pipeline::published_pages(published, branch)?);
    }
    let executable = engine_executable::executable_tables::is_initialized().then(|| {
        let identity = engine_executable::executable_tables::get().identity();
        (identity.size, identity.sha256)
    });
    let identity = serde_json::to_string(&(publication, pages, key_digest(key)?, executable))?;
    let dependent =
        crate::products::find(&request.product).is_none_or(|product| product.content_dependent);
    Ok(crate::reply_cache::Entry::prepared(
        source,
        &request.product,
        dependent,
        &request.params,
        &identity,
        content,
    ))
}

/// Content files and the recorded mod stack are bound by the observation
/// scope's SourceKey. Presentation also depends on today's language and folders
/// (including the verification choice), even when files() reuses that key.
/// Preserve settings errors in the binding: products may present a warning
/// rather than refusing the entire reply.
#[derive(Serialize)]
struct ReplyPresentation {
    content: Option<dashboard_base::catalogue::Binding>,
    localisation: Option<Result<dashboard_base::content::LocalisationKey, String>>,
    force_pow2_textures: Option<Result<bool, String>>,
}

fn reply_presentation(content: Option<&ContentOptions>) -> ReplyPresentation {
    ReplyPresentation {
        content: content.map(dashboard_base::catalogue::binding),
        localisation: content.map(|options| {
            dashboard_base::content::localisation_key(options).map_err(|error| error.to_string())
        }),
        force_pow2_textures: content.map(|options| {
            installed_content_settings::settings::read_keys(&options.options.user_dir)
                .map(|settings| settings.texture_decode().force_pow2_textures)
                .map_err(|error| error.to_string())
        }),
    }
}

fn cache_product(product: &str) -> Option<&'static str> {
    Some(match product {
        "series-summary" => "queries:series-summary",
        "series-state" => "queries:series-state",
        "air-superiority" => "queries:air-superiority",
        "event-feed" => "queries:event-feed",
        "supply-health" => "queries:supply-health",
        "party-history" => "queries:party-history",
        "division-log-manifest" => "queries:division-log-manifest",
        "air-aces" => "queries:air-aces",
        "army-size" => "queries:army-size",
        "volunteers-expeditionaries" => "queries:volunteers-expeditionaries",
        "trade-flows" => "queries:trade-flows",
        "wars" => "queries:wars",
        "casualty-breakdown" => "queries:casualty-breakdown",
        "military-air-ledger" => "queries:military-air-ledger",
        "special-projects" => "queries:special-projects",
        "resource-balance" => "queries:resource-balance",
        "air-missions" => "queries:air-missions",
        "air-statistics" => "queries:air-statistics",
        "regional-weather" => "queries:regional-weather",
        "minefields" => "queries:minefields",
        "naval-supremacy" => "queries:naval-supremacy",
        "doctrine-mastery" => "queries:doctrine-mastery",
        "occupation-reserve-manpower" => "queries:occupation-reserve-manpower",
        "intel-network-coverage" => "queries:intel-network-coverage",
        "naval-repairs" => "queries:naval-repairs",
        "naval-refits" => "queries:naval-refits",
        "division-equipment-history" => "queries:division-equipment-history",
        "faction-state" => "queries:faction-state",
        "diplomacy-state" => "queries:diplomacy-state",
        "fuel-branches" => "queries:fuel-branches",
        "market-history" => "queries:market-history",
        "market-ledger" => "queries:market-ledger",
        "lend-lease-balance" => "queries:lend-lease-balance",
        "production-catalogue" => "queries:production-catalogue",
        "convoy-health" => "queries:convoy-health",
        _ => return None,
    })
}

/// Every leader's career on one branch as its family last published them, kept in a slot of its
/// own rather than the observation cache, whose least recently used entries other products'
/// reads displace: every selection and hour of the leaders' widgets is projected from it.
#[derive(Default)]
struct AllCareers(
    Option<(
        store::SourceKey,
        u64,
        Vec<(String, Option<[u64; 3]>)>,
        std::sync::Arc<Option<crate::products::unit_leaders::prepared::Careers>>,
    )>,
);

/// [`crate::products::unit_leaders::prepared::Careers`] of `branch`, read once per publication
/// of the leaders' own family: the identities a selection reads are applied as it is answered.
fn all_careers(
    source: &Source,
    published: &gates::Published,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    branch_id: &str,
    cancelled: &AtomicBool,
) -> io::Result<std::sync::Arc<Option<crate::products::unit_leaders::prepared::Careers>>> {
    let pages = super::pipeline::gate_pages(published, branch, "unit-leaders")?;
    let state = source.state::<AllCareers>();
    // Held while they are read, so the leaders' widgets asking at once read them once.
    let mut kept = state.lock().unwrap_or_else(|error| error.into_inner());
    check_cancel(cancelled)?;
    if let Some((held, held_branch, held_pages, careers)) = &kept.0
        && held == key
        && *held_branch == branch
        && *held_pages == pages
    {
        return Ok(std::sync::Arc::clone(careers));
    }
    #[cfg(test)]
    CAREERS_READ.with(|count| count.set(count.get() + 1));
    let careers = std::sync::Arc::new(crate::products::unit_leaders::prepared::Careers::read(
        store, key, branch_id, cancelled,
    )?);
    // A cancelled read may have stopped short of a whole one.
    check_cancel(cancelled)?;
    kept.0 = Some((key.clone(), branch, pages, std::sync::Arc::clone(&careers)));
    Ok(careers)
}

/// Whether a reply is pending (null) or partial (its coverage is missing or
/// incomplete), so that its client re-asks as preparation goes on.
fn waits(data: &Value) -> bool {
    match data {
        Value::Null => true,
        Value::Object(reply)
            if reply
                .get("approximate")
                .and_then(Value::as_array)
                .is_some_and(|spans| !spans.is_empty()) =>
        {
            true
        }
        Value::Object(reply) => reply
            .get("preparedCoverage")
            .or_else(|| reply.get("coverage"))
            .is_some_and(|coverage| {
                coverage.is_null()
                    || coverage.get("complete").and_then(Value::as_bool) != Some(true)
            }),
        _ => false,
    }
}

/// Supply health's reply from its family's publication, joined with the identity histories;
/// null before the family publishes the branch.
fn supply_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::supply_health;
    let branch = params["branchId"]
        .as_str()
        .and_then(|id| id.parse::<u64>().ok())
        .ok_or_else(|| crate::invalid("Invalid supply branch"))?;
    let Some(opened) = published::open(
        source,
        content,
        "supply",
        supply_health::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(Value::Null);
    };
    let (store, key) = super::files(source, content)?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "supply-health",
        cancelled,
    )?;
    let tags: Vec<String> = serde_json::from_value(params["tags"].clone()).unwrap_or_default();
    let history = dashboard_base::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        coverage.as_ref(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        source.replay_ref(),
        &tags,
        cancelled,
    )?;
    let reply = opened.read(cancelled, |snapshot, directory| {
        supply_health::query::reply_to(snapshot, directory, params, &history, cancelled)
    })?;
    Ok(serde_json::to_value(reply)?)
}

/// A yard view at an hour, from its family's publication (the family is named as its product);
/// null past what the family has accepted.
fn yards_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    product: &str,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use dashboard_naval_yards::products::yards::{self, at, layout};
    let params: at::YardsAtParams = serde_json::from_value(params.clone())?;
    params.validate().map_err(crate::invalid)?;
    let branch = branch(source, &params.branch_id)?;
    let Some(opened) = published::open(source, content, product, layout::SCHEMA, branch)?
    else {
        return Ok(Value::Null);
    };
    // An unloaded catalogue is the reply's warning, never names silently missing.
    let catalogue = content
        .map(|options| crate::catalogue::catalogue(source, options))
        .transpose()
        .map_err(|why| why.to_string());
    let refits = product == "naval-refits";
    let Some(reply) = opened.read(cancelled, |snapshot, directory| {
        let catalogue = catalogue.as_ref().map(Option::as_ref).map_err(String::as_str);
        at::query(snapshot, directory, refits, &params, catalogue)
    })?
    else {
        return Ok(Value::Null);
    };
    // The view's gate's prefix: hours after it are pending, never empty.
    let prefix = super::prefix::read_or_empty(source, content, product, branch, None, cancelled)?;
    let mut data = yards::checked(&reply)?;
    prefix.mark(&mut data)?;
    Ok(data)
}

/// Provinces' and states' names at their hours from the place-names family's publication; null
/// before the family publishes the branch, and while an hour asked is past what it accepted.
fn names_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::place_names::{self, PlaceNamesParams, emit, query};
    let params: PlaceNamesParams = serde_json::from_value(params.clone())?;
    place_names::validate(&params).map_err(crate::invalid)?;
    let branch = branch(source, &params.branch_id)?;
    let Some(opened) = published::open(source, content, place_names::NAME, emit::SCHEMA, branch)?
    else {
        return Ok(Value::Null);
    };
    let context = crate::products::LoadContext {
        content: content.cloned(),
    };
    let reply = opened.read(cancelled, |snapshot, directory| {
        query::reply(source, &context, snapshot, directory, branch, &params, cancelled)
    })?;
    Ok(match reply {
        Some(reply) => serde_json::to_value(reply)?,
        None => Value::Null,
    })
}

/// A country's manpower view at an hour: its operands from the manpower family, its holder's
/// elections and exiles from `politics_state`, and each exile card's air crews from the family;
/// null past what the family has accepted.
fn people_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::{country_people, country_people::query, politics_state::history};
    use dashboard_base::products::Product as _;
    let params: country_people::PeopleParams = serde_json::from_value(params.clone())?;
    country_people::CountryPeople::validate(&params).map_err(crate::invalid)?;
    let branch = branch(source, &params.branch_id)?;
    let Some(opened) = published::open(
        source,
        content,
        country_people::NAME,
        country_people::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(Value::Null);
    };
    let hour = params.hour;
    let Some(found) = opened.read(cancelled, |snapshot, directory| {
        if hour > snapshot.publication().accepted.last {
            return Ok(None);
        }
        query::operands_at(snapshot, directory, &params.tag, hour).map(Some)
    })?
    else {
        return Ok(Value::Null);
    };
    let (operands, (slot, _)) =
        found.ok_or_else(|| crate::invalid("Country unavailable at this hour"))?;
    let (store, key) = super::files(source, content)?;
    let politics = history::at_hour(source, &store, &key, branch, slot, hour, cancelled)?;
    let air = opened.read(cancelled, |snapshot, directory| {
        politics
            .iter()
            .flat_map(|at| &at.exiles)
            .map(|exile| match &exile.tag {
                Some(tag) => Ok(query::operands_at(snapshot, directory, tag, hour)?
                    .and_then(|(operands, _)| operands.deployed[4])),
                None => Ok(None),
            })
            .collect::<io::Result<Vec<_>>>()
    })?;
    let defines = content
        .ok_or_else(|| "Recorded content is unavailable".to_owned())
        .and_then(|options| {
            let layers = source
                .content(&options.options)
                .map_err(|error| error.to_string())?;
            country_people::content_defines(&layers.layers)
        });
    Ok(serde_json::to_value(query::assemble(
        &params,
        &operands,
        defines,
        politics.as_ref(),
        &air,
    ))?)
}

/// The equipment stockpile at an hour, its catalogue or its chart lines, from its family's
/// publication; null before the family publishes the branch, and an hour past what it accepted.
fn stockpile_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    product: &str,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::equipment_stockpile::{at, catalogue, content::Reading, emit};
    let id = params["branchId"]
        .as_str()
        .ok_or_else(|| crate::invalid("Invalid stockpile branch"))?;
    let branch = branch(source, id)?;
    let Some(opened) = published::open(source, content, "stockpile", emit::SCHEMA, branch)? else {
        return Ok(Value::Null);
    };
    let (_, key) = super::files(source, content)?;
    let context = crate::products::LoadContext {
        content: content.cloned(),
    };
    let reading = Reading::new(source, &context, &key)?;
    let mut data = match product {
        "equipment-stockpile-at" => {
            let params: at::StockpileAtParams = serde_json::from_value(params.clone())?;
            params.validate().map_err(crate::invalid)?;
            let reply = opened.read(cancelled, |snapshot, directory| {
                at::query(snapshot, directory, &params, &reading, cancelled)
            })?;
            match reply {
                Some(reply) => serde_json::to_value(reply)?,
                None => return Ok(Value::Null),
            }
        }
        "equipment-stockpile-catalogue" => {
            let params: catalogue::StockpileCatalogueParams =
                serde_json::from_value(params.clone())?;
            params.validate().map_err(crate::invalid)?;
            serde_json::to_value(opened.read(cancelled, |snapshot, directory| {
                catalogue::query(snapshot, directory, &params, &reading, cancelled)
            })?)?
        }
        _ => {
            let params: catalogue::StockpileChartParams = serde_json::from_value(params.clone())?;
            params.validate().map_err(crate::invalid)?;
            serde_json::to_value(opened.read(cancelled, |snapshot, directory| {
                catalogue::chart(snapshot, directory, &params, &reading, cancelled)
            })?)?
        }
    };
    // A later read can still classify a variant again until the branch is prepared.
    super::prefix::read_or_empty(source, content, product, branch, None, cancelled)?
        .mark_revisable(source, branch, &mut data)?;
    Ok(data)
}

/// Keyboard navigation over supply health's runs; null before the identity histories or the
/// family publish the branch.
fn supply_next_change(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::supply_health;
    let branch = params["branchId"]
        .as_str()
        .and_then(|id| id.parse::<u64>().ok())
        .ok_or_else(|| crate::invalid("Invalid branch"))?;
    let Some(history) = crate::products::country_history::prepared_branch_histories(
        source, branch, content, cancelled,
    )?
    else {
        return Ok(Value::Null);
    };
    let Some(opened) = published::open(
        source,
        content,
        "supply",
        supply_health::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(Value::Null);
    };
    opened.read(cancelled, |snapshot, directory| {
        supply_health::query::next_change(snapshot, directory, params, &history, cancelled)
    })
}

/// The politics ledger from its family's publication, named through the recorded mod stack;
/// pending before the family publishes the branch.
fn ledger_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::politics_ledger;
    let Some(opened) = published::open(
        source,
        content,
        "politics_ledger",
        politics_ledger::emit::SCHEMA,
        branch,
    )?
    else {
        return politics_ledger::query::unpublished(params);
    };
    let recorded = politics_ledger::prepared_content(source, content);
    let recorded = recorded.as_ref().ok().and_then(Option::as_ref);
    opened.read(cancelled, |snapshot, directory| {
        politics_ledger::query::query(params, snapshot, directory, recorded, cancelled)
    })
}

/// Army size from the army family's publication, a template roster from the recording; null
/// before the family publishes the branch.
fn army_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::army_size;
    let params: army_size::ArmySizeParams = serde_json::from_value(params.clone())?;
    let context = crate::products::LoadContext {
        content: content.cloned(),
    };
    if params.template_at.is_some() {
        return Ok(serde_json::to_value(army_size::template_roster(
            source, &params, &context, cancelled,
        )?)?);
    }
    let branch = branch(source, &params.branch_id)?;
    let Some(opened) = published::open(source, content, "army", army_size::emit::SCHEMA, branch)?
    else {
        return Ok(Value::Null);
    };
    let (store, key) = super::files(source, content)?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "army-size",
        cancelled,
    )?;
    let history = dashboard_base::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        coverage.as_ref(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        source.replay_ref(),
        &params.tags,
        cancelled,
    )?;
    let preparing = !coverage.as_ref().is_some_and(|coverage| coverage.complete);
    let reply = opened.read(cancelled, |snapshot, directory| {
        army_size::query::reply(
            source, &context, &params, snapshot, directory, &history, preparing, cancelled,
        )
    })?;
    Ok(serde_json::to_value(reply)?)
}

/// An `army-size` series request (charts or navigation) answered from the army family's
/// publication; null before it publishes the branch.
fn army_series(
    source: &Source,
    content: Option<&ContentOptions>,
    product: &str,
    params: &Value,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::army_size;
    let Some(opened) = published::open(source, content, "army", army_size::emit::SCHEMA, branch)?
    else {
        return Ok(Value::Null);
    };
    let tags: Vec<String> = serde_json::from_value(params["tags"].clone()).unwrap_or_default();
    let (store, key) = super::files(source, content)?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "army-size",
        cancelled,
    )?;
    let history = dashboard_base::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        coverage.as_ref(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        source.replay_ref(),
        &tags,
        cancelled,
    )?;
    match product {
        "series-summary" => {
            let start = source
                .replay_ref()
                .branches
                .iter()
                .find(|start| start.id == branch)
                .ok_or_else(|| crate::invalid("Unknown army branch"))?;
            let bounds = crate::metadata::branch_bounds(source.replay_ref(), start);
            opened.read(cancelled, |snapshot, directory| {
                army_size::query::summary(params, snapshot, directory, &history, bounds, cancelled)
            })
        }
        "series-next-change" => opened.read(cancelled, |snapshot, directory| {
            army_size::query::next_change(params, snapshot, directory, &history, cancelled)
        }),
        _ => Err(crate::invalid("Unknown prepared series family")),
    }
}

/// A country's division equipment history from its family's publication, marked with its
/// gate's prefix; null before the family publishes the branch.
fn division_equipment_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::division_equipment::history;
    let id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::invalid("Missing prepared query branch"))?;
    let branch = branch(source, id)?;
    let Some(opened) = published::open(
        source,
        content,
        history::NAME,
        history::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(Value::Null);
    };
    let reply = opened.read(cancelled, |snapshot, directory| {
        history::query::reply(source, content, params, snapshot, directory, cancelled)
    })?;
    dashboard_base::products::check_reply_budget(&reply)?;
    let mut data = serde_json::to_value(reply)?;
    super::prefix::read_or_empty(source, content, history::QUERY, branch, None, cancelled)?
        .mark(&mut data)?;
    Ok(data)
}

/// Volunteers and expeditionaries from the army family's publication through the gate's
/// prefix; every accepted hour unobserved before the family publishes the branch.
fn volunteers_reply(
    source: &Source,
    content: Option<&ContentOptions>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::{army_size, volunteers_expeditionaries as aid};
    let params: aid::VolunteersExpeditionariesParams = serde_json::from_value(params.clone())?;
    let branch = aid::branch(&params)?;
    let prefix = super::prefix::read_or_empty(
        source,
        content,
        "volunteers-expeditionaries",
        branch,
        None,
        cancelled,
    )?;
    let reply = match published::open(source, content, "army", army_size::emit::SCHEMA, branch)? {
        Some(opened) => opened.read(cancelled, |snapshot, directory| {
            aid::read(snapshot, directory, &params, prefix, cancelled)
        })?,
        None => aid::unpublished(prefix),
    };
    Ok(serde_json::to_value(aid::finish(source, branch, reply)?)?)
}

/// A `country-stability` series request answered from the stability family's publication;
/// null before it publishes the branch.
fn stability_series(
    source: &Source,
    content: Option<&ContentOptions>,
    product: &str,
    params: &Value,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::country_stability;
    let Some(opened) = published::open(
        source,
        content,
        "stability",
        country_stability::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(Value::Null);
    };
    opened.read(cancelled, |snapshot, directory| {
        country_stability::query::query(product, params.clone(), snapshot, directory, cancelled)
    })
}

fn branch(source: &Source, value: &str) -> io::Result<u64> {
    let id = value
        .parse()
        .map_err(|_| crate::invalid("Invalid prepared query branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|branch| branch.id == id)
    {
        return Err(crate::invalid(
            "Prepared query branch is not in this recording",
        ));
    }
    Ok(id)
}

/// Whether `product`'s data on `branch` is accepted through `hour`, or
/// completely: the coverage of the family prepared on its own that gates
/// the product, or the branch's ([`super::pipeline::product_coverage`]).
fn coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    product: &str,
    branch: u64,
    hour: Option<i32>,
    require_complete: bool,
    cancelled: &AtomicBool,
) -> io::Result<bool> {
    let Some(coverage) =
        super::pipeline::product_coverage(source, content, branch, product, cancelled)?
    else {
        return Ok(false);
    };
    if !coverage.complete
        && (require_complete || hour.is_some_and(|hour| hour > coverage.last_hour))
    {
        return Ok(false);
    }
    Ok(true)
}

/// Whether an experience series request names banked measures alone: its
/// measure, and each of the measures it navigates by.
fn banked_experience(params: &Value) -> bool {
    let banked = |measure: Option<&str>| {
        measure.is_some_and(|measure| {
            crate::products::country_experience::SAMPLED_MEASURES.contains(&measure)
        })
    };
    banked(
        params
            .get("family")
            .and_then(|family| family.get("measure"))
            .and_then(Value::as_str),
    ) && params
        .get("measures")
        .and_then(Value::as_array)
        .is_none_or(|measures| measures.iter().all(|measure| banked(measure.as_str())))
}

fn series_coverage(product: &str, params: &Value) -> io::Result<(Option<i32>, bool)> {
    let invalid = || crate::invalid("Invalid series query hour");
    let integer = |value: &Value| -> io::Result<i32> {
        i32::try_from(value.as_i64().ok_or_else(invalid)?).map_err(|_| invalid())
    };
    let terminal = |field: &str| -> io::Result<i32> {
        let hour = params
            .get(field)
            .and_then(Value::as_f64)
            .ok_or_else(invalid)?
            .ceil();
        if !hour.is_finite() || hour < f64::from(i32::MIN) || hour > f64::from(i32::MAX) {
            return Err(invalid());
        }
        Ok(hour as i32)
    };
    match product {
        "series-state" => {
            let reads = params
                .get("reads")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            let mut last = None;
            for read in reads {
                let hour = integer(read.get("hour").ok_or_else(invalid)?)?;
                last = Some(last.map_or(hour, |old: i32| old.max(hour)));
            }
            Ok((last, false))
        }
        "series-summary" | "events-clusters" => Ok((Some(terminal("to")?), false)),
        "series-next-change" => Ok((Some(terminal("lastHour")?), false)),
        "events-page" => match params.get("to").filter(|value| !value.is_null()) {
            Some(_) => Ok((Some(terminal("to")?), false)),
            None => Ok((None, true)),
        },
        _ => unreachable!("only explicit series products reach this helper"),
    }
}

/// Whether a prepared query answers `product`; any other is its product route's.
pub(crate) fn answers(product: &str) -> bool {
    Hours::of(&ProductRequest {
        source_id: String::new(),
        revision: String::new(),
        product: product.into(),
        params: Value::Null,
    })
    .is_ok()
}

/// What a product request asks of its branch, as its request type names its hours.
#[derive(Clone, Copy, Debug)]
enum Hours {
    /// Every hour of the branch: it names none, or its readers read history past the hours it
    /// names, or bound their own within it.
    Branch,
    /// One hour: its own, or the last of the window it names.
    At(i32),
    /// The recording block window at this index ([`Source::block_windows`]).
    Block(usize),
    /// The espionage log's daily transport window at this index.
    Day(usize),
    /// The hours through the field `to` names (the branch's end while the request names none),
    /// none of which it asks from the hours the fields `before` name, when it names them.
    Range {
        before: &'static [&'static str],
        to: &'static str,
    },
    /// State reads, each naming its hour.
    Reads,
}

impl Hours {
    /// What `request` asks, as its product's request type names its hours. An hour it names
    /// malformed asks every hour, and its product refuses it as before. Every product has its
    /// own entry: one without is no prepared query.
    fn of(request: &ProductRequest) -> io::Result<Self> {
        let params = &request.params;
        let named = |field: &str| params.get(field).is_some_and(|value| !value.is_null());
        let view = |view: &str| params.get("view").and_then(Value::as_str) == Some(view);
        let hour = |field: &str| {
            params
                .get(field)
                .and_then(Value::as_i64)
                .and_then(|hour| i32::try_from(hour).ok())
                .map_or(Self::Branch, Self::At)
        };
        let index = |field: &str| {
            params
                .get(field)
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
        };
        let range = |before: &'static [&'static str], to: &'static str| Self::Range { before, to };
        Ok(match request.product.as_str() {
            "series-state" => Self::Reads,
            "series-summary" | "events-page" | "events-clusters" => range(&["from"], "to"),
            "series-next-change" => range(&["firstHour", "from"], "lastHour"),
            "event-feed"
            | "event-choices"
            | "decisions"
            | "division-log-totals"
            | "division-equipment-history"
            | "decryption"
            | "operative-placements"
            | "agency-comparison"
            | "operatives-roster" => range(&["firstHour"], "lastHour"),
            "equipment-stockpile-at" | "naval-repairs" | "naval-refits" | "country-people" => {
                hour("hour")
            }
            // Names of rows each at its own hour: through the last one asked.
            "place-names" => ["places", "states"]
                .into_iter()
                .filter_map(|field| params.get(field).and_then(Value::as_array))
                .flatten()
                .filter_map(|entry| entry.get("hour")?.as_i64())
                .max()
                .and_then(|hour| i32::try_from(hour).ok())
                .map_or(Self::Branch, Self::At),
            "politics-ledger" if !named("navigation") && !named("marks") && !named("within") => {
                range(&["from"], "hour")
            }
            "idea-history" if named("through") => range(&["hour"], "through"),
            "naval-losses" if named("hour") => hour("hour"),
            "air-aces" if view("roster") => hour("hour"),
            "diplomacy-state" if view("opinions") => hour("hour"),
            "unit-leaders" if view("leaderboard") => hour("hour"),
            "focus-hour" | "focus-unavailable" | "focus-order-page" | "production-select"
            | "ships-sunk" | "idea-history" => hour("hour"),
            "naval-losses" | "casualty-breakdown" | "casualty-pairs" | "occupied-states" => {
                index("window").map_or(Self::Branch, Self::Block)
            }
            "espionage-log" => index("window").map_or(Self::Branch, Self::Day),
            "production-line-detail" => params
                .get("window")
                .and_then(Value::as_i64)
                .and_then(|window| window.checked_add(1))
                .and_then(|window| {
                    window.checked_mul(i64::from(crate::products::production_lines::DETAIL_HOURS))
                })
                .and_then(|hour| i32::try_from(hour - 1).ok())
                .map_or(Self::Branch, Self::At),
            // Histories, manifests, catalogues and views of the whole branch; views whose
            // readers read history past the hour they name (careers and choices, special
            // projects' episodes, ledger navigation); and products whose readers bound their own
            // hours within it: battle and construction windows, cumulative spending a later hour
            // can revise.
            dashboard_base::products::country_styles::FACTS_PRODUCT
            | "production-lines"
            | "production-line-breakdown"
            | "diplomacy-state"
            | "party-history"
            | "fuel-branches"
            | "market-history"
            | "market-ledger"
            | "lend-lease-balance"
            | "production-catalogue"
            | "convoy-health"
            | "civil-wars"
            | "politics-state-history"
            | "politics-ledger"
            | "scoreboard"
            | "equipment-stockpile-catalogue"
            | "equipment-stockpile-chart"
            | "equipment-losses"
            | "land-battles"
            | "land-battles-manifest"
            | "land-battles-ranking"
            | "land-battle-course"
            | "naval-battles"
            | "naval-battles-manifest"
            | "naval-battles-ranking"
            | "naval-battle-details"
            | "division-log"
            | "division-log-manifest"
            | "ships-sunk-manifest"
            | "ships-sunk-battle-titles"
            | "volunteers-expeditionaries"
            | "army-size"
            | "supply-health"
            | "land-owners"
            | "country-identities"
            | "air-missions"
            | "air-statistics"
            | "regional-weather"
            | "minefields"
            | "naval-supremacy"
            | "doctrine-mastery"
            | "occupation-reserve-manpower"
            | "intel-network-coverage"
            | "air-superiority"
            | "raids"
            | "air-aces"
            | "air-ace-portraits"
            | "military-air-ledger"
            | "construction-log"
            | "construction-log-manifest"
            | "construction-spending"
            | "resource-balance"
            | "trade-flows"
            | "production-spending"
            | "dockyard-spending"
            | "military-factory-spending"
            | "wars"
            | "subjects"
            | "occupied-states-manifest"
            | "unit-leaders"
            | "faction-state"
            | "special-projects"
            | "country-groups"
            | "focus-completions"
            | "focus-static-text" => Self::Branch,
            product => return Err(crate::invalid(format!("Unknown prepared query: {product}"))),
        })
    }

    /// The hour or range its window index names on `branch`, the branch the request names.
    fn resolve(self, source: &Source, branch: Option<u64>) -> Self {
        let windows = branch.map_or(&[][..], |branch| source.block_windows(branch));
        let window = match self {
            Self::Block(index) => windows.get(index).copied(),
            Self::Day(index) => match (windows.first(), windows.last()) {
                (Some(first), Some(last)) => crate::products::espionage_log::day_window(
                    first.first_hour,
                    last.last_hour,
                    index,
                ),
                _ => None,
            },
            hours => return hours,
        };
        window.map_or(Self::Branch, |window| Self::At(window.last_hour))
    }

    /// The last hour it asks in `params`; None for every hour of the branch.
    fn through(self, params: &Value) -> Option<i32> {
        let parse = |value: &Value| {
            value
                .as_f64()
                .map(f64::ceil)
                .filter(|hour| (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(hour))
                .map(|hour| hour as i32)
        };
        match self {
            Self::Branch | Self::Block(_) | Self::Day(_) => None,
            Self::At(hour) => Some(hour),
            Self::Range { to, .. } => params.get(to).and_then(parse),
            Self::Reads => params
                .get("reads")?
                .as_array()?
                .iter()
                .map(|read| read.get("hour").and_then(parse))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .max(),
        }
    }

    /// `params` asking only the hours before `hole`, the first hole its gate leaves in them:
    /// nothing it asks from the hole on is answered, since those values need observations the
    /// hole lacks. Only fields the request names are rewritten: a range ends before the hole and
    /// a state keeps only its reads before it, while hours it does not name end where its
    /// coverage, cut before the hole, ends. None when what it asks lies from the hole on.
    fn before(self, params: &Value, hole: i32) -> io::Result<Option<Value>> {
        let Some(last) = hole.checked_sub(1) else {
            return Ok(None);
        };
        let invalid = || crate::invalid("Invalid prepared query hours");
        let after = |value: Option<&Value>| {
            value
                .and_then(Value::as_f64)
                .map(|hour| hour.ceil() > f64::from(last))
        };
        let mut params = params.clone();
        match self {
            Self::Branch | Self::Block(_) | Self::Day(_) => Ok(Some(params)),
            Self::At(hour) => Ok((hour <= last).then_some(params)),
            Self::Range { before, to } => {
                if before
                    .iter()
                    .any(|field| after(params.get(*field)) == Some(true))
                {
                    return Ok(None);
                }
                if after(params.get(to)) == Some(true) {
                    params
                        .as_object_mut()
                        .ok_or_else(invalid)?
                        .insert(to.to_owned(), Value::from(last));
                }
                Ok(Some(params))
            }
            Self::Reads => {
                let reads = params
                    .get_mut("reads")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(invalid)?;
                reads.retain(|read| {
                    read.get("hour")
                        .and_then(Value::as_f64)
                        .is_some_and(|hour| hour <= f64::from(last))
                });
                Ok((!reads.is_empty()).then_some(params))
            }
        }
    }
}

/// A request its entry point admitted ([`admit`]): the only way to its product's readers.
struct Admitted<'a> {
    request: &'a ProductRequest,
    /// What its gate answers for its hours.
    ranges: super::coverage::Ranges,
}

/// The one way a query reaches its product's readers. The hours its request names ([`Hours`])
/// answer with the refusal of a family it reads when they reach that refusal; else they are asked
/// of its gate, cut before the gate's first hole there and declared, so that every reader answers
/// for them alone ([`super::coverage::Entry::declare`]).
fn admit(
    source: &Source,
    request: &ProductRequest,
    content: Option<&ContentOptions>,
    branch: Option<u64>,
    name: &str,
    published: &gates::Published,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let entry = super::pipeline::install_gates();
    let hours = Hours::of(request)?.resolve(source, branch);
    let through = hours.through(&request.params);
    let Some(branch) = branch else {
        let admitted = Admitted {
            request,
            ranges: super::coverage::Ranges::default(),
        };
        let declared = super::coverage::Declared {
            through,
            before: None,
        };
        return entry.declare(declared, || {
            query_data(source, &admitted, content, cancelled)
        });
    };
    if let Some((hour, error)) = super::pipeline::refusal_from(published, branch, name)?
        && through.is_none_or(|through| through >= hour)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, error));
    }
    // A series request waits for its family kind's coverage; factory faction totals wait only for
    // the families they read.
    let gate = if name == "faction-totals" {
        crate::products::faction_totals::gate(&request.params)
    } else {
        name
    };
    let ranges = match super::coverage::ranges(source, content, branch, gate, through, cancelled) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            super::coverage::Ranges::default()
        }
        other => other?,
    };
    let before = ranges.holes.first().map(|hole| hole.first_hour);
    let clipped;
    let request = match before {
        Some(hole) => match hours.before(&request.params, hole)? {
            Some(params) => {
                clipped = ProductRequest {
                    params,
                    ..request.clone()
                };
                &clipped
            }
            None => return Ok(Value::Null),
        },
        None => request,
    };
    let admitted = Admitted { request, ranges };
    let declared = super::coverage::Declared { through, before };
    entry.declare(declared, || {
        query_data(source, &admitted, content, cancelled)
    })
}

/// A casualty state request whose reads past `prefix` are read at its last
/// hour instead ([`super::prefix::Prefix::through`]).
fn casualty_reads_through(params: &Value, prefix: &super::prefix::Prefix) -> Value {
    let mut params = params.clone();
    for read in params
        .get_mut("reads")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(hour) = read
            .get("hour")
            .and_then(Value::as_i64)
            .and_then(|hour| i32::try_from(hour).ok())
            && let Some(through) = prefix.through(Some(hour))
        {
            read["hour"] = Value::from(through);
        }
    }
    params
}

/// The query of a keyed family's product ([`dashboard_keyed`]).
fn keyed_query(product: &str, query: &dashboard_keyed::query::Query<'_>) -> io::Result<Value> {
    use crate::products::{
        air_missions, air_statistics, doctrine_mastery, intel_coverage, minefields, naval_supremacy,
        occupation_reserve, regional_weather,
    };
    match product {
        "air-missions" => air_missions::query(query),
        "air-statistics" => air_statistics::query(query),
        "regional-weather" => regional_weather::query(query),
        "minefields" => minefields::query(query),
        "naval-supremacy" => naval_supremacy::query(query),
        "doctrine-mastery" => doctrine_mastery::query(query),
        "occupation-reserve-manpower" => occupation_reserve::query(query),
        "intel-network-coverage" => intel_coverage::query(query),
        _ => Err(crate::invalid("Unknown keyed product")),
    }
}

/// The economy families' whole-branch histories and ledger pages, through
/// their gate's prepared prefix; the hours after it are pending, never empty.
fn economy(
    source: &Source,
    request: &ProductRequest,
    product: &str,
    content: Option<&ContentOptions>,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use crate::products::{
        convoy_health, fuel_branches, lend_lease_balance, market, production_catalogue,
    };
    use dashboard_base::products::equipment::{self, Texts};
    let branch_id = request
        .params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::invalid("Missing economy branch"))?;
    let branch = branch(source, branch_id)?;
    let requested = (product == "market-ledger")
        .then(|| request.params.get("lastHour").and_then(Value::as_i64))
        .flatten()
        .and_then(|hour| i32::try_from(hour).ok());
    let prefix =
        super::prefix::read_or_empty(source, content, product, branch, requested, cancelled)?;
    let through = (prefix.coverage.last_hour >= prefix.coverage.first_hour)
        .then(|| prefix.through(requested).unwrap_or(i32::MAX));
    let context = crate::products::LoadContext {
        content: content.cloned(),
    };
    let mut warnings = Vec::new();
    let catalogue = |warnings: &mut Vec<String>| match context
        .content()
        .and_then(|options| dashboard_base::catalogue::catalogue(source, options))
    {
        Ok(catalogue) => Some(catalogue),
        Err(error) => {
            warnings.push(format!("Names unavailable: {error}"));
            None
        }
    };
    let mut data = match product {
        "fuel-branches" => {
            let params: fuel_branches::FuelBranchesParams =
                serde_json::from_value(request.params.clone())?;
            let labels = fuel_branches::labels(source, &context, &mut warnings);
            serde_json::to_value(fuel_branches::query(
                store, key, &params, through, labels, warnings, cancelled,
            )?)?
        }
        "market-history" => {
            let params: market::MarketHistoryParams =
                serde_json::from_value(request.params.clone())?;
            let catalogue = catalogue(&mut warnings);
            let texts = catalogue
                .as_deref()
                .map(|catalogue| catalogue as &dyn Texts);
            serde_json::to_value(market::history(
                store, key, &params, through, texts, warnings, cancelled,
            )?)?
        }
        "market-ledger" => {
            let params: market::MarketLedgerParams =
                serde_json::from_value(request.params.clone())?;
            let catalogue = catalogue(&mut warnings);
            let texts = catalogue
                .as_deref()
                .map(|catalogue| catalogue as &dyn Texts);
            serde_json::to_value(market::ledger(
                store, key, &params, through, texts, warnings, cancelled,
            )?)?
        }
        "lend-lease-balance" => {
            let params: lend_lease_balance::LendLeaseBalanceParams =
                serde_json::from_value(request.params.clone())?;
            serde_json::to_value(lend_lease_balance::query(
                store, key, &params, through, cancelled,
            )?)?
        }
        "production-catalogue" => {
            let params: production_catalogue::ProductionCatalogueParams =
                serde_json::from_value(request.params.clone())?;
            let catalogue = catalogue(&mut warnings);
            let units = catalogue.as_ref().and_then(|catalogue| {
                equipment::units(source, catalogue)
                    .map_err(|error| warnings.push(format!("Equipment types unavailable: {error}")))
                    .ok()
            });
            // Each organisation's icon is chosen and drawn together, through the catalogue the
            // request shows.
            let icon = |raw: &dashboard_base::products::production_lines::RawOrganisation| {
                use dashboard_base::products::production_lines::{
                    organisation_picture, organisation_shown,
                };
                catalogue.as_ref().and_then(|catalogue| {
                    organisation_shown(&organisation_picture(catalogue, raw, 48), catalogue, raw)
                })
            };
            let naming = production_catalogue::Naming {
                texts: catalogue
                    .as_deref()
                    .map(|catalogue| catalogue as &dyn Texts),
                units: units.as_deref(),
                icon: &icon,
            };
            serde_json::to_value(production_catalogue::query(
                store, key, &params, through, &naming, warnings, cancelled,
            )?)?
        }
        "convoy-health" => {
            let params: convoy_health::ConvoyHealthParams =
                serde_json::from_value(request.params.clone())?;
            let regions = convoy_health::region_names(source, &context, &mut warnings);
            // Only the names are shown: the recording's.
            let naming =
                dashboard_base::products::resources::ResourceNaming::stored(source, &context);
            let resources = |list: &[(u32, String)]| match naming.infos(list, cancelled) {
                Ok((infos, notes)) => (
                    infos
                        .into_iter()
                        .filter_map(|info| Some((info.id, info.name?)))
                        .collect(),
                    notes,
                ),
                Err(error) => (
                    Default::default(),
                    vec![format!("Resource names unavailable: {error}")],
                ),
            };
            serde_json::to_value(convoy_health::query(
                store, key, &params, through, &regions, &resources, warnings, cancelled,
            )?)?
        }
        _ => unreachable!("only economy products reach this helper"),
    };
    prefix.mark(&mut data)?;
    Ok(data)
}

fn query_data(
    source: &Source,
    admitted: &Admitted<'_>,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let request = admitted.request;
    let (store, key) = super::files(source, content)?;
    dashboard_base::prepared::query::observations::bind(source, &key);
    Ok(match request.product.as_str() {
        "production-lines" | "production-line-detail" | "production-line-breakdown" => {
            use crate::products::production_lines::{
                ProductionBreakdownParams, ProductionDetailParams, ProductionLineBreakdown,
                ProductionLineDetail, ProductionLines, ProductionLinesParams,
            };
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            let branch = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid production branch"))?;
            let branch = self::branch(source, branch)?;
            let through = if request.product == "production-line-detail" {
                request
                    .params
                    .get("window")
                    .and_then(Value::as_i64)
                    .and_then(|window| window.checked_add(1))
                    .and_then(|window| {
                        window
                            .checked_mul(i64::from(crate::products::production_lines::DETAIL_HOURS))
                    })
                    .and_then(|hour| i32::try_from(hour - 1).ok())
            } else {
                None
            };
            if !coverage(
                source,
                content,
                &request.product,
                branch,
                through,
                request.product == "production-line-breakdown"
                    || request.params.get("exact").and_then(Value::as_bool) == Some(true),
                cancelled,
            )? {
                return Ok(Value::Null);
            }
            let complete = crate::products::production_lines::prepared::current(
                &store, &key, source, &context, branch,
            )?;
            if !complete {
                super::coverage::note_short("production-lines");
                let directory =
                    crate::products::production_lines::prepared::directory(&store, &key, branch)?;
                if request.product == "production-line-breakdown"
                    || request.params.get("exact").and_then(Value::as_bool) == Some(true)
                    || directory.as_ref().is_none_or(|directory| {
                        through.is_some_and(|hour| hour > directory.coverage.last_hour)
                    })
                {
                    return Ok(Value::Null);
                }
            }
            match request.product.as_str() {
                "production-lines" => {
                    let params: ProductionLinesParams =
                        serde_json::from_value(request.params.clone())?;
                    ProductionLines::validate(&params).map_err(crate::invalid)?;
                    serde_json::to_value(ProductionLines::load(
                        source, &params, &context, cancelled,
                    )?)?
                }
                "production-line-detail" => {
                    let params: ProductionDetailParams =
                        serde_json::from_value(request.params.clone())?;
                    ProductionLineDetail::validate(&params).map_err(crate::invalid)?;
                    serde_json::to_value(ProductionLineDetail::load(
                        source, &params, &context, cancelled,
                    )?)?
                }
                _ => {
                    let params: ProductionBreakdownParams =
                        serde_json::from_value(request.params.clone())?;
                    ProductionLineBreakdown::validate(&params).map_err(crate::invalid)?;
                    serde_json::to_value(ProductionLineBreakdown::load(
                        source, &params, &context, cancelled,
                    )?)?
                }
            }
        }
        "diplomacy-state" => {
            use crate::products::diplomacy_state::{self as diplomacy, DiplomacyParams};
            let params: DiplomacyParams = serde_json::from_value(request.params.clone())?;
            diplomacy::DiplomacyState::validate(&params).map_err(crate::invalid)?;
            let branch = branch(source, params.branch_id())?;
            let mut prefix = super::prefix::read_or_empty(
                source,
                content,
                "diplomacy-state",
                branch,
                params.hour(),
                cancelled,
            )?;
            let pair = matches!(&params, DiplomacyParams::Pair { .. });
            if pair {
                let identities = super::prefix::read_or_empty(
                    source,
                    content,
                    "country-identities",
                    branch,
                    None,
                    cancelled,
                )?;
                prefix.coverage.last_hour =
                    prefix.coverage.last_hour.min(identities.coverage.last_hour);
                prefix.coverage.complete &= identities.coverage.complete;
                prefix = super::prefix::Prefix::new(source, branch, prefix.coverage, None);
            }
            // Opinions past the prefix wait for their hour to be prepared.
            if params.hour().is_some_and(|hour| hour > prefix.coverage.last_hour) {
                return Ok(Value::Null);
            }
            let reply = match published::open(
                source,
                content,
                "diplomacy_state",
                diplomacy::emit::SCHEMA,
                branch,
            )? {
                Some(opened) => {
                    let history = pair
                        .then(|| {
                            dashboard_base::prepared::query::progressive::histories(
                                &store,
                                &key,
                                branch,
                                Some(&prefix.coverage),
                                &std::collections::BTreeMap::new(),
                                &std::collections::BTreeMap::new(),
                                source.replay_ref(),
                                &[],
                                cancelled,
                            )
                        })
                        .transpose()?;
                    opened.read(cancelled, |snapshot, directory| {
                        diplomacy::query::reply(
                            source,
                            &params,
                            content,
                            snapshot,
                            directory,
                            history.as_ref(),
                            prefix,
                            cancelled,
                        )
                    })?
                }
                None => diplomacy::query::unpublished(&params, prefix)?,
            };
            serde_json::to_value(reply)?
        }
        "party-history" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing party branch"))?;
            let branch = branch(source, id)?;
            let accepted = match super::pipeline::product_coverage(
                source,
                content,
                branch,
                &request.product,
                cancelled,
            ) {
                Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
                other => other?,
            };
            // The full parity history includes catalogues first seen later. Native
            // popularity and prepared event pages can answer during preparation.
            if request.params.get("mode").and_then(Value::as_str) == Some("history")
                && !accepted.as_ref().is_some_and(|coverage| coverage.complete)
            {
                return Ok(Value::Null);
            }
            crate::products::party_history::prepared::query(
                source,
                serde_json::from_value(request.params.clone())?,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                accepted.as_ref(),
                cancelled,
            )?
        }

        product @ ("fuel-branches"
        | "market-history"
        | "market-ledger"
        | "lend-lease-balance"
        | "production-catalogue"
        | "convoy-health") => economy(source, request, product, content, &store, &key, cancelled)?,

        "civil-wars" => {
            use crate::products::politics_state::civil_wars::{self, CivilWarsParams};
            let params: CivilWarsParams = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            // The rival lists and the feed's annexations, through the hours
            // both have prepared.
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                None,
                cancelled,
            )?;
            let through = (prefix.coverage.last_hour >= prefix.coverage.first_hour)
                .then(|| prefix.through(None).unwrap_or(i32::MAX));
            let annexations = match through {
                Some(through) => crate::products::event_feed::prepared::script::annexations(
                    &store, &key, branch, through, cancelled,
                )?,
                None => Vec::new(),
            };
            let mut data = serde_json::to_value(civil_wars::query(
                &store,
                &key,
                &params,
                through,
                &annexations,
                cancelled,
            )?)?;
            prefix.mark(&mut data)?;
            data
        }
        "politics-state-history" => {
            use crate::products::politics_state::history::{self, PoliticsHistoryParams};
            let params: PoliticsHistoryParams = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            // Whole-branch histories through the prepared prefix; the hours
            // after it are pending, never an empty state.
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                None,
                cancelled,
            )?;
            let through = (prefix.coverage.last_hour >= prefix.coverage.first_hour)
                .then(|| prefix.through(None).unwrap_or(i32::MAX));
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            let catalogue = history::presentation_catalogue(source, &params, &context);
            let mut data = serde_json::to_value(history::query(
                &store, &key, &params, through, catalogue, cancelled,
            )?)?;
            prefix.mark(&mut data)?;
            data
        }

        "scoreboard" => crate::products::scoreboard::prepared::query(
            source,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            &store,
            &key,
            cancelled,
        )?,
        product @ ("equipment-stockpile-at"
        | "equipment-stockpile-catalogue"
        | "equipment-stockpile-chart") => {
            stockpile_reply(source, content, product, &request.params, cancelled)?
        }
        "place-names" => names_reply(source, content, &request.params, cancelled)?,
        "country-people" => people_reply(source, content, &request.params, cancelled)?,
        product @ ("land-battles"
        | "land-battles-manifest"
        | "land-battles-ranking"
        | "land-battle-course") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid battle branch"))?;
            let branch = branch(source, id)?;
            if !coverage(
                source,
                content,
                &request.product,
                branch,
                None,
                true,
                cancelled,
            )? {
                return Ok(crate::products::land_battles::prepared::prefix_query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    &store,
                    &key,
                    cancelled,
                )?
                .unwrap_or(Value::Null));
            }
            crate::products::land_battles::prepared::query(
                source,
                product,
                request.params.clone(),
                &store,
                &key,
                cancelled,
            )?
        }
        product @ ("naval-battles"
        | "naval-battles-manifest"
        | "naval-battles-ranking"
        | "naval-battle-details") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid battle branch"))?;
            let branch = branch(source, id)?;
            if !coverage(
                source,
                content,
                &request.product,
                branch,
                None,
                true,
                cancelled,
            )? {
                return Ok(crate::products::naval_battles::prepared::prefix_query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    &store,
                    &key,
                    cancelled,
                )?
                .unwrap_or(Value::Null));
            }
            crate::products::naval_battles::prepared::query(
                source,
                product,
                request.params.clone(),
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?
        }
        product @ ("division-log" | "division-log-manifest" | "division-log-totals") => {
            crate::products::division_log::prepared::first_view(
                source,
                product,
                request.params.clone(),
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                cancelled,
            )?
        }
        product @ ("ships-sunk" | "ships-sunk-manifest" | "ships-sunk-battle-titles") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid ships sunk branch"))?;
            let branch = branch(source, id)?;
            let prefix = super::prefix::read(source, content, product, branch, None, cancelled)?;
            let requested = if product == "ships-sunk" {
                Some(
                    request.params["hour"]
                        .as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        .ok_or_else(|| crate::invalid("Invalid sinking hour"))?,
                )
            } else {
                None
            };
            let mut data = if prefix.as_ref().is_some_and(|prefix| {
                requested.is_none_or(|hour| hour <= prefix.coverage.last_hour)
            }) {
                crate::products::ships_sunk::prepared::query(
                    source,
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    content,
                    cancelled,
                )?
            } else {
                crate::products::ships_sunk::prepared::empty(source, product, &request.params)?
            };
            match prefix {
                Some(prefix) => prefix.mark_revisable(source, branch, &mut data)?,
                None => super::prefix::mark_unprepared(source, branch, None, &mut data)?,
            }
            data
        }
        "naval-losses" => {
            let branch = branch(
                source,
                request
                    .params
                    .get("branchId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| crate::invalid("Invalid naval loss branch"))?,
            )?;
            let requested =
                if let Some(hour) = request.params.get("hour").filter(|hour| !hour.is_null()) {
                    Some(
                        hour.as_i64()
                            .and_then(|hour| i32::try_from(hour).ok())
                            .ok_or_else(|| crate::invalid("Invalid naval loss hour"))?,
                    )
                } else if let Some(window) = request
                    .params
                    .get("window")
                    .filter(|window| !window.is_null())
                {
                    let index = window
                        .as_u64()
                        .and_then(|index| usize::try_from(index).ok())
                        .ok_or_else(|| crate::invalid("Invalid naval loss window"))?;
                    Some(
                        source
                            .block_windows(branch)
                            .get(index)
                            .ok_or_else(|| crate::invalid("Unknown naval loss window"))?
                            .last_hour,
                    )
                } else {
                    None
                };
            let prefix = super::prefix::read(
                source,
                content,
                &request.product,
                branch,
                requested,
                cancelled,
            )?;
            let mut data = if prefix.as_ref().is_some_and(|prefix| {
                requested.is_none_or(|hour| hour <= prefix.coverage.last_hour)
            }) {
                crate::products::naval_losses::prepared::query(
                    source,
                    "naval-losses",
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    &store,
                    &key,
                    cancelled,
                )?
            } else {
                crate::products::naval_losses::prepared::empty(source, &request.params)?
            };
            match prefix {
                Some(prefix) => prefix.mark_revisable(source, branch, &mut data)?,
                None => super::prefix::mark_unprepared(source, branch, requested, &mut data)?,
            }
            data
        }
        "equipment-losses" => crate::products::equipment_losses::prepared::query(
            source,
            &request.params,
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "volunteers-expeditionaries" => {
            volunteers_reply(source, content, &request.params, cancelled)?
        }
        "division-equipment-history" => {
            division_equipment_reply(source, content, &request.params, cancelled)?
        }
        "army-size" | "supply-health" => {
            let mut data = if request.product == "army-size" {
                army_reply(source, content, &request.params, cancelled)?
            } else {
                supply_reply(source, content, &request.params, cancelled)?
            };
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid force branch"))?;
            super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch(source, id)?,
                None,
                cancelled,
            )?
            .mark(&mut data)?;
            data
        }
        "event-feed" => {
            use crate::products::event_feed::prepared::query::{Params, cursor_live};
            let mut params: Params = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            let requested = params.last_hour;
            // The feed answers backwards from the joint coverage's last hour,
            // as from any live hour inside it; a continuation keeps the live
            // hour its first page was answered through.
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                Some(requested),
                cancelled,
            )?;
            if prefix.coverage.last_hour < prefix.coverage.first_hour
                || params
                    .first_hour
                    .is_some_and(|first| first > prefix.coverage.last_hour)
            {
                let mut prefix = prefix;
                if let Some(first) = params.first_hour {
                    prefix.pending.retain_mut(|span| {
                        span.first_hour = span.first_hour.max(first);
                        span.first_hour <= span.last_hour
                    });
                }
                let mut data = serde_json::to_value(
                    crate::products::event_feed::prepared::query::empty(source, &params)?,
                )?;
                prefix.mark(&mut data)?;
                return Ok(data);
            }
            let mut live = prefix.through(Some(requested)).unwrap_or(requested);
            if let Some(kept) = params.cursor.as_deref().and_then(cursor_live)
                && kept < live
            {
                live = kept;
            }
            params.last_hour = live;
            let mut data = serde_json::to_value(crate::products::event_feed::prepared::query(
                source,
                params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?)?;
            super::prefix::Prefix {
                pending: super::prefix::pending(source, branch, live, Some(requested)),
                coverage: prefix.coverage,
            }
            .mark(&mut data)?;
            data
        }
        product @ ("event-choices" | "decisions") => {
            use crate::products::event_feed::prepared::script::{
                self, Adapter, ScriptPageParams, cursor_live,
            };
            let adapter = if product == "decisions" {
                Adapter::Decisions
            } else {
                Adapter::EventChoices
            };
            let mut params: ScriptPageParams = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            let requested = params.last_hour;
            // As the feed: pages answer backwards from the facts' prepared
            // prefix, and a continuation keeps its first page's live hour.
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                Some(requested),
                cancelled,
            )?;
            if prefix.coverage.last_hour < prefix.coverage.first_hour
                || params
                    .first_hour
                    .is_some_and(|first| first > prefix.coverage.last_hour)
            {
                let mut prefix = prefix;
                if let Some(first) = params.first_hour {
                    prefix.pending.retain_mut(|span| {
                        span.first_hour = span.first_hour.max(first);
                        span.first_hour <= span.last_hour
                    });
                }
                let first = params.first_hour.unwrap_or(prefix.coverage.first_hour);
                let mut data = serde_json::to_value(script::empty(&params, first, requested))?;
                prefix.mark(&mut data)?;
                return Ok(data);
            }
            let mut live = prefix.through(Some(requested)).unwrap_or(requested);
            if let Some(kept) = params.cursor.as_deref().and_then(cursor_live)
                && kept < live
            {
                live = kept;
            }
            params.last_hour = live;
            let mut data = serde_json::to_value(script::query(
                adapter,
                source,
                params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                cancelled,
            )?)?;
            super::prefix::Prefix {
                pending: super::prefix::pending(source, branch, live, Some(requested)),
                coverage: prefix.coverage,
            }
            .mark(&mut data)?;
            data
        }
        dashboard_base::products::country_styles::FACTS_PRODUCT => {
            use dashboard_base::products::country_styles::{CountryFactsParams, CountryFactsReply};
            let params: CountryFactsParams = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                None,
                cancelled,
            )?;
            let localisation = dashboard_base::content::name_localisation(source, content);
            let mut warnings = Vec::new();
            match (&localisation, content) {
                (Err(reason), _) => warnings.push(reason.clone()),
                // Names read from unverified content keep its standing warning.
                (Ok(_), Some(content)) => dashboard_base::content::check_replay(
                    &source.replay(),
                    content.allow_unverified,
                    &mut warnings,
                )?,
                (Ok(_), None) => {}
            }
            let mut reply = crate::products::style_runs::country_facts(
                source,
                content,
                branch,
                [prefix.coverage.first_hour, prefix.coverage.last_hour],
                localisation.as_deref().ok(),
                cancelled,
            )?
            .unwrap_or_else(|| CountryFactsReply {
                branch_id: params.branch_id.clone(),
                ..CountryFactsReply::default()
            });
            reply.warnings = warnings;
            let mut data = serde_json::to_value(reply)?;
            prefix.mark(&mut data)?;
            data
        }
        "land-owners" => {
            let _: crate::products::land_owners::LandOwnersParams =
                serde_json::from_value(request.params.clone())?;
            let Some(file) = store.read(&key, "land-owners", 0)? else {
                return Ok(Value::Null);
            };
            let scan = crate::products::land_owners::Scan::read(io::BufReader::new(file))?;
            if !scan.complete {
                return Ok(Value::Null);
            }
            check_cancel(cancelled)?;
            serde_json::to_value(scan.reply())?
        }
        "country-identities" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing prepared identity branch"))?;
            let branch = branch(source, id)?;
            serde_json::to_value(crate::products::country_history::prepared_branch_histories(
                source, branch, content, cancelled,
            )?)?
        }
        "idea-history" => {
            let options = content.ok_or_else(|| {
                crate::invalid("Choose the game and user folders in Settings first")
            })?;
            let content = crate::products::politics_content::politics_content(source, options)?;
            crate::products::idea_history::progressive::query(
                source,
                &content,
                options,
                &request.params,
                &store,
                &key,
                cancelled,
            )?
        }
        product @ ("air-missions"
        | "air-statistics"
        | "regional-weather"
        | "minefields"
        | "naval-supremacy"
        | "doctrine-mastery"
        | "occupation-reserve-manpower"
        | "intel-network-coverage") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing prepared query branch"))?;
            let branch = branch(source, id)?;
            // Keyed families answer from their gate's accepted prefix; the hours
            // after it are pending, never empty.
            let prefix =
                super::prefix::read_or_empty(source, content, product, branch, None, cancelled)?;
            let query = dashboard_keyed::query::Query {
                source,
                params: &request.params,
                content,
                store: &store,
                key: &key,
                branch,
                prefix: &prefix,
                cancelled,
            };
            let mut data = keyed_query(product, &query)?;
            prefix.mark(&mut data)?;
            data
        }
        product @ ("naval-repairs" | "naval-refits") => {
            yards_reply(source, content, product, &request.params, cancelled)?
        }
        "air-superiority" => crate::products::air_superiority::prepared::query(
            source,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "raids" => crate::products::raids::prepared::query(
            source,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "air-aces" | "air-ace-portraits" => crate::products::air_aces::prepared::query(
            source,
            &request.product,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        "military-air-ledger" => {
            let branch = branch(
                source,
                request
                    .params
                    .get("branchId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| crate::invalid("Invalid ledger branch"))?,
            )?;
            let prefix =
                super::prefix::read(source, content, &request.product, branch, None, cancelled)?;
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            let mut data = if prefix.is_some() {
                crate::products::military_air_ledger::prepared::query(
                    source,
                    &store,
                    &key,
                    request.params.clone(),
                    &context,
                    cancelled,
                )?
            } else {
                crate::products::military_air_ledger::prepared::empty(
                    source,
                    request.params.clone(),
                    &context,
                    cancelled,
                )?
            };
            match prefix {
                Some(prefix) => prefix.mark(&mut data)?,
                None => super::prefix::mark_unprepared(source, branch, None, &mut data)?,
            }
            data
        }
        product @ ("decryption"
        | "operative-placements"
        | "agency-comparison"
        | "operatives-roster"
        | "espionage-log") => crate::products::intelligence_prepared::query(
            source,
            product,
            request.params.clone(),
            &crate::products::LoadContext {
                content: content.cloned(),
            },
            cancelled,
        )?,
        product @ ("construction-log" | "construction-log-manifest" | "construction-spending") => {
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid construction branch"))?;
            branch(source, id)?;
            crate::products::construction_log::prepared::query(
                source,
                product,
                request.params.clone(),
                content,
                cancelled,
            )?
            .unwrap_or(Value::Null)
        }
        "resource-balance" => {
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid resource branch"))?;
            branch(source, id)?;
            crate::products::resource_balance::prepared::query(
                source,
                request.params.clone(),
                content,
                cancelled,
            )?
            .unwrap_or(Value::Null)
        }
        "trade-flows" => {
            let id = request.params["branchId"]
                .as_str()
                .ok_or_else(|| crate::invalid("Invalid trade branch"))?;
            branch(source, id)?;
            crate::products::trade_flows::prepared::query(
                source,
                request.params.clone(),
                content,
                cancelled,
            )?
            .unwrap_or(Value::Null)
        }
        "production-spending" => crate::products::production_spending::prepared::query(
            source,
            request.params.clone(),
            content,
            cancelled,
        )?
        .unwrap_or(Value::Null),
        "dockyard-spending" => crate::products::dockyard_spending::prepared::query(
            source,
            request.params.clone(),
            content,
            cancelled,
        )?
        .unwrap_or(Value::Null),
        "military-factory-spending" => crate::products::military_factory_spending::prepared::query(
            source,
            request.params.clone(),
            content,
            cancelled,
        )?
        .unwrap_or(Value::Null),

        "casualty-breakdown" => {
            let params: crate::products::war_casualties::breakdown::CasualtyBreakdownParams =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            // Lifetime counters joined with the wars' app continuity wait for
            // both families' joint coverage. Before it is complete, a window
            // it has not reached answers from its newest window, and the
            // requested hours after it are pending.
            let requested = params
                .window
                .and_then(|window| source.block_windows(branch).get(window))
                .map(|window| window.last_hour);
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                requested,
                cancelled,
            )?;
            let through = (!prefix.coverage.complete).then_some(prefix.coverage.last_hour);
            let reply = crate::products::war_casualties::breakdown::prepared::query_through(
                source,
                &params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                through,
                cancelled,
            )?;
            let reply = match reply {
                Some(reply) => reply,
                None => {
                    crate::products::war_casualties::breakdown::prepared::empty(source, &params)?
                }
            };
            let mut data = serde_json::to_value(reply)?;
            if through.is_some() {
                prefix.mark(&mut data)?;
            }
            data
        }
        "casualty-pairs" => crate::products::war_casualties::pairs::prepared_query(
            source,
            &serde_json::from_value(request.params.clone())?,
            content,
            cancelled,
        )?,
        "wars" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing prepared wars branch"))?;
            let branch = branch(source, id)?;
            // App continuity numbers groups across consecutive observed hours,
            // so the lanes are the walk of the prepared prefix's change inputs;
            // the walk's hours after its last input are pending, not
            // unobserved. Requested `current` hours list native groups and
            // contributions from each hour's frame alone, independently of
            // the prefix's progress.
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                None,
                cancelled,
            )?;
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            // Read first: a prepared attempt needing the hours' frames is refused before it
            // walks. Any other failure is told after the walk's, as before.
            let current = match crate::products::wars::prepared::current(
                source,
                &request.params,
                &context,
                cancelled,
            ) {
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        && progressive::block_decodes_disabled() =>
                {
                    return Err(error);
                }
                current => current,
            };
            let mut data = crate::products::wars::prepared::query(
                source,
                &request.params,
                &store,
                &key,
                &context,
                cancelled,
            )?;
            let current = current?;
            if !current.is_empty()
                && let Some(reply) = data.as_object_mut()
            {
                reply.insert("current".into(), serde_json::to_value(current)?);
            }
            if !prefix.coverage.complete
                && let Some(reply) = data.as_object_mut()
            {
                for field in ["unobserved", "capitulationsUnobserved"] {
                    if let Some(spans) = reply.get_mut(field) {
                        let mut clipped: Vec<crate::products::country_groups::HourSpan> =
                            serde_json::from_value(spans.take())?;
                        prefix.clip(&mut clipped);
                        *spans = serde_json::to_value(clipped)?;
                    }
                }
                // Native capitulation dates may precede their first read by
                // any number of hours, revising earlier timeline glyphs.
                prefix.mark_revisable(source, branch, &mut data)?;
            }
            data
        }
        "subjects" => {
            let params: crate::products::subjects::SubjectParams =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            // The subject scan is the family's continuation, saved with each
            // commit of its coverage, so before completion it answers the
            // prepared prefix. Each later block's keyframe supplies its
            // relations and levels, marked approximate; a block it cannot
            // sample stays pending.
            let mut prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                None,
                cancelled,
            )?;
            let scan: crate::products::subjects::Scan =
                match store.read(&key, &format!("subjects-{branch}"), 0)? {
                    Some(file) => serde_json::from_reader(zstd::stream::Decoder::new(file)?)?,
                    None => crate::products::subjects::Scan::new(source.replay_ref(), branch),
                };
            check_cancel(cancelled)?;
            let mut scan = scan.finish(source.replay_ref(), branch);
            let approximate = if prefix.coverage.complete {
                None
            } else {
                Some(scan.sample_tail(source, branch, prefix.coverage.last_hour, cancelled)?)
            };
            let mut reply = scan.reply(
                source,
                &params.branch_id,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
            )?;
            if let Some(approximate) = approximate {
                prefix.clip(&mut reply.unobserved);
                // Recorded gaps after the prefix are unobserved however far
                // preparation has gone.
                let after = prefix.coverage.last_hour;
                let gaps = super::prefix::gaps_after(source, branch, after);
                for gap in gaps {
                    crate::products::country_groups::mark_unobserved(
                        &mut reply.unobserved,
                        gap.first_hour.max(after.saturating_add(1)),
                        gap.last_hour,
                    );
                }
                prefix.sampled(&approximate);
                reply.approximate = Some(approximate);
            }
            let mut data = serde_json::to_value(reply)?;
            prefix.mark(&mut data)?;
            data
        }
        product @ ("occupied-states" | "occupied-states-manifest") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Missing occupation branch"))?;
            let branch = branch(source, id)?;
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            let accepted = super::pipeline::product_coverage(
                source,
                content,
                branch,
                &request.product,
                cancelled,
            )?;
            if product == "occupied-states-manifest" {
                // The windows are the branch's blocks; the laws and warnings
                // those published so far wrote, the rest still being prepared.
                let mut data = crate::products::occupied_states::prepared::window_query(
                    source,
                    &context,
                    product,
                    request.params.clone(),
                    cancelled,
                )?;
                match accepted {
                    Some(coverage) => super::prefix::Prefix::new(source, branch, coverage, None)
                        .mark(&mut data)?,
                    None => {
                        if let Some(reply) = data.as_object_mut() {
                            reply.insert(
                                "coverage".into(),
                                serde_json::to_value(super::prefix::extent(source, branch, None)?)?,
                            );
                        }
                    }
                }
                return Ok(data);
            }
            let window = request
                .params
                .get("window")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| crate::invalid("Invalid occupied states window"))?;
            let last_hour = source
                .block_windows(branch)
                .get(window)
                .ok_or_else(|| crate::invalid("Occupied states window not in this recording"))?
                .last_hour;
            if accepted
                .as_ref()
                .is_some_and(|coverage| coverage.complete || last_hour <= coverage.last_hour)
            {
                crate::products::occupied_states::prepared::window_query(
                    source,
                    &context,
                    product,
                    request.params.clone(),
                    cancelled,
                )?
            } else {
                // A window preparation has not reached decodes alone from its
                // block and a proven look-back, exactly as preparation will
                // write it; null while a block it needs cannot be read.
                match crate::products::occupied_states::prepared::look_back_window(
                    source, &context, branch, window, cancelled,
                )? {
                    Some(reply) => serde_json::to_value(reply)?,
                    None => {
                        let interval = source.block_windows(branch)[window];
                        let mut data = serde_json::to_value(
                            crate::products::occupied_states::OccupiedStatesReply {
                                branch_id: branch.to_string(),
                                window,
                                interval,
                                texts: Vec::new(),
                                states: Vec::new(),
                                observed: Vec::new(),
                                warnings: Vec::new(),
                                laws: None,
                            },
                        )?;
                        super::prefix::read_or_empty(
                            source,
                            content,
                            product,
                            branch,
                            Some(last_hour),
                            cancelled,
                        )?
                        .mark(&mut data)?;
                        data
                    }
                }
            }
        }
        "unit-leaders" => {
            use crate::products::unit_leaders::{UnitLeaders, UnitLeadersParams};
            let mut selection = request.params.clone();
            let hour = selection
                .as_object_mut()
                .and_then(|params| params.remove("hour"))
                .map(|hour| {
                    hour.as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        .ok_or_else(|| crate::invalid("Invalid leader hour"))
                })
                .transpose()?;
            let params: UnitLeadersParams = serde_json::from_value(selection)?;
            UnitLeaders::validate(&params).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if let Some(hour) = hour {
                let start = source
                    .replay_ref()
                    .branches
                    .iter()
                    .find(|start| start.id == branch)
                    .expect("validated branch");
                let bounds = crate::metadata::branch_bounds(source.replay_ref(), start);
                if hour < bounds.0 || hour > bounds.1 {
                    return Err(crate::invalid("Leader hour outside recording"));
                }
            }
            let prefix =
                super::prefix::read(source, content, &request.product, branch, hour, cancelled)?;
            let countries = matches!(
                params.selection,
                crate::products::unit_leaders::LeaderSelection::Countries { .. }
            );
            let history = if countries {
                crate::products::country_history::prepared_branch_histories(
                    source, branch, content, cancelled,
                )?
            } else {
                None
            };
            if countries && history.is_none() {
                return Ok(Value::Null);
            }
            if params.view != crate::products::unit_leaders::LeaderView::Career
                && hour.is_some_and(|hour| {
                    prefix
                        .as_ref()
                        .is_none_or(|prefix| hour > prefix.coverage.last_hour)
                })
            {
                let mut data = crate::products::unit_leaders::prepared::at_hour(
                    source,
                    &params,
                    hour.unwrap(),
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    history.as_deref(),
                    cancelled,
                )?;
                match prefix {
                    Some(prefix) => prefix.mark(&mut data)?,
                    None => super::prefix::mark_unprepared(source, branch, hour, &mut data)?,
                }
                return Ok(data);
            }
            let Some(prefix) = prefix else {
                return Ok(Value::Null);
            };
            if params.view != crate::products::unit_leaders::LeaderView::Choices {
                let published = pinned(&store, &key)?;
                let careers = all_careers(
                    source,
                    &published,
                    &store,
                    &key,
                    branch,
                    &params.branch_id,
                    cancelled,
                )?;
                let mut reply = (*careers)
                    .as_ref()
                    .map(|careers| careers.answer(&params, history.as_deref(), hour))
                    .transpose()?;
                if let Some(reply) = &mut reply {
                    crate::products::unit_leaders::present_traits(
                        source,
                        content,
                        &mut reply.traits,
                    )?;
                }
                let mut data = serde_json::to_value(reply)?;
                prefix.mark(&mut data)?;
                return Ok(data);
            }
            // A roster's choices are kept until the leaders or the countries' identities publish
            // again.
            let published = pinned(&store, &key)?;
            let mut pages = super::pipeline::query_pages(&published, branch, "unit-leaders")?;
            pages.extend(super::pipeline::query_pages(
                &published,
                branch,
                "country-identities",
            )?);
            pages.push((
                gates::identities_section(branch),
                published.identities(branch),
            ));
            let careers = dashboard_base::prepared::query::observations::scope(
                source,
                "unit-leaders:careers",
                Some(&key),
            )
            .get_entry(
                branch,
                &params,
                &pages,
                cancelled,
                |careers: &Option<crate::products::unit_leaders::UnitLeadersReply>| {
                    dashboard_base::prepared::query::observations::serde_bytes(careers)
                },
                || {
                    crate::products::unit_leaders::prepared::careers(
                        &store,
                        &key,
                        &params,
                        history.as_deref(),
                        cancelled,
                    )
                },
            )?;
            let mut reply = careers
                .as_ref()
                .as_ref()
                .map(|careers| {
                    crate::products::unit_leaders::prepared::answer_at(careers, &params, hour)
                })
                .transpose()?;
            if let Some(reply) = &mut reply {
                crate::products::unit_leaders::present_traits(source, content, &mut reply.traits)?;
            }
            let mut data = serde_json::to_value(reply)?;
            prefix.mark(&mut data)?;
            data
        }
        "faction-state" => {
            let params: crate::products::faction_state::FactionStateParams =
                serde_json::from_value(request.params.clone())?;
            crate::products::faction_state::FactionState::validate(&params)
                .map_err(crate::invalid)?;
            let (id, _) = params.identity();
            let branch = branch(source, id)?;
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                "faction-state",
                branch,
                None,
                cancelled,
            )?;
            serde_json::to_value(crate::products::faction_state::prepared::query(
                source,
                &params,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                &store,
                &key,
                prefix,
                cancelled,
            )?)?
        }
        "special-projects" => {
            let params: crate::products::special_projects::SpecialProjectsParams =
                serde_json::from_value(request.params.clone())?;
            crate::products::special_projects::SpecialProjects::validate(&params)
                .map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            // The projects' pages are committed with their family's coverage:
            // before it is complete they answer the prepared prefix, whose
            // episodes end at its last hour, and name the hours after pending.
            let prefix =
                super::prefix::read(source, content, &request.product, branch, None, cancelled)?;
            let context = crate::products::LoadContext {
                content: content.cloned(),
            };
            // Their history ends where the admitted prefix does, before any hole its pages go on
            // after; nothing of it is admitted without one.
            let through = prefix
                .as_ref()
                .map_or(i32::MIN, |prefix| prefix.coverage.last_hour);
            let mut reply = crate::products::special_projects::prepared::query(
                request.params.clone(),
                &store,
                &key,
                through,
                cancelled,
            )?;
            if let Some(hour) = params.hour
                && prefix.as_ref().is_none_or(|prefix| {
                    !prefix.coverage.complete && hour > prefix.coverage.last_hour
                })
            {
                reply.current = Some(Box::new(
                    crate::products::special_projects::prepared::frame_reply(
                        source, &context, &params, hour, cancelled,
                    )?,
                ));
            }
            if let Some(hour) = params.hour {
                reply.details = Some(crate::products::special_projects::details_at_hour(
                    source, &context, &params, hour, cancelled,
                )?);
            }
            // The art preparation kept as operands, drawn as this request shows it.
            crate::products::special_projects::present_reply(
                source,
                &crate::products::special_projects::ArtSource::new(source, content),
                &mut reply,
            )?;
            let mut data = serde_json::to_value(reply)?;
            if let Some(prefix) = prefix {
                prefix.mark(&mut data)?;
            } else {
                super::prefix::mark_unprepared(source, branch, None, &mut data)?;
            }
            data
        }
        "country-groups" => {
            let params: crate::products::country_groups::CountryGroupParams =
                serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            // The groups' own coverage: they are prepared before other families.
            // The scan is the family's continuation, saved with each commit of
            // that coverage, so before completion it answers the prepared
            // prefix, carried on by each later block's keyframe and marked
            // approximate there; a block it does not sample stays pending.
            let mut prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                None,
                cancelled,
            )?;
            let mut scan = match store.read(&key, &format!("country-groups-{branch}"), 0)? {
                Some(file) => {
                    crate::products::country_groups::Scan::read(io::BufReader::new(file))?
                }
                None => crate::products::country_groups::Scan::new(source.replay_ref(), branch),
            };
            check_cancel(cancelled)?;
            let approximate = if prefix.coverage.complete {
                None
            } else {
                Some(scan.sample_tail(source, branch, prefix.coverage.last_hour, cancelled)?)
            };
            let mut reply = scan.reply(
                source,
                &params.branch_id,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
            )?;
            let Some(approximate) = approximate else {
                return Ok(serde_json::to_value(reply)?);
            };
            // Recorded gaps and discontinuities between blocks after the prefix
            // are unobserved; the other hours there are sampled or pending.
            let after = prefix.coverage.last_hour;
            let gaps = super::prefix::gaps_after(source, branch, after);
            for spans in [
                &mut reply.unobserved.humans,
                &mut reply.unobserved.factions,
                &mut reply.unobserved.wars,
            ] {
                prefix.clip(spans);
                for gap in &gaps {
                    crate::products::country_groups::mark_unobserved(
                        spans,
                        gap.first_hour.max(after.saturating_add(1)),
                        gap.last_hour,
                    );
                }
            }
            prefix.sampled(&approximate);
            // Unlike identity changes, these events retain native dates. A
            // later observation can add one anywhere in the accepted prefix.
            reply.events_approximate = Some(prefix.revisable(source, branch));
            let mut data = serde_json::to_value(reply)?;
            prefix.mark(&mut data)?;
            if let Some(fields) = data.as_object_mut() {
                fields.insert("approximate".into(), serde_json::to_value(approximate)?);
            }
            data
        }
        "politics-ledger" => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid politics ledger branch"))?;
            let branch = branch(source, id)?;
            let hour = request
                .params
                .get("hour")
                .filter(|value| !value.is_null())
                .map(|value| {
                    value
                        .as_i64()
                        .and_then(|value| i32::try_from(value).ok())
                        .ok_or_else(|| crate::invalid("Invalid politics ledger hour"))
                })
                .transpose()?;
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                hour,
                cancelled,
            )?;
            let mut params = request.params.clone();
            if let Some(through) = prefix.through(hour) {
                params["hour"] = Value::from(through);
            }
            let mut data = ledger_reply(source, content, &params, branch, cancelled)?;
            // The icons the reply names as recipes, drawn as this request shows them.
            crate::products::politics_ledger::present_icons(source, content, &mut data)?;
            prefix.mark(&mut data)?;
            data
        }

        "focus-static-text" => {
            let params: FocusStaticText = serde_json::from_value(request.params.clone())?;
            if params
                .tokens
                .iter()
                .any(|token| token.is_empty() || token.contains('\0'))
            {
                return Err(crate::invalid("Invalid focus static text token"));
            }
            // Its own warning list, which serving fills with the content's diagnostics.
            serde_json::json!({
                "texts": crate::prepared_focus::art::static_text(
                    source,
                    content,
                    params.database,
                    &params.tokens,
                    cancelled,
                )?,
                "warnings": [],
            })
        }
        "focus-hour" => {
            let params: FocusPoint = serde_json::from_value(request.params.clone())?;
            crate::products::validate_tag(&params.tag).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(
                source,
                content,
                &request.product,
                branch,
                Some(params.hour),
                false,
                cancelled,
            )? {
                let prefix = super::prefix::read_or_empty(
                    source,
                    content,
                    &request.product,
                    branch,
                    Some(params.hour),
                    cancelled,
                )?;
                let mut data = serde_json::to_value(crate::prepared_focus::sampled_hour(
                    source,
                    branch,
                    &params.tag,
                    params.hour,
                    cancelled,
                )?)?;
                prefix.mark(&mut data)?;
                return Ok(data);
            }
            let mut data = serde_json::to_value(crate::prepared_focus::query_hour(
                source,
                &store,
                &key,
                branch,
                &params.tag,
                params.hour,
                cancelled,
            )?)?;
            super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                Some(params.hour),
                cancelled,
            )?
            .mark(&mut data)?;
            data
        }
        "focus-completions" => {
            let params: FocusCompletions = serde_json::from_value(request.params.clone())?;
            crate::products::validate_tag(&params.tag).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            // The whole branch's completions, as the national focus history holds them: only
            // once the family has prepared all of it.
            if !coverage(
                source,
                content,
                &request.product,
                branch,
                None,
                true,
                cancelled,
            )? {
                return Ok(Value::Null);
            }
            serde_json::json!({
                "completions": crate::prepared_focus::completions(
                    &store,
                    &key,
                    branch,
                    &params.tag,
                    cancelled,
                )?,
                // The history's own, which serving joins the content's diagnostics.
                "warnings": crate::prepared_focus::art::standing_warnings(
                    source,
                    content,
                    cancelled,
                )?,
            })
        }
        "focus-unavailable" => {
            let params: FocusUnavailable = serde_json::from_value(request.params.clone())?;
            let branch = branch(source, &params.branch_id)?;
            if !coverage(
                source,
                content,
                &request.product,
                branch,
                Some(params.hour),
                false,
                cancelled,
            )? {
                return Ok(Value::Null);
            }
            serde_json::to_value(crate::prepared_focus::query_unavailable(
                &store,
                &key,
                branch,
                params.hour,
                cancelled,
            )?)?
        }
        "focus-order-page" => {
            let params: FocusPage = serde_json::from_value(request.params.clone())?;
            crate::products::validate_tag(&params.tag).map_err(crate::invalid)?;
            let branch = branch(source, &params.branch_id)?;
            let page = params
                .page
                .parse::<u64>()
                .map_err(|_| crate::invalid("Invalid focus page ordinal"))?;
            if params.pages.sampled
                || !coverage(
                    source,
                    content,
                    &request.product,
                    branch,
                    Some(params.hour),
                    false,
                    cancelled,
                )?
            {
                return crate::prepared_focus::sampled_page(
                    source,
                    branch,
                    &params.tag,
                    params.hour,
                    &params.pages,
                    page,
                    cancelled,
                );
            }
            crate::prepared_focus::order_page(
                &store,
                &key,
                branch,
                &params.tag,
                params.hour,
                &params.pages,
                page,
                cancelled,
            )?
        }
        "production-select" => {
            let params: ProductionSelection = serde_json::from_value(request.params.clone())?;
            crate::products::equipment_produced::EquipmentProduced::validate(&params.selection)
                .map_err(crate::invalid)?;
            let branch = branch(source, &params.selection.branch_id)?;
            // Later tokens can reclassify earlier designs, and a Rocket join
            // can be repaired throughout its still-open supplying interval.
            // Neither has a fixed backdating limit: the prefix is provisional.
            let prefix = super::prefix::read_or_empty(
                source,
                content,
                &request.product,
                branch,
                Some(params.hour),
                cancelled,
            )?;
            let hour = prefix.through(Some(params.hour)).unwrap_or(params.hour);
            let offset = params
                .offset
                .parse::<usize>()
                .map_err(|_| crate::invalid("Invalid production page offset"))?;
            let value = crate::prepared_production::select_at(
                source,
                content,
                &store,
                &key,
                hour,
                &params.selection,
                offset,
                params.limit,
                params.ranked,
                cancelled,
            )?;
            let value = match value {
                Some(value) => value,
                None => crate::prepared_production::empty_selection(
                    hour,
                    &params.selection,
                    offset,
                    params.limit,
                    params.ranked,
                    cancelled,
                )?,
            };
            let mut data = serde_json::to_value(value)?;
            prefix.mark_revisable(source, branch, &mut data)?;
            data
        }
        product @ ("series-state" | "series-summary" | "series-next-change")
            if request
                .params
                .get("family")
                .and_then(|family| family.get("kind"))
                .and_then(Value::as_str)
                == Some("autonomy") =>
        {
            crate::products::autonomy::prepared::progressive_query(
                source,
                product,
                &request.params,
                &store,
                &key,
                &crate::products::LoadContext {
                    content: content.cloned(),
                },
                cancelled,
            )?
        }
        product @ ("series-state" | "series-summary" | "series-next-change" | "events-page"
        | "events-clusters") => {
            let id = request
                .params
                .get("branchId")
                .and_then(Value::as_str)
                .ok_or_else(|| crate::invalid("Invalid series query branch"))?;
            let branch = branch(source, id)?;
            let family = request
                .params
                .get("family")
                .and_then(|family| family.get("kind"))
                .and_then(Value::as_str);
            // A series request waits for its family kind's coverage, as its
            // refusal answers by that kind: a family prepared on its own gates
            // the kind (peace deals, territory transfers, research events).
            // Factory faction totals wait only for the families they read.
            let gate = match family {
                Some("faction-totals") => crate::products::faction_totals::gate(&request.params),
                _ => family.unwrap_or(product),
            };
            let (hour, complete) = series_coverage(product, &request.params)?;
            let accepted = admitted.ranges.coverage.clone();
            if family == Some("game-score") && product == "series-state" {
                return crate::products::game_score::prepared::state(
                    source,
                    request.params.clone(),
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    cancelled,
                );
            }
            match family {
                Some("country-fuel") => {
                    return crate::products::country_fuel::query::query(
                        source,
                        content,
                        product,
                        request.params.clone(),
                        cancelled,
                    );
                }
                Some("country-factories") => {
                    return crate::products::country_factories::prepared::query(
                        source,
                        product,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    );
                }
                Some("country-nukes") => {
                    return crate::products::country_nukes::prepared::query(
                        source,
                        product,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    );
                }
                _ => {}
            }
            if family == Some("research") && product == "series-state" {
                return crate::products::research::prepared::state_query(
                    source,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    &request.params,
                    &store,
                    &key,
                    cancelled,
                );
            }
            if family == Some("research") && product == "events-page" {
                let prefix = super::prefix::read_or_empty(
                    source, content, "research", branch, hour, cancelled,
                )?;
                let mut data = crate::products::research::prepared::query(
                    source,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    cancelled,
                )?;
                prefix.mark(&mut data)?;
                return Ok(data);
            }
            // Banked experience is a daily read held until the next: its
            // sampler carries its own scan from hour to hour, and navigation
            // samples a long tail at each block's first read. Gains and changes need
            // every hour of a day, so only preparation answers them.
            let experience = (family == Some("country-experience")
                && banked_experience(&request.params))
            .then(|| crate::products::country_experience::Sampler::new(source.replay_ref().format));
            // A summary draws the accepted prefix immediately. Sampling every
            // remaining block would compete with preparation before it can draw.
            // Reached values still read their own supplying block below.
            let experience_prefix = experience.is_some() && product == "series-summary";
            let experience_sample = |frame: &replay_frame::frame::DecodedFrame,
                                     requested: Option<&[(String, String)]>|
             -> io::Result<progressive::Samples> {
                experience
                    .as_ref()
                    .ok_or_else(|| crate::invalid("Experience sampler missing"))?
                    .sample(frame, requested)
            };
            // Families whose requests name one measure per tag register here; others
            // expand their own request shape and call `progressive::tail` themselves.
            let sampler: Option<(&[replay_data::rows::TableId], progressive::Sample<'_>)> =
                match family {
                    Some("country-measure") => Some((
                        crate::prepared_measure::TABLES,
                        &crate::prepared_measure::sample,
                    )),
                    Some("country-experience") if experience.is_some() => Some((
                        crate::products::country_experience::SAMPLE_TABLES,
                        &experience_sample,
                    )),
                    _ => None,
                };
            let partial_events = matches!(product, "events-page" | "events-clusters");
            // Casualty totals sum lifetime counters through their hour, so no
            // frame samples them: a state read past the prepared prefix answers
            // at the prefix's last hour, as at any hour inside it, and names the
            // requested hours after it pending.
            let state_prefix = if family == Some("war-casualties") && product == "series-state" {
                Some(super::prefix::Prefix::new(
                    source,
                    branch,
                    super::prefix::extent(source, branch, accepted.clone())?,
                    hour,
                ))
            } else {
                None
            };
            // Stability parts expand countries to parts and sample their own tail.
            let parts_tail = family == Some("stability-parts")
                && matches!(
                    product,
                    "series-state" | "series-summary" | "series-next-change"
                );
            if family != Some("territory")
                && !matches!(family, Some("world-tension" | "occupation-trend"))
                && sampler.is_none()
                && !parts_tail
                && !partial_events
                && state_prefix.is_none()
                && !matches!(
                    family,
                    Some("army-size" | "supply-health" | "air-force-size" | "navy-size")
                )
                // A faction-total state past coverage answers from its frames
                // where it can, and waits otherwise (`faction_totals::prepared`).
                && !(family == Some("faction-totals") && product == "series-state")
                && !coverage(source, content, gate, branch, hour, complete, cancelled)?
            {
                return Ok(Value::Null);
            }
            let partial = !accepted.as_ref().is_some_and(|coverage| coverage.complete);
            let mut sampled = (
                std::collections::BTreeMap::new(),
                Vec::new(),
                std::collections::BTreeMap::new(),
            );

            if let Some((tables, sample)) = sampler
                && partial
                && !experience_prefix
                && matches!(
                    product,
                    "series-state" | "series-summary" | "series-next-change"
                )
            {
                let measure = request
                    .params
                    .get("family")
                    .and_then(|family| family.get("measure"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| crate::invalid("Missing series measure"))?;
                let labels: Vec<String> = if product == "series-next-change" {
                    request
                        .params
                        .get("measures")
                        .and_then(Value::as_array)
                        .filter(|values| !values.is_empty())
                        .map(|values| {
                            values
                                .iter()
                                .map(|value| {
                                    value
                                        .as_str()
                                        .map(str::to_owned)
                                        .ok_or_else(|| crate::invalid("Invalid series measure"))
                                })
                                .collect()
                        })
                        .transpose()?
                        .unwrap_or_else(|| vec![measure.to_owned()])
                } else {
                    vec![measure.to_owned()]
                };
                let mut requested = Vec::new();
                let mut hours = Vec::new();
                if product == "series-state" {
                    for read in request
                        .params
                        .get("reads")
                        .and_then(Value::as_array)
                        .ok_or_else(|| crate::invalid("Missing series reads"))?
                    {
                        let tag = read
                            .get("tag")
                            .and_then(Value::as_str)
                            .ok_or_else(|| crate::invalid("Invalid series tag"))?;
                        let hour = read
                            .get("hour")
                            .and_then(Value::as_i64)
                            .and_then(|hour| i32::try_from(hour).ok())
                            .ok_or_else(|| crate::invalid("Invalid series hour"))?;
                        hours.push(hour);
                        for label in &labels {
                            requested.push((tag.to_owned(), label.clone()));
                        }
                    }
                } else {
                    for tag in request
                        .params
                        .get("tags")
                        .and_then(Value::as_array)
                        .ok_or_else(|| crate::invalid("Missing series tags"))?
                    {
                        let tag = tag
                            .as_str()
                            .ok_or_else(|| crate::invalid("Invalid series tag"))?;
                        for label in &labels {
                            requested.push((tag.to_owned(), label.clone()));
                        }
                    }
                }
                requested.sort();
                requested.dedup();
                for (tag, _) in &requested {
                    crate::products::validate_tag(tag).map_err(crate::invalid)?;
                }
                let mut sampled_requested = requested.clone();
                if family == Some("country-experience") {
                    sampled_requested.extend(requested.iter().map(|(tag, _)| {
                        (
                            tag.clone(),
                            crate::prepared_measure::query::READ_HOUR.to_owned(),
                        )
                    }));
                    sampled_requested.sort();
                    sampled_requested.dedup();
                }
                let channels: &[&str] = &["politicalPower", "commandPower"];
                let mut cached_requested: Vec<_> = requested
                    .iter()
                    .flat_map(|(tag, _)| {
                        channels
                            .iter()
                            .map(move |field| (tag.clone(), (*field).to_owned()))
                    })
                    .collect();
                cached_requested.sort();
                cached_requested.dedup();
                sampled = if product == "series-state" {
                    if let Some(sampler) = &experience {
                        progressive::state_tail_recorded(
                            source,
                            "country-experience:exact",
                            branch,
                            accepted.as_ref(),
                            tables,
                            replay_data::cadence::STAGGER_PERIOD,
                            &sampled_requested,
                            &crate::products::country_experience::Sampler::cache_keys(&requested),
                            &hours,
                            &|frame, _, requested| sampler.sample(frame, requested),
                            cancelled,
                        )?
                    } else {
                        progressive::state_tail_cached(
                            source,
                            "country-measure:series",
                            branch,
                            accepted.as_ref(),
                            tables,
                            &sampled_requested,
                            &cached_requested,
                            &hours,
                            sample,
                            cancelled,
                        )?
                    }
                } else {
                    let (from, to) = if product == "series-summary" {
                        super::series::range(
                            request.params.get("from").and_then(Value::as_f64),
                            request.params.get("to").and_then(Value::as_f64),
                        )?
                    } else {
                        super::series::range(
                            request.params.get("firstHour").and_then(Value::as_f64),
                            request.params.get("lastHour").and_then(Value::as_f64),
                        )?
                    };
                    if product == "series-summary" {
                        let width = request
                            .params
                            .get("width")
                            .and_then(Value::as_u64)
                            .and_then(|width| u32::try_from(width).ok())
                            .filter(|width| *width > 0)
                            .ok_or_else(|| crate::invalid("Invalid series width"))?;
                        super::series::summary_budget(from, to, width, requested.len())?;
                    }
                    let window = (from.floor() as i32, to.floor() as i32);
                    if let Some(sampler) = &experience {
                        progressive::tail_first_read(
                            source,
                            "country-experience:exact",
                            branch,
                            accepted.as_ref(),
                            tables,
                            replay_data::cadence::STAGGER_PERIOD,
                            &sampled_requested,
                            &crate::products::country_experience::Sampler::cache_keys(&requested),
                            window,
                            &|frame, _, requested| sampler.sample(frame, requested),
                            &|block| {
                                crate::products::country_experience::Sampler::first_block(
                                    source,
                                    block,
                                    &sampled_requested,
                                    cancelled,
                                )
                            },
                            cancelled,
                        )?
                    } else {
                        progressive::tail_cached(
                            source,
                            "country-measure:series",
                            branch,
                            accepted.as_ref(),
                            tables,
                            &sampled_requested,
                            &cached_requested,
                            window,
                            sample,
                            cancelled,
                        )?
                    }
                };
            }
            let tail_context = (sampler.is_some() && partial).then(|| {
                let start = source
                    .replay_ref()
                    .branches
                    .iter()
                    .find(|start| start.id == branch)
                    .unwrap();
                (
                    crate::metadata::branch_bounds(source.replay_ref(), start),
                    &sampled.0,
                    sampled.1.as_slice(),
                    &sampled.2,
                    accepted.is_some(),
                )
            });
            let mut data = match family {
                Some("country-stability") => {
                    stability_series(source, content, product, &request.params, branch, cancelled)?
                }
                Some("country-measure" | "country-experience") => {
                    crate::prepared_measure::query::query(
                        product,
                        request.params.clone(),
                        &store,
                        &key,
                        tail_context,
                        cancelled,
                    )?
                }
                Some("naval-losses") => crate::products::naval_losses::prepared::query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    &store,
                    &key,
                    cancelled,
                )?,
                Some("equipment-losses") => {
                    crate::products::equipment_losses::prepared::series_query(
                        source,
                        product,
                        &request.params,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        cancelled,
                    )?
                }
                Some("war-casualties") => crate::products::war_casualties::prepared::query(
                    product,
                    match &state_prefix {
                        Some(prefix) => casualty_reads_through(&request.params, prefix),
                        None => request.params.clone(),
                    },
                    &store,
                    &key,
                    cancelled,
                )?,
                Some("world-tension") => {
                    crate::products::world_tension::prepared::progressive_query(
                        source,
                        content,
                        product,
                        &request.params,
                        &store,
                        &key,
                        cancelled,
                    )?
                }
                Some("occupation-trend") => {
                    crate::products::occupied_states::prepared::trend_query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        product,
                        request.params.clone(),
                        cancelled,
                    )?
                }
                Some("territory") => crate::products::territory::prepared::query(
                    source,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    product,
                    request.params.clone(),
                    cancelled,
                )?,
                Some("faction-totals") => crate::products::faction_totals::prepared::query(
                    source,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    product,
                    request.params.clone(),
                    cancelled,
                )?,
                Some("peace-deals") => {
                    // This adapter builds complete outcomes from earlier reads,
                    // so it samples no unprepared span. Its event lists answer
                    // the prepared prefix and name the later hours pending.
                    if !partial_events
                        && !coverage(source, content, gate, branch, hour, complete, cancelled)?
                    {
                        return Ok(Value::Null);
                    }
                    crate::products::peace_deals::prepared::query_through(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        request.params.clone(),
                        &store,
                        &key,
                        (!accepted.as_ref().is_some_and(|coverage| coverage.complete))
                            .then(|| {
                                super::prefix::extent(source, branch, accepted.clone())
                                    .map(|coverage| coverage.last_hour)
                            })
                            .transpose()?,
                        cancelled,
                    )?
                }
                Some("research") => {
                    // Assignment history and carried slot state need the scan:
                    // its event pages answer the prepared prefix and name the
                    // later hours pending; its values wait for their hours.
                    if !partial_events
                        && !coverage(source, content, gate, branch, hour, complete, cancelled)?
                    {
                        return Ok(Value::Null);
                    }
                    crate::products::research::prepared::query(
                        source,
                        &crate::products::LoadContext {
                            content: content.cloned(),
                        },
                        product,
                        request.params.clone(),
                        &store,
                        &key,
                        cancelled,
                    )?
                }
                Some("stability-parts") => crate::products::stability_parts::prepared_query(
                    source,
                    content,
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    accepted.as_ref(),
                    cancelled,
                )?,
                Some("army-size") => {
                    army_series(source, content, product, &request.params, branch, cancelled)?
                }
                Some("navy-size") => crate::products::navy::prepared::query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    cancelled,
                )?,
                Some("air-force-size") => crate::products::air_force::prepared::query(
                    source,
                    product,
                    &request.params,
                    &crate::products::LoadContext {
                        content: content.cloned(),
                    },
                    cancelled,
                )?,
                Some("supply-health") if product == "series-next-change" => {
                    supply_next_change(source, content, &request.params, cancelled)?
                }
                Some("game-score") => crate::products::game_score::prepared::query(
                    product,
                    request.params.clone(),
                    &store,
                    &key,
                    cancelled,
                )?,
                _ => return Err(crate::invalid("Unknown prepared series family")),
            };
            if let Some(states) = data.get_mut("states").and_then(Value::as_array_mut) {
                for state in states
                    .iter_mut()
                    .filter(|state| state["approximate"] == true)
                {
                    if let Some(hour) = state["hour"]
                        .as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        && let Some(block) = source.replay_ref().index.iter().find(|block| {
                            block.branch == branch
                                && block.first_hour <= hour
                                && block.last_hour >= hour
                        })
                    {
                        state["sampleHour"] = Value::from(block.first_hour);
                        if !state["run"].is_null() {
                            for field in ["firstHour", "lastHour", "segmentFirstHour"] {
                                state["run"][field] = Value::from(block.first_hour);
                            }
                        }
                    }
                }
            }
            // Accepted-only replies keep the requested hours after their
            // prefix pending.
            let pending = match state_prefix {
                Some(prefix) => Some(prefix.pending),
                None if partial_events || experience_prefix && partial => Some(
                    super::prefix::Prefix::new(
                        source,
                        branch,
                        super::prefix::extent(source, branch, accepted.clone())?,
                        hour,
                    )
                    .pending,
                ),
                None => None,
            };
            if let Some(reply) = data.as_object_mut() {
                if !matches!(family, Some("world-tension" | "occupation-trend"))
                    || !reply.contains_key("coverage")
                {
                    reply.insert(
                        "coverage".into(),
                        serde_json::to_value(super::prefix::extent(
                            source,
                            branch,
                            accepted.clone(),
                        )?)?,
                    );
                }
                if let Some(pending) = pending {
                    reply.insert("pending".into(), serde_json::to_value(pending)?);
                }
            }
            if family == Some("peace-deals") && partial_events {
                super::prefix::Prefix::new(
                    source,
                    branch,
                    super::prefix::extent(source, branch, accepted.clone())?,
                    hour,
                )
                .mark_revisable(source, branch, &mut data)?;
            }
            data
        }
        _ => {
            return Err(crate::invalid(format!(
                "Unknown prepared query: {}",
                request.product
            )));
        }
    })
}

/// Where each prepared reply keeps its content warnings, as paths from the
/// reply's root (`*` is each element of an array), for `product` routed as
/// `name` (its series family kind, or else the product key). Battle and
/// division logs keep them in `contentWarnings`, beside a `warnings` list of
/// recorded battle warnings; the sinking manifest in `contentWarnings` and its
/// classes reply, which the log shows; wrapped replies in the reply they wrap;
/// fuel series in each country's reply; naval loss summaries in the sampled
/// reply they carry. Every other reply keeps them in its
/// root `warnings`, or, when it answers a tag selection with one reply per
/// tag, in each of those replies.
fn content_warning_paths(product: &str, name: &str) -> &'static [&'static [&'static str]] {
    match (product, name) {
        (
            "land-battles"
            | "land-battles-manifest"
            | "land-battles-ranking"
            | "land-battle-course"
            | "naval-battles"
            | "naval-battles-manifest"
            | "naval-battles-ranking"
            | "division-log"
            | "division-log-manifest",
            _,
        ) => &[&["contentWarnings"]],
        ("air-aces", _) => &[
            &["warnings"],
            &["manifest", "warnings"],
            &["roster", "warnings"],
        ],
        ("ships-sunk-manifest", _) => &[&["contentWarnings"], &["classes", "warnings"]],
        ("ships-sunk" | "party-history" | "event-feed", _) | ("series-state", "game-score") => {
            &[&["reply", "warnings"]]
        }
        ("scoreboard", _) => &[&["data", "warnings"]],
        ("series-state" | "series-summary", "country-fuel") => &[&["replies", "*", "warnings"]],
        ("series-summary", "naval-losses") => &[&["warnings"], &["sampled", "warnings"]],
        _ => &[&["warnings"], &["*", "warnings"]],
    }
}

/// Visits every value at `path` below `value`.
#[cfg(test)]
fn each_at(value: &mut Value, path: &[&str], visit: &mut dyn FnMut(&mut Value)) {
    match path.split_first() {
        None => visit(value),
        Some((&"*", rest)) => {
            for item in value.as_array_mut().into_iter().flatten() {
                each_at(item, rest, visit);
            }
        }
        Some((key, rest)) => {
            if let Some(next) = value.get_mut(*key) {
                each_at(next, rest, visit);
            }
        }
    }
}

/// The content stack's live diagnostics join a reply's content warnings, at
/// each place the reply keeps them ([`content_warning_paths`]), where it is
/// served ([`Source::append_stack_warnings`]): published payloads leave them
/// out, since other consumers' reads add to them at their own pace.
#[cfg(test)]
pub(in crate::prepared) fn append_stack_warnings(
    source: &Source,
    product: &str,
    name: &str,
    data: &mut Value,
) {
    for path in content_warning_paths(product, name) {
        each_at(data, path, &mut |lines| {
            let Value::Array(lines) = lines else {
                return;
            };
            let mut warnings: Vec<String> = lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_owned))
                .collect();
            if warnings.len() != lines.len() {
                return;
            }
            source.append_stack_warnings(&mut warnings);
            *lines = warnings.into_iter().map(Value::String).collect();
        });
    }
}

/// The lines every served reply adds to its first content warning list, a fresh answer and a
/// kept one alike: what the presentation the request was admitted under could not show, then
/// what the answer drew in the recording's style instead (`answered`).
pub(crate) fn served_lines(
    source: &Source,
    content: Option<&ContentOptions>,
    answered: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    source.append_presentation_warnings(&mut lines, content);
    lines.extend(installed_content_core::content::overlay_refusal_line(answered));
    lines
}

/// `lines` added to a reply's first content warning list, each once.
#[cfg(test)]
fn append_lines(product: &str, name: &str, lines: Vec<String>, data: &mut Value) {
    if lines.is_empty() {
        return;
    }
    let mut lines = Some(lines);
    for path in content_warning_paths(product, name) {
        each_at(data, path, &mut |warnings| {
            if let (Value::Array(warnings), Some(lines)) = (warnings, lines.take()) {
                for line in lines {
                    if !warnings.iter().any(|kept| kept.as_str() == Some(&line)) {
                        warnings.push(Value::String(line));
                    }
                }
            }
        });
    }
}

/// One prepared query's reply. The key its answer is kept under and the artwork it draws both
/// read the presentation `content` carries (`ContentOptions::presentation`), so another
/// presentation's art is never kept under it.
pub fn query(
    source: &Source,
    request: &ProductRequest,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<String> {
    super::pipeline::install_gates();
    check_cancel(cancelled)?;
    if source.key().revision != request.revision {
        return Err(crate::invalid("Prepared query recording revision changed"));
    }
    let (store, key) = super::files(source, content)?;
    let name = route(request);
    // The whole query reads one publication: its gates, decoded when it was
    // published, and its pages through that publication alone. A query whose
    // files a later publication removed starts again on that one.
    let mut tried: Option<Arc<gates::Published>> = None;
    loop {
        let current = gates::current(&store, &key);
        let Some(published) = current.filter(|current| {
            tried
                .as_ref()
                .is_none_or(|tried| !Arc::ptr_eq(tried, current))
        }) else {
            return pending(request, name, cancelled);
        };
        // Every read of the store under the query's key reads this publication alone; a store
        // publishing nothing to the key is pinned as an empty one.
        let scope = published.scope();
        let held = gates::pin(&published);
        #[cfg(test)]
        ON_PINNED.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook();
            }
        });
        let answer = answer(
            source,
            request,
            content,
            cancelled,
            (Arc::clone(&store), key.clone()),
            Arc::clone(&published),
            name,
        );
        // Whatever its readers made of a file they missed, it is no answer of one publication.
        if !scope.superseded() {
            return answer;
        }
        drop((held, scope));
        tried = Some(published);
    }
}

#[cfg(test)]
thread_local! {
    /// Run each time a query has pinned its publication, before it answers from it.
    pub(crate) static ON_PINNED: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
}

/// What a query routes by: its series family kind, or else its product key.
fn route(request: &ProductRequest) -> &str {
    match request.product.as_str() {
        "series-state" | "series-summary" | "series-next-change" | "events-page"
        | "events-clusters" => request
            .params
            .get("family")
            .and_then(|family| family.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        product => product,
    }
}

/// The reply of a query whose store's publication could not be decoded: pending, waiting for what
/// its product waits for, with no reader run.
fn pending(request: &ProductRequest, name: &str, cancelled: &AtomicBool) -> io::Result<String> {
    bounded_json(
        &Reply {
            source_id: &request.source_id,
            revision: &request.revision,
            product: &request.product,
            gate: Some(super::pipeline::gate(name)),
            holes: None,
            data: (),
        },
        cancelled,
    )
    .map_err(crate::invalid)
}

/// The gates the query this thread runs pinned.
fn pinned(store: &store::Store, key: &store::SourceKey) -> io::Result<Arc<gates::Published>> {
    gates::current(store, key).ok_or_else(|| crate::invalid("Prepared gates are read in a query"))
}

/// [`query`]'s reply from `published`, the publication pinned on this thread.
fn answer(
    source: &Source,
    request: &ProductRequest,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    (_store, key): (Arc<store::Store>, store::SourceKey),
    published: Arc<gates::Published>,
    name: &str,
) -> io::Result<String> {
    let _cpu = matches!(request.product.as_str(), "series-summary" | "series-state").then(|| {
        replay_frame::cpu::scope_named("prepared query", &format!("{}:{name}", request.product))
    });
    let branch = request
        .params
        .get("branchId")
        .or_else(|| request.params.get("selection")?.get("branchId"))
        .and_then(Value::as_str)
        .and_then(|id| branch(source, id).ok());
    // Null is a query result whose coverage has not been accepted yet. Clients
    // inspect real preparation status and retry; it is not an empty history.
    let read = |previous: Option<std::sync::Arc<CachedQuery>>| -> io::Result<CachedQuery> {
        #[cfg(test)]
        COMPUTED.with(|count| count.set(count.get() + 1));
        let scope = cache_product(&request.product)
            .map(|_| super::series::cache_scope(previous.as_ref().map(|reply| &reply.summaries)));
        // The answer names its own refusals, with those the shared loads it reused noted, never
        // the session's: the same whichever answers ran before it.
        let mut noted = dashboard_text::catalogue::OwnRefusals::default();
        let ((data, holes), short) = dashboard_text::catalogue::own_refusals(&mut noted, || {
            #[cfg(test)]
            if let (Some(note), Some(content)) = (COMPUTING.with(std::cell::Cell::get), content) {
                note(source, content);
            }
            super::coverage::waiting(|| {
                super::coverage::with_holes(|| {
                    admit(
                        source, request, content, branch, name, &published, cancelled,
                    )
                })
            })
        });
        let fallbacks = noted.fallbacks().clone();
        let data = data?;
        Ok(CachedQuery {
            waits: waits(&data),
            holes,
            data: served::Answer::new(
                &data,
                content_warning_paths(&request.product, name),
                request.product == crate::products::division_equipment::history::QUERY,
                cancelled,
            )?,
            short,
            summaries: scope.map(|scope| scope.finish()).unwrap_or_default(),
            // A fallback joins the stack's read warnings, which every reply names, when first
            // noted: an answer that noted one, served from disk, would leave it out of them.
            session: !fallbacks.is_empty() || drawn(source, content),
            fallbacks,
            readers: noted.readers().clone(),
        })
    };
    // A complete answer kept on disk between sessions, else computed and kept when it can be.
    let answered = |previous: Option<std::sync::Arc<CachedQuery>>,
                    publication: String|
     -> io::Result<CachedQuery> {
        // What an answer reports of the lazy readers it used, kept and served only once it is
        // what computing it now would report.
        let settled = |readers: &std::collections::BTreeSet<dashboard_text::catalogue::Reader>| {
            readers.is_empty()
                || content
                    .and_then(|content| dashboard_base::catalogue::catalogue(source, content).ok())
                    .is_some_and(|catalogue| catalogue.reports_settled(readers))
        };
        let stored = stored_answer(source, request, content, &published, &key, &publication)?;
        // Served whatever the session read outside the recorded coverage: a kept answer read
        // none, and those reads join its reply as it is served, as they join a fresh one's.
        if let Some((kept, stored)) = stored
            .as_ref()
            .and_then(crate::reply_cache::Entry::get)
            .and_then(served::Answer::kept::<StoredQuery>)
            && settled(&stored.readers)
        {
            return Ok(CachedQuery {
                data: kept,
                // Only a complete answer is kept.
                waits: false,
                short: Default::default(),
                holes: Vec::new(),
                summaries: Default::default(),
                fallbacks: stored.fallbacks,
                session: false,
                readers: stored.readers,
            });
        }
        let answer = read(previous)?;
        // Only a complete answer under the binding the source still shows, whose computing
        // neither read nor added to the session's, is kept: any session's computing gives it and
        // leaves the session as serving it does. While the read set holds content outside the
        // recorded coverage, computing may have read those unverified bytes, so nothing is kept.
        if let Some(entry) = stored
            && answer.short.is_empty()
            && answer.holes.is_empty()
            && !answer.waits
            && !answer.session
            && settled(&answer.readers)
            && !source.reply_reads_uncovered(content)
            && content.is_none_or(|content| source.still_admitted(content))
        {
            let kept = StoredQuery {
                fallbacks: answer.fallbacks.clone(),
                readers: answer.readers.clone(),
            };
            entry.put_later(answer.data.bytes(), answer.data.keep(kept));
        }
        Ok(answer)
    };
    dashboard_base::prepared::query::observations::bind(source, &key);
    let answer = if let (Some(product), Some(branch)) = (cache_product(&request.product), branch) {
        let supplier = if name == "faction-totals" {
            crate::products::faction_totals::gate(&request.params)
        } else {
            name
        };
        let publication = {
            let _phase = replay_frame::cpu::scope_phase("query:publication");
            let mut pages = super::pipeline::query_pages(&published, branch, name)?;
            if supplier != name {
                pages.extend(super::pipeline::query_pages(&published, branch, supplier)?);
            }
            // Identity pages and their coverage can advance independently of the
            // numeric supplier. The supplier's unchanged gate alone is not a proof
            // that naming and exactness are unchanged.
            pages.extend(super::pipeline::query_pages(
                &published,
                branch,
                "country-identities",
            )?);
            pages.push((
                gates::identities_section(branch),
                published.identities(branch),
            ));
            (pages, reply_presentation(content))
        };
        dashboard_base::prepared::query::observations::scope(source, product, Some(&key))
            .continue_entry(
                branch,
                &request.params,
                &publication,
                &name,
                cancelled,
                |reply: &CachedQuery| {
                    let mut bytes = dashboard_base::prepared::prepass::HeldBytes::default();
                    bytes.heap(&reply.short);
                    bytes.heap(&reply.fallbacks);
                    std::mem::size_of::<CachedQuery>()
                        + reply.holes.capacity() * std::mem::size_of::<super::coverage::Hole>()
                        + reply.data.bytes()
                        + bytes.total() as usize
                        + reply.summaries.bytes()
                },
                |previous| answered(previous, serde_json::to_string(&publication)?),
            )?
    } else {
        std::sync::Arc::new(answered(
            None,
            serde_json::to_string(&reply_presentation(content))?,
        )?)
    };
    check_cancel(cancelled)?;
    // What a pending or partial reply waits for: the coverage it found short,
    // or, when it consulted none, the coverage its product waits for.
    let gate = answer.waits.then(|| {
        if answer.short.is_empty() {
            super::pipeline::published_gate(&published, name)
        } else {
            answer.short.iter().copied().collect()
        }
    });
    let data = served::Served::new(source, &answer.data, content, &answer.fallbacks);
    let mut reply = bounded_json(
        &Reply {
            source_id: &request.source_id,
            revision: &request.revision,
            product: &request.product,
            gate,
            holes: (!answer.holes.is_empty()).then_some(answer.holes.as_slice()),
            data: (),
        },
        cancelled,
    )
    .map_err(crate::invalid)?;
    // The answer's text takes the place of the null its field serialized as.
    reply.truncate(reply.len() - "null}".len());
    let length = data.len() + 1;
    if reply.len() + length > MAX_REPLY_BYTES {
        return Err(crate::invalid(
            "Projected response exceeds the transport byte budget",
        ));
    }
    reply
        .try_reserve_exact(length)
        .map_err(|_| crate::invalid("Not enough memory for projected response"))?;
    data.write(&mut reply);
    reply.push('}');
    Ok(reply)
}
