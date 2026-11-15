//! A faction's state (family 36): one faction at an hour (`faction-details`: its goals, pools
//! and shared doctrines, a rows-at-an-hour product followed as a panel, every number that moves
//! a live cell), and the history of its rules, programmes, upgrades, doctrines and members'
//! manpower over a timeline (`faction-state`), each lane the runs of recorded cells
//! ([`replay::lanes::history`]). Names, texts and art come from the recording's content stack at
//! the reply; countries are named by the one identity rule.

pub mod details;
mod goals;
pub mod history;
pub mod rewards;
#[cfg(test)]
mod tests;

use crate::catalogue::{self, Catalogue};
use dashboard_text::faction_names;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{
    collections::HashMap,
    io,
    sync::{Mutex, OnceLock},
};

/// A faction's recorded id from its decimal text; zero is the faction system's own pool.
pub(crate) fn faction_id(text: &str) -> Result<u64, &'static str> {
    text.parse::<u64>().ok().filter(|id| *id != 0).ok_or("Invalid faction identity")
}

/// The rows of `faction` among `rows` ordered by faction first.
pub(crate) fn owned<T>(rows: &[T], faction: u64, owner: impl Fn(&T) -> u64) -> &[T] {
    let first = rows.partition_point(|row| owner(row) < faction);
    let end = rows[first..].partition_point(|row| owner(row) == faction);
    &rows[first..first + end]
}

/// The keys of `faction`'s rows in a table keyed by faction above a 32-bit ordinal.
pub(crate) fn ordinals(faction: u64) -> (u128, u128) {
    let low = u128::from(faction) << 32;
    (low, low | u128::from(u32::MAX))
}

/// The faction artwork a follow has drawn, each sprite once, and the content's default faction
/// icon, read once: drawing and reading the defines cost more than a reply.
#[derive(Default)]
pub(crate) struct Art {
    default_icon: OnceLock<Result<String, String>>,
    drawn: Mutex<HashMap<(String, u32), Result<String, String>>>,
}

impl Art {
    /// Sprite `name` drawn at `size` pixels, as the selection shows it, else the recording's own.
    fn sprite(&self, cat: &Catalogue, name: &str, size: u32, what: &str) -> Result<String, String> {
        let key = (name.to_owned(), size);
        if let Some(found) = self.drawn.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return found.clone();
        }
        let read = |cat: &Catalogue| -> io::Result<String> {
            let sprite = cat.sprites.lookup(name).map_err(crate::invalid)?;
            let decode = cat.gui_decode.as_ref().map_err(crate::invalid)?;
            catalogue::sprite_frame_png(&cat.content.layers, sprite, *decode, 0, size)
        };
        let drawn = catalogue::draw(|| format!("{what} {name}"), cat, |cat| read(cat).map_err(|why| format!("{what} {name}: {why}")))
            .get()
            .clone();
        self.drawn.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, drawn.clone());
        drawn
    }

    /// A goal's icon: its group's sprite.
    fn goal_icon(&self, cat: &Catalogue, group: Option<&[u8]>) -> Result<Option<String>, String> {
        let group = group.ok_or("Faction goal artwork: group is not authored")?;
        let group = std::str::from_utf8(group).map_err(|_| "Faction goal artwork: group is not UTF-8")?;
        self.sprite(cat, &format!("GFX_{group}"), 64, "Faction goal artwork").map(Some)
    }

    /// The faction's logo as the game draws it: its template's sprite, else the content's
    /// default for a faction without one.
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
                    installed_content_core::defines::read(&cat.content.layers)
                        .and_then(|defines| defines.text("NFactions", "FACTION_DEFAULT_ICON"))
                        .map_err(|why| format!("Faction logo: {why}"))
                })
                .clone()?,
        };
        self.sprite(cat, &name, 128, "Faction logo")
    }
}
