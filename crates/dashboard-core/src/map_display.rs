use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod option_defaults {
    use content_reader_settings::settings::MapOptions;
    pub const DAY_NIGHT: bool = MapOptions::DEFAULT.day_night;
    pub const ALLIED_PLANS: bool = MapOptions::DEFAULT.allied_plans;
    pub const COUNTRY_COUNTER_COLOURS: bool = MapOptions::DEFAULT.country_counter_colours;
    pub const NATO_COUNTERS: bool = MapOptions::DEFAULT.nato_counters;
    pub const FOG: bool = true;
    pub const PLAYER_COUNTERS: bool = false;
    pub const RADAR: bool = false;
}

macro_rules! inheritable {
    (
        $(#[$meta:meta])*
        pub struct $name:ident / $overrides:ident {
            $( $(#[$field_meta:meta])* $field:ident : $ty:ty = $default:expr ),* $(,)?
            $(
                ;
                $( $(#[$optional_meta:meta])* $optional:ident : Option<$optional_ty:ty> ),* $(,)?
            )?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, TS)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name {
            $( $(#[$field_meta])* pub $field: $ty, )*
            $( $( $(#[$optional_meta])* pub $optional: Option<$optional_ty>, )* )?
        }

        impl Default for $name {
            fn default() -> Self {
                Self { $( $field: $default, )* $( $( $optional: None, )* )? }
            }
        }

        #[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, TS)]
        #[serde(rename_all = "camelCase", default, deny_unknown_fields)]
        pub struct $overrides {
            $( pub $field: Option<$ty>, )*
            $( $( pub $optional: Option<$optional_ty>, )* )?
        }

        impl $overrides {
            pub fn resolve(&self, parent: &$name) -> $name {
                $name {
                    $( $field: self.$field.unwrap_or(parent.$field), )*
                    $( $( $optional: self.$optional.or(parent.$optional), )* )?
                }
            }
            pub fn is_empty(&self) -> bool {
                true $( && self.$field.is_none() )* $( $( && self.$optional.is_none() )* )?
            }
        }
    };
}

inheritable! {
    pub struct MapGraphics / MapGraphicsOverrides {
        samples: u32 = 4,
        texture_quality: u8 = 0,
        #[doc = " The world target size; `draw_map_full_res`."]
        full_resolution: bool = true,
        high_gfx_shaders: bool = true,
        water_reflections: bool = true,
        #[doc = " Cast shadows; `draw_shadows`."]
        shadows: bool = true,
        #[doc = " The day/night cycle, the game's `hide_daynight_cycle` inverted. Off fixes the virtual"]
        #[doc = " lights while simulation time and province clocks stay authoritative."]
        day_night: bool = option_defaults::DAY_NIGHT,
        water: bool = true,
        #[doc = " Main switch of the four border parts."]
        borders: bool = true,
        land_borders: bool = true,
        sea_borders: bool = true,
        demilitarized_zones: bool = true,
        strait_crossings: bool = true,
        rivers: bool = true,
        trees: bool = true,
        #[doc = " Main switch of rails, bridges and supply trains."]
        railways: bool = true,
        rails: bool = true,
        railway_bridges: bool = true,
        supply_trains: bool = true,
        city_sprawl: bool = true,
        #[doc = " Buildings, facilities and floating harbours."]
        buildings: bool = true,
        #[doc = " The map table frame and its ambient actors."]
        map_table: bool = true,
        #[doc = " Nuclear and thermonuclear impact effects."]
        nuclear_strikes: bool = true,
        #[doc = " Critical shore bombardment explosions."]
        bombardment_explosions: bool = true,
        #[doc = " The explosions where missiles land."]
        missile_explosions: bool = true,
        #[doc = " Main switch of the floating map text kinds."]
        map_text: bool = true,
        text_fake_divisions: bool = true,
        text_railway_guns: bool = true,
        text_bombardments: bool = true,
        text_generals: bool = true,
        text_paradrops: bool = true,
        text_resistance: bool = true,
        text_buildings: bool = true,
        weather: bool = true,
        sky: bool = true,
        #[doc = " World country names."]
        country_names: bool = true,
        #[doc = " Main switch of the 3-D unit figures and aircraft; `draw_units`."]
        units: bool = true,
        #[doc = " Land, railway-gun, ship and raid figures."]
        unit_figures: bool = true,
        #[doc = " Main switch of the aircraft mission parts."]
        aircraft: bool = true,
        #[doc = " Patrol and training flights."]
        air_patrols: bool = true,
        air_reconnaissance: bool = true,
        #[doc = " Close air support."]
        air_support: bool = true,
        #[doc = " Strategic, carpet and fire bombing."]
        air_bombing: bool = true,
        #[doc = " Dogfights and interceptions."]
        air_combat: bool = true,
        air_kamikaze: bool = true,
        air_supply: bool = true,
        #[doc = " Missiles in flight."]
        air_missiles: bool = true,
        #[doc = " Main switch of every battle-plan part, raid paths included."]
        battle_plans: bool = true,
        front_lines: bool = true,
        #[doc = " Offensive, fallback and withdrawal lines."]
        offensive_lines: bool = true,
        planned_movement: bool = true,
        naval_invasions: bool = true,
        paradrops: bool = true,
        raid_paths: bool = true,
        plan_labels: bool = true,
        #[doc = " Allied battle plans, the game's `show_allied_plans`."]
        show_allied_plans: bool = option_defaults::ALLIED_PLANS,
        #[doc = " The selected units' movement arrows."]
        movement_arrows: bool = true,
        #[doc = " Main switch of every counter and map icon part."]
        counters: bool = true,
        land_counters: bool = true,
        railway_gun_counters: bool = true,
        naval_counters: bool = true,
        #[doc = " Ships in port on naval bases."]
        port_counters: bool = true,
        #[doc = " Air-base, naval-base, naval HQ and facility icons."]
        base_icons: bool = true,
        #[doc = " Faction-theatre pins."]
        theatre_pins: bool = true,
        land_combat_markers: bool = true,
        #[doc = " Naval combat, result and mission icons."]
        naval_markers: bool = true,
        raid_icons: bool = true,
        #[doc = " Capital and victory-point names and symbols."]
        city_names: bool = true,
        #[doc = " Map hover tooltips; the hover itself keeps working."]
        tooltips: bool = true,
        ;
        #[doc = " NATO counter frames (`template_counter`); none, when the game's settings could not"]
        #[doc = " be read, uses the recording's game choice."]
        nato_counters: Option<bool>,
    }
}

impl Eq for MapGraphics {}
impl Eq for MapGraphicsOverrides {}

impl MapGraphics {
    pub fn land_borders(&self) -> bool {
        self.borders && self.land_borders
    }
    pub fn sea_borders(&self) -> bool {
        self.borders && self.sea_borders
    }
    pub fn demilitarized_zones(&self) -> bool {
        self.borders && self.demilitarized_zones
    }
    pub fn strait_crossings(&self) -> bool {
        self.borders && self.strait_crossings
    }
    pub fn rails(&self) -> bool {
        self.railways && self.rails
    }
    pub fn railway_bridges(&self) -> bool {
        self.railways && self.railway_bridges
    }
    pub fn supply_trains(&self) -> bool {
        self.railways && self.supply_trains
    }
    pub fn unit_figures(&self) -> bool {
        self.units && self.unit_figures
    }
    pub fn aircraft(&self) -> bool {
        self.units && self.aircraft
    }
    pub fn air_mission(&self, mission: AirMission) -> bool {
        self.aircraft()
            && match mission {
                AirMission::Patrol => self.air_patrols,
                AirMission::Reconnaissance => self.air_reconnaissance,
                AirMission::Support => self.air_support,
                AirMission::Bombing => self.air_bombing,
                AirMission::Combat => self.air_combat,
                AirMission::Kamikaze => self.air_kamikaze,
                AirMission::Supply => self.air_supply,
                AirMission::Missile => self.air_missiles,
            }
    }
    pub fn plan(&self, kind: PlanKind) -> bool {
        self.battle_plans
            && match kind {
                PlanKind::Front => self.front_lines,
                PlanKind::Offensive => self.offensive_lines,
                PlanKind::Movement => self.planned_movement,
                PlanKind::Invasion => self.naval_invasions,
                PlanKind::Paradrop => self.paradrops,
                PlanKind::Raid => self.raid_paths,
                PlanKind::TradeRoute => false,
            }
    }
    pub fn plan_labels(&self) -> bool {
        self.battle_plans && self.plan_labels
    }
    pub fn counter(&self, part: bool) -> bool {
        self.counters && part
    }
    pub fn map_text(&self, kind: MapTextKind) -> bool {
        self.map_text
            && match kind {
                MapTextKind::FakeDivisions => self.text_fake_divisions,
                MapTextKind::RailwayGuns => self.text_railway_guns,
                MapTextKind::Bombardments => self.text_bombardments,
                MapTextKind::Generals => self.text_generals,
                MapTextKind::Paradrops => self.text_paradrops,
                MapTextKind::Resistance => self.text_resistance,
                MapTextKind::Buildings => self.text_buildings,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MapTextKind {
    FakeDivisions,
    RailwayGuns,
    Bombardments,
    Generals,
    Paradrops,
    Resistance,
    Buildings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AirMission {
    Patrol,
    Reconnaissance,
    Support,
    Bombing,
    Combat,
    Kamikaze,
    Supply,
    Missile,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlanKind {
    #[default]
    Front,
    Offensive,
    Movement,
    Invasion,
    Paradrop,
    Raid,
    TradeRoute,
}

inheritable! {
    pub struct MapAudio / MapAudioOverrides {
        #[doc = " `master_volume`."]
        master: f32 = 100.0,
        #[doc = " `ambient_volume`; a fresh install stores 50."]
        ambient: f32 = 50.0,
        #[doc = " Unit, vehicle, train and aircraft sounds; `sound_fx_volume`."]
        units: f32 = 100.0,
        #[doc = " Combat and strike sounds; `sound_fx_volume`."]
        combat: f32 = 100.0,
        #[doc = " The interface's button sounds; `sound_fx_volume`."]
        interface: f32 = 100.0,
        #[doc = " `voice_volume`."]
        voice: f32 = 100.0,
        #[doc = " The music player; `music_volume`."]
        music: f32 = 75.0,
    }
}

impl MapAudio {
    pub fn gain(&self, category: f32) -> f32 {
        self.master / 100.0 * category / 100.0
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CounterFilter {
    #[default]
    All,
    Players,
    Countries { tags: Vec<String> },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CampaignMapSettings {
    pub graphics: MapGraphicsOverrides,
    pub audio: MapAudioOverrides,
    #[doc = " None inherits the parent's counter filter."]
    pub counters: Option<CounterFilter>,
    #[doc = " Installed mods shown over the recording's artwork. None inherits the parent scope's"]
    #[doc = " choice; at the root it shows none. Ids are matched against the installed catalogue,"]
    #[doc = " never joined to a path; a mod no longer installed keeps its name."]
    #[ts(type = "Array<{ id: string, name: string }> | null")]
    pub visual_mods: Option<Vec<content_reader_layers::content::visual::Choice>>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMapSettings {
    pub graphics: MapGraphics,
    pub audio: MapAudio,
    pub counters: CounterFilter,
    #[doc = " The chosen visual mods as a set: sorted by id, without repeats, and empty when none."]
    #[ts(type = "Array<{ id: string, name: string }> | null")]
    pub visual_mods: Option<Vec<content_reader_layers::content::visual::Choice>>,
}

impl ResolvedMapSettings {
    pub fn child(&self, settings: &CampaignMapSettings) -> Self {
        Self {
            graphics: settings.graphics.resolve(&self.graphics),
            audio: settings.audio.resolve(&self.audio),
            counters: settings
                .counters
                .clone()
                .unwrap_or_else(|| self.counters.clone()),
            visual_mods: settings
                .visual_mods
                .clone()
                .or_else(|| self.visual_mods.clone()),
        }
    }
}

impl CampaignMapSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .graphics
            .texture_quality
            .is_some_and(|quality| quality > 2)
        {
            return Err("Invalid texture quality");
        }
        if self
            .graphics
            .samples
            .is_some_and(|samples| !matches!(samples, 1 | 2 | 4 | 8))
        {
            return Err("Invalid anti-aliasing sample count");
        }
        let audio = self.audio;
        if [
            audio.master,
            audio.ambient,
            audio.units,
            audio.combat,
            audio.interface,
            audio.voice,
            audio.music,
        ]
        .into_iter()
        .flatten()
        .any(|volume| !(0.0..=100.0).contains(&volume))
        {
            return Err("Invalid volume");
        }
        if let Some(CounterFilter::Countries { tags }) = &self.counters {
            for tag in tags {
                crate::protocol::validate_tag(tag)?;
            }
            if tags.iter().collect::<std::collections::BTreeSet<_>>().len() != tags.len() {
                return Err("Duplicate country in the counter filter");
            }
        }
        if let Some(mods) = &self.visual_mods
            && (mods.iter().any(|choice| choice.id.is_empty())
                || mods
                    .iter()
                    .map(|choice| &choice.id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != mods.len())
        {
            return Err("Invalid visual mod selection");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_choice_inherits_independently() {
        let parent = MapGraphics {
            trees: false,
            samples: 2,
            ..Default::default()
        };
        let own = MapGraphicsOverrides {
            rivers: Some(false),
            ..Default::default()
        };
        let resolved = own.resolve(&parent);
        assert!(!resolved.rivers);
        assert!(!resolved.trees);
        assert_eq!(resolved.samples, 2);
        assert!(MapGraphicsOverrides::default().is_empty());
        assert!(!own.is_empty());
        let on = MapGraphicsOverrides {
            trees: Some(true),
            ..Default::default()
        };
        assert!(on.resolve(&parent).trees);
        assert_eq!(
            MapGraphicsOverrides::default()
                .resolve(&parent)
                .nato_counters,
            None
        );
        let nato = MapGraphicsOverrides {
            nato_counters: Some(true),
            ..Default::default()
        };
        assert_eq!(nato.resolve(&parent).nato_counters, Some(true));
        assert_eq!(
            MapGraphicsOverrides::default()
                .resolve(&MapGraphics {
                    nato_counters: Some(false),
                    ..parent
                })
                .nato_counters,
            Some(false)
        );
    }

    #[test]
    fn parts_draw_only_under_their_main_switch() {
        let mut graphics = MapGraphics::default();
        assert!(graphics.land_borders() && graphics.plan(PlanKind::Raid));
        graphics.borders = false;
        assert!(!graphics.land_borders() && !graphics.sea_borders());
        assert!(!graphics.demilitarized_zones() && !graphics.strait_crossings());
        graphics.battle_plans = false;
        assert!(!graphics.plan(PlanKind::Front) && !graphics.plan_labels());
        graphics.battle_plans = true;
        graphics.paradrops = false;
        assert!(!graphics.plan(PlanKind::Paradrop) && graphics.plan(PlanKind::Movement));
        graphics.text_paradrops = false;
        assert!(!graphics.map_text(MapTextKind::Paradrops));
        assert!(graphics.map_text(MapTextKind::Buildings));
        graphics.map_text = false;
        assert!(!graphics.map_text(MapTextKind::Buildings));
        graphics.air_bombing = false;
        assert!(!graphics.air_mission(AirMission::Bombing));
        assert!(graphics.air_mission(AirMission::Patrol));
        graphics.units = false;
        assert!(!graphics.unit_figures() && !graphics.aircraft());
        assert!(!graphics.air_mission(AirMission::Patrol));
        graphics.railways = false;
        assert!(!graphics.rails() && !graphics.supply_trains() && !graphics.railway_bridges());
    }

    #[test]
    fn stored_overrides_omit_inherited_choices_and_refuse_unknown_fields() {
        let settings: CampaignMapSettings =
            serde_json::from_str(r#"{"graphics":{"trees":false}}"#).unwrap();
        assert_eq!(settings.graphics.trees, Some(false));
        assert_eq!(settings.graphics.rivers, None);
        assert_eq!(settings.counters, None);
        let json = serde_json::to_value(CampaignMapSettings::default()).unwrap();
        assert!(json["graphics"]["trees"].is_null());
        assert!(serde_json::from_str::<CampaignMapSettings>(r#"{"graphic":{}}"#).is_err());
        assert!(
            serde_json::from_str::<CampaignMapSettings>(r#"{"graphics":{"tree":false}}"#).is_err()
        );
    }

    #[test]
    fn validation_refuses_invalid_quality_volume_and_filters() {
        for graphics in [
            MapGraphicsOverrides {
                samples: Some(3),
                ..Default::default()
            },
            MapGraphicsOverrides {
                texture_quality: Some(3),
                ..Default::default()
            },
        ] {
            assert!(
                CampaignMapSettings {
                    graphics,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        let volumes: [(&str, fn(&mut MapAudioOverrides) -> &mut Option<f32>); 7] = [
            ("master", |audio| &mut audio.master),
            ("ambient", |audio| &mut audio.ambient),
            ("units", |audio| &mut audio.units),
            ("combat", |audio| &mut audio.combat),
            ("interface", |audio| &mut audio.interface),
            ("voice", |audio| &mut audio.voice),
            ("music", |audio| &mut audio.music),
        ];
        for (name, field) in volumes {
            for (volume, valid) in [
                (0.0, true),
                (100.0, true),
                (100.5, false),
                (5000.0, false),
                (-1.0, false),
                (f32::NAN, false),
            ] {
                let mut audio = MapAudioOverrides::default();
                *field(&mut audio) = Some(volume);
                assert_eq!(
                    CampaignMapSettings {
                        audio,
                        ..Default::default()
                    }
                    .validate()
                    .is_ok(),
                    valid,
                    "{name} {volume}"
                );
            }
        }
        let filter = |tags: &[&str]| CampaignMapSettings {
            counters: Some(CounterFilter::Countries {
                tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            }),
            ..Default::default()
        };
        assert!(filter(&["GER", "SOV"]).validate().is_ok());
        assert!(filter(&["GER", "GER"]).validate().is_err());
        assert!(filter(&[""]).validate().is_err());
        let many: Vec<String> = (0..5_000).map(|n| format!("C{n:04}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(filter(&many).validate().is_ok());
    }

}
