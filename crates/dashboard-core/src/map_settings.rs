use crate::protocol::{
    CampaignMapSettings, MapAudio, MapGraphics, MapGraphicsOverrides, ResolvedMapSettings,
};
use serde::Serialize;
use std::{collections::BTreeMap, fs, io, path::PathBuf, sync::atomic::AtomicBool};
use ts_rs::TS;

static SAMPLE_COUNTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn set_sample_counts(counts: impl IntoIterator<Item = u32>) {
    let mask = counts
        .into_iter()
        .filter(|count| matches!(count, 1 | 2 | 4 | 8))
        .fold(0, |mask, count| mask | count);
    SAMPLE_COUNTS.store(mask, std::sync::atomic::Ordering::Relaxed);
}

pub fn sample_counts() -> Option<Vec<u32>> {
    let mask = SAMPLE_COUNTS.load(std::sync::atomic::Ordering::Relaxed);
    (mask != 0).then(|| {
        [1, 2, 4, 8]
            .into_iter()
            .filter(|count| mask & count != 0)
            .collect()
    })
}

pub fn campaign_key(replay: &replay::campaign::Replay) -> String {
    try_campaign_key(replay).expect("campaign key allocation failed")
}

pub fn try_campaign_key(replay: &replay::campaign::Replay) -> io::Result<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let length = engine_allocation::mul(replay.campaign_id.len(), 2)?;
    let mut key = engine_allocation::vec(length)?;
    for byte in &replay.campaign_id {
        key.push(HEX[(byte >> 4) as usize]);
        key.push(HEX[(byte & 15) as usize]);
    }
    Ok(String::from_utf8(key).expect("hexadecimal is ASCII"))
}

#[derive(Default)]
pub struct Store {
    entries: BTreeMap<String, CampaignMapSettings>,
    error: Option<String>,
    game: Game,
    game_error: Option<String>,
    generation: u64,
}

pub fn widget_key(campaign: &str, id: &str) -> String {
    format!("{campaign}/widget/{id}")
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Game {
    pub graphics: MapGraphics,
    pub audio: MapAudio,
}

fn path() -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| crate::invalid("LOCALAPPDATA is not set"))?,
    )
    .join("BattlePlan")
    .join("map-settings.json"))
}

