//! Logistics archetype pictures: the best variant from recorded roster kind 2,
//! drawn with its production-row icon. Stock quantities and obsolete flags do
//! not participate in this selector.

use dashboard_base::products::{
    CountryParams, Product, equipment,
    equipment_icon::{self, Icon, IconContent, IconHour},
};
use dashboard_base::{catalogue, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::Catalogue,
    metadata::{link_slot, tagged_country, validate_campaign},
};
use installed_content_core::{content::ArtworkMode, equipment::Database};
use replay::container::SeekResult;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct StockpileArtwork;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileArtworkParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    /// Exact variant rows need their own pictures, instead of a type's best.
    pub variants: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileArt {
    /// `type:<archetype>` or `variant:<handle>`.
    pub key: String,
    pub variant: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileArtworkReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub entries: Vec<StockpileArt>,
    pub warnings: Vec<String>,
}

/// An unknown candidate can beat any known one; refusing the selection avoids
/// drawing a guessed design when a definition/ranking operand is missing.
fn best_variants(
    frame: &DecodedFrame,
    database: &Database,
    owner: u32,
    slot: u32,
) -> Result<BTreeMap<String, u64>, String> {
    if let Some(why) = &database.lost {
        return Err(format!("equipment database is refused: {why}"));
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let lexical = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|why| why.to_string())?;
    let definitions: BTreeMap<_, _> = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|why| why.to_string())?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut candidates = Vec::new();
    for row in frame
        .complete::<tables::AvailableEquipmentVariants>()
        .map_err(|why| why.to_string())?
    {
        if row.roster_kind != 2 {
            continue;
        }
        match link_slot(links, row.country) {
            Some(candidate_slot) if candidate_slot == slot => candidates.push(row),
            Some(_) => {}
            None => {
                *failures
                    .entry(format!(
                        "best-candidate country link {} is not recorded",
                        row.country
                    ))
                    .or_default() += 1
            }
        }
    }
    candidates.sort_by_key(|row| row.ordinal);
    let mut best: BTreeMap<String, (u64, equipment::VariantOrder)> = BTreeMap::new();
    for candidate in candidates {
        let operands = (|| {
            let definition = definitions.get(&candidate.variant).ok_or_else(|| {
                format!(
                    "best candidate {} has no recorded definition",
                    candidate.variant
                )
            })?;
            let token = equipment::variant_token(definition, lexical);
            let index = database.get(&token).ok_or_else(|| {
                format!(
                    "best candidate {} names equipment {token:?} absent from the recorded content",
                    candidate.variant
                )
            })?;
            let rank = equipment::variant_order(database, index, definition, frame)?;
            let key = database.equipment[rank.archetype_ordinal]
                .name_str()
                .map_err(|why| why.to_string())?
                .to_owned();
            Ok::<_, String>((key, rank))
        })();
        let (key, rank) = match operands {
            Ok(operands) => operands,
            Err(why) => {
                *failures.entry(why).or_default() += 1;
                continue;
            }
        };
        let replaces = best.get(&key).is_none_or(|(_, previous)| {
            equipment::compare_variants(owner as i32, slot as i32, rank, *previous).is_lt()
        });
        if replaces {
            best.insert(key, (candidate.variant, rank));
        }
    }
    if !failures.is_empty() {
        return Err(failures
            .into_iter()
            .map(|(why, count)| format!("{count} refused: {why}"))
            .collect::<Vec<_>>()
            .join("; "));
    }
    Ok(best
        .into_iter()
        .map(|(key, (variant, _))| (key, variant))
        .collect())
}

/// The icon content and pictures of the catalogue a request shows and, when it shows visual mods,
/// of the recording's own catalogue it keeps ([`Catalogue::recorded`]): the selector and the
/// picture run against either ([`catalogue::draw`]).
#[derive(Default)]
struct Cache {
    catalogue: Option<Arc<Catalogue>>,
    content: Option<Arc<IconContent>>,
    recorded: Option<Arc<IconContent>>,
    images: Images,
}

/// Each picture by the catalogue it was drawn through (the recording's own or the shown one),
/// the artwork mode it was drawn in and its icon, with their retained bytes.
#[derive(Default)]
struct Images {
    drawn: BTreeMap<(bool, ArtworkMode, Icon), catalogue::Drawn<Result<String, String>>>,
    bytes: usize,
}

impl Images {
    /// `icon`'s picture through `cat`, drawn once per catalogue and mode.
    fn get(&mut self, recorded: bool, cat: &Catalogue, icon: &Icon) -> Result<String, String> {
        let key = (recorded, installed_content_core::content::artwork_mode(), icon.clone());
        if let Some(image) = self.drawn.get(&key) {
            return image.get().clone();
        }
        let drawn = catalogue::Drawn::new(|| {
            equipment_icon::image(cat, icon, dashboard_base::products::artwork::ICON_EDGE)
                .map_err(|why| why.to_string())
        });
        let image = drawn.get().clone();
        let bytes = match &image {
            Ok(text) | Err(text) => text.len(),
        } + match icon {
            Icon::Path(key) | Icon::Sprite(key) => key.len(),
        };
        let budget = dashboard_base::products::artwork::MAX_RETAINED_ARTWORK_BYTES;
        if bytes <= budget {
            if self.bytes > budget - bytes {
                self.drawn.clear();
                self.bytes = 0;
            }
            self.bytes += bytes;
            self.drawn.insert(key, drawn);
        }
        image
    }
}

impl Product for StockpileArtwork {
    const KEY: &'static str = "stockpile-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = StockpileArtworkParams;
    type Reply = StockpileArtworkReply;

