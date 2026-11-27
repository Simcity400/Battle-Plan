use dashboard_base::catalogue;

pub fn region_name(cat: &catalogue::Catalogue, content: &content_reader_regions::regions::StrategicRegions, id: u32) -> Result<String, String> {
    let region = content.regions.get(&id).ok_or_else(|| format!("Region {id} absent from the recorded stack"))?;
    cat.localised_or_why(&region.name_key, catalogue::View::Scripted)
        .map_err(|why| format!("Region {id}: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_regions::regions::{StrategicRegion, StrategicRegions};

    #[test]
    fn region_name_distinguishes_absent_key_from_scripted_expression() {
        let (cat, _dir) = dashboard_base::catalogue::tests::folder_catalogue(&[(
            "localisation/english/regions_l_english.yml",
            "\u{feff}l_english:\n REG_PLAIN:0 \"Northern Sea\"\n REG_SCRIPTED:0 \"[Root.GetName] Sector\"\n",
        )]);
        let mut regions = StrategicRegions::default();
        for (id, key) in [(1, "REG_PLAIN"), (2, "REG_MISSING"), (3, "REG_SCRIPTED")] {
            regions.regions.insert(id, StrategicRegion { id, name_key: key.to_owned(), ..Default::default() });
        }
        assert_eq!(region_name(&cat, &regions, 1), Ok("Northern Sea".to_owned()));
        let absent = region_name(&cat, &regions, 2).unwrap_err();
        assert!(absent.contains("REG_MISSING") && absent.contains("missing"), "{absent}");
        let scripted = region_name(&cat, &regions, 3).unwrap_err();
        assert!(scripted.contains("REG_SCRIPTED") && scripted.contains("[Root.GetName]"), "{scripted}");
        assert_ne!(absent, scripted);
    }
}
