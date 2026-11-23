
use dashboard_base::products::{Country, validate_country};
use super::{SpendCurrency, naming::undated};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read}}};
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_follow::live::Source as Cell;
use dashboard_text::catalogue::View;
use replay_data::rows::{LexicalTokenRow, PowerSpendCoverageRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, Tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const FIXED5: f64 = 1e-5;
const DAILY: &str = "Daily figures";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PowerDayParams {
    pub branch_id: String,
    pub country: Country,
    pub currency: SpendCurrency,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerDayPart {
    #[doc = " The item as `power-spending` names it."]
    pub item: String,
    pub payments: u32,
    #[doc = " Every payment read while the capture covered the currency."]
    pub covered: bool,
    #[doc = " Its charge was not recorded: the cell is missing."]
    pub unknown: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerDayHold {
    pub id: String,
    #[doc = " The recorded cause of the allocation."]
    pub kind: String,
    pub name: Option<String>,
    pub unknown: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerDayReply {
    pub currency: SpendCurrency,
    #[doc = " The open day's midnight."]
    pub day: i32,
    #[doc = " The day's daily row: its cells (`income`: `gross`, `costs`, `net`, `applied` for political"]
    #[doc = " power, `gain`, `applied` for command power). Why it is missing, when it is."]
    pub income_missing: Option<String>,
    #[doc = " The day's daily row is the open day's (dated `day`)."]
    pub income_day: Option<i32>,
    pub parts: Vec<PowerDayPart>,
    #[doc = " Why the parts are not the day's whole spending: its currency was not covered since midnight."]
    pub parts_missing: Option<String>,
    #[doc = " Command power: the allocations the holder's holds name, and the total its stored counter"]
    #[doc = " holds (`holds`, `total`); what the holds do not name is that less their sum."]
    pub holds: Vec<PowerDayHold>,
    pub holds_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerDayReply);

pub struct PowerDay;

pub type DayContext = Result<Arc<PoliticsContent>, String>;

fn field(table: TableId, name: &str) -> io::Result<usize> {
    dashboard_declare::field(table, name)
}

fn part_range(slot: u32, currency: SpendCurrency) -> (u128, u128) {
    let low = u128::from(slot) << 40 | u128::from(currency.code()) << 32;
    (low, low | u128::from(u32::MAX))
}

impl AtHour for PowerDay {
    const KEY: &'static str = "power-day";
    const TITLE: &'static str = "Power day";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PowerDayParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, currency];
    type Reply = PowerDayReply;
    type Context = DayContext;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid power day branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![PowerDayParams::decl(), PowerDayReply::decl(), PowerDayPart::decl(), PowerDayHold::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![
            TableId::CountryPowerDaily,
            TableId::PowerDayParts,
            TableId::PowerItems,
            TableId::PowerSpendCoverage,
            TableId::PowerHolds,
            TableId::CountryCommandPower,
            TableId::LexicalTokens,
            TableId::Characters,
        ]
    }

    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => {
                let (low, high) = part_range(owner.slot, params.currency);
                let holds = u128::from(owner.slot) << 64;
                vec![
                    Read::these(TableId::CountryPowerDaily, [u128::from(owner.id)]),
                    Read { table: TableId::PowerDayParts, keys: Keys::Range(low, high) },
                    Read::these(TableId::PowerSpendCoverage, [u128::from(owner.slot)]),
                    Read { table: TableId::PowerHolds, keys: Keys::Range(holds, holds | u128::from(u64::MAX)) },
                    Read::these(TableId::CountryCommandPower, [u128::from(owner.slot)]),
                ]
            }
            1 => {
                let items = held.complete::<tables::PowerDayParts>().map(|rows| rows.iter().map(|row| u128::from(row.holder) << 32 | u128::from(row.ordinal)).collect::<Vec<_>>()).unwrap_or_default();
                let holds = held.complete::<tables::PowerHolds>().map(|rows| rows.to_vec()).unwrap_or_default();
                let tokens: BTreeSet<u128> = holds.iter().filter_map(|hold| hold.item_token).map(u128::from).collect();
                let characters: BTreeSet<u128> = holds.iter().filter_map(|hold| hold.character).map(u128::from).collect();
                vec![Read::these(TableId::PowerItems, items), Read::these(TableId::LexicalTokens, tokens), Read::these(TableId::Characters, characters)]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let Some(options) = load.content.as_ref() else { return Ok(Err("Names need the game folders.".into())) };
        match dashboard_base::products::politics_content::politics_content(source, options) {
            Ok(content) => Ok(Ok(content)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => Ok(Err(format!("Politics content is unavailable: {error}"))),
        }
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Power day country is unavailable at this hour"))?;
        day(frame, params.currency, owner, at)
    }
}

fn day(frame: &DecodedFrame, currency: SpendCurrency, owner: Owner, at: &At<'_, DayContext>) -> io::Result<PowerDayReply> {
    let hour = frame.hour;
    let midnight = hour.div_euclid(24) * 24;
    let mut reply = PowerDayReply {
        currency,
        day: midnight,
        income_missing: None,
        income_day: None,
        parts: Vec::new(),
        parts_missing: None,
        holds: Vec::new(),
        holds_missing: None,
        warnings: Vec::new(),
    };
    match frame.complete::<tables::CountryPowerDaily>() {
        Err(error) => reply.income_missing = Some(format!("{DAILY} are not recorded at this hour: {}", undated(&error.to_string()))),
        Ok(rows) => match rows.iter().find(|row| row.country == owner.id) {
            None => reply.income_missing = Some(format!("{DAILY} were not read for this country")),
            Some(row) => {
                reply.income_day = Some(row.day_hour);
                let cell = |name: &str| -> io::Result<Cell> { Ok(Cell { table: TableId::CountryPowerDaily, column: field(TableId::CountryPowerDaily, name)?, key: u128::from(owner.id) }) };
                match currency {
                    SpendCurrency::Political => {
                        at.live.value("income", "gross", cell("pp_gross_gain_raw")?, FIXED5);
                        at.live.value("income", "costs", cell("pp_cost_raw")?, FIXED5);
                        at.live.value("income", "net", cell("pp_net_gain_raw")?, FIXED5);
                        at.live.value("income", "applied", cell("pp_realized_delta_raw")?, FIXED5);
                    }
                    SpendCurrency::Command => {
                        at.live.value("income", "gain", cell("cp_gain_raw")?, FIXED5);
                        at.live.value("income", "applied", cell("cp_realized_delta_raw")?, FIXED5);
                    }
                }
                if row.day_hour != midnight {
                    reply.income_missing = Some(format!("{DAILY} of this day are not read yet"));
                }
            }
        },
    }
    let coverage = frame.complete::<tables::PowerSpendCoverage>().ok().and_then(|rows| rows.iter().find(|row| row.holder == owner.slot).cloned());
    reply.parts_missing = match &coverage {
        None => Some("Spending coverage is not recorded at this hour".into()),
        Some(row) => {
            let (covered, since) = match currency {
                SpendCurrency::Political => (row.political, row.political_since),
                SpendCurrency::Command => (row.command, row.command_since),
            };
            match (covered, since) {
                (true, Some(since)) if since <= midnight => None,
                (true, _) => Some("Spending was not recorded whole since this day began".into()),
                (false, _) => Some(PowerSpendCoverageRow::REASONS.get(usize::from(row.reason)).filter(|text| !text.is_empty()).map_or("Spending was not recorded whole".into(), |text| (*text).to_owned())),
            }
        }
    };
    match frame.complete::<tables::PowerDayParts>() {
        Err(error) => reply.parts_missing = Some(format!("Spending so far today is not recorded at this hour: {}", undated(&error.to_string()))),
        Ok(parts) => {
            let items: BTreeMap<u32, (i32, bool)> = frame.complete::<tables::PowerItems>().map(|rows| rows.iter().map(|row| (row.ordinal, (row.first_read, row.holder == owner.slot))).collect()).unwrap_or_default();
            let spent = field(TableId::PowerDayParts, "spent")?;
            for part in parts.iter().filter(|part| part.holder == owner.slot && part.currency == currency.code() && part.day == midnight) {
                let Some(&(first_read, true)) = items.get(&part.ordinal) else {
                    reply.warnings.push("Politics spending item is not recorded in power items (count: 1)".into());
                    continue;
                };
                let item = format!("{}:{}:{first_read}", part.holder, part.ordinal);
                let key = u128::from(part.holder) << 40 | u128::from(part.currency) << 32 | u128::from(part.ordinal);
                at.live.value(&item, "spent", Cell { table: TableId::PowerDayParts, column: spent, key }, FIXED5);
                reply.parts.push(PowerDayPart {
                    item,
                    payments: u32::try_from(part.payments.max(0)).unwrap_or(u32::MAX),
                    covered: part.covered,
                    unknown: matches!(part.spent, replay_data::rows::Recorded::NotRecorded),
                });
            }
        }
    }
    if currency == SpendCurrency::Command {
        holds(frame, owner, at, &mut reply)?;
    }
    Ok(reply)
}

fn holds(frame: &DecodedFrame, owner: Owner, at: &At<'_, DayContext>, reply: &mut PowerDayReply) -> io::Result<()> {
    match frame.complete::<tables::CountryCommandPower>() {
        Ok(rows) if rows.iter().any(|row| row.canonical_slot == owner.slot) => {
            let total = Cell { table: TableId::CountryCommandPower, column: field(TableId::CountryCommandPower, "allocated_command_power_raw")?, key: u128::from(owner.slot) };
            at.live.value("holds", "total", total, FIXED5);
        }
        Ok(_) => reply.holds_missing = Some("Command power held in reserve was not read for this country".into()),
        Err(error) => reply.holds_missing = Some(format!("Command power held in reserve is not recorded at this hour: {}", undated(&error.to_string()))),
    }
    let rows = match frame.complete::<tables::PowerHolds>() {
        Ok(rows) => rows,
        Err(error) => {
            reply.holds_missing.get_or_insert(format!("Command power holds are not recorded at this hour: {}", undated(&error.to_string())));
            return Ok(());
        }
    };
    let amount = field(TableId::PowerHolds, "amount")?;
    let tokens = frame.complete::<tables::LexicalTokens>().ok();
    let characters = frame.complete::<tables::Characters>().ok();
    for hold in rows.iter().filter(|hold| hold.holder == owner.slot) {
        let id = format!("{:016x}", hold.hold);
        let key = u128::from(hold.holder) << 64 | u128::from(hold.hold);
        at.live.value(&id, "amount", Cell { table: TableId::PowerHolds, column: amount, key }, FIXED5);
        let token = hold.item_token.and_then(|token| {
            let rows = tokens?;
            let at = rows.binary_search_by_key(&token, |row| row.token).ok()?;
            std::str::from_utf8(&rows[at].literal).ok().map(str::to_owned)
        });
        let character = hold.character.and_then(|id| {
            let rows = characters?;
            let at = rows.binary_search_by_key(&id, |row| row.id).ok()?;
            let row = &rows[at];
            let lexical = frame.complete_rows::<tables::LexicalTokens>(replay_data::rows::mask::<LexicalTokenRow>(&["literal"])).ok();
            let script = lexical.and_then(|lexical| dashboard_text::identity::character_script_token_in(row, lexical)).map(str::to_owned);
            Some((script, std::str::from_utf8(&row.token).ok()?.to_owned()))
        });
        let named = match at.context {
            Err(reason) => Err(reason.clone()),
            Ok(content) => holder_name(hold.ability, token.as_deref(), character.as_ref().map(|(script, saved)| (script.as_deref(), saved.as_str())), content),
        };
        let (name, unknown) = match named {
            Ok(name) => (Some(name), None),
            Err(reason) => (None, Some(reason)),
        };
        reply.holds.push(PowerDayHold { id, kind: if hold.ability { "Ability".into() } else { format!("{:?}", hold.cause) }, name, unknown });
    }
    Ok(())
}

fn holder_name(ability: bool, token: Option<&str>, character: Option<(Option<&str>, &str)>, content: &PoliticsContent) -> Result<String, String> {
    if ability {
        let token = token.ok_or("ability holder identity is not recorded")?;
        let power = content.power_content()?;
        let key = power.abilities.name_key(token.as_bytes()).ok_or_else(|| format!("ability {token}: name key is not registered in recorded content"))?;
        return content.localised(key, View::Plain).ok_or_else(|| format!("ability {token}: name is not localised"));
    }
    if let Some((script, saved)) = character {
        return content.character_name(script, saved).ok_or_else(|| "recorded holder character name is not identified".into());
    }
    Err("holder naming form is not identified".into())
}