    fn types() -> Vec<String> {
        vec![
            StockpileArtworkParams::decl(),
            StockpileArt::decl(),
            StockpileArtworkReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
        }
        .validate()?;
        if params.variants.iter().any(|id| id.parse::<u64>().is_err()) {
            return Err("Invalid stockpile artwork variant");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let mut reply = StockpileArtworkReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            entries: Vec::new(),
            warnings: Vec::new(),
        };
        let catalogue = match context
            .content()
            .and_then(|options| catalogue::catalogue(source, options))
        {
            Ok(catalogue) => catalogue,
            Err(why) => {
                reply
                    .warnings
                    .push(format!("Stockpile artwork is unavailable: {why}"));
                return Ok(reply);
            }
        };
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Stockpile artwork cancelled",
            ));
        }
        let database = match equipment::units(source, &catalogue) {
            Ok(database) => database,
            Err(why) => {
                reply
                    .warnings
                    .push(format!("Stockpile artwork equipment is refused: {why}"));
                return Ok(reply);
            }
        };
        reply
            .warnings
            .extend(catalogue.verification.iter().cloned());
        reply.warnings.extend(catalogue.warnings.iter().cloned());
        let frame = match source.replay().seek(
            params
                .branch_id
                .parse()
                .map_err(|_| dashboard_core::invalid("Invalid stockpile artwork branch"))?,
            params.hour,
        )? {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { reason, .. } => {
                reply.warnings.push(format!(
                    "Stockpile artwork is unavailable at this hour: {reason}"
                ));
                return Ok(reply);
            }
        };
        validate_campaign(&frame, &source.replay())?;
        let links = frame.complete::<tables::CountryLinks>()?;
        let countries = frame.complete::<tables::Countries>()?;
        let Some((country, slot)) = tagged_country(countries, links, &params.tag).found() else {
            reply.warnings.push(format!(
                "{} has no recorded identity at this hour",
                params.tag
            ));
            return Ok(reply);
        };
        let mut requested: BTreeMap<String, u64> =
            match best_variants(&frame, &database, country.id, slot) {
                Ok(best) => best
                    .into_iter()
                    .map(|(key, id)| (format!("type:{key}"), id))
                    .collect(),
                Err(why) => {
                    reply
                        .warnings
                        .push(format!("1 best-variant selection refused: {why}"));
                    BTreeMap::new()
                }
            };
        for id in &params.variants {
            requested.insert(
                format!("variant:{id}"),
                id.parse()
                    .map_err(|_| dashboard_core::invalid("Invalid stockpile artwork variant"))?,
            );
        }
        let lexical = frame.complete::<tables::LexicalTokens>()?;
        let definitions: BTreeMap<_, _> = frame
            .complete::<tables::VariantDefinitions>()?
            .iter()
            .map(|row| (row.id, row))
            .collect();
        let tags = equipment::Tags::new(
            countries,
            links,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|why| why.into_inner());
        if !cache
            .catalogue
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &catalogue))
        {
            let recorded = catalogue
                .recorded()
                .map(|recorded| {
                    IconContent::with_equipment(Arc::clone(recorded), Some(Arc::clone(&database)))
                        .map(Arc::new)
                })
                .transpose()?;
            *cache = Cache {
                content: Some(Arc::new(IconContent::with_equipment(
                    Arc::clone(&catalogue),
                    Some(database),
                )?)),
                recorded,
                catalogue: Some(Arc::clone(&catalogue)),
                images: Images::default(),
            };
        }
        let Cache {
            content,
            recorded,
            images,
            ..
        } = &mut *cache;
        let content = content.as_ref().expect("icon content prepared");
        reply.warnings.extend(content.warnings());
        // The hour's operands are recorded facts, read alike from either content.
        let icon_hour = IconHour::read(&frame, content)?;
        let images = std::cell::RefCell::new(images);
        for (key, id) in requested {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Stockpile artwork cancelled",
                ));
            }
            let Some(definition) = definitions.get(&id) else {
                reply
                    .warnings
                    .push(format!("Stockpile artwork variant {id} has no definition"));
                continue;
            };
            let raw = equipment::raw_variant(definition, params.hour, &tags, lexical);
            let name = equipment::variant_names(&raw, Some(&*catalogue)).0;
            if name.is_none() {
                reply.warnings.push(format!("1 stockpile artwork name refused for variant {id}: absent from recorded content"));
            }
            // The native selector and the picture, as one operation through the catalogue drawing
            // it: when the shown one cannot draw its choice, the recording's own choice is drawn.
            let drawn = catalogue::draw(
                || format!("equipment icon of variant {id}"),
                &catalogue,
                |cat| {
                    let drawing_recorded = !std::ptr::eq(cat, &*catalogue);
                    let content = if drawing_recorded {
                        recorded.as_ref().expect("the recording's icon content prepared")
                    } else {
                        content
                    };
                    match equipment_icon::variant_icon(content, &icon_hour, definition, &raw.token)? {
                        Some(icon) => images.borrow_mut().get(drawing_recorded, cat, &icon),
                        None => Err(
                            "The native icon selector found no picture in the recorded content"
                                .to_owned(),
                        ),
                    }
                },
            );
            let (icon, reason) = match drawn.get().clone() {
                Ok(image) => (Some(image), None),
                Err(why) => (None, Some(why)),
            };
            if let Some(why) = &reason {
                reply
                    .warnings
                    .push(format!("Stockpile artwork {key}, variant {id}: {why}"));
            }
            reply.entries.push(StockpileArt {
                key,
                variant: id.to_string(),
                name,
                icon,
                reason,
            });
        }
        reply.warnings.extend(catalogue.refusal_lines());
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
