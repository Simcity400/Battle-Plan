//! A payment's item as a reply names it: words, categories, tooltips and the recipe of its icon,
//! resolved when the reply is asked from the facts its hour recorded ([`super::items`]) and the
//! recorded content; icons are drawn from their recipes as the request's selection shows them.

use super::items::{self, Dated, NamedCountry, SpriteChoice};
use super::*;
use dashboard_base::products::idea_history::{self, IdeaInfo};
use dashboard_base::products::politics_content::{PoliticsContent, recorded_politics_content};
use dashboard_text::catalogue::View;
use dashboard_text::identity::NamingCountry;

/// What an item's icon is drawn from, kept in place of the image and drawn when a reply is
/// served, as the request shows it ([`present_icons`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerArt {
    /// An idea's or advisor's art for its country at the action hour, superseded by its
    /// recorded holder's portrait when that draws.
    Idea {
        token: String,
        character: Option<String>,
        tag: String,
        naming: NamingCountry,
        /// The recorded holder and its dated portrait.
        holder: Option<(
            u32,
            dashboard_base::products::character_portraits::RecordedPortrait,
        )>,
    },
    /// A character's dated portrait.
    Portrait {
        character: u32,
        portrait: dashboard_base::products::character_portraits::RecordedPortrait,
    },
    /// A decision's or focus's icon ([`SpriteChoice`]).
    Sprites(SpriteChoice),
    /// The sprite recorded facts name (an ability's, a leader trait's).
    Sprite(String),
    /// An organisation's icon, chosen among its recorded icon and its owner's and token's
    /// fallbacks by what the presentation declares.
    Organisation {
        organisation: dashboard_base::products::production_lines::RawOrganisation,
        token_known: bool,
    },
}

/// An item's drawn icon (`None` where the game shows none), with what the item names about it:
/// a holder's portrait that could not be drawn where the idea's own art stands in.
type Shown = (Option<String>, Option<String>);

/// `art` drawn through `content`'s catalogue: the whole choice made against what it declares.
fn draw_art(content: &PoliticsContent, art: &LedgerArt) -> Result<Shown, String> {
    let sprite = |name: &str| {
        content
            .sprite_png(name)
            .map_err(|error| format!("sprite {name}: {error}"))?
            .ok_or_else(|| format!("sprite {name} is not declared"))
    };
    let declared = |name: &str| {
        content
            .catalogue
            .sprites
            .find(name)
            .map(|found| found.is_some())
            .map_err(|error| format!("sprite {name}: {error}"))
    };
    match art {
        LedgerArt::Idea {
            token,
            character,
            tag,
            naming,
            holder,
        } => {
            let mut note = None;
            if let Some((id, portrait)) = holder {
                match dashboard_base::products::character_portraits::portrait_png(content, portrait)
                {
                    Ok(icon) => return Ok((Some(icon), None)),
                    // A recorded portrait the selection cannot draw fails the selection's
                    // attempt, so the recording's own art is drawn again, never the generic
                    // art in its place.
                    Err(reason)
                        if installed_content_core::content::is_strict()
                            && matches!(
                                portrait,
                                dashboard_base::products::character_portraits::RecordedPortrait::Entry { .. }
                            ) =>
                    {
                        return Err(format!("character {id} portrait: {reason}"));
                    }
                    Err(reason) => note = Some(format!("character {id} portrait: {reason}")),
                }
            }
            let (artwork, problem) =
                content.idea_artwork(token, character.as_deref(), naming.for_tag(tag));
            match (artwork.icon, problem) {
                (Some(icon), _) => Ok((Some(icon), note)),
                (None, Some(problem)) => {
                    Err(note.map_or(problem.clone(), |note| format!("{note}; {problem}")))
                }
                (None, None) => Ok((None, note)),
            }
        }
        LedgerArt::Portrait {
            character,
            portrait,
        } => dashboard_base::products::character_portraits::portrait_png(content, portrait)
            .map(|icon| (Some(icon), None))
            .map_err(|reason| format!("character {character} portrait: {reason}")),
        LedgerArt::Sprites(choice) => {
            for candidate in &choice.candidates {
                if declared(candidate)? {
                    return sprite(candidate).map(|icon| (Some(icon), None));
                }
            }
            match &choice.otherwise {
                Ok(Some(name)) => sprite(name).map(|icon| (Some(icon), None)),
                Ok(None) => Ok((None, None)),
                Err(reason) => Err(reason.clone()),
            }
        }
        LedgerArt::Sprite(name) => sprite(name).map(|icon| (Some(icon), None)),
        LedgerArt::Organisation {
            organisation,
            token_known,
        } => {
            let refused = std::cell::RefCell::new(None);
            let chosen = dashboard_base::products::production_lines::organisation_icon_with_token(
                organisation,
                *token_known,
                |name| match declared(name) {
                    Ok(found) => found,
                    Err(reason) => {
                        refused.borrow_mut().get_or_insert(reason);
                        false
                    }
                },
            );
            if let Some(reason) = refused.into_inner() {
                return Err(reason);
            }
            match chosen
                .map_err(|reason| format!("organisation {}: {reason}", organisation.token))?
            {
                Some(name) => sprite(&name).map(|icon| (Some(icon), None)),
                None => Ok((None, None)),
            }
        }
    }
}

