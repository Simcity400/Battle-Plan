//! Named ship sinkings from the shared retained-history scan. The event date
//! comes from fill_common@140C1E960, independently of the daily supplying read.
//! A battle is the victim task force's first combat reference, not a retained
//! result foreign key. Drawn battle titles resolve through the native region/VP
//! selector and the current recorded PlaceNames ladder.

use super::{
    BranchParams, LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_groups::HourSpan, sinkings,
};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals, View},
    scripted_text,
    transport::json_len,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub(crate) mod battle_titles;
pub use battle_titles::ShipsSunkBattleTitles;

pub struct ShipsSunk;

const ICON_EDGE: u32 = 48;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipClassInfo {
    pub token: u32,
    /// The recorded lexical token, resolved to the native sub-unit spelling.
    pub key: Option<String>,
    pub name: Option<String>,
    /// PNG sent once per class, never once per sinking.
    pub icon: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ShipSinkingCause {
    Hostile,
    Assisted,
    /// The cause constructor records a description, but no accident/mine enum.
    Cause,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkRow {
    pub sink_hour: i32,
    pub known_at: i32,
    pub name: Option<String>,
    pub class_index: u32,
    pub recorded_name_index: Option<u32>,
    pub victim: Option<String>,
    pub attacker: Option<String>,
    /// Null uses the sinking hour; otherwise names and flags use this read.
    pub victim_as_of: Option<i32>,
    pub attacker_as_of: Option<i32>,
    pub place_index: Option<u32>,
    pub cause: ShipSinkingCause,
    pub attribution_name: Option<String>,
    pub assisted_kill: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkReply {
    pub branch_id: String,
    /// A complete empty read is available and differs from no recorded table.
    pub available: bool,
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_read: Option<i32>,
    pub unread: Vec<HourSpan>,
    pub classes: Vec<ShipClassInfo>,
    pub places: Vec<String>,
    pub recorded_names: Vec<String>,
    /// Native read order; the log kit orders by event hour and numeric row index.
    pub rows: Vec<ShipsSunkRow>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Named {
    classes: Vec<ShipClassInfo>,
    class_indices: BTreeMap<u32, u32>,
    recorded_names: Vec<String>,
    recorded_name_indices: BTreeMap<Vec<u8>, Option<u32>>,
    places: BTreeMap<u32, String>,
    stored_names: BTreeMap<Vec<u8>, Option<String>>,
    warnings: Vec<String>,
}

impl Named {
    pub(crate) fn ship(&self, ship: &sinkings::ShipEvent) -> (Option<String>, Option<String>) {
        let name = self
            .stored_names
            .get(&ship.entry.ship_name)
            .cloned()
            .flatten();
        let class = self
            .class_indices
            .get(&ship.entry.class_token)
            .and_then(|index| self.classes.get(*index as usize))
            .and_then(|class| class.name.clone())
            .or_else(|| {
                self.recorded_name_indices
                    .get(&ship.entry.class_name)
                    .copied()
                    .flatten()
                    .and_then(|index| self.recorded_names.get(index as usize))
                    .map(|name| format!("{name} (recorded class)"))
            });
        (name, class)
    }
    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

fn cancelled(flag: &AtomicBool) -> io::Result<()> {
    if flag.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Ships sunk cancelled",
        ))
    } else {
        Ok(())
    }
}

fn text(bytes: &[u8], what: &str, refused: &mut Refusals) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    match catalogue::display_text(bytes) {
        Ok(text) if !text.is_empty() => Some(text),
        Ok(_) => None,
        Err(reason) => {
            refused.note(reason, what);
            None
        }
    }
}

fn named(
    scan: &sinkings::Scanned,
    cat: Option<&Catalogue>,
    flag: &AtomicBool,
) -> io::Result<Named> {
    let mut out = Named::default();
    let mut refused = Refusals::default();
    let tokens: BTreeSet<_> = scan
        .ships
        .iter()
        .map(|ship| ship.entry.class_token)
        .collect();
    let units = cat.and_then(|cat| match installed_content::unit_leader::sub_units(&cat.content.layers) {
        Ok(units) => Some(units),
        Err(reason) => {
            out.warnings.push(format!("Ship class content is unavailable: {reason}; classes show their recorded name or token without artwork."));
            None
        }
    });
    if let Some(units) = &units {
        out.warnings.extend(units.warnings().iter().cloned());
    }
    for token in tokens {
        cancelled(flag)?;
        let mut key = scan.tokens.get(&token).cloned();
        let mut name = None;
        let mut icon = None;
        if let (Some(cat), Some(units), Some(literal)) = (cat, &units, &key) {
            // 140AB5EE0 resolves the runtime token against the common/units
            // database; matching preserves its first registered spelling.
            match units.token_name(&installed_content::tokens::Token::c_str(literal.as_bytes())) {
                Ok(spelling) if !spelling.is_empty() => {
                    let spelling = spelling.to_owned();
                    // Distinguish a defined name from the localiser's key echo
                    // so the row can fall back to its recorded class name.
                    if cat.localisation.get(&spelling).is_some() {
                        name = cat.localised(&spelling, View::Plain);
                    }
                    // SelectUnitTypeIconSprite@140B367C0 with param_5=0 uses
                    // definition +0x18; exe prefix@1429344D0 and suffix@1429344A8.
                    let sprite_name = format!("GFX_unit_{spelling}_icon_medium");
                    match cat.sprites.find(&sprite_name) {
                        Ok(Some(sprite)) => {
                            let result = cat.gui_decode.as_ref().map_err(crate::invalid).and_then(
                                |decode| {
                                    catalogue::sprite_frame_png(
                                        &cat.content.layers,
                                        sprite,
                                        *decode,
                                        0,
                                        ICON_EDGE,
                                    )
                                },
                            );
                            match result {
                                Ok(image) => icon = Some(image),
                                Err(reason) => refused.note(reason.to_string(), &sprite_name),
                            }
                        }
                        Ok(None) => {}
                        Err(reason) => refused.note(reason.to_string(), &sprite_name),
                    }
                    if name.is_none() {
                        refused.note("class localisation is unavailable", &spelling);
                    }
                    key = Some(spelling);
                }
                Ok(_) => refused.note("recorded class is absent from common/units", literal),
                Err(reason) => refused.note(reason, literal),
            }
        } else if key.is_none() {
            refused.note(
                "class is absent from LexicalTokens",
                format!("token {token}"),
            );
        }
        out.class_indices.insert(token, out.classes.len() as u32);
        out.classes.push(ShipClassInfo {
            token,
            key,
            name,
            icon,
        });
    }
    let mut indices = BTreeMap::<String, u32>::new();
    for ship in &scan.ships {
        let bytes = &ship.entry.class_name;
        if out.recorded_name_indices.contains_key(bytes) {
            continue;
        }
        let index = text(bytes, "recorded class name", &mut refused).map(|name| {
            if let Some(index) = indices.get(&name) {
                return *index;
            }
            let index = out.recorded_names.len() as u32;
            out.recorded_names.push(name.clone());
            indices.insert(name, index);
            index
        });
        out.recorded_name_indices.insert(bytes.clone(), index);
    }
    if let Some(cat) = cat {
        out.warnings
            .extend(sinkings::retention(scan, Some(cat)).warnings);
        let regions = match installed_content::regions::read(&cat.content.layers) {
            Ok(regions) => regions,
            Err(reason) => {
                out.warnings.push(format!(
                    "Ship sinking places are unavailable: {reason}; no place name is invented."
                ));
                Default::default()
            }
        };
        out.warnings.extend(regions.notes);
        let wanted: BTreeSet<_> = scan
            .ships
            .iter()
            .filter(|ship| ship.entry.province_present != 0)
            .map(|ship| ship.entry.province)
            .collect();
        let mut memberships = BTreeMap::<u32, Vec<&installed_content::regions::StrategicRegion>>::new();
        for region in regions.regions.values() {
            let mut seen = BTreeSet::new();
            for province in &region.provinces {
                if wanted.contains(province) && seen.insert(*province) {
                    memberships.entry(*province).or_default().push(region);
                }
            }
        }
        for province in wanted {
            cancelled(flag)?;
            match memberships.get(&province).map(Vec::as_slice) {
                // CStrategicRegion::GetName@140E0EB10 reads the content
                // template's name key; no vanilla region ID/key is assumed.
                Some([region]) => {
                    if let Some(name) = cat.localised(&region.name_key, View::Plain) {
                        out.places.insert(province, name);
                    } else {
                        refused.note("place localisation is unavailable", &region.name_key);
                    }
                }
                Some(_) => refused.note(
                    "province belongs to multiple content strategic regions",
                    format!("province {province}"),
                ),
                None => refused.note(
                    "province is absent from content strategic regions",
                    format!("province {province}"),
                ),
            }
        }
        // Constructors@140C1DF70/140C1DE60 copy ship names without their keys;
        // cause_constructor@140C1E220 also stores translated cause text.
        // PLAN #164 applies to all these localised-or-literal fields.
        for ship in &scan.ships {
            for (bytes, what) in [
                (&ship.entry.ship_name, "ship name"),
                (
                    &ship.entry.attribution_name,
                    if ship.entry.cause_record != 0 {
                        "sinking cause"
                    } else {
                        "attribution ship name"
                    },
                ),
            ] {
                if out.stored_names.contains_key(bytes) {
                    continue;
                }
                let value = match scripted_text::stored_text(
                    &cat.localisation,
                    &cat.recorded_language,
                    bytes,
                    what,
                ) {
                    Ok(value) => text(value.as_bytes(), what, &mut refused),
                    Err(reason) => {
                        refused.note(reason, what);
                        None
                    }
                };
                out.stored_names.insert(bytes.clone(), value);
            }
        }
        out.warnings
            .extend(cat.verification.iter().chain(&cat.warnings).cloned());
        out.warnings.extend(cat.refusal_lines());
    }
    out.warnings.extend(refused.lines("Ships sunk fields"));
    Ok(out)
}

pub(crate) fn check_content(cat: &Catalogue) -> io::Result<()> {
    // Corpus readers own their notes; naval-losses owns retention findings.
    installed_content::unit_leader::sub_units(&cat.content.layers)?;
    installed_content::regions::read(&cat.content.layers)?;
    Ok(())
}

fn reply(
    branch: &str,
    scan: &sinkings::Scanned,
    names: &Named,
    flag: &AtomicBool,
) -> io::Result<ShipsSunkReply> {
    let places: Vec<_> = names
        .places
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let place_indices: BTreeMap<_, _> = places
        .iter()
        .enumerate()
        .map(|(index, name)| (name, index as u32))
        .collect();
    let mut rows = Vec::with_capacity(scan.ships.len());
    for ship in &scan.ships {
        cancelled(flag)?;
        let entry = &ship.entry;
        // Both flags are independent: cause wins over assisted attribution.
        let cause = if entry.cause_record != 0 {
            ShipSinkingCause::Cause
        } else if entry.assisted_kill != 0 {
            ShipSinkingCause::Assisted
        } else {
            // combat_source_constructor@140C1DF70 clears both flags. An
            // unresolved attribution country does not change this record kind.
            ShipSinkingCause::Hostile
        };
        let province = (entry.province_present != 0).then_some(entry.province);
        let class_index = names
            .class_indices
            .get(&entry.class_token)
            .copied()
            .ok_or_else(|| {
                io::Error::other(format!(
                    "Ship class token {} has no resolved class index",
                    entry.class_token
                ))
            })?;
        let recorded_name_index = names
            .recorded_name_indices
            .get(&entry.class_name)
            .copied()
            .ok_or_else(|| io::Error::other("Ship recorded class name has no interned index"))?;
        rows.push(ShipsSunkRow {
            sink_hour: entry.sink_hour,
            known_at: ship.read_hour,
            name: names.stored_names.get(&entry.ship_name).cloned().flatten(),
            class_index,
            recorded_name_index,
            victim: ship.victim.clone(),
            attacker: ship.attacker.clone(),
            victim_as_of: ship.victim_as_of,
            attacker_as_of: ship.attacker_as_of,
            place_index: province
                .and_then(|province| names.places.get(&province))
                .and_then(|name| place_indices.get(name))
                .copied(),
            cause,
            attribution_name: names
                .stored_names
                .get(&entry.attribution_name)
                .cloned()
                .flatten(),
            assisted_kill: entry.assisted_kill != 0,
        });
    }
    let mut warnings = scan.warnings.clone();
    warnings.extend(names.warnings.iter().cloned());
    let reply = ShipsSunkReply {
        branch_id: branch.to_owned(),
        available: scan.first_ship_read.is_some(),
        first_hour: scan.first_hour,
        last_hour: scan.last_hour,
        first_read: scan.first_ship_read,
        unread: scan.ship_unread.clone(),
        classes: names.classes.clone(),
        places,
        recorded_names: names.recorded_names.clone(),
        rows,
        warnings,
    };
    if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "Ships sunk history exceeds its reply budget",
        ));
    }
    Ok(reply)
}

