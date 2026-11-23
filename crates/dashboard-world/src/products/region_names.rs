use dashboard_base::catalogue;

pub fn region_name(cat: &catalogue::Catalogue, content: &content_reader_regions::regions::StrategicRegions, id: u32) -> Result<String, String> {
    let region = content.regions.get(&id).ok_or_else(|| format!("Region {id} absent from the recorded stack"))?;
    cat.localised(&region.name_key, catalogue::View::Plain).ok_or_else(|| format!("Region {id} name unsupported"))
}