/// The ledger art drawn for one selection, by recipe; one per kind of catalogue.
#[derive(Default)]
struct LedgerIcons(Option<dashboard_base::catalogue::Redrawn<String, Shown>>);

/// Draws a served ledger reply's icons from their recipes ([`LedgerArt`]) as the request
/// admitted under `content` shows them: every item's icon and the idea tooltips' artwork. When
/// the selection cannot draw one, the recording's own choice is drawn and named on every reply
/// that shows it; one neither can draw says why on its item. Without content, or where it does
/// not load, each says so and the rest of the reply stands; only cancellation fails.
pub fn present_icons(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    data: &mut serde_json::Value,
) -> io::Result<()> {
    if data.get("spendItems").is_none() {
        return Ok(());
    }
    let Some(options) = content else {
        return draw_icons(data, |_| {
            Ok(Err("recorded content has not been resolved".into()))
        });
    };
    let contents = dashboard_base::products::politics_content::politics_content(source, options)
        .and_then(|admitted| Ok((admitted, recorded_politics_content(source, options)?)));
    let (admitted, recorded) = match contents {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            let reason = format!("politics ledger icons are unavailable: {error}");
            return draw_icons(data, |_| Ok(Err(reason.clone())));
        }
    };
    let state = source.state::<dashboard_base::catalogue::ByCatalogue<LedgerIcons>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let drawn = dashboard_base::catalogue::Redrawn::bind(
        &mut state.of(&admitted.catalogue).0,
        &admitted.catalogue,
        &recorded.catalogue,
    );
    draw_icons(data, |art| {
        Ok(drawn
            .get(
                serde_json::to_string(art)?,
                || "politics ledger icon".into(),
                |cat| {
                    let content = if std::ptr::eq(cat, &*admitted.catalogue) {
                        &admitted
                    } else {
                        &recorded
                    };
                    draw_art(content, art)
                },
            )
            .clone())
    })
}

