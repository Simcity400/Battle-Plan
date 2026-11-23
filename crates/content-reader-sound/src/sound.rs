
use crate::{ResolvedLayers, crt, defines, parser};
use content_reader_layers::script_files::ScriptPath;
use std::{collections::BTreeMap, io};

pub(crate) fn registered(name: &str) -> &[u8] {
    &name.as_bytes()[..name.len().min(63)]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub name: String,
    pub path: String,
    pub volume: f32,
    pub priority: f32,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub sound: usize,
    pub weight: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Fail,
    Silent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Logarithmic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Falloff {
    pub name: String,
    pub min_distance: f32,
    pub max_distance: f32,
    pub height_scale: f32,
    pub curve: Curve,
}

impl Default for Falloff {
    fn default() -> Self {
        Self {
            name: String::new(),
            min_distance: 0.0,
            max_distance: 100.0,
            height_scale: 1.0,
            curve: Curve::Logarithmic,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Range {
    pub first: f32,
    pub second: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub name: String,
    pub choices: Vec<Choice>,
    pub volume: f32,
    pub falloff: Option<usize>,
    pub looping: bool,
    pub positional: bool,
    pub max_audible: i32,
    pub overflow: Overflow,
    pub fade_in: f32,
    pub fade_out: f32,
    pub volume_offset: Range,
    pub delay_offset: Range,
    pub playback_rate: f32,
    pub playback_rate_offset: Range,
    pub looping_volume_offset: bool,
    pub looping_delay_offset: bool,
    pub looping_playback_rate_offset: bool,
    pub random_sound_when_looping: bool,
    pub prevent_random_repetition: bool,
    pub polyphony: i32,
    pub category: Option<usize>,
    pub source: String,
}

impl Effect {
    pub fn new(name: String, source: String) -> Self {
        Self {
            name,
            choices: Vec::new(),
            volume: 1.0,
            falloff: None,
            looping: false,
            positional: false,
            max_audible: -1,
            overflow: Overflow::Silent,
            fade_in: 0.0,
            fade_out: 0.0,
            volume_offset: Range::default(),
            delay_offset: Range::default(),
            playback_rate: 1.0,
            playback_rate_offset: Range::default(),
            looping_volume_offset: false,
            looping_delay_offset: false,
            looping_playback_rate_offset: false,
            random_sound_when_looping: true,
            prevent_random_repetition: false,
            polyphony: 1,
            category: None,
            source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compressor {
    pub enabled: bool,
    pub pregain: f32,
    pub postgain: f32,
    pub ratio: f32,
    pub threshold: f32,
    pub attack: f32,
    pub release: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            enabled: true,
            pregain: 1.0,
            postgain: 1.0,
            ratio: 1.0,
            threshold: 0.0,
            attack: 0.0,
            release: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub name: String,
    pub compressor: Option<Compressor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Defines {
    pub height_category: String,
    pub height_min_altitude: f32,
    pub height_max_altitude: f32,
    pub height_min_volume: f32,
    pub height_max_volume: f32,
    pub voice_category: String,
    pub voice_cool_down: f32,
}

impl Defines {
    pub fn load(defines: &defines::Defines) -> io::Result<Self> {
        let number = |key: &str| -> io::Result<f32> {
            let value = defines.number("NSound", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.NSound.{key} is outside f32")))
            }
        };
        Ok(Self {
            height_category: defines.text("NSound", "HEIGHT_SOUND_CATEGORY")?,
            height_min_altitude: number("HEIGHT_SOUND_MIN_ALTITUDE")?,
            height_max_altitude: number("HEIGHT_SOUND_MAX_ALTITUDE")?,
            height_min_volume: number("HEIGHT_SOUND_MIN_VOLUME")?,
            height_max_volume: number("HEIGHT_SOUND_MAX_VOLUME")?,
            voice_category: defines.text("NSound", "VOICE_OVER_CATEGORY")?,
            voice_cool_down: number("VOICE_OVER_COOL_DOWN")?,
        })
    }

    pub fn height_volume(&self, altitude: f32) -> f32 {
        let clamped = if altitude < self.height_min_altitude {
            self.height_min_altitude
        } else if altitude > self.height_max_altitude {
            self.height_max_altitude
        } else {
            altitude
        };
        (clamped - self.height_min_altitude) / (self.height_max_altitude - self.height_min_altitude)
            * (self.height_max_volume - self.height_min_volume)
            + self.height_min_volume
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes(Vec<(String, usize)>);

impl Notes {
    pub fn note(&mut self, message: impl Into<String>) {
        let message = message.into();
        match self.0.iter_mut().find(|(known, _)| *known == message) {
            Some((_, count)) => *count += 1,
            None => self.0.push((message, 1)),
        }
    }

    pub fn entries(&self) -> &[(String, usize)] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|(message, count)| match count {
                1 => message.clone(),
                _ => format!("{message} (x{count})"),
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub sounds: Vec<Sound>,
    pub effects: Vec<Effect>,
    pub falloffs: Vec<Falloff>,
    pub categories: Vec<Category>,
    pub master_compressor: Option<Compressor>,
    pub music_compressor: Option<Compressor>,
    pub files: Vec<ScriptPath>,
    pub notes: Notes,
    sound_names: BTreeMap<Vec<u8>, usize>,
    effect_names: BTreeMap<String, usize>,
    falloff_names: BTreeMap<String, usize>,
    category_names: BTreeMap<String, usize>,
}

struct Pending {
    effect: usize,
    sounds: Vec<(String, u32)>,
    falloff: Option<String>,
}

type Assignment = (usize, Vec<String>, String);

impl Catalog {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let listed = layers.descriptor_paths("sound", &[".asset"])?;
        let paths = defines::load_order("sound/", ".asset", &listed);
        let mut catalog = Self::default();
        let mut pending = Vec::new();
        let mut assignments = Vec::new();
        for file in paths {
            let bytes = file.read_uncovered(layers)?;
            let path = file.text();
            let document = match parser::parse(&bytes) {
                Ok(document) => document,
                Err(error) => {
                    catalog
                        .notes
                        .note(format!("{path}: unreadable sound asset ({error})"));
                    continue;
                }
            };
            let directory = file.folder_text();
            catalog.files.push(file.clone());
            for item in &document.items {
                let Some(fields) = item.value.items() else {
                    catalog.notes.note(format!(
                        "{path}: refused top-level value {}",
                        key_text(item)
                    ));
                    continue;
                };
                if item.keyword_is("sound") {
                    catalog.declare_sound(fields, &path, directory);
                } else if item.keyword_is("soundeffect") {
                    if let Some(entry) = catalog.declare_effect(fields, &path) {
                        pending.push(entry);
                    }
                } else if item.keyword_is("falloff") {
                    catalog.declare_falloff(fields, &path);
                } else if item.keyword_is("category") {
                    if let Some(entry) = catalog.declare_category(fields, &path) {
                        assignments.push(entry);
                    }
                } else if item.keyword_is("master_compressor") {
                    catalog.master_compressor = Some(compressor(fields, &path, &mut catalog.notes));
                } else if item.keyword_is("music_compressor") {
                    catalog.music_compressor = Some(compressor(fields, &path, &mut catalog.notes));
                } else {
                    catalog.notes.note(format!(
                        "{path}: refused top-level block {}",
                        key_text(item)
                    ));
                }
            }
        }
        catalog.resolve(pending, assignments);
        Ok(catalog)
    }

    pub fn sound(&self, name: &str) -> Option<usize> {
        self.sound_names.get(name.as_bytes()).copied()
    }

    pub fn effect(&self, name: &str) -> Option<usize> {
        self.effect_names.get(name).copied()
    }

    pub fn falloff(&self, name: &str) -> Option<usize> {
        self.falloff_names.get(name).copied()
    }

    pub fn category(&self, name: &str) -> Option<usize> {
        self.category_names.get(name).copied()
    }

    fn declare_sound(&mut self, fields: &[parser::Item], path: &str, directory: Option<&str>) {
        let Some(name) = text(fields, "name") else {
            self.notes
                .note(format!("{path}: refused sound without a name"));
            return;
        };
        let Some(file) = text(fields, "file") else {
            self.notes
                .note(format!("{path}: refused sound {name} without a file"));
            return;
        };
        if self.sound_names.contains_key(registered(&name)) {
            self.notes
                .note(format!("{path}: refused repeated sound {name}"));
            return;
        }
        let Some(directory) = directory else {
            self.notes.note(format!(
                "{path}: refused sound {name}: the file's folder name isn't UTF-8, so its sample path can't be named"
            ));
            return;
        };
        let mut sound = Sound {
            path: join(directory, &file),
            name: name.clone(),
            volume: 1.0,
            priority: 1.0,
            source: path.to_owned(),
        };
        for field in fields {
            match key(field).as_deref() {
                Some("name" | "file" | "always_load") => {}
                Some("volume") => number_into(field, &mut sound.volume, path, &mut self.notes),
                Some("priority") => number_into(field, &mut sound.priority, path, &mut self.notes),
                _ => self.notes.note(format!(
                    "{path}: sound {name} refused field {}",
                    key_text(field)
                )),
            }
        }
        self.sound_names
            .insert(registered(&name).to_vec(), self.sounds.len());
        self.sounds.push(sound);
    }

    fn declare_effect(&mut self, fields: &[parser::Item], path: &str) -> Option<Pending> {
        let Some(name) = text(fields, "name") else {
            self.notes
                .note(format!("{path}: refused soundeffect without a name"));
            return None;
        };
        if self.effect_names.contains_key(&name) {
            self.notes
                .note(format!("{path}: refused repeated soundeffect {name}"));
            return None;
        }
        let mut effect = Effect::new(name.clone(), path.to_owned());
        let mut sounds = Vec::new();
        let mut falloff = None;
        let notes = &mut self.notes;
        for field in fields {
            match key(field).as_deref() {
                Some("name") => {}
                Some("sounds") => read_sounds(field, &name, path, &mut sounds, notes),
                Some("volume") => number_into(field, &mut effect.volume, path, notes),
                Some("falloff") => falloff = field.value.scalar_str().map(str::to_owned),
                Some("loop") => bool_into(field, &mut effect.looping, path, notes),
                Some("is3d") => bool_into(field, &mut effect.positional, path, notes),
                Some("max_audible") => {
                    let mut value = effect.max_audible as f32;
                    number_into(field, &mut value, path, notes);
                    effect.max_audible = value as i32;
                }
                Some("max_audible_behaviour") => match field.value.scalar_str() {
                    Some("fail") => effect.overflow = Overflow::Fail,
                    Some("silent") => effect.overflow = Overflow::Silent,
                    other => notes.note(format!(
                        "{path}: soundeffect {name} refused max_audible_behaviour {}",
                        other.unwrap_or("block")
                    )),
                },
                Some("fade_in") => number_into(field, &mut effect.fade_in, path, notes),
                Some("fade_out") => number_into(field, &mut effect.fade_out, path, notes),
                Some("volume_random_offset") => {
                    range_into(field, &mut effect.volume_offset, 1.0, path, notes)
                }
                Some("delay_random_offset") => {
                    range_into(field, &mut effect.delay_offset, 1.0, path, notes)
                }
                Some("playbackrate") => {
                    let mut percent = 100.0;
                    number_into(field, &mut percent, path, notes);
                    effect.playback_rate = percent / 100.0;
                }
                Some("playbackrate_random_offset") => {
                    range_into(field, &mut effect.playback_rate_offset, 100.0, path, notes)
                }
                Some("looping_volume_random_offset") => {
                    bool_into(field, &mut effect.looping_volume_offset, path, notes)
                }
                Some("looping_delay_random_offset") => {
                    bool_into(field, &mut effect.looping_delay_offset, path, notes)
                }
                Some("looping_playbackrate_random_offset") => {
                    bool_into(field, &mut effect.looping_playback_rate_offset, path, notes)
                }
                Some("random_sound_when_looping") => {
                    bool_into(field, &mut effect.random_sound_when_looping, path, notes)
                }
                Some("prevent_random_repetition") => {
                    bool_into(field, &mut effect.prevent_random_repetition, path, notes)
                }
                Some("polyphony") => {
                    let mut value = effect.polyphony as f32;
                    number_into(field, &mut value, path, notes);
                    effect.polyphony = value as i32;
                }
                _ => notes.note(format!(
                    "{path}: soundeffect {name} refused field {}",
                    key_text(field)
                )),
            }
        }
        let index = self.effects.len();
        self.effect_names.insert(name, index);
        self.effects.push(effect);
        Some(Pending {
            effect: index,
            sounds,
            falloff,
        })
    }

    fn declare_falloff(&mut self, fields: &[parser::Item], path: &str) {
        let Some(name) = text(fields, "name") else {
            self.notes
                .note(format!("{path}: refused falloff without a name"));
            return;
        };
        if self.falloff_names.contains_key(&name) {
            self.notes
                .note(format!("{path}: refused repeated falloff {name}"));
            return;
        }
        let mut falloff = Falloff {
            name: name.clone(),
            ..Falloff::default()
        };
        let notes = &mut self.notes;
        for field in fields {
            match key(field).as_deref() {
                Some("name") => {}
                Some("min_distance") => number_into(field, &mut falloff.min_distance, path, notes),
                Some("max_distance") => number_into(field, &mut falloff.max_distance, path, notes),
                Some("height_scale") => number_into(field, &mut falloff.height_scale, path, notes),
                Some("type") => match field.value.scalar_str() {
                    Some("linear") => falloff.curve = Curve::Linear,
                    Some("logarithmic") => falloff.curve = Curve::Logarithmic,
                    other => notes.note(format!(
                        "{path}: falloff {name} refused type {}",
                        other.unwrap_or("block")
                    )),
                },
                _ => notes.note(format!(
                    "{path}: falloff {name} refused field {}",
                    key_text(field)
                )),
            }
        }
        self.falloff_names.insert(name, self.falloffs.len());
        self.falloffs.push(falloff);
    }

    fn declare_category(&mut self, fields: &[parser::Item], path: &str) -> Option<Assignment> {
        let Some(name) = text(fields, "name") else {
            self.notes
                .note(format!("{path}: refused category without a name"));
            return None;
        };
        let mut members = Vec::new();
        let mut compressor_block = None;
        for field in fields {
            match key(field).as_deref() {
                Some("name") => {}
                Some("soundeffects") => match field.value.items() {
                    Some(items) => members.extend(
                        items
                            .iter()
                            .filter(|item| item.key.is_none())
                            .filter_map(|item| item.value.scalar_str().map(str::to_owned)),
                    ),
                    None => self.notes.note(format!(
                        "{path}: category {name} refused scalar soundeffects"
                    )),
                },
                Some("compressor") => match field.value.items() {
                    Some(items) => {
                        compressor_block = Some(compressor(items, path, &mut self.notes))
                    }
                    None => self
                        .notes
                        .note(format!("{path}: category {name} refused scalar compressor")),
                },
                _ => self.notes.note(format!(
                    "{path}: category {name} refused field {}",
                    key_text(field)
                )),
            }
        }
        let index = match self.category_names.get(&name) {
            Some(&index) => {
                if compressor_block.is_some() {
                    self.categories[index].compressor = compressor_block;
                }
                index
            }
            None => {
                let index = self.categories.len();
                self.category_names.insert(name.clone(), index);
                self.categories.push(Category {
                    name: name.clone(),
                    compressor: compressor_block,
                });
                index
            }
        };
        Some((index, members, path.to_owned()))
    }

    fn resolve(&mut self, pending: Vec<Pending>, assignments: Vec<Assignment>) {
        for entry in pending {
            let effect_name = self.effects[entry.effect].name.clone();
            let source = self.effects[entry.effect].source.clone();
            for (sound, weight) in entry.sounds {
                match self.sound_names.get(sound.as_bytes()) {
                    Some(&index) => self.effects[entry.effect].choices.push(Choice {
                        sound: index,
                        weight,
                    }),
                    None => self.notes.note(format!(
                        "{source}: soundeffect {effect_name} names missing sound {sound}"
                    )),
                }
            }
            if let Some(falloff) = entry.falloff {
                match self.falloff_names.get(&falloff) {
                    Some(&index) => self.effects[entry.effect].falloff = Some(index),
                    None => self.notes.note(format!(
                        "{source}: soundeffect {effect_name} names missing falloff {falloff}; it plays without attenuation"
                    )),
                }
            }
        }
        let mut lists: Vec<Vec<(&str, &str)>> = vec![Vec::new(); self.categories.len()];
        for (category, members, source) in &assignments {
            lists[*category].extend(
                members
                    .iter()
                    .map(|member| (member.as_str(), source.as_str())),
            );
        }
        for (category, members) in lists.into_iter().enumerate() {
            for (member, source) in members {
                match self.effect_names.get(member) {
                    Some(&effect) => {
                        if let Some(previous) = self.effects[effect].category
                            && previous != category
                        {
                            self.notes.note(format!(
                                "{source}: soundeffect {member} moves from category {} to {}",
                                self.categories[previous].name, self.categories[category].name
                            ));
                        }
                        self.effects[effect].category = Some(category);
                    }
                    None => self.notes.note(format!(
                        "{source}: category {} names missing soundeffect {member}",
                        self.categories[category].name
                    )),
                }
            }
        }
    }
}

fn read_sounds(
    field: &parser::Item,
    effect: &str,
    path: &str,
    out: &mut Vec<(String, u32)>,
    notes: &mut Notes,
) {
    let Some(items) = field.value.items() else {
        notes.note(format!(
            "{path}: soundeffect {effect} refused scalar sounds"
        ));
        return;
    };
    for item in items {
        if item.keyword_is("sound") {
            match item.value.scalar_str() {
                Some(name) => out.push((name.to_owned(), 1)),
                None => notes.note(format!(
                    "{path}: soundeffect {effect} refused a sound block"
                )),
            }
        } else if item.keyword_is("weighted_sound") {
            let Some(fields) = item.value.items() else {
                notes.note(format!(
                    "{path}: soundeffect {effect} refused a scalar weighted_sound"
                ));
                continue;
            };
            for field in fields {
                if !(field.keyword_is("sound") || field.keyword_is("weight")) {
                    notes.note(format!(
                        "{path}: soundeffect {effect} weighted_sound refused field {}",
                        key_text(field)
                    ));
                }
            }
            let Some(name) = text(fields, "sound") else {
                notes.note(format!(
                    "{path}: soundeffect {effect} refused weighted_sound without a sound"
                ));
                continue;
            };
            let mut weight = 1.0f32;
            if let Some(value) = fields.iter().rev().find(|f| f.keyword_is("weight")) {
                number_into(value, &mut weight, path, notes);
            }
            out.push((name, weight.max(0.0) as u32));
        } else {
            notes.note(format!(
                "{path}: soundeffect {effect} refused sounds entry {}",
                key_text(item)
            ));
        }
    }
}

fn compressor(fields: &[parser::Item], path: &str, notes: &mut Notes) -> Compressor {
    let mut out = Compressor::default();
    for field in fields {
        match key(field).as_deref() {
            Some("enabled") => bool_into(field, &mut out.enabled, path, notes),
            Some("pregain") => out.pregain = decibels(field, path, notes),
            Some("postgain") => out.postgain = decibels(field, path, notes),
            Some("ratio") => {
                number_into(field, &mut out.ratio, path, notes);
                if out.ratio == 0.0 {
                    out.ratio = 1.0;
                }
            }
            Some("threshold") => number_into(field, &mut out.threshold, path, notes),
            Some("attacktime") => number_into(field, &mut out.attack, path, notes),
            Some("releasetime") => number_into(field, &mut out.release, path, notes),
            _ => notes.note(format!(
                "{path}: compressor refused field {}",
                key_text(field)
            )),
        }
    }
    out
}

fn decibels(field: &parser::Item, path: &str, notes: &mut Notes) -> f32 {
    let mut value = 0.0;
    number_into(field, &mut value, path, notes);
    10f32.powf(value / 20.0)
}

fn number_into(field: &parser::Item, out: &mut f32, path: &str, notes: &mut Notes) {
    match field.value.scalar().and_then(crt::scan_f32) {
        Some(value) if value.is_finite() => *out = value,
        _ => notes.note(format!("{path}: refused non-numeric {}", key_text(field))),
    }
}

fn bool_into(field: &parser::Item, out: &mut bool, path: &str, notes: &mut Notes) {
    match field.value.scalar().and_then(parser::read_bool) {
        Some(value) => *out = value,
        None => notes.note(format!("{path}: refused non-boolean {}", key_text(field))),
    }
}

fn range_into(field: &parser::Item, out: &mut Range, scale: f32, path: &str, notes: &mut Notes) {
    let values: Option<Vec<f32>> = field.value.items().and_then(|items| {
        items
            .iter()
            .map(|item| {
                item.key
                    .is_none()
                    .then(|| item.value.scalar().and_then(crt::scan_f32))
                    .flatten()
                    .filter(|value| value.is_finite())
            })
            .collect()
    });
    match values.as_deref() {
        Some(&[first, second]) => {
            out.first = first / scale;
            out.second = second / scale;
        }
        _ => notes.note(format!("{path}: refused malformed {}", key_text(field))),
    }
}

fn key(field: &parser::Item) -> Option<String> {
    field
        .keyword()
        .map(|keyword| String::from_utf8_lossy(&keyword.folded()).into_owned())
}

fn text(fields: &[parser::Item], key: &str) -> Option<String> {
    fields
        .iter()
        .rev()
        .find(|field| field.keyword_is(key))
        .and_then(|field| field.value.scalar_str())
        .map(str::to_owned)
}

fn key_text(item: &parser::Item) -> String {
    item.key
        .as_deref()
        .map(|key| String::from_utf8_lossy(key).into_owned())
        .unwrap_or_else(|| "(anonymous)".into())
}

fn join(directory: &str, file: &str) -> String {
    let file = file.replace('\\', "/");
    if directory.is_empty() {
        file
    } else {
        format!("{directory}/{file}")
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