impl Store {
    pub fn open() -> Self {
        Self::load().unwrap_or_else(|error| Self {
            entries: BTreeMap::new(),
            error: Some(error.to_string()),
            ..Self::default()
        })
    }
    pub fn check(&self) -> Result<(), String> {
        self.error.clone().map_or(Ok(()), Err)
    }
    fn load() -> io::Result<Self> {
        Self::load_from(&path()?)
    }
    pub fn reload(&mut self) {
        self.reload_from(path());
    }
    fn reload_from(&mut self, path: io::Result<PathBuf>) {
        self.generation += 1;
        match path.and_then(|path| Self::load_from(&path)) {
            Ok(loaded) => {
                self.entries = loaded.entries;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn load_from(path: &std::path::Path) -> io::Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(crate::invalid("Map settings exceed their size limit"));
        }
        let entries: BTreeMap<String, CampaignMapSettings> = serde_json::from_slice(&bytes)?;
        for entry in entries.values() {
            entry.validate().map_err(crate::invalid)?;
        }
        Ok(Self {
            entries,
            error: None,
            ..Self::default()
        })
    }
    pub fn configure_game(&mut self, user: &std::path::Path) {
        self.set_game(read_game(user));
    }
    pub fn set_game(&mut self, game: io::Result<Game>) {
        self.generation += 1;
        match game {
            Ok(game) => {
                self.game = game;
                self.game_error = None;
            }
            Err(error) => {
                self.game_error = Some(error.to_string());
            }
        }
    }
    pub fn game(&self) -> Option<Game> {
        self.game_error.is_none().then_some(self.game)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn game_error(&self) -> Option<String> {
        self.game_error.clone()
    }
    pub fn inherited(&self, key: &str) -> ResolvedMapSettings {
        if key == "$global" {
            ResolvedMapSettings {
                graphics: self.game.graphics,
                audio: self.game.audio,
                ..Default::default()
            }
        } else if let Some((campaign, _)) = key.split_once("/widget/") {
            self.get(campaign)
        } else {
            self.get("$global")
        }
    }
    pub fn preferences(&self, key: &str) -> CampaignMapSettings {
        self.entries.get(key).cloned().unwrap_or_default()
    }
    pub fn get(&self, key: &str) -> ResolvedMapSettings {
        let mut settings = self.inherited(key).child(&self.preferences(key));
        let mut mods = settings.visual_mods.take().unwrap_or_default();
        mods.sort_by(|a, b| a.id.cmp(&b.id));
        mods.dedup_by(|a, b| a.id == b.id);
        settings.visual_mods = Some(mods);
        settings
    }
    pub fn flipped(
        &self,
        key: &str,
        setting: crate::protocol::MapSetting,
    ) -> Result<CampaignMapSettings, &'static str> {
        use crate::protocol::MapSetting;
        let current = self.get(key).graphics;
        let mut settings = self.preferences(key);
        match setting {
            MapSetting::DayNight => settings.graphics.day_night = Some(!current.day_night),
            MapSetting::AlliedPlans => {
                settings.graphics.show_allied_plans = Some(!current.show_allied_plans);
            }
            MapSetting::Fog => return Err("Fog is the view's own switch, not a map setting"),
        }
        Ok(settings)
    }

    pub fn flip(&mut self, key: String, setting: crate::protocol::MapSetting) -> io::Result<Option<String>> {
        self.update(key, |saved, key| {
            saved.flipped(key, setting).map_err(crate::invalid)
        })
    }

    pub fn save(&mut self, key: String, settings: CampaignMapSettings) -> io::Result<Option<String>> {
        self.update(key, |_, _| Ok(settings))
    }

    fn update(
        &mut self,
        key: String,
        choose: impl FnOnce(&Self, &str) -> io::Result<CampaignMapSettings>,
    ) -> io::Result<Option<String>> {
        self.check().map_err(crate::invalid)?;
        self.update_at(&replay_settings::exit::EXIT, &path()?, key, choose)
    }

    fn update_at(
        &mut self,
        exit: &replay_settings::exit::Exit,
        path: &std::path::Path,
        key: String,
        choose: impl FnOnce(&Self, &str) -> io::Result<CampaignMapSettings>,
    ) -> io::Result<Option<String>> {
        let applied = exit.under_lock(&path.with_extension("lock"), replay_settings::exit::What::Change, |receipt| {
            let saved = Self {
                entries: Self::load_from(path)?.entries,
                game: self.game,
                ..Self::default()
            };
            let settings = choose(&saved, &key)?;
            settings.validate().map_err(crate::invalid)?;
            let mut entries = saved.entries;
            entries.insert(key, settings);
            let bytes = serde_json::to_vec(&entries)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(crate::invalid("Map settings exceed their size limit"));
            }
            Ok(receipt.replace(path, |out| io::Write::write_all(out, &bytes))?.with(entries, ()))
        })?;
        self.entries = applied.value;
        self.generation += 1;
        Ok(applied.warning)
    }
}

#[derive(Clone, Debug, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordedMod {
    pub recorded_id: String,
    pub name: String,
}