/// Replaces each item's recipe in a serialized reply with what `draw` draws of it: the item's
/// icon or, for an item shown by its idea tooltip, that tooltip's artwork. A recipe `draw`
/// cannot draw, or draws with a note, says why on its item and among the counted warnings.
fn draw_icons(
    data: &mut serde_json::Value,
    mut draw: impl FnMut(&LedgerArt) -> io::Result<Result<Shown, String>>,
) -> io::Result<()> {
    let Some(items) = data
        .get_mut("spendItems")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    let mut tooltips = BTreeMap::new();
    let mut failures = BTreeMap::<String, u64>::new();
    for item in items {
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        let Some(recipe) = item.remove("iconArt") else {
            continue;
        };
        let shown = draw(&serde_json::from_value(recipe)?)?;
        let (icon, note) = match shown {
            Ok((icon, note)) => (Ok(icon), note),
            Err(reason) => (Err(()), Some(reason)),
        };
        if let Some(note) = &note {
            *failures.entry(note.clone()).or_default() += 1;
        }
        if note.is_some() {
            let mut unknown = item
                .get("unknown")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            rows::combine_reason(&mut unknown, note);
            item.insert("unknown".into(), unknown.into());
        }
        match item.get("tooltipToken").and_then(serde_json::Value::as_str) {
            Some(token) => {
                tooltips.insert(token.to_owned(), icon);
            }
            None => {
                if let Ok(icon) = icon {
                    item.insert("icon".into(), icon.into());
                }
            }
        }
    }
    if let Some(artwork) = data
        .pointer_mut("/ideaTooltips/artwork")
        .and_then(serde_json::Value::as_array_mut)
    {
        for art in artwork {
            let token = art["token"].as_str().map(str::to_owned);
            if let Some(Ok(icon)) = token.and_then(|token| tooltips.get(&token)) {
                art["icon"] = icon.clone().into();
            }
        }
    }
    if let Some(warnings) = data
        .get_mut("warnings")
        .and_then(serde_json::Value::as_array_mut)
    {
        for (reason, count) in failures {
            warnings.push(format!("{reason} (count: {count})").into());
        }
    }
    Ok(())
}

/// A row's spending item, its idea tooltip and its category.
pub(super) type Resolved = (LedgerSpendItem, Option<IdeaInfo>, (String, String));

/// The category a payment of `cause` is spent in, as the recorded content files it: an idea's
/// authored category, laws, or the native cause's.
pub(super) fn category_of(
    cause: PowerCause,
    token: Option<&str>,
    content: Option<&PoliticsContent>,
) -> (String, String) {
    let (id, name) = rows::category(cause);
    let mut category = (id.to_owned(), name.to_owned());
    let Some(content) = content else {
        return category;
    };
    let key = match category.0.as_str() {
        "diplomacy" => Some("TOPBAR_DIPLOMACY"),
        "decisions" => Some("TOPBAR_DECISIONVIEW"),
        _ => None,
    };
    if let Some(key) = key
        && let Some(name) = content.localised(key, View::Plain)
    {
        category.1 = name;
    }
    if cause == PowerCause::Idea
        && let Some(token) = token
        && let Ok(Some(idea)) = content.idea(token)
        && !idea.law
        && let Some(authored) = content
            .ideas
            .tags
            .categories()
            .0
            .iter()
            .find(|category| category.slots.iter().any(|slot| slot == &idea.group))
    {
        category.0 = format!("ideas:{}", authored.name);
        category.1 = content
            .localised(&authored.name, View::Plain)
            .unwrap_or_else(|| "Not localised".into());
    }
    if matches!(
        cause,
        PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval
    ) && let Some(token) = token
        && content
            .idea(token)
            .ok()
            .flatten()
            .is_some_and(|idea| idea.law)
    {
        category = ("laws".into(), "Laws".into());
    }
    category
}

