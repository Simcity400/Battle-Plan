use super::*;
pub fn regions(map: &MapData, frame: &DecodedFrame) -> io::Result<Regions> {
    let countries = core::<tables::Countries>(frame)?
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect::<BTreeMap<_, _>>();
    let links = core::<tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect::<BTreeMap<_, _>>();
    let provinces = core::<tables::Provinces>(frame)?;
    let count = province_table_len(map, provinces)?;
    let mut result = Regions {
        country_of: vec![NO_REGION; count].into(),
        owner_of: vec![NO_REGION; count].into(),
        state_of: vec![NO_REGION; count].into(),
        country_colors: Default::default(),
        warnings: Vec::new(),
        strategic_attributes: BTreeMap::new(),
        mode: None,
    };
    let mut unresolved = Vec::new();
    let mut uncoloured = BTreeSet::new();
    let mut observed = BTreeSet::new();
    let inherited = inherited_colors(map, frame, &links, &mut result.warnings)?;
    let appearance = rows::<tables::CountryAppearance>(frame)?;
    let gap = match &appearance {
        Available::Complete(_) => "CountryAppearance has no row for them".to_owned(),
        Available::Truncated { missing, .. } => {
            format!("CountryAppearance is truncated ({missing} missing rows)")
        }
        Available::Unavailable { reason } => format!("CountryAppearance is unavailable: {reason}"),
        Available::NotRecorded { mode } => {
            format!("CountryAppearance is not recorded in a {mode} recording")
        }
    };
    let recorded = appearance
        .rows()
        .iter()
        .map(|r| (r.country, r.map_color()))
        .collect::<BTreeMap<_, _>>();
    let mut colors = HashMap::new();
    for province in provinces {
        let definition = map.definitions.get(&province.id).ok_or_else(|| {
            invalid(format!(
                "recorded province {} is absent from map/definition.csv",
                province.id
            ))
        })?;
        observed.insert(province.id);
        if definition.kind == ProvinceKind::Sea
            || (definition.kind == ProvinceKind::Lake && province.controller == 0)
        {
            continue;
        }
        let index = province.id as usize;
        if province.owner != 0 {
            result.owner_of[index] = slot_of(&links, province.owner, "owner")?
                .checked_add(1)
                .ok_or_else(|| invalid("owner country slot exceeds the border key range"))?;
        }
        result.state_of[index] = province.state;
        if province.controller == 0 {
            unresolved.push(province.id);
            continue;
        }
        let slot = slot_of(&links, province.controller, "controller")?;
        let country = countries.get(&slot).ok_or_else(|| {
            invalid(format!(
                "political fill country link {} resolves to missing country slot {slot}",
                province.controller
            ))
        })?;
        let color = match colors.get(&slot) {
            Some(&color) => color,
            None => {
                let color = color_of(slot, &countries, &inherited, &recorded)?;
                colors.insert(slot, color);
                color
            }
        };
        let color = match color {
            None => UNAVAILABLE_COLOR,
            Some(color) => color.unwrap_or_else(|| {
                uncoloured.insert(String::from_utf8_lossy(&country.tag).into_owned());
                UNAVAILABLE_COLOR
            }),
        };
        let key = slot
            .checked_add(1)
            .ok_or_else(|| invalid("country slot exceeds the gradient border region key range"))?;
        result.country_colors = result.country_colors.inserted(key, color)?;
        result.country_of[index] = key;
    }
    for &id in map.definitions.keys() {
        if id != 0 && !observed.contains(&id) {
            return Err(invalid(format!(
                "hour {} has no recorded row for map province {id}",
                frame.hour
            )));
        }
    }
    if !unresolved.is_empty() {
        result.warnings.push(format!(
            "{} land provinces have a zero controller reference and take no political fill; province ids {}",
            unresolved.len(),
            examples(unresolved.iter().map(u32::to_string))
        ));
    }
    if !uncoloured.is_empty() {
        result.warnings.push(format!(
            "{} countries have no recorded colour this hour ({gap}) and are filled grey: {}",
            uncoloured.len(),
            examples(uncoloured.into_iter())
        ));
    }
    Ok(result)
}

fn examples(items: impl Iterator<Item = String>) -> String {
    let items = items.collect::<Vec<_>>();
    let mut text = items
        .iter()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > 12 {
        text.push_str(", …");
    }
    text
}

fn province_table_len(map: &MapData, provinces: &[ProvinceRow]) -> io::Result<usize> {
    let largest = map
        .definitions
        .last_key_value()
        .map(|(&id, _)| id)
        .unwrap_or(0)
        .max(provinces.iter().map(|row| row.id).max().unwrap_or(0));
    usize::try_from(largest)
        .ok()
        .and_then(|id| id.checked_add(1))
        .ok_or_else(|| invalid("province ids exceed address space"))
}
