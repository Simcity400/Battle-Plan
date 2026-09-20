//! Global, campaign and widget preferences: read once at service startup, written on the data loader.
use desktop_protocol::CampaignMapSettings;
use serde::Serialize;
use std::{collections::BTreeMap, fs, io, path::PathBuf, sync::atomic::AtomicBool};

// The browser bridge has no adapter. Only universally supported modes are
// advertised there; the desktop replaces this mask during device creation.
static SAMPLE_COUNTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1 | 4);
pub fn set_sample_counts(mask: u32) {
    SAMPLE_COUNTS.store(mask, std::sync::atomic::Ordering::Relaxed);
}
pub fn sample_counts() -> Vec<u32> {
    let mask = SAMPLE_COUNTS.load(std::sync::atomic::Ordering::Relaxed);
    [1, 2, 4, 8]
        .into_iter()
        .filter(|count| mask & count != 0)
        .collect()
}

pub fn campaign_key(replay: &replay::container::Replay) -> String {
    replay
        .campaign_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Default)]
pub struct Store {
    entries: BTreeMap<String, CampaignMapSettings>,
    error: Option<String>,
    game_graphics: desktop_protocol::MapGraphics,
    game_graphics_error: Option<String>,
}

fn path() -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| crate::invalid("LOCALAPPDATA is not set"))?,
    )
    .join("HOI4Replay")
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
        let path = path()?;
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
        match game_graphics(user) {
            Ok(graphics) => {
                self.game_graphics = graphics;
                self.game_graphics_error = None;
            }
            Err(error) => {
                self.game_graphics_error = Some(error.to_string());
            }
        }
    }
    pub fn game_graphics(&self) -> Option<desktop_protocol::MapGraphics> {
        self.game_graphics_error
            .is_none()
            .then_some(self.game_graphics)
    }
    pub fn game_graphics_error(&self) -> Option<String> {
        self.game_graphics_error.clone()
    }
    pub fn inherited(&self, key: &str) -> CampaignMapSettings {
        if key == "$global" {
            CampaignMapSettings {
                graphics: self.game_graphics,
                ..Default::default()
            }
        } else if let Some((campaign, _)) = key.split_once("/widget/") {
            self.get(campaign)
        } else {
            self.get("$global")
        }
    }
    /// Form values preserve inheritance markers; renderer values resolve artwork too.
    pub fn preferences(&self, key: &str) -> CampaignMapSettings {
        let mut settings = self.get(key);
        settings.visual_mods = self
            .entries
            .get(key)
            .and_then(|entry| entry.visual_mods.clone());
        settings
    }
    pub fn get(&self, key: &str) -> CampaignMapSettings {
        let mut settings = self.entries.get(key).cloned().unwrap_or_default();
        if settings.use_game_graphics || settings.visual_mods.is_none() {
            let parent = self.inherited(key);
            if settings.use_game_graphics {
                settings.graphics = parent.graphics;
            }
            if settings.visual_mods.is_none() {
                settings.visual_mods = parent.visual_mods;
            }
        }
        settings
    }
    pub fn save(&mut self, key: String, settings: CampaignMapSettings) -> io::Result<()> {
        self.check().map_err(crate::invalid)?;
        settings.validate().map_err(crate::invalid)?;
        let mut entries = self.entries.clone();
        entries.insert(key, settings);
        let bytes = serde_json::to_vec(&entries)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(crate::invalid("Map settings exceed their size limit"));
        }
        let path = path()?;
        fs::create_dir_all(path.parent().expect("settings parent"))?;
        // A failed write leaves the in-memory selection unchanged.
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, bytes)?;
        fs::rename(temp, path)?;
        self.entries = entries;
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub source_id: String,
    pub revision: String,
    pub settings: CampaignMapSettings,
    pub inherited_settings: CampaignMapSettings,
    pub mods: Vec<installed_content::content::visual::VisualMod>,
    pub recorded_mods: Vec<String>,
    pub sample_counts: Vec<u32>,
    pub game_graphics: Option<desktop_protocol::MapGraphics>,
    pub game_graphics_error: Option<String>,
}

pub fn catalogue(
    options: &installed_content::ResolveOptions,
    cancel: &AtomicBool,
) -> io::Result<Vec<installed_content::content::visual::VisualMod>> {
    installed_content::content::visual::installed(options, cancel)
}

