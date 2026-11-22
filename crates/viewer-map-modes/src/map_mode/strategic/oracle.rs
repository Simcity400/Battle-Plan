use super::*;
impl Content {
    pub(in crate::map_mode) fn oracle_layer(
        &self,
        mode: Stock,
        inputs: &Inputs,
        warnings: &mut Vec<String>,
    ) -> Layer {
        let air = mode == Stock::StrategicAir;
        let mut layer = Layer {
            region_of: vec![
                0;
                inputs
                    .map
                    .definitions
                    .keys()
                    .next_back()
                    .map_or(1, |id| *id as usize + 1)
            ]
            .into(),
            naval_fx: !air,
            ..Default::default()
        };
        let colors = if air {
            self.oracle_air_colors(inputs)
        } else {
            Ok(BTreeMap::new())
        };
        let mut dense = layer.region_of.as_slice().to_vec();
        let mut styles = BTreeMap::new();
        let mut refused = BTreeMap::<String, usize>::new();
        for (&id, region) in &self.regions {
            let Some(key) = id.checked_add(1) else {
                *refused
                    .entry("region identity cannot be represented".into())
                    .or_default() += 1;
                continue;
            };
            for province in &region.provinces {
                if inputs
                    .map
                    .definitions
                    .get(province)
                    .is_some_and(|p| air || p.kind == ProvinceKind::Sea)
                {
                    dense[*province as usize] = key;
                }
            }
            match colors.as_ref().map_err(Clone::clone).and_then(|colors| {
                colors
                    .get(&id)
                    .cloned()
                    .unwrap_or(Ok(if air { self.air[3] } else { self.navy }))
            }) {
                Ok(color) => {
                    styles.insert(key, style(color));
                }
                Err(reason) => {
                    *refused.entry(reason).or_default() += 1;
                }
            }
        }
        for (reason, count) in refused {
            warnings.push(format!(
                "{} map mode: {count} strategic-region colours withheld: {reason}",
                if air { "Air" } else { "Navy" }
            ));
        }
        layer.region_of = dense.into();
        layer.styles = styles.try_into().unwrap();
        layer
    }

    fn oracle_air_colors(
        &self,
        inputs: &Inputs,
    ) -> Result<BTreeMap<u32, Result<[f32; 4], String>>, String> {
        let selected = inputs.selected.ok_or("viewing country is absent")?;
        let countries = super::super::Countries::read(inputs.frame).map_err(|e| e.to_string())?;
        countries
            .raw
            .get(&selected)
            .ok_or("viewing country has no Countries row")?;
        let rows = inputs
            .frame
            .complete::<tables::RaidAirScores>()
            .map_err(|e| e.to_string())?;
        let mut counts_allowed = None;
        let mut out = BTreeMap::new();
        for row in rows
            .iter()
            .filter(|row| countries.slot(row.country) == Some(selected))
        {
            let friendly = if row.friendly == 0 && row.enemy == 0 && row.friendly_missions != 0 {
                counts_allowed
                    .get_or_insert_with(|| {
                        let counter = inputs
                            .frame
                            .complete::<tables::CountryCounterInputs>()
                            .map_err(|e| e.to_string())?
                            .iter()
                            .find(|row| countries.slot(row.country) == Some(selected))
                            .ok_or("viewing country's owned-state count is absent")?;
                        if counter.owned_state_count < 1 {
                            return Ok(false);
                        }
                        if selected == 0 {
                            return Err("Country0's enemy roster is not recorded".into());
                        }
                        Ok(!replay_derive::directed_war_derive::ReceiverWars::read(
                            inputs.frame,
                            selected,
                        )?
                        .targets()
                        .is_empty())
                    })
                    .clone()
            } else {
                Ok(false)
            };
            let color = friendly.and_then(|counts| {
                air_color(row, if counts { row.friendly_missions } else { 0 })
                    .map(|index| self.air[index])
            });
            out.insert(row.region, color);
        }
        Ok(out)
    }
}