#[derive(Serialize, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MapSettingsReply")]
pub struct Reply {
    pub source_id: String,
    pub revision: String,
    #[doc = " The choices this scope stores; everything else inherits."]
    pub settings: CampaignMapSettings,
    #[doc = " What this scope's unset choices take."]
    pub inherited_settings: ResolvedMapSettings,
    #[doc = " Every installed mod; any can be shown. `problem` says why one cannot be mounted."]
    #[ts(
        type = "{ id: string, name: string, recordedId: string, source: string, problem: string | null }[]"
    )]
    pub mods: Vec<content_reader_layers::content::visual::InstalledMod>,
    #[doc = " The mods the recording was made with, which every scope shows beneath visual mods."]
    pub recorded_mods: Vec<RecordedMod>,
    #[doc = " The resolved selection's mods in the engine's mount order, lowest precedence first."]
    #[ts(type = "{ id: string, name: string }[]")]
    pub applied_mods: Vec<content_reader_layers::content::visual::Choice>,
    #[doc = " Selected mods that are not shown, and why."]
    pub applied_notes: Vec<String>,
    #[doc = " The map renderer's anti-aliasing sample counts; `None` until a renderer has said, when"]
    #[doc = " every count is a choice and the renderer refuses one its adapter lacks."]
    pub sample_counts: Option<Vec<u32>>,
    #[doc = r#" The game's own graphics choices, which a scope's "use the game's settings" reverts to."#]
    pub game_graphics: Option<MapGraphics>,
    #[doc = " Why the game's settings (graphics and volumes) could not be read."]
    pub game_settings_error: Option<String>,
    #[doc = " Why this save, which BattlePlan applied, may not have reached the disk."]
    pub save_warning: Option<String>,
    #[doc = " The service's presentation generation once this job checked every open source's visual"]
    #[doc = " mods: a later one than a client has seen means widget artwork changed."]
    #[ts(type = "number")]
    pub presentation_generation: u64,
}

pub fn catalogue(
    options: &content_reader_layers::ResolveOptions,
    cancel: &AtomicBool,
) -> io::Result<Vec<content_reader_layers::content::visual::InstalledMod>> {
    content_reader_layers::content::visual::installed(options, cancel)
}

pub fn game_audio(keys: &content_reader_settings::settings::SettingsKeys) -> MapAudio {
    let volume = |key: &str, fallback: f32| {
        keys.f32_at(&[key])
            .filter(|value| value.is_finite())
            .map_or(fallback, |value| value.clamp(0.0, 100.0))
    };
    let defaults = MapAudio::default();
    MapAudio {
        master: volume("master_volume", defaults.master),
        ambient: volume("ambient_volume", defaults.ambient),
        units: volume("sound_fx_volume", defaults.units),
        combat: volume("sound_fx_volume", defaults.combat),
        interface: volume("sound_fx_volume", defaults.interface),
        voice: volume("voice_volume", defaults.voice),
        music: volume("music_volume", defaults.music),
    }
}

pub fn game_dev_master(keys: &content_reader_settings::settings::SettingsKeys) -> f32 {
    keys.f32_at(&["dev_master_volume"])
        .filter(|value| value.is_finite())
        .map_or(0.5, |value| value.clamp(0.0, 100.0) / 100.0)
}