/// Read the user's existing game preferences; never writes to settings.txt.
fn game_graphics(user: &std::path::Path) -> io::Result<desktop_protocol::MapGraphics> {
    parse_game_graphics_document(&installed_content::settings::read(user)?)
}
#[cfg(test)]
fn parse_game_graphics(bytes: &[u8]) -> io::Result<desktop_protocol::MapGraphics> {
    parse_game_graphics_document(&installed_content::parser::parse(bytes)?)
}
fn parse_game_graphics_document(
    document: &installed_content::parser::Document,
) -> io::Result<desktop_protocol::MapGraphics> {
    let graphics = document
        .get("graphics")
        .and_then(|value| value.items())
        .ok_or_else(|| crate::invalid("Missing graphics block in game settings"))?;
    let map = document
        .get("mapRenderingOptions")
        .and_then(|value| value.items())
        .ok_or_else(|| crate::invalid("Missing mapRenderingOptions in game settings"))?;
    fn scalar<'a>(items: &'a [installed_content::parser::Item], key: &str) -> Option<&'a str> {
        items
            .iter()
            .find(|item| item.key.as_deref() == Some(key.as_bytes()))
            .and_then(|item| item.value.scalar_str())
    }
    fn toggle(items: &[installed_content::parser::Item], key: &str, fallback: bool) -> io::Result<bool> {
        match scalar(items, key) {
            Some("yes") => Ok(true),
            Some("no") => Ok(false),
            None => Ok(fallback),
            _ => Err(crate::invalid(format!("Invalid game setting: {key}"))),
        }
    }
    let mut result = desktop_protocol::MapGraphics::default();
    result.trees = toggle(map, "draw_trees", result.trees)?;
    result.shadows =
        toggle(graphics, "shadows", true)? && toggle(map, "draw_shadows", result.shadows)?;
    result.rivers = toggle(map, "draw_rivers", result.rivers)?;
    result.buildings = toggle(map, "draw_buildings", result.buildings)?;
    result.units = toggle(map, "draw_units", result.units)?;
    result.city_sprawl = toggle(map, "draw_citysprawl", result.city_sprawl)?;
    result.weather = toggle(map, "draw_weather_effects", result.weather)?;
    if let Some(value) = scalar(graphics, "multi_sampling") {
        result.samples = value
            .parse::<u32>()
            .map_err(|_| crate::invalid("Invalid game anti-aliasing"))?
            .max(1);
    }
    if let Some(value) = scalar(map, "texture_quality") {
        result.texture_quality = value
            .parse()
            .map_err(|_| crate::invalid("Invalid game texture quality"))?;
    }
    CampaignMapSettings {
        graphics: result,
        visual_mods: None,
        use_game_graphics: true,
    }
    .validate()
    .map_err(crate::invalid)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_inherit_and_reset_without_changing_siblings() {
        let mut store = Store::default();
        store.game_graphics.samples = 1;
        assert_eq!(store.get("campaign/widget/map-a").graphics.samples, 1);
        store.entries.insert(
            "$global".into(),
            CampaignMapSettings {
                use_game_graphics: false,
                graphics: desktop_protocol::MapGraphics {
                    samples: 4,
                    trees: false,
                    ..Default::default()
                },
                visual_mods: Some(vec!["mod/global.mod".into()]),
            },
        );
        assert!(!store.get("campaign/widget/map-a").graphics.trees);
        assert_eq!(
            store.get("other").visual_mods,
            Some(vec!["mod/global.mod".into()])
        );
        store.entries.insert(
            "campaign".into(),
            CampaignMapSettings {
                use_game_graphics: false,
                graphics: desktop_protocol::MapGraphics {
                    samples: 2,
                    ..Default::default()
                },
                visual_mods: Some(vec![]),
            },
        );
        assert_eq!(store.get("campaign/widget/map-a").graphics.samples, 2);
        assert_eq!(store.get("campaign/widget/map-a").visual_mods, Some(vec![]));
        let key = "campaign/widget/map-a";
        store.entries.insert(
            key.into(),
            CampaignMapSettings {
                use_game_graphics: false,
                graphics: desktop_protocol::MapGraphics {
                    samples: 8,
                    ..Default::default()
                },
                visual_mods: Some(vec!["mod/widget.mod".into()]),
            },
        );
        assert_eq!(store.get(key).graphics.samples, 8);
        assert_eq!(store.get("campaign/widget/map-b").graphics.samples, 2);
        assert_eq!(store.get("other/widget/map-a").graphics.samples, 4);
        store
            .entries
            .insert(key.into(), CampaignMapSettings::default());
        assert_eq!(store.get(key).graphics.samples, 2);
        assert_eq!(store.get(key).visual_mods, Some(vec![]));
        assert!(store.preferences(key).visual_mods.is_none());
        store
            .entries
            .insert("$global".into(), CampaignMapSettings::default());
        assert_eq!(store.get("other").graphics.samples, 1);
        assert_eq!(store.get("campaign").graphics.samples, 2);
    }

    #[test]
    fn imports_real_graphics_and_rejects_invalid_quality() {
        let settings = parse_game_graphics(b"graphics={multi_sampling=0 shadows=no} mapRenderingOptions={draw_trees=no draw_shadows=yes texture_quality=2 draw_units=no}").unwrap();
        assert_eq!(settings.samples, 1);
        assert_eq!(settings.texture_quality, 2);
        assert!(!settings.shadows && !settings.trees && !settings.units);
        assert!(
            parse_game_graphics(b"graphics={multi_sampling=3} mapRenderingOptions={}").is_err()
        );
        assert!(
            parse_game_graphics(b"graphics={} mapRenderingOptions={texture_quality=9}").is_err()
        );
    }
    #[test]
    fn inherited_graphics_follow_game_changes_even_with_saved_mods() {
        let mut store = Store::default();
        let inherited = CampaignMapSettings {
            visual_mods: Some(vec!["mod/art.mod".into()]),
            ..Default::default()
        };
        let explicit = CampaignMapSettings {
            use_game_graphics: false,
            ..Default::default()
        };
        store.entries.insert("inherited".into(), inherited);
        store.entries.insert("explicit".into(), explicit.clone());
        store.game_graphics.samples = 1;
        store.game_graphics.trees = false;
        assert_eq!(store.get("new").graphics, store.game_graphics);
        assert_eq!(store.get("inherited").graphics, store.game_graphics);
        assert_eq!(
            store.get("inherited").visual_mods,
            Some(vec!["mod/art.mod".into()])
        );
        assert_eq!(store.get("explicit"), explicit);
    }
    #[test]
    fn settings_remain_local_to_the_campaign_and_keep_original_artwork_by_default() {
        let mut store = Store::default();
        let changed = CampaignMapSettings {
            visual_mods: Some(vec!["mod/art.mod".into()]),
            ..Default::default()
        };
        store.entries.insert("campaign-a".into(), changed.clone());
        assert_eq!(store.get("campaign-a"), changed);
        assert_eq!(store.get("campaign-b"), CampaignMapSettings::default());
        assert!(store.get("campaign-b").visual_mods.is_none());
    }
}
