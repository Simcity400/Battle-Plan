//! CCountry's balance-of-power properties (141A7B350 cases 46..58).
use super::*;
use installed_content::localisation::Argument;

const PROPERTIES: [&str; 8] = [
    "GetPowerBalanceName",
    "GetLeftSideName",
    "GetRightSideName",
    "GetActiveSideName",
    "GetTrendingSideName",
    "GetActiveRangeName",
    "GetChangeRateDesc",
    "GetBopTrendTextIcon",
];

impl ScriptedText<'_> {
    pub(super) fn balance(&self, slot: u32, name: &str) -> Result<Option<String>, String> {
        let Some(which) = PROPERTIES.iter().position(|p| named(p, name)) else {
            return Ok(None);
        };
        // find_by_country 140E424F0: first manager record containing the country.
        // Whole membership is required, including records preceding a found one.
        let members = self
            .frame
            .complete::<tables::PowerBalanceMembers>()
            .map_err(|e| format!("{name}: {e}"))?;
        let mut selected = None;
        for row in members {
            let owner = self.scopes.slot(row.country).ok_or_else(|| {
                format!(
                    "{name}: balance member country {} is unresolved",
                    row.country
                )
            })?;
            if owner == slot {
                selected = Some(selected.map_or(row.record, |old: u32| old.min(row.record)));
            }
        }
        let records = self
            .frame
            .complete::<tables::PowerBalances>()
            .map_err(|e| format!("{name}: {e}"))?;
        let record = match selected {
            Some(id) => Some(
                records
                    .iter()
                    .find(|r| r.ordinal == id)
                    .ok_or_else(|| format!("{name}: balance record {id} is missing"))?,
            ),
            None => None,
        };
        if which < 6 {
            let Some(record) = record else {
                // Null identities are empty: 140A7C300, 140A7C610, 140A7C800.
                return Ok(Some(String::new()));
            };
            let key = match which {
                0 => &record.balance,
                1 => &record.left,
                2 => &record.right,
                3 if record.value_raw < 0 => &record.left,
                3 => &record.right,
                4 => &record.trending,
                _ => &record.active_range,
            };
            let text = self.localisation.localise(key).map_err(|e| e.to_string())?;
            // 140A7E7B0 starts a fresh empty event; the range vf9 instead
            // receives a fresh country event (140A7E780).
            if which == 5 {
                let context = ScriptedText::new(self.frame, self.localisation, self.content, slot);
                return context.expand(&text).map(Some);
            }
            if text.contains('[') && text.contains(']') {
                return Err(format!(
                    "{name}: the definition name requires scripted localisation in an empty event scope (140A7E7B0), which this country view cannot reproduce"
                ));
            }
            return Ok(Some(text));
        }
        let (mut daily, mut weekly) = (0_i64, 0_i64);
        if let Some(record) = record {
            for m in members.iter().filter(|m| m.record == record.ordinal) {
                let native = engine::power_balance_record::MemberRow {
                    daily_raw: m.daily_raw,
                    weekly_raw: m.weekly_raw,
                    daily_flags: m.daily_flags,
                    weekly_flags: m.weekly_flags,
                    ..Default::default()
                };
                daily = daily.wrapping_add(native.daily_effective());
                weekly = weekly.wrapping_add(native.weekly_effective());
            }
        }
        // 140E41430: signed 64-bit seven-times-daily plus weekly.
        let weekly_change = daily.wrapping_mul(7);
        let weekly_change = weekly_change.wrapping_add(weekly);
        if which == 7 {
            // Executable literals and trailing space at 142700920.
            return Ok(Some(if weekly_change == 0 {
                String::new()
            } else {
                format!(
                    "\u{13}BoP_{}_texticon ",
                    if weekly_change < 0 { "left" } else { "right" }
                )
            }));
        }
        // 140E420B0 preserves its sentinel path and wrapping fixed5 product.
        let converted = if !(i64::MIN + 2..=i64::MAX - 1).contains(&weekly) {
            0xffff_ffff
        } else {
            weekly.wrapping_mul(100_000) / 700_000
        };
        let change = daily.wrapping_add(converted);
        let (key, value) = if change == 0 {
            (b"BOP_CHANGE_RATE_WEEKLY".as_slice(), weekly_change)
        } else {
            (b"BOP_CHANGE_RATE_DAILY".as_slice(), change)
        };
        self.localisation
            .localise_with(key, &[("VALUE", Argument::Fixed5(value.wrapping_abs()))])
            .map(Some)
            .map_err(|e| e.to_string())
    }
}
