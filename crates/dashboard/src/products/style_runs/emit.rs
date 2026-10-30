//! Country styles' emit: each tag's holder, map colour, naming operands and flag choice as runs
//! on the family's series publication, and the hours whose styles are known. Naming operands and
//! flag choices are catalogue cells numbered by value in first-seen order, each once; a run
//! holds the number. A flag's candidate chain is chosen in the recorded mod stack once.

use super::{Configured, facts};
use crate::flag_choices::Chain;
use dashboard_base::products::country_styles::CountryFlagChoice;
use dashboard_series::{
    Evidence, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use installed_content_country_names::country_names::Operands;
use std::{collections::HashMap, io, sync::Arc};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's holder: its raw link id above its canonical slot.
pub const HOLDER: u64 = 0;
/// Its holder's map colour, `0xRRGGBB`.
pub const COLOUR: u64 = 1;
/// The number of its naming operands ([`NAMINGS`]).
pub const NAMING: u64 = 2;
/// The number of its flag choice ([`CHOICES`]).
pub const FLAG: u64 = 3;
/// [`FAMILY`]'s one measure: 1 over every hour whose styles are known.
pub const KNOWN: u64 = 4;
/// The coverage every key belongs to: the hours the family read.
const DOMAIN: u64 = 0;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Each naming operands' catalogue id, by number.
pub const NAMINGS: u64 = 1 << 28;
/// Each flag choice's catalogue id, by number.
pub const CHOICES: u64 = 1 << 30;

fn invalid() -> io::Error {
    crate::invalid("Prepared country styles are not valid")
}

/// A tag's values, by measure ([`HOLDER`] to [`FLAG`]).
type Track = [Option<i128>; 4];

/// A holder as [`HOLDER`] keeps it.
pub fn holder((id, slot): (u32, u32)) -> i128 {
    i128::from(id) << 32 | i128::from(slot)
}

/// A colour as [`COLOUR`] keeps it.
pub fn colour([red, green, blue]: [u8; 3]) -> i128 {
    i128::from(red) << 16 | i128::from(green) << 8 | i128::from(blue)
}

fn text(text: &str) -> Scalar {
    Scalar::Text(text.as_bytes().into())
}

fn optional(value: Option<&str>) -> Scalar {
    value.map_or(Scalar::Null, text)
}

fn read_text(scalar: &Scalar) -> io::Result<String> {
    String::from_utf8(scalar.text().ok_or_else(invalid)?.to_vec()).map_err(|_| invalid())
}

fn read_optional(scalar: &Scalar) -> io::Result<Option<String>> {
    match scalar {
        Scalar::Null => Ok(None),
        scalar => read_text(scalar).map(Some),
    }
}

fn read_u32(scalar: &Scalar) -> io::Result<u32> {
    scalar
        .integer()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(invalid)
}

/// Naming operands as their cell holds them: which of slot zero's names are built, then each
/// slot as `[slot, reason]` or `[slot, overlord, tag, cosmetic, four suffixes]`.
pub fn naming_cell(naming: &facts::Naming) -> Scalar {
    let chain = naming
        .chain
        .iter()
        .map(|(slot, operands)| {
            let mut cell = vec![Scalar::Integer(i128::from(*slot))];
            match operands {
                Err(reason) => cell.push(text(reason)),
                Ok(operands) => {
                    let (overlord, texts) = operands.parts();
                    cell.push(
                        overlord.map_or(Scalar::Null, |slot| Scalar::Integer(i128::from(slot))),
                    );
                    cell.extend(texts.map(text));
                }
            }
            Scalar::List(cell)
        })
        .collect();
    Scalar::List(vec![
        naming
            .built
            .map_or(Scalar::Null, |built| Scalar::Integer(i128::from(built))),
        Scalar::List(chain),
    ])
}

/// The naming operands [`naming_cell`] made `cell` of.
pub fn naming_of(cell: &Scalar) -> io::Result<facts::Naming> {
    let [built, chain] = cell.list() else {
        return Err(invalid());
    };
    let built = match built {
        Scalar::Null => None,
        built => Some(
            built
                .integer()
                .and_then(|built| u8::try_from(built).ok())
                .ok_or_else(invalid)?,
        ),
    };
    let mut naming = facts::Naming {
        chain: Vec::with_capacity(chain.list().len()),
        built,
    };
    for item in chain.list() {
        let operands = match item.list() {
            [slot, reason] => (read_u32(slot)?, Err(read_text(reason)?)),
            [slot, overlord, texts @ ..] if texts.len() == 6 => {
                let overlord = match overlord {
                    Scalar::Null => None,
                    overlord => Some(read_u32(overlord)?),
                };
                let mut parts: [String; 6] = Default::default();
                for (part, scalar) in parts.iter_mut().zip(texts) {
                    *part = read_text(scalar)?;
                }
                (read_u32(slot)?, Ok(Operands::from_parts(overlord, parts)))
            }
            _ => return Err(invalid()),
        };
        naming.chain.push(operands);
    }
    Ok(naming)
}

/// A flag choice as its cell holds it: its three keys, then its three reasons.
pub fn choice_cell(choice: &CountryFlagChoice) -> Scalar {
    Scalar::List(
        choice
            .keys
            .iter()
            .chain(&choice.reasons)
            .map(|value| optional(value.as_deref()))
            .collect(),
    )
}

/// The flag choice [`choice_cell`] made `cell` of.
pub fn choice_of(cell: &Scalar) -> io::Result<CountryFlagChoice> {
    let values = cell.list();
    if values.len() != 6 {
        return Err(invalid());
    }
    let mut choice = CountryFlagChoice::default();
    for at in 0..3 {
        choice.keys[at] = read_optional(&values[at])?;
        choice.reasons[at] = read_optional(&values[3 + at])?;
    }
    Ok(choice)
}

/// Naming operands as one comparable value, which numbers them.
fn naming_key(naming: &facts::Naming) -> Box<[u8]> {
    let mut key = Vec::new();
    let mut put = |bytes: &[u8]| {
        key.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        key.extend_from_slice(bytes);
    };
    put(&naming.built.map_or([0, 0], |built| [1, built]));
    for (slot, operands) in &naming.chain {
        put(&slot.to_le_bytes());
        match operands {
            Err(reason) => {
                put(b"");
                put(reason.as_bytes());
            }
            Ok(operands) => {
                let (overlord, texts) = operands.parts();
                put(&overlord.map_or([0; 5], |slot| {
                    let mut bytes = [1; 5];
                    bytes[1..].copy_from_slice(&slot.to_le_bytes());
                    bytes
                }));
                for part in texts {
                    put(part.as_bytes());
                }
            }
        }
    }
    key.into_boxed_slice()
}

/// The family's emit on one branch.
pub struct Emit {
    configured: Configured,
    actors: Actors,
    carry: Carry,
    /// Each projection index's actor, for the segment under way.
    indices: Vec<u64>,
    /// By actor.
    tracks: Vec<Track>,
    /// Actors holding a value.
    held: Vec<u64>,
    last: Option<i32>,
    /// The first hour read since the last cut or gap, declared covered at the next.
    first: Option<i32>,
    namings: HashMap<Box<[u8]>, i128>,
    choices: HashMap<CountryFlagChoice, i128>,
    /// Each candidate chain's choice, chosen once.
    chosen: HashMap<Arc<Chain>, i128>,
}

impl Emit {
    pub(super) fn new(configured: &Configured) -> Self {
        Self {
            configured: configured.clone(),
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            indices: Vec::new(),
            tracks: Vec::new(),
            held: Vec::new(),
            last: None,
            first: None,
            namings: HashMap::new(),
            choices: HashMap::new(),
            chosen: HashMap::new(),
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, actor: u64, measure: u64) -> io::Result<u64> {
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension: None,
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )
    }

    /// Declares the hours read since the last cut or gap, through `last`, covered.
    fn covered(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// The number of `naming`, catalogued when new.
    fn naming(&mut self, writer: &mut Writer<'_>, naming: &facts::Naming) -> io::Result<i128> {
        let key = naming_key(naming);
        if let Some(&number) = self.namings.get(&key) {
            return Ok(number);
        }
        let number = self.namings.len() as i128;
        let cell = family::cell(writer, &naming_cell(naming))?;
        writer.catalogue(NAMINGS + number as u64, cell)?;
        engine_allocation::reserve_map(&mut self.namings, 1)?;
        self.namings.insert(key, number);
        Ok(number)
    }

    /// The number of the choice `chain` makes in the recorded mod stack, catalogued when new; a
    /// refused content names its refusal in every atlas.
    fn choice(&mut self, writer: &mut Writer<'_>, chain: &Arc<Chain>) -> io::Result<i128> {
        if let Some(&number) = self.chosen.get(chain) {
            return Ok(number);
        }
        let choice = match &self.configured.flags {
            Ok(folders) => folders.choose(chain)?,
            Err(refusal) => CountryFlagChoice {
                keys: Default::default(),
                reasons: [0; 3].map(|_| Some(refusal.clone())),
            },
        };
        let number = match self.choices.get(&choice) {
            Some(&number) => number,
            None => {
                let number = self.choices.len() as i128;
                let cell = family::cell(writer, &choice_cell(&choice))?;
                writer.catalogue(CHOICES + number as u64, cell)?;
                engine_allocation::reserve_map(&mut self.choices, 1)?;
                self.choices.insert(choice, number);
                number
            }
        };
        engine_allocation::reserve_map(&mut self.chosen, 1)?;
        self.chosen.insert(Arc::clone(chain), number);
        Ok(number)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            self.carry.close(writer, last)?;
            self.covered(writer, last)?;
            for actor in std::mem::take(&mut self.held) {
                self.tracks[actor as usize] = Track::default();
            }
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        let known = self.key(writer, FAMILY, KNOWN)?;
        self.carry
            .value(writer, known, at, hour.known.then_some(1), false)?;
        for (index, tag) in hour.named {
            let actor = self.actors.actor(writer, tag.as_bytes(), at)?;
            let index = index as usize;
            if index >= self.indices.len() {
                let more = index + 1 - self.indices.len();
                engine_allocation::reserve(&mut self.indices, more)?;
                self.indices.resize(index + 1, u64::MAX);
            }
            self.indices[index] = actor;
            if actor as usize >= self.tracks.len() {
                let more = actor as usize + 1 - self.tracks.len();
                engine_allocation::reserve(&mut self.tracks, more)?;
                self.tracks.resize(actor as usize + 1, Track::default());
            }
        }
        let mut handed = Vec::with_capacity(hour.countries.len());
        for country in &hour.countries {
            let actor = *self
                .indices
                .get(country.index as usize)
                .filter(|&&actor| actor != u64::MAX)
                .ok_or_else(|| crate::invalid("A styled country was never named"))?;
            handed.push(actor);
            self.apply(writer, at, actor, country.fact.as_ref())?;
        }
        if hour.whole {
            handed.sort_unstable();
            let silent: Vec<u64> = self
                .held
                .iter()
                .copied()
                .filter(|actor| handed.binary_search(actor).is_err())
                .collect();
            for actor in silent {
                self.apply(writer, at, actor, None)?;
            }
        }
        Ok(())
    }

    fn apply(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        actor: u64,
        fact: Option<&facts::Fact>,
    ) -> io::Result<()> {
        let next: Track = match fact {
            None => Track::default(),
            Some(fact) => [
                Some(holder(fact.holder)),
                fact.colour.map(colour),
                Some(self.naming(writer, &fact.naming)?),
                Some(self.choice(writer, &fact.flag)?),
            ],
        };
        let track = self.tracks[actor as usize];
        for (measure, (was, is)) in (HOLDER..).zip(track.into_iter().zip(next)) {
            if was != is {
                let key = self.key(writer, actor, measure)?;
                self.carry.value(writer, key, hour, is, false)?;
            }
        }
        let (was, is) = (track != Track::default(), next != Track::default());
        self.tracks[actor as usize] = next;
        if is && !was {
            engine_allocation::reserve(&mut self.held, 1)?;
            self.held.push(actor);
        } else if was && !is {
            self.held.retain(|&held| held != actor);
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.covered(writer, last)
    }

    /// What the emit goes on with after a cut: its last hour, each held tag's values and its
    /// open edges. A restart goes on at a segment's first hour, whose projection names its
    /// tags anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        for hour in [self.last, self.first] {
            match hour {
                Some(hour) => {
                    out.push(1);
                    out.extend_from_slice(&hour.to_le_bytes());
                }
                None => out.push(0),
            }
        }
        let mut held = self.held.clone();
        held.sort_unstable();
        out.extend_from_slice(&(held.len() as u64).to_le_bytes());
        for actor in held {
            out.extend_from_slice(&actor.to_le_bytes());
            for value in self.tracks[actor as usize] {
                match value {
                    Some(value) => {
                        out.push(1);
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                    None => out.push(0),
                }
            }
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(configured: &Configured, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new(configured);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.tracks = vec![Track::default(); directory.actors.len()];
        for number in 0.. {
            let Some(cell) = directory.scalar(NAMINGS + number) else {
                break;
            };
            let key = naming_key(&naming_of(cell)?);
            emit.namings.insert(key, i128::from(number));
        }
        for number in 0.. {
            let Some(cell) = directory.scalar(CHOICES + number) else {
                break;
            };
            emit.choices.insert(choice_of(cell)?, i128::from(number));
        }
        let mut d = Decode::new(bytes);
        for hour in [&mut emit.last, &mut emit.first] {
            *hour = match d.u8()? {
                0 => None,
                1 => Some(d.i32()?),
                _ => return Err(invalid()),
            };
        }
        for _ in 0..d.u64()? {
            let actor = d.u64()?;
            let mut track = Track::default();
            for value in &mut track {
                *value = match d.u8()? {
                    0 => None,
                    1 => Some(d.i128()?),
                    _ => return Err(invalid()),
                };
            }
            *emit
                .tracks
                .get_mut(usize::try_from(actor).map_err(|_| invalid())?)
                .ok_or_else(invalid)? = track;
            emit.held.push(actor);
        }
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        Ok(emit)
    }
}
