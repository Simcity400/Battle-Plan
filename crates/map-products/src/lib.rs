mod country_index;
mod dependency_index;
pub mod display;
pub mod landscape;
pub mod loader;
pub mod music_inputs;
mod visions;
pub mod witness;
mod zone_projection;

use content_reader_layers::ResolvedLayers;
use content_reader_map::map::MapData;
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::tables as row_tables;
use replay_frame::frame::DecodedFrame;
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    io,
    sync::{Arc, OnceLock},
};
use viewer_counter_products::map::orders;
use viewer_foundation::{
    perspective,
    tables::{self, Limitations},
    visibility::FogContent,
    trigger_content::TriggerContent,
};
use viewer_gpu::textures::{self, IntelMap};
use viewer_runtime::profiling::BuilderSpans;
use viewer_map_text::animated_map_text::events;
use viewer_map_icons::labels;
use viewer_models::strike_effects;
use witness::ContentReads;

pub struct ProductInputs {
    pub map: Arc<MapData>,
    pub fog: Arc<Result<FogContent, String>>,
    pub anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    pub default_anchors: Arc<BTreeMap<u32, [f32; 3]>>,
    pub bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    pub localisation: Arc<Localisation>,
    pub map_text: events::Content,
    pub audio: Option<Arc<viewer_audio::Content>>,
    pub triggers: Arc<TriggerContent>,
}

pub struct ProductContent {
    pub reads: ContentReads,
    inputs: ProductInputs,
    intel_cells: Arc<textures::IntelCellsCache>,
    visions: visions::Visions,
    weather: std::sync::Mutex<display::WeatherMemo>,
    snow: OnceLock<Result<display::SnowContent, (io::ErrorKind, String)>>,
    snow_building: std::sync::Mutex<()>,
    snow_loader: Option<loader::Loader>,
    landscape: landscape::Projections,
    display_binding: Result<[u8; 32], String>,
}

impl std::ops::Deref for ProductContent {
    type Target = ProductInputs;
    fn deref(&self) -> &ProductInputs {
        &self.inputs
    }
}

impl ProductContent {
    pub fn load(
        layers: &ResolvedLayers,
        user: &std::path::Path,
        language: &str,
    ) -> io::Result<Self> {
        loader::load(layers, user, language)
    }

    pub fn from_inputs(reads: ContentReads, inputs: ProductInputs) -> Self {
        let intel_cells = textures::IntelCellsCache::prewarm(Arc::clone(&inputs.map));
        let display_binding = reads
            .witness()
            .map(|witness| witness.digest)
            .map_err(|error| error.to_string());
        Self {
            reads,
            display_binding,
            inputs,
            intel_cells,
            visions: Default::default(),
            weather: Default::default(),
            snow: OnceLock::new(),
            snow_building: Default::default(),
            snow_loader: None,
            landscape: Default::default(),
        }
    }

    pub fn display_binding(&self) -> io::Result<[u8; 32]> {
        self.display_binding
            .as_ref()
            .copied()
            .map_err(|reason| io::Error::other(reason.clone()))
    }

    pub fn prepare_display_content(&self) -> io::Result<()> {
        self.intel_cells.get_or_init()?;
        let _ = self.snow_content();
        Ok(())
    }

    pub fn intel_map_from_visibility(
        &self,
        visibility: Option<&BTreeMap<u32, u8>>,
    ) -> io::Result<IntelMap> {
        if visibility.is_none() {
            return textures::clear_intel_map(&self.map);
        }
        self.intel_cells.get_or_init()?.try_intel_map(visibility)
    }

    pub fn impact_place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        Some((
            viewer_unit_reads::counter_anchors::stack_anchor(
                &self.anchors,
                &self.default_anchors,
                province,
                viewer_foundation::unit_rules::STANDSTILL,
            )?,
            *self.bounds.get(&province)?,
        ))
    }
}

pub use map_display_data::Header;

pub fn note_truncated_perspective(out: &mut Limitations, table: &str) {
    out.note_field(
        &format!("{table} (perspective)"),
        if table == "UnitIntel" {
            "the rows this hour did not reach may hold any unit's detection, so a foreign \
             unit without a recorded row is drawn in full, as with fog of war off, \
             including where the game hid it or drew its question mark"
        } else {
            "the rows this hour did not reach are not read as an absent value, so the \
             intel they would decide is withheld"
        },
    );
}

