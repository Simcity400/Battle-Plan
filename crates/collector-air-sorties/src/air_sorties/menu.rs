use super::*;

impl Sorties {
    pub(super) fn menu_modifiers<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
        cached: &mut Cached,
        observer: &mut impl RegionObserver,
    ) -> Result<(), &'static str> {
        let manager = *cached
            .weather_manager
            .get_or_insert_with(|| native::weather_manager(memory, base, game_state));
        let regions = cached
            .weather
            .or_else(|| native::weather_regions_from_manager(memory, manager.ok()?).ok());
        if let Some(regions) = regions {
            cached.weather = Some(regions);
            for region in 1..regions.count {
                let first = (self.modifiers.len, self.modifiers.required);
                let complete = self
                    .menu_operand(
                        memory,
                        base,
                        game_state,
                        countries,
                        cached,
                        Key {
                            scope: Scope::Weather,
                            region,
                            ..Default::default()
                        },
                        observer,
                    )
                    .is_ok();
                self.menu_status(first, 0, region, complete)?;
            }
        }
        for slot in 1..countries.count() {
            let first = (self.modifiers.len, self.modifiers.required);
            let country = match countries.at(slot) {
                Ok(Some(country)) if country.id as i32 > 0 => country,
                Ok(_) => continue,
                Err(_) => {
                    self.menu_status(first, slot, 0, false)?;
                    continue;
                }
            };
            let complete = (|| {
                self.menu_operand(
                    memory,
                    base,
                    game_state,
                    countries,
                    cached,
                    Key {
                        scope: Scope::Country,
                        slot,
                        country: country.pointer,
                        ..Default::default()
                    },
                    observer,
                )?;
                let map =
                    map_air_country_region_modifier::country_map(memory, base, country.pointer)?;
                cached.region_map = Some((country.pointer, map));
                map.visit(memory, base, |pointer, region, object| {
                    let key = Key {
                        scope: Scope::CountryRegion,
                        slot,
                        region,
                        object: pointer,
                        country: country.pointer,
                        ..Default::default()
                    };
                    if self.keys.rows[..self.keys.len]
                        .binary_search_by_key(&key.order(), Key::order)
                        .is_ok()
                    {
                        return Ok(());
                    }
                    observer.observed(country.pointer, pointer, Some(object));
                    let raw = object.efficiency(memory)?.raw;
                    if raw != 0 {
                        self.modifiers.push(AirSortieModifierRow {
                            scope: AirSortieModifierRow::COUNTRY_REGION,
                            subject: slot,
                            region,
                            raw,
                        })?;
                    }
                    Ok(())
                })
            })()
            .is_ok();
            self.menu_status(first, slot, 0, complete)?;
        }
        Ok(())
    }

    fn menu_status(
        &mut self,
        first: (usize, usize),
        country: u32,
        region: u32,
        complete: bool,
    ) -> Result<(), &'static str> {
        if !complete {
            self.modifiers.len = first.0;
            self.modifiers.required = first.1;
        }
        self.modifiers.push(AirSortieModifierRow {
            scope: AirSortieModifierRow::MENU_MODIFIERS_STATUS,
            subject: country,
            region,
            raw: i64::from(complete),
        })
    }

    fn menu_operand<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
        cached: &mut Cached,
        key: Key,
        observer: &mut impl RegionObserver,
    ) -> Result<(), &'static str> {
        if self.keys.rows[..self.keys.len]
            .binary_search_by_key(&key.order(), Key::order)
            .is_ok()
        {
            return Ok(());
        }
        self.operand(memory, base, game_state, countries, cached, key, observer)
    }
}
