use dashboard_base::Source;
use dashboard_base::catalogue::{self, Catalogue, CachedResult, Refusals, View};
use dashboard_query::read;
use dashboard_base::products::LoadContext;
use super::sinking_details::{ShipSinkingAttribution, resolve_attribution};
pub mod tail;
use content_reader_key_names::unit_leader::{SubUnits, sub_units};
use content_reader_localisation::localisation::display_text;
use replay_lanes::at::Seeker;
use replay_data::rows::{SunkShipEntry, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const ICON_EDGE: u32 = 48;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipClassInfo {
    pub token: u32,
    #[doc = " The recorded lexical token, resolved to the content's sub-unit spelling."]
    pub key: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
}

pub fn known_at(reads: &[(i32, i32)], hour: i32) -> Option<i32> {
    let at = reads.partition_point(|run| run.1 < hour);
    reads.get(at).map(|run| run.0.max(hour))
}

pub fn sinkings_seen(seeker: &mut Seeker<'_>, run: u32, hour: i32) -> io::Result<Option<(i32, Vec<SunkShipEntry>)>> {
    sinkings_in(&dashboard_query::cost::storage(|| seeker.frame(run, hour, &[read::Read::all(TableId::SunkShips)]))?)
}

fn sinkings_in(frame: &replay_frame::frame::DecodedFrame) -> io::Result<Option<(i32, Vec<SunkShipEntry>)>> {
    let sampled = frame.tables[TableId::SunkShips.index()].sampled_at();
    let Ok(rows) = frame.complete::<tables::SunkShips>() else { return Ok(None) };
    Ok(sampled.map(|sampled| (sampled, rows.first().map(|row| row.entries.clone()).unwrap_or_default())))
}

pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    units: Result<Arc<SubUnits>, String>,
    regions: Result<(Arc<content_reader_regions::regions::StrategicRegions>, BTreeMap<u32, Vec<u32>>), String>,
}

impl Content {
    pub fn load(source: &Source, context: &LoadContext) -> Self {
        let catalogue = context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| error.to_string());
        Self::from_catalogue(catalogue)
    }

    fn from_catalogue(catalogue: Result<Arc<Catalogue>, String>) -> Self {
        let units = catalogue
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|cat| cat.content.layers.derived::<CachedResult<SubUnits>>().get_or_init(|| sub_units(&cat.content.layers)).map_err(|error| error.to_string()));
        let regions = catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
            let regions = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| error.to_string())?;
            let mut memberships = BTreeMap::<u32, Vec<u32>>::new();
            for (&id, region) in &regions.regions {
                let mut seen = BTreeSet::new();
                for &province in &region.provinces {
                    if seen.insert(province) {
                        memberships.entry(province).or_default().push(id);
                    }
                }
            }
            Ok((regions, memberships))
        });
        Self { catalogue, units, regions }
    }

    pub fn catalogue(&self) -> Option<&Catalogue> {
        self.catalogue.as_deref().ok()
    }

    pub fn warnings(&self) -> Vec<String> {
        match &self.catalogue {
            Ok(cat) => cat.verification.iter().chain(&cat.warnings).cloned().collect(),
            Err(reason) => vec![format!("Ship and fleet names are unavailable: {reason}")],
        }
    }
}

pub fn ship_details(
    seeker: &mut Seeker<'_>,
    run: u32,
    hour: i32,
    entries: &[SunkShipEntry],
    content: &Content,
    refused: &mut Refusals,
) -> io::Result<Vec<(Option<String>, Option<String>, ShipSinkingAttribution)>> {
    let held: Vec<&SunkShipEntry> = entries.iter().collect();
    let named = Named::new(seeker, run, hour, &held, content)?;
    refused.merge(&named.refused);
    Ok(entries
        .iter()
        .map(|entry| {
            let class = named.classes.get(&entry.class_token).and_then(|class| class.name.clone()).or_else(|| {
                text(&entry.class_name, "recorded class name", refused).map(|name| format!("{name} (recorded class)"))
            });
            let sinking_cause = named.attribution_for(entry, content, refused);
            (stored(&entry.ship_name, "ship name", content, refused), class, sinking_cause)
        })
        .collect())
}