/// The item `row` (a payment of `cause`, dated with `dated`) resolves to under `content`.
pub(super) fn resolve(
    dated: &Dated,
    row: &PaidRow,
    cause: PowerCause,
    content: Option<&PoliticsContent>,
    id: String,
    daily_component: Option<&str>,
) -> Resolved {
    let facts = &dated.facts;
    let own: Option<&NamedCountry> = dated.own.as_ref();
    let target: Option<&NamedCountry> = dated.target.as_ref();
    let token = facts.token.clone().or_else(|| {
        (!row.item.key.is_empty())
            .then(|| std::str::from_utf8(&row.item.key).ok().map(str::to_owned))
            .flatten()
    });
    let recorded = facts.character.as_ref();
    let character = row
        .item
        .character
        .or_else(|| recorded.map(|recorded| recorded.id));
    let category = category_of(cause, token.as_deref(), content);
    let mut item = LedgerSpendItem {
        id,
        group_id: String::new(),
        name: None,
        action: String::new(),
        icon: None,
        icon_art: None,
        tooltip_token: None,
        character,
        target_tag: target.map(|target| target.tag.clone()),
        target_name: target.and_then(|target| target.name.clone()),
        unknown: None,
    };
    let mut info = None;
    if let Some(content) = content {
        if matches!(row.cause.as_str(), "Idea" | "Advisor" | "AdvisorRemoval") {
            if let Some(token) = token.as_deref() {
                if let Some(own) = own {
                    let script = recorded.and_then(|recorded| recorded.script_token.as_deref());
                    // Named from recorded facts; the art is drawn when a reply is served, so
                    // only a definition the content refuses is named here.
                    let (name, problem) =
                        content.idea_named(token, script, own.naming.for_tag(&own.tag));
                    item.name = name;
                    item.unknown = problem;
                    item.icon_art = Some(LedgerArt::Idea {
                        token: token.to_owned(),
                        character: script.map(str::to_owned),
                        tag: own.tag.clone(),
                        naming: own.naming.clone(),
                        holder: recorded.map(|recorded| (recorded.id, recorded.portrait.clone())),
                    });
                    if let Some(recorded) = recorded {
                        item.name = content
                            .character_name(recorded.script_token.as_deref(), &recorded.saved_name);
                    }
                    let built = recorded.map(|recorded| &recorded.template);
                    let localise = |key: &str, view: View| content.localised(key, view);
                    let joined = idea_history::Joined::of(content, &localise);
                    let (found, _) = idea_history::prepare_info(
                        &joined,
                        token,
                        std::str::from_utf8(&row.item.slot_key)
                            .ok()
                            .filter(|slot| !slot.is_empty()),
                        built
                            .map(|built| built.as_ref().map_err(String::as_str))
                            .unwrap_or(Err(
                                "the recorded holder/template is unavailable at the action hour",
                            )),
                        false,
                    );
                    info = Some(found);
                } else {
                    item.unknown = Some(format!(
                        "{token}: country identity unavailable at action hour"
                    ));
                }
            }
        } else if let Some(token) = token.as_deref() {
            // Abilities and traits display their recorded token's localisation. Refuse
            // scripted text without its event scope rather than print it.
            item.name = None;
            if matches!(
                row.cause.as_str(),
                "Decision" | "TargetedDecision" | "DecisionTimeout" | "Focus" | "AbilityActivation"
            ) {
                match content.power_content() {
                    Err(reason) => item.unknown = Some(reason),
                    Ok(power) => {
                        if row.cause == "AbilityActivation" {
                            if let Some(key) = power.abilities.name_key(token.as_bytes()) {
                                item.name = content.localised(key, View::Plain);
                            }
                            if let Some(sprite) = power.abilities.static_sprite(token.as_bytes()) {
                                item.icon_art = Some(LedgerArt::Sprite(sprite.to_owned()));
                            }
                        }
                    }
                }
            }
        }
        if matches!(
            row.cause.as_str(),
            "LeaderPromotion"
                | "LeaderAssignment"
                | "LeaderRecruitment"
                | "ScientistRecruitment"
                | "ScientistFaction"
                | "OfficerUpgrade"
                | "CaptainUpgrade"
        ) && let Some(id) = character
        {
            if let Some(recorded) = recorded {
                // Named from recorded facts; the portrait is drawn when a reply is served.
                item.name =
                    content.character_name(recorded.script_token.as_deref(), &recorded.saved_name);
                item.unknown = None;
                item.icon_art = Some(LedgerArt::Portrait {
                    character: recorded.id,
                    portrait: recorded.portrait.clone(),
                });
            } else {
                item.unknown = Some(format!(
                    "character {id}: unit-leader portrait role unavailable at action hour"
                ));
            }
        }
    } else {
        item.unknown = Some("recorded content has not been resolved".into());
    }
    if let Some(content) = content {
        if let Some(text) = facts.text.as_ref() {
            if !matches!(row.cause.as_str(), "Advisor" | "AdvisorRemoval") {
                item.name = text.name.clone();
            }
            rows::combine_reason(&mut item.unknown, text.unknown.clone());
            if let Some(choice) = &text.sprites {
                item.icon_art = Some(LedgerArt::Sprites(choice.clone()));
            }
        }
        if row.cause == "Diplomacy" {
            // The daily relation title uses the recorded relation TYPE_ID token, not a
            // diplomatic action.
            let key = if daily_component == Some("Relation") {
                Some(installed_content_core::diplomacy::improve_relation_title_key().to_owned())
            } else if !row.item.key.is_empty() {
                installed_content_core::diplomacy::scripted_action_name_key(
                    std::str::from_utf8(&row.item.key).unwrap_or_default(),
                )
                .ok()
            } else if row.charged {
                row.item
                    .token
                    .and_then(installed_content_core::diplomacy::charged_action_name_key)
                    .map(str::to_owned)
            } else {
                None
            };
            item.name = key
                .as_deref()
                .and_then(|key| content.localised(key, View::Plain));
            if key.is_none() {
                item.unknown = Some(format!(
                    "diplomatic action token {:?}: native action-name branch is not recorded/identified",
                    row.item.token
                ));
            }
        }
        if row.cause == "LeaderTrait"
            && let Some(token) = token.as_deref()
        {
            match content.power_content() {
                Err(reason) => item.unknown = Some(reason),
                Ok(power) => {
                    if let Some(own) = own {
                        match power
                            .unit_traits
                            .name_key(token, &own.tag, |key| content.defines(key))
                        {
                            Ok(key) => {
                                item.name = key.and_then(|key| content.localised(&key, View::Plain))
                            }
                            Err(reason) => item.unknown = Some(reason.into()),
                        }
                    } else {
                        item.unknown = Some(format!(
                            "trait {token}: country identity unavailable at action hour"
                        ));
                    }
                    match power.unit_traits.assignment_sprite(token) {
                        Ok(Some(sprite)) => item.icon_art = Some(LedgerArt::Sprite(sprite)),
                        Ok(None) => {
                            item.unknown = Some(format!(
                                "trait {token} is not registered in recorded content"
                            ))
                        }
                        Err(reason) => item.unknown = Some(reason.into()),
                    }
                }
            }
        }
    }
    if row.cause == "DailyGain"
        && let Some(content) = content
    {
        match daily_component {
            Some("PoliticalPowerCostModifier") => {
                if let Some(modifier) = facts.modifier.as_ref() {
                    let key = std::str::from_utf8(&modifier.name_key).ok();
                    let argument = std::str::from_utf8(&modifier.argument).ok();
                    match key.zip(argument) {
                        Some((key, argument))
                            if content.catalogue.localisation.get(key).is_some() =>
                        {
                            // Modifier names substitute the recorded argument as ARG (type 1).
                            let arguments = [(
                                "ARG",
                                installed_content_localisation::localisation::Argument::Text(argument),
                            )];
                            let arguments: &[_] =
                                if argument.is_empty() { &[] } else { &arguments };
                            match dashboard_text::catalogue::localised_with(
                                &content.catalogue.localisation,
                                key,
                                arguments,
                                View::Scripted,
                            ) {
                                Ok(name) => item.name = Some(name),
                                Err(reason) => item.unknown = Some(reason),
                            }
                        }
                        Some((key, _)) => {
                            item.unknown = Some(format!("daily cost modifier {key}: not localised"))
                        }
                        None => {
                            item.unknown =
                                Some("daily cost modifier definition text is not UTF-8".into())
                        }
                    }
                } else {
                    item.unknown = Some("daily cost modifier index 0x26: ModifierDefinitions is unavailable at action hour".into());
                }
            }
            Some("MioResearch" | "MioProduction") => {
                if let Some(organisation) = facts.organisation.as_ref() {
                    // The scoped name was frozen at its read; the artwork is chosen and drawn
                    // when a reply is served.
                    item.icon_art = Some(LedgerArt::Organisation {
                        organisation: organisation.clone(),
                        token_known: facts.organisation_token_known,
                    });
                    if item.name.is_none() && item.unknown.is_none() {
                        item.unknown = Some(format!(
                            "organisation {}: recorded name is not localised or needs unavailable scope",
                            organisation.token
                        ));
                    }
                } else {
                    item.unknown = Some(format!(
                        "organisation {:?}: IndustrialOrganisations identity is unavailable at action hour",
                        row.item.object_reference
                    ));
                }
            }
            _ => {}
        }
    }
    // The net daily CP gain can be a negative operand: an identified deficit, not an
    // unidentified authored purchase.
    if row.currency == "Command" && row.cause == "DailyGain" {
        item.name = Some("Daily deficit".into());
        item.unknown = None;
    }
    if item.name.is_none()
        && !matches!(row.cause.as_str(), "NotIdentified" | "AbilityUpkeep")
        && item.unknown.is_none()
    {
        item.unknown = Some(format!("{} item {:?} is not identified", row.cause, token));
    }
    let name = item.name.as_deref().unwrap_or("Not identified");
    let target_name = item.target_name.as_deref().unwrap_or("Not identified");
    item.action = match row.cause.as_str() {
        "Advisor" => format!(
            "{} {name}",
            if row.item.action == "Replace" {
                "Replaced advisor with"
            } else {
                "Hired"
            }
        ),
        "AdvisorRemoval" => format!("Removed advisor: {name}"),
        "Idea" if category.0 == "laws" => name.to_owned(),
        "Idea" => format!(
            "{} {name}",
            match row.item.action.as_str() {
                "Remove" => "Removed",
                "Replace" => "Replaced with",
                _ => "Changed to",
            }
        ),
        "Decision" | "TargetedDecision" | "DecisionTimeout" => format!(
            "{name}{}",
            if row.item.target_country.is_some() {
                format!(": {target_name}")
            } else {
                String::new()
            }
        ),
        "Diplomacy" => format!(
            "{}: {target_name}",
            item.name.as_deref().unwrap_or("Not identified")
        ),
        "WarJustification" => format!("War justification: {target_name}"),
        "JoinWar" => format!("Joined war: {target_name}"),
        "Focus" => format!("Started focus: {name}"),
        "AbilityActivation" => format!("Activated: {name}"),
        "AbilityUpkeep" => "Ability upkeep".into(),
        "DailyGain" if row.currency == "Command" => "Daily deficit".into(),
        "LeaderTrait" => format!("Learned trait: {name}"),
        "LeaderPromotion" => format!("Promoted: {name}"),
        "LeaderAssignment" => format!("Assigned: {name}"),
        "LeaderRecruitment" => format!("Recruited: {name}"),
        "NotIdentified" => "Not identified".into(),
        _ => format!("{}: {name}", category.1),
    };
    let template = recorded
        .and_then(|recorded| recorded.template.as_ref().ok())
        .map(|template| {
            (
                &template.file,
                template.line,
                &template.tag,
                template.generated,
            )
        });
    let mut identity = row.item.clone();
    identity.action.clear();
    item.group_id = serde_json::to_string(&(
        &category.0,
        &identity,
        &token,
        &character,
        &template,
        &item.target_tag,
        &item.target_name,
        &item.name,
    ))
    .expect("display identity serializes");
    // Equal dated identity (including resolved name, description and portrait) shares one
    // tooltip and one artwork payload across repeated payments.
    if let Some(found) = info.as_mut() {
        let tooltip = serde_json::to_string(&(
            &item.group_id,
            &item.name,
            &own.map(|own| format!("{:?}", own.naming)),
            &recorded.map(|recorded| format!("{:?}", recorded.portrait)),
            &found,
            facts
                .text
                .as_ref()
                .and_then(|text| text.description.as_deref()),
        ))
        .expect("dated tooltip identity serializes");
        found.token = tooltip.clone();
        item.tooltip_token = Some(tooltip);
    }
    item.unknown = item.unknown.map(|reason| rows::undated_reason(&reason));
    // Only what the art is drawn from is kept; it is drawn when a reply is served.
    item.icon = None;
    (item, info, category)
}

