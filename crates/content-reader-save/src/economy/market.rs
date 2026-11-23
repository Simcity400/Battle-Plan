use super::*;
use crate::{
    SavedDate,
    production::{self, Pool},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub variant: u64,
    pub level: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subsidy {
    pub cic: Decimal,
    pub archetype: Vec<u8>,
    pub targets: Option<Vec<Vec<u8>>>,
    pub trigger: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryMarket {
    pub inventory: Pool,
    pub prices: Vec<Level>,
    pub subsidies: Vec<Subsidy>,
    pub auto_send_market_access: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terms {
    pub runtime: Option<Runtime>,
    pub request: bool,
    pub id: u64,
    pub seller: Vec<u8>,
    pub buyer: Vec<u8>,
    pub speed: i32,
    pub equipment: Pool,
    pub prices: Pool,
    pub levels: Vec<Level>,
    pub subsidies: Vec<Subsidy>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseTerms {
    pub monthly: Pool,
    pub percentage: Pool,
    pub once: Pool,
    pub active_since: i32,
    pub last_delivery: i32,
    pub fuel_daily: i64,
    pub fuel_percentage: Decimal,
    pub fuel_sent: i64,
    pub fuel_sunk: i64,
    pub last_fuel_delivered: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub route_equipment: Pool,
    pub route_length: i32,
    pub route_type: u8,
    pub route_request: i32,
    pub route_country: Vec<u8>,
    pub route_sunk_convoys: Decimal,
    pub period_days: i32,
    pub meta_cic: Decimal,
    pub meta_convoys: Decimal,
    pub meta_efficiency: Decimal,
    pub history_factory: Decimal,
    pub history_equipment: Decimal,
    pub history_collected: Decimal,
    pub variables: Vec<(Vec<u8>, Decimal)>,
    pub variable_state: Option<[u32; 2]>,
}
fn runtime(fields: &[Item]) -> io::Result<Option<Runtime>> {
    let Some(meta) = field(fields, b"contract_meta")? else {
        return Ok(None);
    };
    let meta = block(meta)?;
    let history = required_block(meta, b"history")?;
    let route = required_block(fields, b"delivery_route_handler")?;
    let client = required_block(route, b"convoy_client")?;
    let mut variables = Vec::new();
    let mut variable_state = None;
    for entry in required_block(fields, b"variables")? {
        let name = entry
            .key
            .clone()
            .ok_or_else(|| invalid("market contract variable lacks name"))?;
        if name == b"random"
            && let Some(words) = entry.value.items()
        {
            if words.len() != 2 || words.iter().any(|word| word.key.is_some()) {
                return Err(invalid("market variable random state needs two words"));
            }
            if variable_state
                .replace([
                    integer::<u32>(scalar(&words[0].value)?)?,
                    integer::<u32>(scalar(&words[1].value)?)?,
                ])
                .is_some()
            {
                return Err(invalid("duplicate market variable random state"));
            }
        } else {
            variables.push((name, Decimal::parse(scalar(&entry.value)?)?));
        }
    }
    Ok(Some(Runtime {
        route_equipment: production::parse_pool(required_block(client, b"equipments")?)?,
        route_length: required_integer(client, b"length")?,
        route_type: required_integer(client, b"type")?,
        route_request: required_integer(client, b"request")?,
        route_country: required_bytes(client, b"country")?,
        route_sunk_convoys: required_decimal(route, b"sunk_convoys")?,
        period_days: required_integer(fields, b"days")?,
        meta_cic: required_decimal(meta, b"cic")?,
        meta_convoys: required_decimal(meta, b"convoys")?,
        meta_efficiency: required_decimal(meta, b"efficiency_due_to_lost_convoys")?,
        history_factory: required_decimal(history, b"factory_cic_progress")?,
        history_equipment: required_decimal(history, b"equipment_transfer_progress")?,
        history_collected: required_decimal(history, b"collected")?,
        variables,
        variable_state,
    }))
}
pub fn levels(fields: &[Item]) -> io::Result<Vec<Level>> {
    let mut result = Vec::new();
    for entry in required_block(fields, b"levels")? {
        if entry.key.is_some() {
            return Err(invalid("market price pair must be unkeyed"));
        }
        let pair = block(&entry.value)?;
        if pair.len() != 2 || pair.iter().any(|entry| entry.key.is_some()) {
            return Err(invalid("market price pair must hold reference and level"));
        }
        result.push(Level {
            variant: typed_handle(&pair[0].value)?,
            level: scalar(&pair[1].value)?.to_vec(),
        });
    }
    Ok(result)
}
pub fn subsidies(fields: &[Item]) -> io::Result<Vec<Subsidy>> {
    let Some(entries) = field(fields, b"subsidies")? else {
        return Ok(Vec::new());
    };
    block(entries)?
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("market subsidy entry must be unkeyed"));
            }
            let fields = block(&entry.value)?;
            let targets = field(fields, b"targets")?
                .map(|value| {
                    block(value)?
                        .iter()
                        .map(|entry| {
                            if entry.key.is_some() {
                                return Err(invalid("market subsidy target must be unkeyed"));
                            }
                            Ok(scalar(&entry.value)?.to_vec())
                        })
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?;
            let trigger = bytes(fields, b"trigger")?;
            if targets.is_some() && trigger.is_some() {
                return Err(invalid("market subsidy contains both alternatives"));
            }
            Ok(Subsidy {
                cic: required_decimal(fields, b"cic")?,
                archetype: required_bytes(fields, b"archetype")?,
                targets,
                trigger,
            })
        })
        .collect()
}
pub fn country(fields: &[Item]) -> io::Result<Option<CountryMarket>> {
    let Some(value) = field(fields, b"equipment_market")? else {
        return Ok(None);
    };
    let fields = block(value)?;
    let automation = required_block(
        required_block(fields, b"market_request_automation")?,
        b"option",
    )?;
    let auto_send_market_access = field(automation, b"auto_send_market_access")?
        .map(boolean)
        .transpose()?
        .ok_or_else(|| invalid("market automation flag absent"))?;
    Ok(Some(CountryMarket {
        inventory: production::parse_pool(required_block(
            required_block(fields, b"market_stockpile")?,
            b"equipments",
        )?)?,
        prices: levels(required_block(fields, b"price_levels")?)?,
        subsidies: subsidies(required_block(fields, b"subsidies")?)?,
        auto_send_market_access,
    }))
}
pub fn definition(id: u64, fields: &[Item]) -> io::Result<Terms> {
    let draft = required_block(fields, b"contract_draft")?;
    Ok(Terms {
        runtime: None,
        request: false,
        id,
        seller: required_bytes(draft, b"seller")?,
        buyer: required_bytes(draft, b"buyer")?,
        speed: required_integer(draft, b"speed")?,
        equipment: production::parse_pool(required_block(draft, b"equipments")?)?,
        prices: production::parse_pool(required_block(fields, b"prices")?)?,
        levels: levels(required_block(fields, b"price_levels")?)?,
        subsidies: subsidies(required_block(draft, b"subsidies")?)?,
    })
}
pub fn terms(items: &[Item]) -> io::Result<Option<Vec<Terms>>> {
    let Some(value) = field(items, b"equipment_market")? else {
        return Ok(None);
    };
    let fields = block(value)?;
    let mut result = Vec::new();
    for entry in required_block(fields, b"contracts")? {
        if entry.key.as_deref() != Some(b"purchase_contract") {
            return Err(invalid("unexpected market contract"));
        }
        let fields = block(&entry.value)?;
        let id = typed_handle(
            field(fields, b"id")?.ok_or_else(|| invalid("market contract lacks id"))?,
        )?;
        let mut row = definition(id, required_block(fields, b"contract_definition")?)?;
        row.runtime = runtime(fields)?;
        result.push(row);
    }
    let requests = required_block(fields, b"requests")?;
    let Some(size) = requests.first().filter(|entry| entry.key.is_none()) else {
        return Err(invalid("market requests lacks sparse size"));
    };
    let _size = integer::<u32>(scalar(&size.value)?)?;
    for entry in &requests[1..] {
        if entry.key.is_some() {
            return Err(invalid("market request sparse entry must be unkeyed"));
        }
        let holder = block(&entry.value)?;
        let _index = required_integer::<u32>(holder, b"index")?;
        if let Some(entries) = field(holder, b"value")? {
            for request in block(entries)? {
                if request.key.is_some() {
                    return Err(invalid("market request must be unkeyed"));
                }
                let fields = block(&request.value)?;
                let id = typed_handle(
                    field(fields, b"id")?.ok_or_else(|| invalid("market request lacks id"))?,
                )?;
                let mut row = definition(id, required_block(fields, b"contract_definition")?)?;
                row.request = true;
                result.push(row);
            }
        }
    }
    Ok(Some(result))
}
pub fn lease(fields: &[Item]) -> io::Result<Option<LeaseTerms>> {
    if field(fields, b"equipment_need")?.is_none() {
        return Ok(None);
    };
    let pool = |key| production::parse_pool(required_block(fields, key)?);
    let date = |key| SavedDate::parse(&required_bytes(fields, key)?)?.engine_hour();
    let fuel = |key| FuelQuantity::parse(&required_bytes(fields, key)?)?.raw();
    Ok(Some(LeaseTerms {
        monthly: pool(b"equipment_need")?,
        percentage: pool(b"percentage_need")?,
        once: pool(b"once_need")?,
        active_since: date(b"active_since")?,
        last_delivery: date(b"last_delivery_date")?,
        fuel_daily: fuel(b"fuel_daily")?,
        fuel_percentage: required_decimal(fields, b"fuel_percentage")?,
        fuel_sent: fuel(b"fuel_sent")?,
        fuel_sunk: fuel(b"fuel_sunk")?,
        last_fuel_delivered: fuel(b"last_fuel_delivered")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn market_save_runtime_keeps_history_route_and_literal_random_variable_independent() {
        let document = content_reader_parser::parser::parse(br#"
            days=19
            contract_meta={cic=4.00004 convoys=5.00005 efficiency_due_to_lost_convoys=0.75
                history={factory_cic_progress=1.00001 equipment_transfer_progress=-2.00002 collected=3.00003}}
            delivery_route_handler={sunk_convoys=6.00006 convoy_client={
                length=7 type=2 request=9 country=BB
                equipments={allow_zero_entries=yes equipment={id={id=7 type=70} amount=-0.00001}}}}
            variables={random=0.0 ""=-0.00001 random={4294967295 17}}
        "#).unwrap();
        let value = runtime(&document.items).unwrap().unwrap();
        assert_eq!(
            (
                value.period_days,
                value.route_length,
                value.route_type,
                value.route_request
            ),
            (19, 7, 2, 9)
        );
        assert_eq!(value.route_country, b"BB");
        assert_eq!(value.route_equipment.allow_zero_entries, Some(true));
        assert_eq!(value.route_equipment.equipment[0].amount.raw, -1);
        assert_eq!(
            [
                value.meta_cic.raw,
                value.meta_convoys.raw,
                value.meta_efficiency.raw,
                value.history_factory.raw,
                value.history_equipment.raw,
                value.history_collected.raw,
                value.route_sunk_convoys.raw
            ],
            [400004, 500005, 75000, 100001, -200002, 300003, 600006]
        );
        assert_eq!(
            value
                .variables
                .iter()
                .map(|(key, value)| (key.as_slice(), value.raw))
                .collect::<Vec<_>>(),
            vec![(b"random".as_slice(), 0), (b"".as_slice(), -1)]
        );
        assert_eq!(value.variable_state, Some([u32::MAX, 17]));
    }
    #[test]
    fn market_save_preserves_requested_items_independent_prices_and_subsidy_alternatives() {
        let document=content_reader_parser::parser::parse(br#"equipment_market={contracts={purchase_contract={id={id=17 type=81} contract_definition={contract_draft={seller=AA buyer=BB speed=3 equipments={allow_zero_entries=yes equipment={id={id=7 type=70} amount=-0.00001}} subsidies={subsidies={ {cic=90071992547.40993 archetype=synthetic targets={AA BB AA}} {cic=-1 archetype=synthetic trigger=""} }}} prices={allow_zero_entries=no equipment={id={id=7 type=70} amount=7.5}} price_levels={levels={ {{id=7 type=70} low} }}}}} requests={5001}}"#).unwrap();
        let parsed = terms(&document.items).unwrap().unwrap();
        assert_eq!(parsed[0].equipment.equipment[0].amount.raw, -1);
        assert_eq!(parsed[0].prices.equipment[0].amount.raw, 750000);
        assert_eq!(parsed[0].levels[0].level, b"low");
        assert_eq!(parsed[0].subsidies[0].cic.raw, 9007199254740993);
        assert_eq!(parsed[0].subsidies[0].targets.as_ref().unwrap().len(), 3);
        assert_eq!(parsed[0].subsidies[1].trigger, Some(Vec::new()));
    }
    #[test]
    fn market_save_keeps_pending_requests_and_empty_country_holders() {
        let document=content_reader_parser::parser::parse(br#"equipment_market={contracts={} requests={5001 {index=5000 value={ {id={id=2 type=82} contract_definition={contract_draft={seller=AA buyer=BB speed=0 equipments={allow_zero_entries=no} subsidies={}} prices={allow_zero_entries=yes} price_levels={levels={}}}} }}}}"#).unwrap();
        let parsed = terms(&document.items).unwrap().unwrap();
        assert!(parsed[0].request);
        assert_eq!(parsed[0].id, (2_u64 << 32) | 82);
        let document=content_reader_parser::parser::parse(br#"equipment_market={market_stockpile={equipments={allow_zero_entries=no}} price_levels={levels={}} subsidies={} market_request_automation={option={auto_send_market_access=yes}}}"#).unwrap();
        let parsed = country(&document.items).unwrap().unwrap();
        assert!(parsed.inventory.equipment.is_empty());
        assert!(parsed.auto_send_market_access);
        assert!(country(&[]).unwrap().is_none());
    }
}