#[derive(Default)]
struct Named {
    classes: BTreeMap<u32, ShipClassInfo>,
    attr_classes: HashMap<u32, (Option<String>, Option<String>)>,
    places: HashMap<u32, String>,
    refused: Refusals,
    warnings: Vec<String>,
}

fn literals(seeker: &mut Seeker<'_>, run: u32, hour: i32, tokens: &BTreeSet<u32>) -> io::Result<HashMap<u32, String>> {
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let keys: Vec<u128> = tokens.iter().map(|&token| u128::from(token)).collect();
    let frame = dashboard_query::cost::storage(|| seeker.frame(run, hour, &[read::Read { table: TableId::LexicalTokens, columns: None, keys: read::Keys::These(&keys) }]))?;
    let Ok(rows) = frame.complete::<tables::LexicalTokens>() else { return Ok(HashMap::new()) };
    Ok(rows.iter().map(|row| (row.token, String::from_utf8_lossy(&row.literal).into_owned())).collect())
}

fn class_info(token: u32, literal: Option<String>, content: &Content, refused: &mut Refusals) -> ShipClassInfo {
    let mut info = ShipClassInfo { token, key: literal, name: None, icon: None };
    let (Ok(cat), Ok(units), Some(literal)) = (&content.catalogue, &content.units, info.key.clone()) else {
        if info.key.is_none() {
            refused.note("class is absent from lexical tokens", format!("token {token}"));
        }
        return info;
    };
    match units.token_name(&content_reader_parser::tokens::Token::c_str(literal.as_bytes())) {
        Ok(spelling) if !spelling.is_empty() => {
            let spelling = spelling.to_owned();
            if cat.localisation.get(&spelling).is_some() {
                info.name = cat.localised(&spelling, View::Plain);
            }
            if info.name.is_none() {
                refused.note("class localisation is unavailable", &spelling);
            }
            let sprite = format!("GFX_unit_{spelling}_icon_medium");
            match cat.sprites.find(&sprite) {
                Ok(Some(found)) => match dashboard_base::products::artwork::resolve_sprite_type(cat, found, 0, ICON_EDGE).into_result() {
                    Ok(Some(image)) => info.icon = Some(image),
                    Ok(None) => {}
                    Err(reason) => refused.note(reason, &sprite),
                },
                Ok(None) => {}
                Err(reason) => refused.note(reason, &sprite),
            }
            info.key = Some(spelling);
        }
        Ok(_) => refused.note("recorded class is absent from common/units", &literal),
        Err(reason) => refused.note(reason, &literal),
    }
    info
}

fn text(bytes: &[u8], what: &str, refused: &mut Refusals) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    match display_text(bytes) {
        Ok(text) if !text.is_empty() => Some(text),
        Ok(_) => None,
        Err(reason) => {
            refused.note(reason, what);
            None
        }
    }
}

fn stored(bytes: &[u8], what: &str, content: &Content, refused: &mut Refusals) -> Option<String> {
    let Ok(cat) = &content.catalogue else { return text(bytes, what, refused) };
    match dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, what) {
        Ok(value) => text(value.as_bytes(), what, refused),
        Err(reason) => {
            refused.note(reason, what);
            None
        }
    }
}

fn class_key_and_name(literal: Option<String>, content: &Content) -> (Option<String>, Option<String>) {
    let Some(literal) = literal.filter(|text| !text.is_empty()) else {
        return (None, None);
    };
    let (Ok(cat), Ok(units)) = (&content.catalogue, &content.units) else {
        return (Some(literal), None);
    };
    match units.token_name(&content_reader_parser::tokens::Token::c_str(literal.as_bytes())) {
        Ok(spelling) if !spelling.is_empty() => {
            let spelling = spelling.to_owned();
            let name = cat.localisation.get(&spelling).and_then(|_| cat.localised(&spelling, View::Plain));
            (Some(spelling), name)
        }
        _ => (Some(literal), None),
    }
}