/// The facts of a row no frame of its hour was read for: its recorded key alone, and the text
/// content gives without a frame.
fn frameless(
    identity: &PowerIdentity,
    cause: PowerCause,
    component: Option<PowerDailyCostKind>,
    content: Option<&PoliticsContent>,
) -> Dated {
    let mut facts = items::event_facts(None, identity, cause, component);
    if let Some(content) = content {
        let texts = items::FrameTexts::default();
        facts.text = items::item_text(
            None,
            &items::Item { cause, identity },
            &facts,
            content,
            None,
            &texts,
        );
    }
    Dated {
        facts,
        own: None,
        target: None,
        modifiers: None,
    }
}

/// The idea tooltip an item shows: its idea, artwork and effect formats, the warnings formatting
/// them gives counted in `formats`.
fn tooltip(
    item: &LedgerSpendItem,
    info: IdeaInfo,
    dated: Option<&Dated>,
    content: Option<&PoliticsContent>,
    formats: &mut BTreeSet<String>,
) -> LedgerIdeaTooltips {
    let infos = [info];
    let recorded = dated.and_then(|dated| dated.modifiers.as_deref());
    let mut modifiers = Vec::new();
    let mut texts = Vec::new();
    if let Some(content) = content {
        let (keys, text_keys) = idea_history::effect_keys(&infos);
        let index: BTreeMap<String, ModifierDefinitionRow> = recorded
            .unwrap_or_default()
            .iter()
            .map(|(key, row)| (key.clone(), row.row()))
            .collect();
        let (found, warnings) =
            dashboard_base::products::idea_format::modifier_formats_from_recorded(
                recorded.is_some().then_some(&index),
                &keys,
                content,
            );
        modifiers = found;
        formats.extend(warnings);
        texts = dashboard_base::products::idea_format::texts(content, &text_keys);
    }
    let [info] = infos;
    LedgerIdeaTooltips {
        artwork: vec![dashboard_core::dto::Artwork {
            token: info.token.clone(),
            name: item.name.clone(),
            description: dated
                .and_then(|dated| dated.facts.text.as_ref())
                .and_then(|text| text.description.clone()),
            icon: None,
            base_duration_days: None,
        }],
        ideas: vec![info],
        modifiers,
        texts,
    }
}

