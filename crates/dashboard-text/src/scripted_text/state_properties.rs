use super::*;
use replay_data::rows::TableId;

fn recorded<T: TableSchema>(frame: HourFrame<'_>, mask: u64) -> Result<Rowset<'_, T::Row>, String> {
    crate::triggers::inputs::table(frame, T::ID);
    frame.complete::<T>(mask).map_err(|why| why.to_string())
}

impl ScriptedText<'_> {
    pub(super) fn state_occupied(&self, id: u32) -> Result<Object, String> {
        self.world()?
            .table_fresh(tables::StateResistance::ID)
            .map_err(|why| why.to_string())?;
        let row = keyed(
            &recorded::<tables::StateResistance>(self.frame, RESISTANCE)?,
            u128::from(id),
        )?
        .ok_or_else(|| format!("Occupied: state resistance lacks state {id}"))?;
        let country = if row.country == 0 {
            0
        } else {
            self.scopes
                .slot(row.country)
                .ok_or_else(|| format!("Occupied: country {} is unresolved", row.country))?
        };
        Ok(Object::Country(country))
    }

    pub(super) fn state_continent(&self, id: u32) -> Result<String, String> {
        let row = keyed(&recorded::<tables::States>(self.frame, STATE)?, u128::from(id))?
            .ok_or_else(|| format!("GetContinent: States lacks state {id}"))?;
        let token = row
            .continent
            .require("GetContinent: continent of states")?
            .ok_or("GetContinent: continent capture unavailable")?;
        if token == 0 {
            return Ok(String::new());
        }
        let literal = spelled(&recorded::<tables::LexicalTokens>(self.frame, 0)?, token)?
            .ok_or("GetContinent: continent token has no recorded spelling")?;
        let key =
            std::str::from_utf8(literal).map_err(|_| "GetContinent: continent name is not UTF-8")?;
        let controller = if row.controller == 0 {
            0
        } else {
            self.scopes
                .slot(row.controller)
                .ok_or("GetContinent: controller is unresolved")?
        };
        crate::triggers::inputs::table(self.frame, TableId::CountryAppearance);
        crate::triggers::inputs::table(self.frame, TableId::ReservedCountryNaming);
        let tag = installed_content_country_names::country_names::original_or_own_tag_in(
            &self.frame,
            controller,
        )?;
        let prefixed = format!("{tag}_{key}");
        self.localise(if self.localisation.get(&prefixed).is_some() {
            &prefixed
        } else {
            key
        })
    }

    pub(super) fn state_capital_victory_point_name(&self, id: u32) -> Result<String, String> {
        let row = keyed(&recorded::<tables::States>(self.frame, STATE)?, u128::from(id))?
            .ok_or_else(|| format!("GetCapitalVictoryPointName: States lacks state {id}"))?;
        let index = row
            .best_vp_index
            .require("GetCapitalVictoryPointName: best vp index of states")?
            .ok_or(
                "GetCapitalVictoryPointName: cached index was not demanded or its capture failed",
            )?;
        let world = self.world()?;
        let members = world.state_order(id).map_err(|why| why.to_string())?;
        let province = members
            .get(index.max(0) as usize)
            .ok_or("GetCapitalVictoryPointName: cached province index is outside the state")?;
        let text = self.province_display_name(*province)?;
        if text.is_empty() {
            self.state_name(id)
        } else {
            Ok(text)
        }
    }
}
