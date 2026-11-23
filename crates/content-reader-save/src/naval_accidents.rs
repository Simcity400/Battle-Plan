use super::{
    CountriesView, GameDate, block, boolean, bytes, field, integer, invalid, required_bytes,
    typed_handle,
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub kind: u8,
    pub ordinal: u32,
    pub region: u32,
    pub equipment: u64,
    pub ship: u64,
    pub sunk: bool,
    pub countries: Vec<Vec<u8>>,
    pub expire_date: GameDate,
}
pub(super) fn parse(countries: CountriesView<'_>) -> io::Result<BTreeMap<Vec<u8>, Vec<Report>>> {
    let mut result = BTreeMap::new();
    for country in countries.iter() {
        let tag = country
            .key
            .clone()
            .ok_or_else(|| invalid("naval incident country has no tag"))?;
        let items = block(&country.value)?;
        let mut reports = Vec::new();
        if let Some(navy) = field(items, b"strategic_navy")? {
            let mut ordinals = [0_u32; 2];
            for item in block(navy)? {
                let kind = match item.key.as_deref() {
                    Some(b"naval_accident") => 0,
                    Some(b"mine_report") => 1,
                    _ => continue,
                };
                let fields = block(&item.value)?;
                let tags = fields
                    .iter()
                    .filter(|item| item.key.as_deref() == Some(b"country"))
                    .map(|item| super::scalar(&item.value).map(<[u8]>::to_vec))
                    .collect::<io::Result<Vec<_>>>()?;
                if tags.len() != usize::from(kind) + 1 {
                    return Err(invalid(
                        "naval incident country-key count differs from serializer",
                    ));
                }
                let ordinal = ordinals[usize::from(kind)];
                ordinals[usize::from(kind)] = ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("naval incident ordinal overflow"))?;
                reports.push(Report {
                    kind,
                    ordinal,
                    region: integer(&required_bytes(fields, b"region")?)?,
                    equipment: typed_handle(
                        field(fields, b"equipment")?
                            .ok_or_else(|| invalid("naval incident equipment missing"))?,
                    )?,
                    ship: typed_handle(
                        field(fields, b"ship")?
                            .ok_or_else(|| invalid("naval incident ship missing"))?,
                    )?,
                    sunk: field(fields, b"is_sunk")?
                        .map(boolean)
                        .transpose()?
                        .unwrap_or(false),
                    countries: tags,
                    expire_date: GameDate::parse(
                        &bytes(fields, b"to_discard_date")?
                            .ok_or_else(|| invalid("naval incident expiry date missing"))?,
                    )?,
                });
            }
        }
        if result.insert(tag, reports).is_some() {
            return Err(invalid("duplicate naval incident country tag"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(input: &[u8]) -> io::Result<BTreeMap<Vec<u8>, Vec<Report>>> {
        let tree = content_reader_parser::parser::parse_save(input)?;
        parse(CountriesView::from_root(&tree.items)?)
    }
    #[test]
    fn preserves_report_order_repeats_and_duplicate_mine_country_keys() {
        let pack = sample(br#"countries={MOD={strategic_navy={
          naval_accident={region=9001 equipment={type=70 id=4} ship={type=51 id=8} country=MOD to_discard_date=1943.5.2.1}
          mine_report={region=9001 equipment={type=70 id=4} ship={type=51 id=8} is_sunk=yes country=MOD to_discard_date=1943.5.2.1 country=ZZZ}
          naval_accident={region=9001 equipment={type=70 id=4} ship={type=51 id=8} country=MOD to_discard_date=1943.5.2.1}
        }}}"#).unwrap();
        let reports = &pack[b"MOD".as_slice()];
        assert_eq!(
            reports
                .iter()
                .map(|r| (r.kind, r.ordinal))
                .collect::<Vec<_>>(),
            [(0, 0), (1, 0), (0, 1)]
        );
        assert_eq!(reports[1].countries, [b"MOD".to_vec(), b"ZZZ".to_vec()]);
        assert!(reports[1].sunk);
        assert!(!reports[0].sunk);
        assert_eq!(reports[0].equipment, 70 | (4_u64 << 32));
    }
    #[test]
    fn refuses_missing_unconditional_operands_and_wrong_mine_country_multiplicity() {
        assert!(sample(br#"countries={MOD={strategic_navy={mine_report={region=1 equipment={type=70 id=4} ship={type=51 id=8} country=MOD to_discard_date=1943.5.2.1}}}}"#).is_err());
        assert!(sample(br#"countries={MOD={strategic_navy={naval_accident={region=1 country=MOD to_discard_date=1943.5.2.1}}}}"#).is_err());
    }
}
