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
    // Stored primary colour is already cosmetic-resolved and HSV-modified; pass it directly to the
    // gradient builder.
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
        // Owned/controller lakes belong to the country field. Province-region lookup has no terrain-
        // kind exclusion. Treating these lakes as sea adds false shore borders; unassigned lakes
        // remain outside the layer.
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
        let color = color.unwrap_or_else(|| {
            uncoloured.insert(String::from_utf8_lossy(&country.tag).into_owned());
            UNAVAILABLE_COLOR
        });
        // `NO_REGION` is zero, so a region key is the slot plus one and slot zero stays addressable.
        let key = slot
            .checked_add(1)
            .ok_or_else(|| invalid("country slot exceeds the gradient border region key range"))?;
        result.country_colors = result.country_colors.inserted(key, color)?;
        result.country_of[index] = key;
    }
    // A Provinces table that parses can still be short of the map; nothing downstream would notice,
    // so a province the hour never mentions is refused here rather than drawn unfilled.
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

/// The political fill's colour sources: [`inheritance`], each undecided country filled with its
/// own colour and named in `warnings`.
pub fn inherited_colors(
    map: &MapData,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    warnings: &mut Vec<String>,
) -> io::Result<BTreeMap<u32, u32>> {
    let inheritance = inheritance(map, frame, links)?;
    if inheritance.absent.is_some() {
        for slot in inheritance.subjects.difference(&inheritance.with_status) {
            warnings.push(format!(
                "Subject slot {slot} current autonomy is unavailable; its own colour is used"
            ));
        }
    }
    for (slot, why) in &inheritance.undecided {
        warnings.push(format!(
            "Country slot {slot}'s colour source is unknown ({why}); its own colour is used"
        ));
    }
    Ok(inheritance.inherited)
}

/// Controller colour from its resolved colour source, or None when the hour has no colour for that
/// source. Follow subject colour inheritance to the end of the overlord chain.
pub fn color_source(slot: u32, inherited: &BTreeMap<u32, u32>) -> io::Result<u32> {
    let mut source = slot;
    let mut visited = BTreeSet::new();
    while let Some(&overlord) = inherited.get(&source) {
        if !visited.insert(source) {
            return Err(invalid("cycle in subject colour inheritance"));
        }
        source = overlord;
    }
    Ok(source)
}

fn color_of(
    slot: u32,
    countries: &BTreeMap<u32, &CountryRow>,
    inherited: &BTreeMap<u32, u32>,
    recorded: &BTreeMap<u32, [u8; 3]>,
) -> io::Result<Option<[u8; 3]>> {
    let source = color_source(slot, inherited)?;
    let country = countries.get(&source).ok_or_else(|| {
        invalid(format!(
            "political fill country slot {source} has no Countries row"
        ))
    })?;
    Ok(recorded.get(&country.id).copied())
}

/// The loader resolves every bitmap ID through definitions, so this never scans pixels per hour.
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