pub fn note_unreadable_names(out: &mut Limitations, unreadable: &[u32]) {
    if unreadable.is_empty() {
        return;
    }
    out.note_field(
        "ProvinceNames.name",
        &format!(
            "the recorded name of {} {} ({}) is not text this port can lay out, so {} \
             keeps the static localised name of its province",
            unreadable.len(),
            if unreadable.len() == 1 {
                "province"
            } else {
                "provinces"
            },
            unreadable
                .iter()
                .take(8)
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            if unreadable.len() == 1 { "it" } else { "each" },
        ),
    );
}

#[derive(Clone)]
pub struct FogProduct {
    pub intel: Arc<IntelMap>,
    pub limitations: Limitations,
    pub header: Arc<Header>,
    pub map_text: Arc<events::Prepared>,
    pub music: Option<Arc<viewer_audio::Chances>>,
    pub battles: Option<Arc<viewer_audio::Battles>>,
    music_inputs: Option<music_inputs::Kept>,
}

type ProvinceFog = viewer_foundation::perspective::values::Values<u32, u8>;

struct IntelMemo {
    choice: perspective::Choice,
    visibility: Option<Arc<ProvinceFog>>,
    intel: std::sync::Weak<IntelMap>,
}

#[derive(Default)]
pub struct FogMemos {
    intel: std::sync::Mutex<Vec<IntelMemo>>,
    pub music: music_inputs::Memo,
}

impl FogMemos {
    pub fn new() -> Self {
        Self::default()
    }

    fn intel(
        &self,
        choice: &perspective::Choice,
    ) -> Option<(Option<Arc<ProvinceFog>>, Arc<IntelMap>)> {
        let mut memos = self
            .intel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        memos.retain(|memo| memo.intel.strong_count() != 0);
        memos
            .iter()
            .find(|memo| memo.choice == *choice)
            .and_then(|memo| {
                memo.intel
                    .upgrade()
                    .map(|intel| (memo.visibility.clone(), intel))
            })
    }

    fn keep_intel(
        &self,
        choice: &perspective::Choice,
        visibility: Option<Arc<ProvinceFog>>,
        intel: &Arc<IntelMap>,
    ) {
        let mut memos = self
            .intel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        memos.retain(|memo| memo.intel.strong_count() != 0 && memo.choice != *choice);
        memos.push(IntelMemo {
            choice: choice.clone(),
            visibility,
            intel: Arc::downgrade(intel),
        });
    }
}

pub const FOG_PART_NAME: &str = "fog";
pub const FOG_PART_RANK: u8 = 6;

pub struct ProductHour<'a> {
    pub content: &'a ProductContent,
    pub frame: &'a DecodedFrame,
    pub changes: replay_frame::frame::HourChanges<'a>,
    pub hour_fraction: f32,
    vision_cache: RefCell<Option<(perspective::Choice, Arc<perspective::Vision>)>>,
    trigger_world: OnceCell<dashboard_text::triggers::World<'a>>,
}

impl<'a> ProductHour<'a> {
    pub fn new(content: &'a ProductContent, frame: &'a DecodedFrame, hour_fraction: f32) -> Self {
        Self::from_input(
            content,
            replay_frame::frame::HourInput::new(frame),
            hour_fraction,
        )
    }

    pub fn from_input(
        content: &'a ProductContent,
        input: replay_frame::frame::HourInput<'a>,
        hour_fraction: f32,
    ) -> Self {
        let frame = input.frame();
        Self {
            content,
            frame,
            changes: input.changes(),
            hour_fraction,
            vision_cache: RefCell::new(None),
            trigger_world: OnceCell::new(),
        }
    }

    pub fn seed_vision(&self, choice: &perspective::Choice, vision: Arc<perspective::Vision>) {
        *self.vision_cache.borrow_mut() = Some((choice.clone(), vision));
    }

