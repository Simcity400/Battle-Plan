//! Country balances' facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! each tag whose holder, political power or command power may have changed, with its holder
//! (the tag's first country by id the links resolve, as every per-country product reads it) and
//! both stored fixed5 balances. Only the countries the hour wrote a balance of, and the tags whose
//! holders it changed, are read. A segment's first hour, an hour after a skipped one and an hour
//! the countries became complete or stopped being list every held tag.

use dashboard_world::{Needs, Roster, Tag, Tagged, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::rows::{CountryRow, TableId};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[TableId::Countries, TableId::CountryLinks];

/// What a projection reads of its walk's world.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A holder's canonical slot and id, with its political and command power, fixed5.
pub type Held = ((u32, u32), [Option<i64>; 2]);

/// One tag at a projected hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Country {
    /// Its tag's index in the projection ([`Hour::named`]).
    pub index: u32,
    /// None when no country the links resolve holds the tag.
    pub held: Option<Held>,
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// Whether `countries` lists every held tag: a tag it leaves out holds nothing.
    pub whole: bool,
    /// Tags whose holder or balances may have changed, at least every one that did.
    pub countries: Vec<Country>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.named.capacity() * size_of::<(u32, Arc<str>)>()
            + self.named.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            + self.countries.capacity() * size_of::<Country>()
    }
}

/// The Countries columns a projection reads.
struct Fields {
    political: usize,
    command: usize,
}

/// One segment's projection.
pub struct Projector {
    format: u16,
    fields: Fields,
    /// Both balance columns' bits in a change mask.
    mask: u64,
    numbers: HashMap<Tag, u32>,
    last: Option<i32>,
    /// Whether the countries and links were complete the hour before.
    complete: Option<bool>,
}

impl Projector {
    pub fn new(format: u16) -> io::Result<Self> {
        let fields = Fields {
            political: columns::field(TableId::Countries, "political_power_raw")?,
            command: columns::field(TableId::Countries, "command_power_raw")?,
        };
        let bit = |field: usize| {
            u32::try_from(field)
                .ok()
                .and_then(|field| 1_u64.checked_shl(field))
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "A balance column has no change bit"))
        };
        Ok(Self {
            format,
            mask: bit(fields.political)? | bit(fields.command)?,
            fields,
            numbers: HashMap::new(),
            last: None,
            complete: None,
        })
    }

    fn number(&mut self, tag: Tag, text: &str, named: &mut Vec<(u32, Arc<str>)>) -> u32 {
        if let Some(&index) = self.numbers.get(&tag) {
            return index;
        }
        let index = self.numbers.len() as u32;
        self.numbers.insert(tag, index);
        named.push((index, Arc::from(text)));
        index
    }

    /// Both balances of the country of raw id `id`, as the row decoder reads them.
    fn balances(&self, state: &State, id: u32) -> io::Result<[Option<i64>; 2]> {
        let held = columns::table(state, TableId::Countries)?;
        let slot = held
            .slot(u128::from(id))
            .ok_or_else(|| invalid("A held country has no Countries row"))?;
        let Fields { political, command } = self.fields;
        if !held.recorded(political, slot) || !held.recorded(command, slot) {
            let row: CountryRow = columns::row(state, TableId::Countries, slot, self.format)?;
            return Ok([row.political_power_raw, Some(row.command_power_raw)]);
        }
        let (values, present) = columns::optional::<i64>(held, political)?;
        let political = present
            .get(slot)
            .then(|| values.get(slot as usize).copied())
            .flatten();
        let command = *columns::plain::<i64>(held, command)?
            .get(slot as usize)
            .ok_or_else(|| invalid("A held country has no command power cell"))?;
        Ok([political, Some(command)])
    }

    /// `tag`'s holder and balances; None when no resolved country holds it.
    fn held(&self, state: &State, roster: &Roster, tag: Tag) -> io::Result<Option<Held>> {
        Ok(match roster.tagged(tag) {
            Tagged::Found(country) => {
                Some(((country.slot, country.id), self.balances(state, country.id)?))
            }
            Tagged::Absent | Tagged::Unresolved => None,
        })
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        if hour.starts {
            self.numbers.clear();
        }
        let state = hour.state;
        let roster = hour.world.countries()?;
        let complete = roster.complete();
        let skipped = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(hour.hour));
        self.last = Some(hour.hour);
        let whole =
            hour.starts || skipped || self.complete != Some(complete) || roster.rebuilt();
        self.complete = Some(complete);
        let mut named = Vec::new();
        let mut countries = Vec::new();
        if !complete {
            // Without every country and link no tag's holder is known.
            out.push(Hour {
                branch: hour.branch,
                hour: hour.hour,
                named,
                whole: true,
                countries,
            })?;
            return Ok(());
        }
        let tags: BTreeSet<Tag> = if whole {
            roster.held().collect()
        } else {
            let mut dirty: BTreeSet<Tag> = roster.changed_tags().iter().copied().collect();
            if hour.world.touched(TableId::Countries, self.mask) {
                let held = columns::table(state, TableId::Countries)?;
                let changes = columns::changes(state, TableId::Countries)?;
                let written = changes
                    .changed
                    .iter()
                    .zip(&changes.masks)
                    .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
                    .filter(|&(_, &mask)| mask & self.mask != 0)
                    .map(|(&slot, _)| slot)
                    .chain(changes.inserted.iter().copied());
                for slot in written {
                    if let Some(country) = held.key(slot).and_then(|id| roster.get(id as u32)) {
                        dirty.insert(country.tag);
                    }
                }
            }
            dirty
        };
        for tag in tags {
            let Ok(text) = std::str::from_utf8(roster.tag(tag)) else {
                continue;
            };
            let held = self.held(state, roster, tag)?;
            if whole && held.is_none() {
                continue;
            }
            countries.push(Country {
                index: self.number(tag, text, &mut named),
                held,
            });
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            named,
            whole,
            countries,
        })?;
        Ok(())
    }
}
