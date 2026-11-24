use content_reader_politics::faction_templates::FactionTemplateProducts as _;

pub mod country_goals;
pub mod details;
mod goals;
pub mod history;
pub mod rewards;
#[cfg(test)]
mod tests;

use dashboard_base::catalogue::{self, Catalogue};
use dashboard_text::faction_names;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{
    collections::HashMap,
    io,
    sync::{Mutex, OnceLock},
};

pub(crate) fn faction_id(text: &str) -> Result<u64, &'static str> {
    text.parse::<u64>().ok().filter(|id| *id != 0).ok_or("Invalid faction identity")
}

pub(crate) fn owned<T>(rows: &[T], faction: u64, owner: impl Fn(&T) -> u64) -> &[T] {
    let first = rows.partition_point(|row| owner(row) < faction);
    let end = rows[first..].partition_point(|row| owner(row) == faction);
    &rows[first..first + end]
}

pub(crate) fn ordinals(faction: u64) -> (u128, u128) {
    let low = u128::from(faction) << 32;
    (low, low | u128::from(u32::MAX))
}

#[derive(Default)]
pub(crate) struct Art {
    default_icon: OnceLock<Result<String, String>>,
    drawn: Mutex<HashMap<([u8; 32], String, u32), Result<String, String>>>,
}

impl Art {
    fn sprite(&self, cat: &Catalogue, name: &str, size: u32, what: &str) -> Result<String, String> {
        let key = (cat.content.layers.art_stack_key(), name.to_owned(), size);
        if let Some(found) = self.drawn.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return found.clone();
        }
        let read = |cat: &Catalogue| -> io::Result<String> {
            let sprite = cat.sprites.lookup(name).map_err(dashboard_core::invalid)?;
            dashboard_base::products::artwork::resolve_sprite_type(cat, sprite, 0, size)
                .into_io_result()?
                .ok_or_else(|| dashboard_core::invalid("faction artwork unavailable"))
        };
        let drawn = catalogue::draw(|| format!("{what} {name}"), cat, |cat| read(cat).map_err(|why| format!("{what} {name}: {why}")))
            .get()
            .clone();
        self.drawn.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, drawn.clone());
        drawn
    }

    fn goal_icon(&self, cat: &Catalogue, group: Option<&[u8]>) -> Result<Option<String>, String> {
        let group = group.ok_or("Faction goal artwork: group is not authored")?;
        let group = std::str::from_utf8(group).map_err(|_| "Faction goal artwork: group is not UTF-8")?;
        self.sprite(cat, &format!("GFX_{group}"), 64, "Faction goal artwork").map(Some)
    }

    fn logo(&self, frame: &DecodedFrame, faction: u64, cat: &Catalogue) -> Result<String, String> {
        let rows = frame.complete::<tables::Factions>().map_err(|why| why.to_string())?;
        let row = rows.iter().find(|row| row.id == faction).ok_or("Faction logo: the faction is absent at this hour")?;
        let templated = match faction_names::template_spelling(frame, row.template_token)? {
            Some(spelling) => cat.content.layers.faction_templates()?.icon(&spelling)?.map(<[u8]>::to_vec),
            None => None,
        };
        let name = match templated {
            Some(name) => String::from_utf8(name).map_err(|_| "Faction logo sprite name is not UTF-8".to_owned())?,
            None => self
                .default_icon
                .get_or_init(|| {
                    content_reader_defines::defines::read(&cat.content.layers)
                        .and_then(|defines| defines.text("NFactions", "FACTION_DEFAULT_ICON"))
                        .map_err(|why| format!("Faction logo: {why}"))
                })
                .clone()?,
        };
        self.sprite(cat, &name, 128, "Faction logo")
    }
}