    pub fn trigger_world(&self) -> &dashboard_text::triggers::World<'a> {
        let inputs: &'a ProductInputs = &self.content.inputs;
        self.trigger_world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, inputs.triggers.facts())
        })
    }

    pub fn weather_field(&self) -> io::Result<map_display_data::WeatherField> {
        self.content
            .weather
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .project(self.frame)
    }

    pub fn nuclear_impacts(&self) -> io::Result<strike_effects::Impacts> {
        strike_effects::Impacts::read(self.frame, |province| self.content.impact_place(province))
    }

    pub fn map_text(
        &self,
        vision: &perspective::Vision,
    ) -> viewer_map_text::animated_map_text::events::Prepared {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        events::Prepared::read(
            self.frame,
            vision,
            &self.content.map_text,
            &self.content.localisation,
            |province| self.content.impact_place(province),
        )
    }

    pub fn limitations(&self, perspective: &perspective::Choice) -> io::Result<Limitations> {
        self.limitations_with(perspective, None)
    }

    pub fn limitations_with(
        &self,
        perspective: &perspective::Choice,
        intel: Option<&IntelMap>,
    ) -> io::Result<Limitations> {
        let frame = self.frame;
        let mut out = tables::limitations(frame);
        out.note_trains(frame);
        out.note_aircraft(frame);
        out.note_point_lights();
        for refused in self.nuclear_impacts()?.refused {
            out.note_field("Nuclear impact effects", &refused);
        }
        let vision = self.vision(perspective)?;
        out.note_perspective(&tables::Perspective {
            tag: self.viewing_tag(&vision)?,
            resolution: vision.resolution().clone(),
            fog: vision.fog(),
        });
        let links: BTreeMap<u32, u32> = tables::core::<row_tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        out.note_battle_plan(orders::own_order_count(frame, &links, vision.ours())?);
        if let Ok(raids) = frame.complete::<row_tables::Raids>() {
            let ours: Vec<_> = raids
                .iter()
                .filter(|raid| raid.parent_slot == vision.ours() && raid.phase != 5)
                .collect();
            let launched = ours.iter().filter(|raid| raid.phase == 4).count();
            out.note_raid_arrows(ours.len(), launched);
        }
        if vision.fog() {
            out.note_fog_on_sight(frame);
            if let Some(table) = vision.undecided() {
                out.note_undecided_sight(table);
            }
            if let Some(input) = vision.visibility_gap() {
                out.note_visibility_gap(input);
            }
        }
        if vision.shared_province_fog().is_some() {
            let built;
            let intel = match intel {
                Some(intel) => intel,
                None => {
                    built = self.intel_map(perspective)?;
                    &built
                }
            };
            out.note_fog_of_war(intel.fogged_texels(), self.hour_fraction);
        }
        for table in vision.truncated() {
            note_truncated_perspective(&mut out, table);
        }
        note_unreadable_names(&mut out, &labels::unreadable_overrides(frame)?);
        labels::note_withheld_capitals(&mut out, frame, &links)?;
        Ok(out)
    }

    pub fn vision(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        if let Some((choice, vision)) = self.vision_cache.borrow().as_ref()
            && choice == perspective
        {
            return Ok(Arc::clone(vision));
        }
        let vision = self.content.visions.get(self, perspective)?;
        *self.vision_cache.borrow_mut() = Some((perspective.clone(), Arc::clone(&vision)));
        Ok(vision)
    }

    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        let ours = vision.ours();
        Ok(tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .find(|country| country.canonical_slot == ours)
            .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
            .unwrap_or_else(|| format!("country slot {ours}")))
    }

    pub fn intel_map(&self, perspective: &perspective::Choice) -> io::Result<IntelMap> {
        let vision = self.vision(perspective)?;
        match vision.shared_province_fog() {
            Some(visibility) => self
                .content
                .intel_cells
                .get_or_init()?
                .intel_values(visibility),
            None => textures::clear_intel_map(&self.content.map),
        }
    }

    pub fn battle_sounds(&self) -> Option<viewer_audio::Battles> {
        let _cost = replay_frame::cpu::scope("viewer", Some("music/battle/audio descriptors"));
        use replay_data::rows::tables;
        use dashboard_text::table_access::{Available, rows};
        let audio = self.content.audio.as_ref()?;
        audio.battle.as_ref()?;
        let mut refusals = Vec::new();
        let mut note = |table: &str, available: Result<bool, String>| match available {
            Ok(true) => {}
            Ok(false) => refusals.push(format!("the recorded {table} table is incomplete")),
            Err(why) => refusals.push(format!("the recorded {table} table is unavailable ({why})")),
        };
        fn state<T: replay_data::rows::TableSchema>(
            available: &std::io::Result<Available<'_, T>>,
        ) -> Result<bool, String> {
            match available {
                Ok(Available::Complete(_)) => Ok(true),
                Ok(Available::Truncated { .. }) => Ok(false),
                Ok(Available::Unavailable { reason }) => Err(reason.to_string()),
                Ok(Available::NotRecorded { mode }) => {
                    Err(format!("not recorded in a {mode} recording"))
                }
                Err(error) => Err(error.to_string()),
            }
        }
        let recorded = rows::<tables::BattleSounds>(self.frame);
        note("BattleSounds", state(&recorded));
        let combats = rows::<tables::LandCombats>(self.frame);
        note("LandCombats", state(&combats));
        let divisions = rows::<tables::Divisions>(self.frame);
        let templates = rows::<tables::DivisionTemplateIcons>(self.frame);
        let tokens = rows::<tables::LexicalTokens>(self.frame);
        let layers = !audio.layers.sounds.is_empty();
        if layers {
            note("Divisions", state(&divisions));
            note("DivisionTemplateIcons", state(&templates));
            note("LexicalTokens", state(&tokens));
        }
        let recorded = recorded.as_ref().map_or(&[][..], Available::rows);
        let combats = combats.as_ref().map_or(&[][..], Available::rows);
        let divisions: BTreeMap<u64, u64> = divisions
            .as_ref()
            .map_or(&[][..], Available::rows)
            .iter()
            .map(|division| (division.id, division.template))
            .collect();
        let templates: BTreeMap<u64, &replay_data::rows::TemplateGrids> = templates
            .as_ref()
            .map_or(&[][..], Available::rows)
            .iter()
            .filter(|template| template.model_unreadable == 0)
            .filter_map(|template| match &template.grids {
                replay_data::rows::Recorded::Value(grids) => Some((template.template, grids)),
                replay_data::rows::Recorded::NotRecorded => None,
            })
            .collect();
        let tokens = tokens.as_ref().map_or(&[][..], Available::rows);
        let name = |token: u32| -> Option<String> {
            let found = tokens.binary_search_by_key(&token, |row| row.token).ok()?;
            Some(String::from_utf8_lossy(&tokens[found].literal).into_owned())
        };
        let (battles, unknown) = recorded_battles(recorded, combats, layers, |division| {
            let template = *divisions.get(&division)?;
            if template == 0 {
                return Some(Vec::new());
            }
            let grids = templates.get(&template)?;
            viewer_audio::battle::composition(
                grids.ambience_slots().map(|subunit| (subunit.token, subunit.count)),
                &name,
            )
        });
        refusals.extend(viewer_audio::battle::unknown_refusal(unknown));
        Some(viewer_audio::Battles { battles, refusals })
    }

    pub fn music_chances_cached(
        &self,
        country: u32,
        memo: &music_inputs::Memo,
    ) -> Option<viewer_audio::Chances> {
        let _cost = replay_frame::cpu::scope("viewer", Some("music/battle/audio descriptors"));
        let audio = self.content.audio.as_ref()?;
        if audio.music.songs.is_empty() {
            return None;
        }
        let world = self.trigger_world();
        Some(memo.chances(self.frame, audio, country, world))
    }

    pub fn music_chances_kept(
        &self,
        country: u32,
        memo: &music_inputs::Memo,
    ) -> Option<(viewer_audio::Chances, music_inputs::Kept)> {
        let _cost = replay_frame::cpu::scope("viewer", Some("music/battle/audio descriptors"));
        let audio = self.content.audio.as_ref()?;
        if audio.music.songs.is_empty() {
            return None;
        }
        let world = self.trigger_world();
        Some(memo.chances_kept(self.frame, audio, country, world))
    }

    pub fn fog(
        &self,
        choice: &perspective::Choice,
        memos: &FogMemos,
        spans: &mut BuilderSpans,
    ) -> io::Result<Arc<FogProduct>> {
        self.fog_with(choice, memos, spans, true)
    }

    pub fn fog_with(
        &self,
        choice: &perspective::Choice,
        memos: &FogMemos,
        spans: &mut BuilderSpans,
        intel: bool,
    ) -> io::Result<Arc<FogProduct>> {
        let vision = spans.measure("fog.vision", |_| self.vision(choice))?;
        let visibility = vision.shared_province_fog();
        let built = intel;
        let intel = spans.measure("fog.intel_patch", |_| -> io::Result<_> {
            if !built {
                return Ok(Arc::new(textures::clear_intel_map(&self.content.map)?));
            }
            let previous = memos.intel(choice);
            if let Some((old, intel)) = &previous {
                match (old, visibility) {
                    (None, None) => return Ok(Arc::clone(intel)),
                    (Some(old), Some(next)) if Arc::ptr_eq(old, next) => {
                        return Ok(Arc::clone(intel));
                    }
                    _ => {}
                }
            }
            let next = match (&previous, visibility) {
                (Some((Some(old), intel)), Some(next)) => self
                    .content
                    .intel_cells
                    .get_or_init()?
                    .patch_values(intel, old, next)?,
                _ => match visibility {
                    Some(visibility) => self
                        .content
                        .intel_cells
                        .get_or_init()?
                        .intel_values(visibility)?,
                    None => textures::clear_intel_map(&self.content.map)?,
                },
            };
            let intel = if let Some((_, previous)) = &previous {
                if previous.fog == next.fog
                    && previous.fogged_texels == next.fogged_texels
                    && previous.texels.ptr_eq(&next.texels)
                {
                    Arc::clone(previous)
                } else {
                    Arc::new(next)
                }
            } else {
                Arc::new(next)
            };
            memos.keep_intel(choice, visibility.cloned(), &intel);
            Ok(intel)
        })?;
        let limitations = spans.measure("fog.limitations", |_| {
            self.limitations_with(choice, Some(&intel))
        })?;
        let (music, kept) = spans
            .measure("fog.music", |_| {
                self.music_chances_kept(vision.ours(), &memos.music)
            })
            .unzip();
        let battles = spans.measure("fog.battles", |_| self.battle_sounds());
        let product = map_display_data::Fog {
            intel,
            limitations: limitations.into_lines(),
            header: spans.measure("fog.header", |_| Header {
                viewing: self
                    .vision(choice)
                    .and_then(|vision| self.viewing_tag(&vision))
                    .ok(),
            }),
            map_text: spans.measure("fog.map_text", |_| self.map_text(&vision).into_display()),
            music,
            battles,
        };
        FogProduct::from_display(product).map(|mut fog| {
            fog.music_inputs = kept;
            Arc::new(fog)
        })
    }
}