impl Named {
    fn attribution_for(&self, entry: &SunkShipEntry, content: &Content, refused: &mut Refusals) -> ShipSinkingAttribution {
        let attr_text = stored(
            &entry.attribution_name,
            if entry.cause_record != 0 { "sinking cause" } else { "attribution ship name" },
            content,
            refused,
        );
        let (key, mut name) = if entry.attribution_class_present != 0 && entry.attribution_class_token != 0 {
            self.attr_classes.get(&entry.attribution_class_token).cloned().unwrap_or((None, None))
        } else {
            (None, None)
        };
        if entry.attribution_class_present != 0 && name.is_none() {
            name = text(&entry.attribution_class_name, "recorded attribution class name", refused);
        }
        resolve_attribution(entry, attr_text, key, name)
    }

    fn new(seeker: &mut Seeker<'_>, run: u32, hour: i32, entries: &[&SunkShipEntry], content: &Content) -> io::Result<Self> {
        let tokens: BTreeSet<u32> = entries.iter().map(|entry| entry.class_token).collect();
        let attr_tokens: BTreeSet<u32> = entries
            .iter()
            .filter(|entry| entry.attribution_class_present != 0 && entry.attribution_class_token != 0)
            .map(|entry| entry.attribution_class_token)
            .collect();
        let all_tokens: BTreeSet<u32> = tokens.union(&attr_tokens).copied().collect();
        let literals = literals(seeker, run, hour, &all_tokens)?;
        Ok(Self::from_literals(tokens, attr_tokens, &literals, entries, content))
    }

    fn from_literals(
        tokens: BTreeSet<u32>,
        attr_tokens: BTreeSet<u32>,
        literals: &HashMap<u32, String>,
        entries: &[&SunkShipEntry],
        content: &Content,
    ) -> Self {
        let mut out = Self::default();
        if let Err(reason) = &content.catalogue {
            out.warnings.push(format!("Ship class names, artwork and sea zones are unavailable: {reason}"));
        } else if let Err(reason) = &content.units {
            out.warnings.push(format!("Ship class content is unavailable: {reason}; classes show their recorded name or token without artwork."));
        }
        if let Ok(units) = &content.units {
            out.warnings.extend(units.warnings().iter().cloned());
        }
        for token in tokens {
            let info = class_info(token, literals.get(&token).cloned(), content, &mut out.refused);
            out.classes.insert(token, info);
        }
        for token in attr_tokens {
            let resolved = class_key_and_name(literals.get(&token).cloned(), content);
            out.attr_classes.insert(token, resolved);
        }
        match (&content.catalogue, &content.regions) {
            (Ok(cat), Ok((regions, memberships))) => {
                out.warnings.extend(regions.notes.iter().cloned());
                for province in entries.iter().filter(|entry| entry.province_present != 0).map(|entry| entry.province).collect::<BTreeSet<_>>() {
                    let name = match memberships.get(&province).map(Vec::as_slice) {
                        Some([region]) => {
                            let key = &regions.regions[region].name_key;
                            let name = cat.localised(key, View::Plain);
                            if name.is_none() {
                                out.refused.note("place localisation is unavailable", key);
                            }
                            name
                        }
                        Some(_) => {
                            out.refused.note("province belongs to multiple content strategic regions", format!("province {province}"));
                            None
                        }
                        None => {
                            out.refused.note("province is absent from content strategic regions", format!("province {province}"));
                            None
                        }
                    };
                    if let Some(name) = name {
                        out.places.insert(province, name);
                    }
                }
            }
            (Ok(_), Err(reason)) => out.warnings.push(format!("Ship sinking places are unavailable: {reason}; no place name is invented.")),
            _ => {}
        }
        if let Ok(cat) = &content.catalogue {
            out.warnings.extend(cat.verification.iter().chain(&cat.warnings).cloned());
            out.warnings.extend(cat.refusal_lines());
        }
        out
    }
}