#[derive(Default)]
struct Cache {
    named: Vec<(u64, Arc<Catalogue>, Arc<Named>)>,
}

/// Uses the same recorded text/content resolution as Ships sunk, without a
/// complete history reply or a second history scan.
pub(crate) fn cached_names(
    source: &Source,
    branch: u64,
    scan: &sinkings::Scanned,
    context: &LoadContext,
    flag: &AtomicBool,
) -> io::Result<Arc<Named>> {
    let cat = context
        .content()
        .and_then(|content| catalogue::catalogue(source, content));
    match cat {
        Ok(cat) => {
            let state = source.state::<Cache>();
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            let names = if let Some((_, _, names)) = cache
                .named
                .iter()
                .find(|(id, from, _)| *id == branch && Arc::ptr_eq(from, &cat))
            {
                Arc::clone(names)
            } else {
                let names = Arc::new(named(scan, Some(&cat), flag)?);
                cache.named.retain(|(id, _, _)| *id != branch);
                cache.named.push((branch, cat, Arc::clone(&names)));
                if cache.named.len() > 2 {
                    cache.named.remove(0);
                }
                names
            };
            drop(cache);
            Ok(names)
        }
        Err(reason) => {
            let mut names = named(scan, None, flag)?;
            names.warnings.push(format!("Ships sunk content names, artwork, retention and the language of stored ship/cause names are unavailable: {reason}; ship-history completeness is unknown."));
            Ok(Arc::new(names))
        }
    }
}

impl Product for ShipsSunk {
    const KEY: &'static str = "ships-sunk";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = ShipsSunkReply;

    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            ShipClassInfo::decl(),
            ShipSinkingCause::decl(),
            ShipsSunkRow::decl(),
            ShipsSunkReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        flag: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| crate::invalid("Invalid branch"))?;
        let scan = sinkings::cached(source, branch, flag)?;
        let names = cached_names(source, branch, &scan, context, flag)?;
        reply(&params.branch_id, &scan, &names, flag)
    }
}

#[cfg(test)]
mod tests;
