//! The regional-weather keyed family. WeatherRegions dates acquisition, not
//! native weather updates. Province temperature failures never hide known flags.

use dashboard_base::prepared::prepass::Family;
use dashboard_keyed::Values;
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{Recorded, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;

mod retained;

#[derive(Default, Serialize, Deserialize)]
pub struct Observer {
    temperature: super::temperature::Observation,
}

impl Observer {
    /// Bind once per source/content binding before folding a branch, including
    /// after restoring its catalogue. Content failures refuse temperature only.
    pub fn bind(
        &mut self,
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> io::Result<()> {
        self.temperature.bind(source, options, cancelled)
    }

    /// The same binding hook for a preparation driver that supplies a check
    /// callback rather than a cancellation flag. Call before fold and fallback.
    pub fn bind_checked(
        &mut self,
        source: &dashboard_base::Source,
        options: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.temperature.bind_checked(source, options, check)
    }

    pub fn temperature_status(&self) -> &str {
        self.temperature.status()
    }

    pub fn temperature_warnings(&self) -> Vec<String> {
        self.temperature.warnings()
    }
}

pub static FAMILY: Family = dashboard_keyed::block::family::<Observer>(
    "regional-weather",
    "regional-weather.extract",
    "regional-weather.sequential",
    "regional-weather.fallback",
);

impl dashboard_keyed::Observer for Observer {
    const NAME: &'static str = "regional-weather";
    const TABLES: &'static [TableId] = &[TableId::WeatherRegions, TableId::WeatherProvinces];
    const READ: TableId = TableId::WeatherRegions;

    fn family() -> &'static Family {
        &FAMILY
    }

    fn keeper() -> Option<Box<dyn dashboard_keyed::block::Keeper<Self>>> {
        Some(Box::new(retained::Keeper::default()))
    }

    fn bind(
        &mut self,
        source: &dashboard_base::Source,
        content: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.bind_checked(source, content, check)
    }

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::named(
                TableId::WeatherRegions,
                &[
                    "light_rain",
                    "heavy_rain",
                    "snow",
                    "blizzard",
                    "sandstorm",
                    "arctic_water",
                    "combat",
                    "air_detection",
                    "map_modifiers_known",
                    "naval_detection",
                    "air_attack",
                    "air_defence",
                ],
            )?,
            TableFields::named(
                TableId::WeatherProvinces,
                &[
                    "temperature.temperature",
                    "temperature.drift",
                    "temperature.staged",
                    "temperature.working",
                    "temperature_pass",
                ],
            )?
            .when_read(),
        ])
    }

    fn changed(changes: &dashboard_base::prepared::frame_changes::FrameChanges) -> bool {
        // An equal pass packet at a later hour is not a certificate for that
        // hour. Check its anchor/pass length even when its rows did not change.
        changes.slot_read[TableId::WeatherProvinces.index()]
            || Self::TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let temperature_known = self.temperature.observe(frame, values);
        let rows = frame.complete::<tables::WeatherRegions>();
        let regional_known = rows.is_ok();
        for row in rows.unwrap_or_default() {
            for (flag, value) in [
                ("light_rain", row.light_rain),
                ("heavy_rain", row.heavy_rain),
                ("snow", row.snow),
                ("blizzard", row.blizzard),
                ("sandstorm", row.sandstorm),
                ("arctic_water", row.arctic_water),
            ] {
                // Names retain explicit zero flags so a region with no active
                // conditions remains distinct from an absent region record.
                values.push(format!("{}|{flag}", row.id), i128::from(value));
            }
            if let Recorded::Value(Some(value)) = row.air_detection {
                values.push(
                    format!("{}|mod:air_detection", row.id),
                    i128::from(value.unwrap_or(0)),
                );
            }
            if row.map_modifiers_known == Recorded::Value(true) {
                for (modifier, scalar) in [
                    ("naval_detection", row.naval_detection),
                    ("air_attack", row.air_attack),
                    ("air_defence", row.air_defence),
                ] {
                    if let Recorded::Value(value) = scalar {
                        values.push(
                            format!("{}|mod:{modifier}", row.id),
                            i128::from(value.unwrap_or(0)),
                        );
                    }
                }
            }
            if let Recorded::Value(Some(combat)) = row.combat {
                for (modifier, value) in [
                    ("army_attack_factor", combat.army_attack_factor),
                    ("breakthrough_factor", combat.breakthrough_factor),
                    ("army_speed_factor", combat.army_speed_factor),
                    ("army_attack_speed_factor", combat.army_attack_speed_factor),
                ] {
                    values.push(
                        format!("{}|mod:{modifier}", row.id),
                        i128::from(value.unwrap_or(0)),
                    );
                }
            }
        }
        Ok(regional_known || temperature_known)
    }
}