pub fn read_game(user: &std::path::Path) -> io::Result<Game> {
    parse_game_keys(&content_reader_settings::settings::read_keys(user)?)
}
#[cfg(test)]
fn parse_game(bytes: &[u8]) -> io::Result<Game> {
    parse_game_keys(&content_reader_settings::settings::SettingsKeys::read(bytes))
}
#[cfg(test)]
fn parse_game_graphics(bytes: &[u8]) -> io::Result<MapGraphics> {
    parse_game(bytes).map(|game| game.graphics)
}
fn parse_game_keys(keys: &content_reader_settings::settings::SettingsKeys) -> io::Result<Game> {
    if keys.is_absent() {
        return Ok(Game {
            graphics: MapGraphics {
                nato_counters: Some(crate::map_display::option_defaults::NATO_COUNTERS),
                ..MapGraphics::default()
            },
            audio: MapAudio::default(),
        });
    }
    if !keys.has(&["graphics"]) {
        return Err(crate::invalid("Missing graphics block in game settings"));
    }
    if !keys.has(&["maprenderingoptions"]) {
        return Err(crate::invalid(
            "Missing mapRenderingOptions in game settings",
        ));
    }
    let map = |key: &str, fallback: bool| {
        keys.bool_at(&["maprenderingoptions", key])
            .unwrap_or(fallback)
    };
    let mut result = MapGraphics::default();
    let (options, _) = content_reader_settings::settings::MapOptions::read(keys);
    result.day_night = options.day_night;
    result.show_allied_plans = options.allied_plans;
    result.nato_counters = Some(options.nato_counters);
    result.full_resolution = map("draw_map_full_res", result.full_resolution);
    result.trees = map("draw_trees", result.trees);
    result.shadows = map("draw_shadows", result.shadows);
    result.rivers = map("draw_rivers", result.rivers);
    result.high_gfx_shaders = map("high_gfx_shaders", result.high_gfx_shaders);
    result.water_reflections = map("draw_water_reflections", result.water_reflections);
    result.sky = map("draw_sky", result.sky);
    result.buildings = map("draw_buildings", result.buildings);
    result.units = map("draw_units", result.units);
    result.city_sprawl = map("draw_citysprawl", result.city_sprawl);
    result.weather = map("draw_weather_effects", result.weather);
    if let Some(value) = keys.i32_at(&["graphics", "multi_sampling"]) {
        result.samples = u32::try_from(value)
            .map_err(|_| crate::invalid("Invalid game anti-aliasing"))?
            .max(1);
    }
    if let Some(value) = keys.i32_at(&["maprenderingoptions", "texture_quality"]) {
        result.texture_quality = value
            .try_into()
            .map_err(|_| crate::invalid("Invalid game texture quality"))?;
    }
    let audio = game_audio(keys);
    let game = Game {
        graphics: result,
        audio,
    };
    CampaignMapSettings {
        graphics: MapGraphicsOverrides {
            samples: Some(game.graphics.samples),
            texture_quality: Some(game.graphics.texture_quality),
            ..Default::default()
        },
        ..Default::default()
    }
    .validate()
    .map_err(crate::invalid)?;
    Ok(game)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CounterFilter, MapAudioOverrides};

    #[test]
    fn sample_counts_are_unknown_until_the_renderer_reports_them() {
        assert_eq!(sample_counts(), None);
        set_sample_counts([8, 1, 2, 16, 3]);
        assert_eq!(sample_counts(), Some(vec![1, 2, 8]));
        set_sample_counts([1, 2, 4, 8]);
        assert_eq!(sample_counts(), Some(vec![1, 2, 4, 8]));
    }

    fn choice(id: &str) -> content_reader_layers::content::visual::Choice {
        content_reader_layers::content::visual::Choice {
            id: id.into(),
            name: id.into(),
        }
    }

    fn scope(graphics: MapGraphicsOverrides) -> CampaignMapSettings {
        CampaignMapSettings {
            graphics,
            ..Default::default()
        }
    }

    #[test]
    fn a_flip_applies_to_the_value_when_it_runs_and_touches_nothing_else() {
        use crate::protocol::MapSetting;
        let mut store = Store::default();
        store.game.graphics.day_night = true;
        let widget = "campaign/widget/map-a".to_owned();
        let audio = MapAudioOverrides {
            voice: Some(40.0),
            ..Default::default()
        };
        store.entries.insert(
            widget.clone(),
            CampaignMapSettings {
                graphics: MapGraphicsOverrides {
                    borders: Some(false),
                    day_night: Some(false),
                    ..Default::default()
                },
                audio,
                visual_mods: Some(Vec::new()),
                ..Default::default()
            },
        );
        store.entries.insert(
            widget.clone(),
            CampaignMapSettings {
                audio,
                visual_mods: Some(Vec::new()),
                ..Default::default()
            },
        );
        let flipped = store.flipped(&widget, MapSetting::DayNight).unwrap();
        assert_eq!(flipped.graphics.day_night, Some(false));
        assert_eq!(flipped.graphics.borders, None);
        assert_eq!(flipped.audio, audio);
        assert_eq!(flipped.visual_mods, Some(Vec::new()));
        store.entries.insert(widget.clone(), flipped);
        let back = store.flipped(&widget, MapSetting::DayNight).unwrap();
        assert_eq!(back.graphics.day_night, Some(true));
        assert_eq!(
            store
                .flipped(&widget, MapSetting::AlliedPlans)
                .unwrap()
                .graphics
                .show_allied_plans,
            Some(!store.get(&widget).graphics.show_allied_plans)
        );
        assert!(store.flipped(&widget, MapSetting::Fog).is_err());
    }

    #[test]
    fn each_choice_inherits_through_every_scope_independently() {
        let mut store = Store::default();
        store.game.graphics.samples = 1;
        store.game.graphics.rivers = false;
        assert_eq!(store.get("campaign/widget/map-a").graphics.samples, 1);
        store.entries.insert(
            "$global".into(),
            CampaignMapSettings {
                graphics: MapGraphicsOverrides {
                    samples: Some(4),
                    trees: Some(false),
                    day_night: Some(false),
                    ..Default::default()
                },
                visual_mods: Some(vec![choice("mod/global.mod")]),
                ..Default::default()
            },
        );
        let widget = store.get("campaign/widget/map-a");
        assert!(!widget.graphics.trees && !widget.graphics.day_night);
        assert!(!widget.graphics.rivers);
        assert_eq!(
            store.get("other").visual_mods,
            Some(vec![choice("mod/global.mod")])
        );
        store.entries.insert(
            "campaign".into(),
            CampaignMapSettings {
                graphics: MapGraphicsOverrides {
                    samples: Some(2),
                    day_night: Some(true),
                    ..Default::default()
                },
                visual_mods: Some(vec![]),
                ..Default::default()
            },
        );
        let widget = store.get("campaign/widget/map-a");
        assert_eq!(widget.graphics.samples, 2);
        assert!(widget.graphics.day_night);
        assert!(!widget.graphics.trees);
        assert_eq!(widget.visual_mods, Some(vec![]));
        let key = "campaign/widget/map-a";
        store.entries.insert(
            key.into(),
            CampaignMapSettings {
                graphics: MapGraphicsOverrides {
                    samples: Some(8),
                    trees: Some(true),
                    ..Default::default()
                },
                visual_mods: Some(vec![choice("mod/widget.mod")]),
                ..Default::default()
            },
        );
        assert_eq!(store.get(key).graphics.samples, 8);
        assert!(store.get(key).graphics.trees);
        assert!(!store.get("campaign/widget/map-b").graphics.trees);
        assert_eq!(store.get("campaign/widget/map-b").graphics.samples, 2);
        assert_eq!(store.get("other/widget/map-a").graphics.samples, 4);
        store
            .entries
            .insert(key.into(), CampaignMapSettings::default());
        assert_eq!(store.get(key).graphics.samples, 2);
        assert!(!store.get(key).graphics.trees);
        assert_eq!(store.get(key).visual_mods, Some(vec![]));
        assert!(store.preferences(key).visual_mods.is_none());
        store
            .entries
            .insert("$global".into(), CampaignMapSettings::default());
        assert_eq!(store.get("other").graphics.samples, 1);
        assert_eq!(store.get("campaign").graphics.samples, 2);
    }

    #[test]
    fn inheriting_choices_follow_game_changes_beside_overrides() {
        let mut store = Store::default();
        store.entries.insert(
            "campaign".into(),
            scope(MapGraphicsOverrides {
                trees: Some(true),
                ..Default::default()
            }),
        );
        store.game.graphics.trees = false;
        store.game.graphics.sky = false;
        store.game.graphics.show_allied_plans = true;
        let campaign = store.get("campaign");
        assert!(campaign.graphics.trees);
        assert!(!campaign.graphics.sky);
        assert!(campaign.graphics.show_allied_plans);
        store.game.graphics.sky = true;
        assert!(store.get("campaign").graphics.sky);
        assert_eq!(store.preferences("campaign").graphics.sky, None);
    }

    #[test]
    fn nato_and_counter_filters_inherit_per_scope() {
        let mut store = Store::default();
        store.game.graphics.nato_counters = Some(true);
        store.entries.insert(
            "campaign".into(),
            scope(MapGraphicsOverrides {
                trees: Some(false),
                ..Default::default()
            }),
        );
        assert_eq!(store.get("campaign").graphics.nato_counters, Some(true));
        let bytes = serde_json::to_vec(&store.entries).unwrap();
        let mut restored = Store {
            entries: serde_json::from_slice(&bytes).unwrap(),
            ..Default::default()
        };
        restored.game.graphics.nato_counters = Some(false);
        assert_eq!(restored.get("campaign").graphics.nato_counters, Some(false));
        assert_eq!(
            restored.preferences("campaign").graphics.nato_counters,
            None
        );
        restored.entries.insert(
            "campaign/widget/map-a".into(),
            CampaignMapSettings {
                graphics: MapGraphicsOverrides {
                    nato_counters: Some(true),
                    ..Default::default()
                },
                counters: Some(CounterFilter::Players),
                audio: MapAudioOverrides {
                    voice: Some(0.0),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let widget = restored.get("campaign/widget/map-a");
        assert_eq!(widget.graphics.nato_counters, Some(true));
        assert_eq!(widget.counters, CounterFilter::Players);
        assert_eq!(widget.audio.voice, 0.0);
        let sibling = restored.get("campaign/widget/map-b");
        assert_eq!(sibling.graphics.nato_counters, Some(false));
        assert_eq!(sibling.counters, CounterFilter::All);
        assert_eq!(sibling.audio.voice, 100.0);
    }

    #[test]
    fn settings_remain_local_to_the_campaign_and_keep_original_artwork_by_default() {
        let mut store = Store::default();
        let changed = CampaignMapSettings {
            visual_mods: Some(vec![choice("mod/art.mod")]),
            ..Default::default()
        };
        store.entries.insert("campaign-a".into(), changed.clone());
        assert_eq!(store.preferences("campaign-a"), changed);
        assert_eq!(store.get("campaign-a").visual_mods, changed.visual_mods);
        assert_eq!(store.get("campaign-b").visual_mods, Some(vec![]));
        assert_eq!(
            store.get("campaign-b").graphics,
            ResolvedMapSettings::default().graphics
        );
    }

    #[test]
    fn imports_real_graphics_and_rejects_invalid_quality() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        let settings = parse_game_graphics(b"template_counter=1 graphics={multi_sampling=0 shadows=no} mapRenderingOptions={draw_trees=no draw_shadows=yes texture_quality=2 draw_units=no high_gfx_shaders=no draw_water_reflections=no draw_sky=no draw_map_full_res=no}").unwrap();
        assert_eq!(settings.nato_counters, Some(true));
        assert_eq!(settings.samples, 1);
        assert_eq!(settings.texture_quality, 2);
        assert!(!settings.high_gfx_shaders);
        assert!(!settings.water_reflections);
        assert!(!settings.sky);
        assert!(!settings.full_resolution);
        assert!(settings.shadows);
        assert!(!settings.trees && !settings.units);
        assert!(
            parse_game_graphics(b"graphics={multi_sampling=3} mapRenderingOptions={}").is_err()
        );
        assert!(
            parse_game_graphics(b"graphics={} mapRenderingOptions={texture_quality=9}").is_err()
        );
        let unread = parse_game_graphics(
            b"template_counter=invalid counter_color_mode=yes hide_daynight_cycle=maybe graphics={} mapRenderingOptions={}",
        )
        .unwrap();
        assert_eq!(
            (unread.nato_counters, unread.day_night),
            (
                Some(crate::map_display::option_defaults::NATO_COUNTERS),
                crate::map_display::option_defaults::DAY_NIGHT
            )
        );
        assert!(
            parse_game_graphics(b"graphics={} mapRenderingOptions={draw_map_full_res=invalid}")
                .unwrap()
                .full_resolution
        );
    }

    #[test]
    fn imports_the_map_mode_choices_and_volumes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        let game = parse_game(
            b"hide_daynight_cycle=yes show_allied_plans=no master_volume=80.000000 \
              ambient_volume=50.4 sound_fx_volume=25 voice_volume=0 music_volume=49.6 \
              graphics={} mapRenderingOptions={}",
        )
        .unwrap();
        assert!(!game.graphics.day_night);
        assert!(!game.graphics.show_allied_plans);
        assert_eq!(
            [
                game.audio.master,
                game.audio.ambient,
                game.audio.units,
                game.audio.combat,
                game.audio.voice,
                game.audio.music
            ],
            [80.0, 50.4, 25.0, 25.0, 0.0, 49.6]
        );
        let keys = content_reader_settings::settings::SettingsKeys::read(b"dev_master_volume=25.000000");
        assert_eq!(game_dev_master(&keys), 0.25);
        assert_eq!(
            game_dev_master(&content_reader_settings::settings::SettingsKeys::read(b"")),
            0.5
        );
        let shown = parse_game(
            b"hide_daynight_cycle=no show_allied_plans=yes graphics={} mapRenderingOptions={}",
        )
        .unwrap();
        assert!(shown.graphics.day_night && shown.graphics.show_allied_plans);
        let unstored = parse_game(b"graphics={} mapRenderingOptions={}").unwrap();
        assert_eq!(
            (unstored.graphics.day_night, unstored.graphics.show_allied_plans),
            (
                crate::map_display::option_defaults::DAY_NIGHT,
                crate::map_display::option_defaults::ALLIED_PLANS
            )
        );
        assert_eq!(unstored.audio, MapAudio::default());
    }

    #[test]
    fn a_fresh_install_imports_as_the_defaults() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        let fresh = parse_game(
            b"graphics={ shadows=no multi_sampling=4 }
              sound_fx_volume=100.000000 music_volume=75.000000 master_volume=100.000000
              ambient_volume=50.000000 voice_volume=100.000000
              mapRenderingOptions={ draw_trees=yes draw_rivers=yes draw_sky=yes
              draw_citysprawl=yes draw_shadows=yes draw_weather_effects=yes
              draw_water_reflections=yes draw_units=yes draw_buildings=yes high_gfx_shaders=yes
              draw_map_full_res=yes texture_quality=0 }
              template_counter=0",
        )
        .unwrap();
        assert_eq!(fresh.audio, MapAudio::default());
        assert_eq!(
            fresh.graphics,
            MapGraphics {
                nato_counters: Some(false),
                ..MapGraphics::default()
            }
        );
    }

    #[test]
    fn map_shadow_option_is_independent_of_device_shadow_preference() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        for device in ["yes", "no"] {
            for (map, expected) in [("yes", true), ("no", false)] {
                let input = format!(
                    "graphics={{shadows={device}}} mapRenderingOptions={{draw_shadows={map}}}"
                );
                assert_eq!(
                    parse_game_graphics(input.as_bytes()).unwrap().shadows,
                    expected,
                    "{input}"
                );
            }
        }
    }

    #[test]
    fn renderers_see_one_selection_for_equivalent_choices() {
        let mut store = Store::default();
        let chosen =
            |mods: Option<Vec<content_reader_layers::content::visual::Choice>>| CampaignMapSettings {
                visual_mods: mods,
                ..Default::default()
            };
        store.entries.insert("none".into(), chosen(None));
        store.entries.insert("empty".into(), chosen(Some(vec![])));
        store.entries.insert(
            "ab".into(),
            chosen(Some(vec![choice("workshop/a"), choice("workshop/b")])),
        );
        store.entries.insert(
            "ba".into(),
            chosen(Some(vec![
                choice("workshop/b"),
                choice("workshop/a"),
                choice("workshop/b"),
            ])),
        );
        assert_eq!(store.get("none"), store.get("empty"));
        assert_eq!(store.get("ab"), store.get("ba"));
        assert_eq!(
            store.preferences("ba").visual_mods.unwrap()[0],
            choice("workshop/b")
        );
    }

    #[test]
    fn a_save_from_a_stale_store_keeps_another_windows_scopes() {
        use crate::protocol::MapSetting;
        let folder =
            std::env::temp_dir().join(format!("hoi4-map-settings-{}", uuid::Uuid::new_v4()));
        let path = folder.join("map-settings.json");
        let (mut first, mut second) = (Store::default(), Store::default());
        let night_off = scope(MapGraphicsOverrides {
            day_night: Some(false),
            ..Default::default()
        });
        let rivers_off = scope(MapGraphicsOverrides {
            rivers: Some(false),
            ..Default::default()
        });
        let writes = replay_settings::exit::Exit::new();
        first
            .update_at(&writes, &path, "campaign-a".into(), |_, _| Ok(night_off.clone()))
            .unwrap();
        second
            .update_at(&writes, &path, "campaign-b".into(), |_, _| Ok(rivers_off.clone()))
            .unwrap();
        let saved = Store::load_from(&path).unwrap();
        assert_eq!(saved.preferences("campaign-a"), night_off);
        assert_eq!(saved.preferences("campaign-b"), rivers_off);
        assert_eq!(second.preferences("campaign-a"), night_off);
        first
            .update_at(&writes, &path, "campaign-b".into(), |saved, key| {
                saved
                    .flipped(key, MapSetting::DayNight)
                    .map_err(crate::invalid)
            })
            .unwrap();
        let saved = Store::load_from(&path).unwrap();
        let flipped = saved.preferences("campaign-b").graphics;
        assert_eq!(flipped.day_night, Some(false));
        assert_eq!(flipped.rivers, Some(false));
        assert_eq!(saved.preferences("campaign-a"), night_off);
        writes.seal();
        let refused = second.update_at(&writes, &path, "campaign-a".into(), |_, _| Ok(rivers_off.clone())).unwrap_err();
        assert_eq!(refused.to_string(), replay_settings::exit::CLOSED, "a save asked for once BattlePlan closes is refused, never cut off part way");
        assert_eq!(Store::load_from(&path).unwrap().preferences("campaign-a"), night_off);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn reloading_reads_another_process_save_and_keeps_game_values() {
        let path = std::env::temp_dir().join(format!(
            "hoi4_map_settings_reload_{}.json",
            std::process::id()
        ));
        let mut store = Store::default();
        store.game.graphics.samples = 1;
        let saved = CampaignMapSettings {
            graphics: MapGraphicsOverrides {
                trees: Some(false),
                ..Default::default()
            },
            visual_mods: Some(vec![choice("workshop/1")]),
            ..Default::default()
        };
        fs::write(
            &path,
            serde_json::to_vec(&BTreeMap::from([("campaign".to_owned(), saved)])).unwrap(),
        )
        .unwrap();
        let before = store.generation();
        store.reload_from(Ok(path.clone()));
        assert_ne!(
            store.generation(),
            before,
            "a reload resolves every map again"
        );
        assert_eq!(
            store.get("campaign").visual_mods,
            Some(vec![choice("workshop/1")])
        );
        assert!(!store.get("campaign").graphics.trees);
        assert_eq!(store.get("campaign").graphics.samples, 1);
        fs::write(&path, b"not json").unwrap();
        let before = store.generation();
        store.reload_from(Ok(path.clone()));
        assert!(store.check().unwrap_err().contains(&path.display().to_string()));
        assert_ne!(store.generation(), before);
        let loud = CampaignMapSettings {
            audio: MapAudioOverrides {
                voice: Some(500.0),
                ..Default::default()
            },
            ..Default::default()
        };
        fs::write(
            &path,
            serde_json::to_vec(&BTreeMap::from([("campaign-loud".to_owned(), loud)])).unwrap(),
        )
        .unwrap();
        store.reload_from(Ok(path.clone()));
        let error = store.check().unwrap_err();
        assert!(error.contains("campaign-loud") && error.contains(&path.display().to_string()), "{error}");
        let before = store.generation();
        store.configure_game(&path);
        assert_ne!(
            store.generation(),
            before,
            "reading the game's settings resolves again"
        );
        fs::remove_file(path).unwrap();
    }
}