fn recorded_battles(
    recorded: &[replay_data::rows::BattleSoundRow],
    combats: &[replay_data::rows::LandCombatRow],
    layers: bool,
    composition: impl Fn(u64) -> Option<Vec<(String, usize)>>,
) -> (Vec<viewer_audio::battle::Battle>, usize) {
    use viewer_audio::battle::{self, Front};
    let mut fronts: BTreeMap<u32, Front> = BTreeMap::new();
    for combat in combats {
        let members = battle::divisions(
            &combat.attackers,
            &combat.attacker_classes,
            &combat.defenders,
            &combat.defender_classes,
        );
        fronts.entry(combat.province).or_default().add(
            combat.attackers.len() + combat.defenders.len(),
            members,
            layers,
            &composition,
        );
    }
    let mut unknown = 0;
    let battles = recorded
        .iter()
        .map(|row| {
            battle::battle(
                row.object,
                row.provinces.clone(),
                [f32::from_bits(row.center_x), f32::from_bits(row.center_z)],
                f32::from_bits(row.radius),
                |province| fronts.get(&province),
                String::clone,
                &mut unknown,
            )
        })
        .collect();
    (battles, unknown)
}
const MUSIC_CHANCE_ONE: f64 = 100_000.0;

const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<ProductContent>();
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod loader_tests;

#[cfg(test)]
mod display_oracle;

#[cfg(test)]
mod fog_oracle;