/// Item `ordinal` of `tag`, first paid by `first` (`row`, of `cause` and `component`), as every
/// reply shows it: named from `dated`, the facts its hour recorded, under `content`.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    tag: &str,
    ordinal: u32,
    first: String,
    row: &PaidRow,
    identity: &PowerIdentity,
    cause: PowerCause,
    dated: Option<&Dated>,
    component: Option<PowerDailyCostKind>,
    content: Option<&PoliticsContent>,
) -> super::records::ItemRecord {
    let component_name = component.map(|component| format!("{component:?}"));
    let unread;
    let facts = match dated {
        Some(dated) => dated,
        None => {
            unread = frameless(identity, cause, component, content);
            &unread
        }
    };
    let (mut item, mut info, category) = resolve(
        facts,
        row,
        cause,
        content,
        first,
        component_name.as_deref(),
    );
    // The item's ordinal keys its tooltip in every reply.
    if item.tooltip_token.is_some() {
        let token = format!("idea:{tag}:{ordinal}");
        item.tooltip_token = Some(token.clone());
        if let Some(info) = &mut info {
            info.token = token;
        }
    }
    let mut formats = BTreeSet::new();
    let ideas = info.map(|info| tooltip(&item, info, dated, content, &mut formats));
    let daily = (component.is_some() && cause == PowerCause::DailyGain).then(|| match component {
        Some(PowerDailyCostKind::MioResearch | PowerDailyCostKind::MioProduction) => {
            ("organisationUpkeep".to_owned(), "Organisation upkeep".to_owned())
        }
        _ => ("dailyCosts".to_owned(), "Daily costs".to_owned()),
    });
    super::records::ItemRecord {
        ability_upkeep: component.is_none()
            && row.currency == "Command"
            && cause == PowerCause::AbilityUpkeep,
        currency: row.currency.clone(),
        country_name: dated
            .and_then(|dated| dated.own.as_ref())
            .and_then(|own| own.name.clone()),
        item,
        category,
        ideas,
        formats: formats.into_iter().collect(),
        daily,
    }
}

/// A command power hold snapshot of `hour` as replies show it: its holders named, the remainder
/// its total leaves unattributed, why any is not named; a reply carries it to later hours.
pub(super) fn hold(
    hour: i32,
    total: Option<i64>,
    holders: &[(String, rows::Holder)],
    missing: Option<&str>,
    content: Option<&PoliticsContent>,
) -> LedgerCommandHold {
    let mut named = Vec::with_capacity(holders.len() + 1);
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut attributed = 0_i128;
    for (id, holder) in holders {
        attributed += holder.amount;
        let character = holder
            .character
            .as_ref()
            .map(|(script, saved)| (script.as_deref(), saved.as_str()));
        let name = match holder_name(
            holder.ability,
            holder.token.as_deref(),
            character,
            &holder.kind,
            content,
        ) {
            Ok(name) => Some(name),
            Err(reason) => {
                *reasons.entry(reason).or_default() += 1;
                None
            }
        };
        named.push(LedgerCommandHolder {
            id: id.clone(),
            kind: holder.kind.clone(),
            name,
            amount_raw: Some(holder.amount.to_string()),
        });
    }
    let Some(total) = total else {
        return LedgerCommandHold {
            read_hour: hour,
            through_hour: hour,
            total_raw: None,
            holders: Vec::new(),
            reason: Some(missing.unwrap_or("CountryCommandPower unavailable").to_owned()),
        };
    };
    let remainder = i128::from(total) - attributed;
    if remainder != 0 {
        named.push(LedgerCommandHolder {
            id: "notIdentified".into(),
            kind: "Not identified".into(),
            name: None,
            amount_raw: Some(remainder.to_string()),
        });
        *reasons
            .entry("Command power holder identity is not recorded for part of the held amount".into())
            .or_default() += 1;
    }
    LedgerCommandHold {
        read_hour: hour,
        through_hour: hour,
        total_raw: Some(total.to_string()),
        holders: named,
        reason: (!reasons.is_empty()).then(|| {
            reasons
                .iter()
                .map(|(reason, count)| format!("{reason} (count: {count})"))
                .collect::<Vec<_>>()
                .join("; ")
        }),
    }
}

/// The name of a command power holder its allocation recorded: an ability's localised name, or
/// its character's.
pub(super) fn holder_name(
    ability: bool,
    token: Option<&str>,
    character: Option<(Option<&str>, &str)>,
    kind: &str,
    content: Option<&PoliticsContent>,
) -> Result<String, String> {
    let content = content.ok_or("recorded content has not been resolved")?;
    if ability {
        let token = token.ok_or("ability holder identity is not recorded")?;
        // Ability names start as the token and can be replaced by authored `name`.
        let power = content.power_content()?;
        let key = power.abilities.name_key(token.as_bytes()).ok_or_else(|| {
            format!("ability {token}: name key is not registered in recorded content")
        })?;
        return content
            .localised(key, View::Plain)
            .ok_or_else(|| format!("ability {token}: name is not localised"));
    }
    if let Some((script, saved)) = character {
        return content
            .character_name(script, saved)
            .ok_or("recorded holder character name is not identified".into());
    }
    Err(format!("{kind} holder naming form is not identified"))
}

#[cfg(test)]
mod tests;
