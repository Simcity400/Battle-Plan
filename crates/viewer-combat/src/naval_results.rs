use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    rows::NavalCombatAirHit, rows::NavalCombatHit, rows::NavalResultPlane, rows::NavalResultRow,
    rows::NavalResultShip, rows::NavalResultSideDetails, rows::VariantDefinitionRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_gpu::gradient_math::hsv_to_rgb;
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::displayable;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::tooltip_text::wing_short_name;
use super::content::Content as StaticContent;
use viewer_foundation::perspective::Vision;
use dashboard_text::table_access::{Available, TokenNames, country_slot, rows};
use {
    super::{
        Values, Widget,
        binding::{Bindings, FLAG, Flags, SPRITE, TEXT, country_flag, hide, text},
        tooltips::{hover_text, set_hover},
    },
    viewer_foundation::tooltip_text::{TextIcons, builtin},
};

tooltip_keys! {
    NAVAL_COMBAT_RESULT_VICTORY = "NAVAL_COMBAT_RESULT_VICTORY",
    NAVAL_COMBAT_RESULT_DEFEAT = "NAVAL_COMBAT_RESULT_DEFEAT",
    NAVAL_COMBAT_RESULT_SURVIVORS = "NAVAL_COMBAT_RESULT_SURVIVORS",
    NAVAL_COMBAT_RESULT_SURVIVOR = "NAVAL_COMBAT_RESULT_SURVIVOR",
    NAVAL_COMBAT_RESULT_SURVIVOR_DMG = "NAVAL_COMBAT_RESULT_SURVIVOR_DMG",
    NAVAL_COMBAT_RESULT_SURVIVOR_COUNT_UNDAMAGED = "NAVAL_COMBAT_RESULT_SURVIVOR_COUNT_UNDAMAGED",
    NAVAL_COMBAT_RESULT_SURVIVOR_CONVOYS = "NAVAL_COMBAT_RESULT_SURVIVOR_CONVOYS",
    NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN = "NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN",
    NAVAL_COMBAT_DEFENSE_AIRWING_TOOLTIP_SHOT_DOWN = "NAVAL_COMBAT_DEFENSE_AIRWING_TOOLTIP_SHOT_DOWN",
    NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY = "NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY",
    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED = "NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED",
    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY = "NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY",
    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT = "NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT",
    SHIP_AND_TYPE = "SHIP_AND_TYPE",
    SHIP_RESULT_SUNK = "SHIP_RESULT_SUNK",
    NAVAL_COMBAT_RESULT_SUNK_BY = "NAVAL_COMBAT_RESULT_SUNK_BY",
    NAVAL_COMBAT_RESULT_SURVIVOR_TYPE = "NAVAL_COMBAT_RESULT_SURVIVOR_TYPE",
    NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME = "NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME",
    AND_X_MORE_SUNK_SHIPS = "AND_X_MORE_SUNK_SHIPS",
    AIR_COUNT_ORIGIN = "AIR_COUNT_ORIGIN",
    HEADER = "HEADER",
}

struct Countries<'a> {
    links: Available<'a, tables::CountryLinks>,
    slots: BTreeMap<u32, u32>,
    viewer: u32,
}
impl<'a> Countries<'a> {
    fn new(links: Available<'a, tables::CountryLinks>, viewer: u32) -> Self {
        let slots = links
            .rows()
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        Self {
            links,
            slots,
            viewer,
        }
    }
    fn slot(&self, country: u32) -> Result<u32, String> {
        if let Some(slot) = self.slots.get(&country) {
            return Ok(*slot);
        }
        Ok(country_slot(&self.links, country)?.unwrap_or(0))
    }
    fn same(&self, a: u32, b: u32) -> Result<bool, String> {
        if a == b {
            return Ok(true);
        }
        if a == 0 || b == 0 {
            return Ok(false);
        }
        Ok(self.slot(a)? == self.slot(b)?)
    }
    fn before(&self, a: u32, b: u32) -> bool {
        let (a, b) = (
            self.slot(a).expect("resolved country"),
            self.slot(b).expect("resolved country"),
        );
        if a == b {
            return false;
        }
        if a == self.viewer {
            return true;
        }
        if b == self.viewer {
            return false;
        }
        (a as i32) < (b as i32)
    }
    fn counts(
        &self,
        entries: impl IntoIterator<Item = (u32, i32)>,
    ) -> Result<Vec<(u32, i32)>, String> {
        let mut groups = Vec::<(u32, i32)>::new();
        let mut by_slot = BTreeMap::<(bool, u32), usize>::new();
        for (country, count) in entries {
            let slot = self.slot(country)?;
            let key = (country != 0, slot);
            if let Some(&index) = by_slot.get(&key) {
                groups[index].1 = groups[index].1.wrapping_add(count);
            } else {
                by_slot.insert(key, groups.len());
                groups.push((country, count));
            }
        }
        native_sort(&mut groups, &|&(a, ac), &(b, bc)| {
            if self.same(a, b).expect("resolved countries") {
                return ac > bc;
            }
            if self.slot(a).expect("resolved country") == self.viewer {
                return true;
            }
            if self.slot(b).expect("resolved country") == self.viewer {
                return false;
            }
            if ac != bc {
                return ac > bc;
            }
            self.before(a, b)
        });
        Ok(groups)
    }
}

fn outcome(
    result: &NavalResultRow,
    countries: &Available<'_, tables::NavalResultCountries>,
    order: &Countries<'_>,
) -> Result<Option<&'static str>, String> {
    if result.readiness != 0 {
        return Ok(None);
    }
    let side = match result.winning_side {
        1 => 0,
        2 => 1,
        selector => {
            return Err(format!(
                "winning-side selector {selector} does not name an embedded side"
            ));
        }
    };
    let mut unknown = None;
    for country in countries
        .rows()
        .iter()
        .filter(|country| country.result == result.id && country.list == side)
    {
        match order.slot(country.country) {
            Ok(slot) if slot == order.viewer => return Ok(Some(NAVAL_COMBAT_RESULT_VICTORY)),
            Ok(_) => {}
            Err(reason) => {
                unknown.get_or_insert(reason);
            }
        }
    }
    if let Some(reason) = unknown {
        return Err(reason);
    }
    if !countries.is_complete() {
        return Err("NavalResultCountries is incomplete for the winning side".into());
    }
    Ok(Some(NAVAL_COMBAT_RESULT_DEFEAT))
}

fn native_sort<T: Clone>(entries: &mut [T], before: &impl Fn(&T, &T) -> bool) {
    if entries.len() <= 32 {
        insertion(entries, before);
    } else {
        let middle = entries.len() - entries.len() / 2;
        let (left, right) = entries.split_at_mut(middle);
        buffered_sort(left, before);
        buffered_sort(right, before);
        let mut first = 0;
        while first < middle && !before(&entries[middle], &entries[first]) {
            first += 1;
        }
        if first == middle {
            return;
        }
        let mut end = entries.len();
        while end - middle > 1 && !before(&entries[end - 1], &entries[middle - 1]) {
            end -= 1;
        }
        if end - middle == 1 {
            entries[first..end].rotate_right(1);
        } else if middle - first == 1 {
            entries[first..end].rotate_left(1);
        } else {
            merge(
                &mut entries[first..end],
                middle - first,
                before,
                end - middle < middle - first,
            );
        }
    }
}

fn insertion<T>(entries: &mut [T], before: &impl Fn(&T, &T) -> bool) {
    for index in 1..entries.len() {
        if before(&entries[index], &entries[0]) {
            entries[..=index].rotate_right(1);
        } else {
            let mut at = index;
            while at > 1 && before(&entries[at], &entries[at - 1]) {
                entries.swap(at, at - 1);
                at -= 1;
            }
        }
    }
}

fn buffered_sort<T: Clone>(entries: &mut [T], before: &impl Fn(&T, &T) -> bool) {
    for run in entries.chunks_mut(32) {
        insertion(run, before);
    }
    let mut width = 32;
    while width < entries.len() {
        for run in entries.chunks_mut(width * 2) {
            if run.len() > width {
                merge(run, width, before, false);
            }
        }
        width *= 2;
    }
}

fn merge<T: Clone>(
    entries: &mut [T],
    middle: usize,
    before: &impl Fn(&T, &T) -> bool,
    backwards: bool,
) {
    let sorted = entries.to_vec();
    if backwards {
        let (mut a, mut b) = (middle, sorted.len());
        for entry in entries.iter_mut().rev() {
            let take_left = a > 0 && (b == middle || before(&sorted[b - 1], &sorted[a - 1]));
            let index = if take_left {
                a -= 1;
                a
            } else {
                b -= 1;
                b
            };
            *entry = sorted[index].clone();
        }
    } else {
        let (mut a, mut b) = (0, middle);
        for entry in entries {
            let take_right = b < sorted.len() && (a == middle || before(&sorted[b], &sorted[a]));
            let index = if take_right {
                let index = b;
                b += 1;
                index
            } else {
                let index = a;
                a += 1;
                index
            };
            *entry = sorted[index].clone();
        }
    }
}

fn ship_before(a: &NavalResultShip, b: &NavalResultShip, order: &Countries<'_>) -> bool {
    if order.same(a.owner, b.owner).expect("resolved countries") {
        if a.build_cost_ic != b.build_cost_ic {
            return a.build_cost_ic > b.build_cost_ic;
        }
        return !(a.is_convoy != 0 && b.is_convoy != 0) && a.strength_ratio > b.strength_ratio;
    }
    if order.slot(a.owner).expect("resolved country") == order.viewer {
        return true;
    }
    (order.slot(a.owner).expect("resolved country") as i32)
        < (order.slot(b.owner).expect("resolved country") as i32)
}

struct AirGroup<'a> {
    model: usize,
    members: Vec<&'a NavalResultPlane>,
}
struct Groups<'a> {
    ships: Vec<Vec<&'a NavalResultShip>>,
    lost_ships: Vec<Vec<&'a NavalResultShip>>,
    planes: Vec<AirGroup<'a>>,
    lost_planes: Vec<AirGroup<'a>>,
}

fn classify<'a>(
    side: &'a NavalResultSideDetails,
    model: &impl Fn(&NavalResultPlane) -> Result<usize, String>,
) -> (Groups<'a>, Vec<(u64, String)>) {
    let mut groups = Groups {
        ships: ship_groups(side, false),
        lost_ships: ship_groups(side, true),
        planes: Vec::new(),
        lost_planes: Vec::new(),
    };
    let mut refused = Vec::new();
    for plane in &side.planes {
        if plane.current_planes <= 0 && plane.lost_planes <= 0 {
            continue;
        }
        let model = match model(plane) {
            Ok(model) => model,
            Err(reason) => {
                refused.push((plane.variant, reason));
                continue;
            }
        };
        for (admitted, list) in [
            (plane.current_planes > 0, &mut groups.planes),
            (plane.lost_planes > 0, &mut groups.lost_planes),
        ] {
            if !admitted {
                continue;
            }
            if let Some(group) = list.iter_mut().find(|group| group.model == model) {
                group.members.push(plane);
            } else {
                list.push(AirGroup {
                    model,
                    members: vec![plane],
                });
            }
        }
    }
    (groups, refused)
}

fn order_survivors<T: Clone>(entries: &mut [(i32, bool, T)], columns: Option<i32>) {
    native_sort(entries, &|a, b| a.0 < b.0);
    if let Some(columns) =
        columns.filter(|columns| *columns > 0 && (*columns as usize) < entries.len())
        && let Some(air) = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.1)
            .min_by_key(|(_, entry)| entry.0)
            .map(|(index, _)| index)
        && air >= columns as usize
    {
        entries.swap(air, columns as usize - 1);
    }
}

fn ship_groups(side: &NavalResultSideDetails, losses: bool) -> Vec<Vec<&NavalResultShip>> {
    let mut groups = Vec::<Vec<&NavalResultShip>>::new();
    for ship in &side.ships {
        if ship.sunk_by.is_empty() == losses {
            continue;
        }
        let index = groups.iter().position(|group| {
            let first = group[0];
            if losses {
                (first.is_convoy != 0) == (ship.is_convoy != 0) && first.sprite == ship.sprite
            } else {
                first.subunit_index == ship.subunit_index
                    && first.formation_type == ship.formation_type
            }
        });
        if let Some(index) = index {
            groups[index].push(ship);
        } else {
            groups.push(vec![ship]);
        }
    }
    if losses {
        groups.sort_by_key(|group| group[0].is_convoy != 0);
    }
    groups
}

fn priority_ships(group: &[&NavalResultShip]) -> i32 {
    group
        .iter()
        .fold(0_i32, |cost, ship| {
            cost.wrapping_add((ship.build_cost_ic / 100_000) as i32)
        })
        .wrapping_neg()
}
fn priority_planes(group: &[&NavalResultPlane]) -> i32 {
    let cost = group.iter().fold(0_i32, |cost, plane| {
        let per_plane = (plane.variant_summary_inputs[0]
            .wrapping_mul(plane.variant_summary_inputs[1])
            / 10_000_000_000) as i32;
        cost.wrapping_add(per_plane.wrapping_mul(plane.current_planes))
    });
    cost.wrapping_neg() / 5
}

struct Content<'a> {
    static_content: &'a StaticContent,
    icons: &'a dyn TextIcons,
    loc: &'a Localisation,
    tokens: Result<TokenNames<'a>, String>,
    variants: BTreeMap<u64, &'a VariantDefinitionRow>,
    names: super::naval_variants::Names<'a>,
    country_names: content_reader_country_names::country_names::CountryNames<'a>,
    viewer_tag: Result<&'a str, String>,
}
impl Content<'_> {
    fn variant(&self, id: u64) -> Result<&VariantDefinitionRow, String> {
        self.variants
            .get(&id)
            .copied()
            .ok_or_else(|| format!("variant {id} has no VariantDefinitions row"))
    }
    fn variant_name(&self, id: u64) -> Result<String, String> {
        super::naval_variants::name(
            self.variant(id)?,
            self.icons,
            self.loc,
            Some(&self.names),
            Some(&self.static_content.recorded_language),
        )
    }
    fn model(&self, plane: &NavalResultPlane) -> Result<usize, String> {
        let variant = self.variant(plane.variant)?;
        let name = self
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .name(variant.archetype_token)
            .map_err(|error| error.to_string())?;
        let database = self
            .static_content
            .assets
            .naval_air_groups
            .equipment()
            .ok_or("the stacked equipment database is unavailable")?;
        let index = database.get(name.as_bytes()).ok_or_else(|| {
            format!("result aircraft archetype {name} is absent from stacked equipment")
        })?;
        let equipment = self
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .name(variant.equipment_token)
            .map_err(|error| error.to_string())?;
        let definition = database.get(equipment.as_bytes()).ok_or_else(|| {
            format!("result aircraft definition {equipment} is absent from stacked equipment")
        })?;
        let effective = database.equipment[definition].archetype
            .ok_or_else(|| format!("result aircraft definition {equipment} has a null archetype, which this binder does not support"))?;
        if effective != index {
            return Err(format!(
                "variant {} recorded archetype {name} differs from its stacked definition",
                variant.id
            ));
        }
        Ok(index)
    }
    fn air_sprite(&self, model: usize, small: bool) -> Result<String, String> {
        let database = self
            .static_content
            .assets
            .naval_air_groups
            .equipment()
            .ok_or("the stacked equipment database is unavailable")?;
        let definition = &database.equipment[model];
        air_icon(&definition.name, &definition.sprite, small, &|sprite| {
            self.static_content.assets.has_sprite(sprite)
        })
    }
    fn stored(&self, bytes: &[u8], what: &str) -> Result<String, String> {
        dashboard_text::scripted_text::stored_text(
            self.loc,
            &self.static_content.recorded_language,
            bytes,
            what,
        )
    }
    fn ship_name(&self, ship: &NavalResultShip) -> Result<String, String> {
        let name = self.stored(&ship.name, "retained ship name")?;
        let kind = self.stored(&ship.type_name, "retained ship type")?;
        builtin(
            self.icons,
            self.loc,
            SHIP_AND_TYPE,
            &[
                ("SHIP", Argument::Text(&name)),
                ("TYPE", Argument::Text(&kind)),
            ],
        )
    }
    fn loss_type(&self, ship: &NavalResultShip) -> Result<String, String> {
        if ship.subunit_index == 0 || ship.is_convoy != 0 {
            return self.stored(&ship.type_name, "retained ship loss type");
        }
        let units = self
            .static_content
            .assets
            .commander_tooltips
            .sub_units()
            .map_err(str::to_owned)?;
        let key = units.name(ship.subunit_index as u32).ok_or_else(|| {
            format!(
                "result sub-unit {} is absent from stacked content",
                ship.subunit_index
            )
        })??;
        country_localised(
            self.loc,
            self.viewer_tag.as_ref().copied().map_err(String::as_str),
            key,
        )
        .and_then(|text| displayable(self.icons, text))
    }
    fn air_name(&self, model: usize) -> Result<String, String> {
        let database = self
            .static_content
            .assets
            .naval_air_groups
            .equipment()
            .ok_or("the stacked equipment database is unavailable")?;
        let key = std::str::from_utf8(&database.equipment[model].name)
            .map_err(|_| "result aircraft archetype name is not UTF-8")?;
        wing_short_name(
            self.loc,
            self.viewer_tag.as_ref().map_err(Clone::clone)?,
            key,
        )
    }
}

fn air_icon(
    name: &[u8],
    sprite: &[u8],
    small: bool,
    exists: &impl Fn(&str) -> bool,
) -> Result<String, String> {
    let prefix = if small {
        "GFX_navalcombat_air_small_icon_"
    } else {
        "GFX_navalcombat_air_icon_"
    };
    for bytes in [name, sprite] {
        let token =
            std::str::from_utf8(bytes).map_err(|_| "result aircraft icon identity is not UTF-8")?;
        let candidate = format!("{prefix}{token}");
        if exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "the content lacks result aircraft icons for {}",
        String::from_utf8_lossy(name)
    ))
}

fn repeat(values: &mut Values, key: &str, template: String, entries: Vec<Values>) {
    values.insert(
        key.into(),
        Widget {
            entry_template: Some(template),
            entries,
            ..Widget::default()
        },
    );
}

fn repeat_mixed(values: &mut Values, key: &str, default: String, entries: Vec<(String, Values)>) {
    let mut templates = BTreeMap::new();
    let mut rows = Vec::new();
    for (template, entry) in entries {
        templates.insert(rows.len(), template);
        rows.push(entry);
    }
    repeat(values, key, default, rows);
    values.get_mut(key).expect("bound grid").entry_templates = templates;
}

fn ship_sprite(
    icons: &super::Assets,
    ship: &NavalResultShip,
    lost: bool,
) -> Result<String, String> {
    let token =
        std::str::from_utf8(&ship.sprite).map_err(|_| "retained ship sprite is not UTF-8")?;
    let name = if lost {
        format!("GFX_navalcombat_ship_icon_{token}")
    } else {
        format!("GFX_unit_{token}_icon_medium_black")
    };
    icons
        .has_sprite(&name)
        .then_some(name.clone())
        .ok_or_else(|| format!("stacked content lacks retained ship sprite {name}"))
}

pub(super) fn pride_color(defines: &content_reader_defines::defines::Defines) -> Result<[f32; 4], String> {
    let values = defines
        .numbers("NGraphics", "PRIDE_OF_THE_FLEET_MODULATE")
        .map_err(|error| error.to_string())?;
    let mut color = match values[..values.len().min(4)] {
        [h, s, v] => {
            let [r, g, b] = hsv_to_rgb([h as f32, s as f32, v as f32]);
            [r, g, b, 1.0]
        }
        [r, g, b, a] => [r as f32, g as f32, b as f32, a as f32],
        _ => return Err("NDefines.NGraphics.PRIDE_OF_THE_FLEET_MODULATE requires three HSV or four RGBA numbers".into()),
    };
    for channel in &mut color[..3] {
        *channel = channel.powf(2.2);
    }
    if !color.iter().all(|channel| channel.is_finite()) {
        return Err(
            "NDefines.NGraphics.PRIDE_OF_THE_FLEET_MODULATE produces nonfinite RGBA".into(),
        );
    }
    Ok(color)
}

fn loss_color(
    out: &mut Bindings,
    values: &mut Values,
    pride: bool,
    color: &Result<[f32; 4], String>,
) {
    let color = if pride { color.clone() } else { Ok([1.0; 4]) };
    match color {
        Ok(color) => values.entry("equipment".into()).or_default().color = Some(color),
        Err(reason) => out.refuse(format!(
            "Naval retained-result pride tint is withheld: {reason}."
        )),
    }
}

pub(super) fn show_details(values: &mut Values, open: bool) {
    let available = values
        .get("open_details")
        .is_none_or(|widget| widget.enabled != Some(false));
    for path in ["attacker_details", "defender_details"] {
        values.entry(path.into()).or_default().hidden = !open || !available;
    }
    values.entry("open_details".into()).or_default().frame = Some(u32::from(open && available));
}
fn flags(out: &mut Bindings, values: &mut Values, countries: &[(u32, i32)], flags: &Flags) {
    for key in [
        "country_flag_left",
        "country_flag_right",
        "country_flag_center",
    ] {
        hide(values, key);
    }
    let slots: &[&str] = if countries.len() == 1 {
        &["country_flag_center"]
    } else {
        &["country_flag_left", "country_flag_right"]
    };
    for (&(owner, _), key) in countries.iter().zip(slots) {
        out.bind_in(values, key, FLAG, country_flag(flags, owner));
        values.entry((*key).into()).or_default().size = Some([12.0, 8.0]);
    }
}
fn hover(out: &mut Bindings, values: &mut Values, keys: &[&str], value: Result<String, String>) {
    let text = match value {
        Ok(text) => hover_text(&text),
        Err(reason) => {
            out.refuse(format!(
                "Naval retained-result hover is withheld: {reason}."
            ));
            None
        }
    };
    for &key in keys {
        set_hover(values, key, text.clone(), None);
    }
}

fn survivor_text(
    content: &Content<'_>,
    order: &Countries<'_>,
    group: &[&NavalResultShip],
    flags: &Flags,
) -> Result<String, String> {
    let mut text = String::new();
    let mut first = 0;
    while first < group.len() {
        let ship = group[first];
        let mut end = first + 1;
        if ship.is_convoy != 0 || ship.strength_ratio == 100_000 {
            while end < group.len() && !ship_before(ship, group[end], order) {
                end += 1;
            }
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        let flag = format!("{}{}", super::INLINE_FLAG, country_flag(flags, ship.owner)?);
        let kind = if ship.is_convoy != 0 {
            String::new()
        } else if end - first == 1 {
            content.ship_name(ship)?
        } else {
            content.stored(&ship.type_name, "retained ship type")?
        };
        text.push_str(&survivor_description(
            content.icons,
            content.loc,
            ship,
            (end - first) as i32,
            &flag,
            &kind,
        )?);
        first = end;
    }
    Ok(text)
}

fn survivor_description(
    icons: &dyn TextIcons,
    loc: &Localisation,
    ship: &NavalResultShip,
    count: i32,
    flag: &str,
    kind: &str,
) -> Result<String, String> {
    let mut args = vec![("FLAG", Argument::Text(flag))];
    let key = if ship.is_convoy != 0 {
        args.push(("COUNT", Argument::Int32(count)));
        NAVAL_COMBAT_RESULT_SURVIVOR_CONVOYS
    } else {
        args.push(("TYPE", Argument::Text(kind)));
        if count > 1 {
            args.push(("COUNT", Argument::Int32(count)));
            NAVAL_COMBAT_RESULT_SURVIVOR_COUNT_UNDAMAGED
        } else if ship.strength_ratio > 99_000 {
            NAVAL_COMBAT_RESULT_SURVIVOR
        } else {
            args.push(("DMG", Argument::Fixed5(ship.strength_ratio)));
            NAVAL_COMBAT_RESULT_SURVIVOR_DMG
        }
    };
    builtin(icons, loc, key, &args)
}

fn air_text(
    content: &Content<'_>,
    order: &Countries<'_>,
    model: usize,
    countries: &[(u32, i32)],
    flags: &Flags,
) -> Result<String, String> {
    let heading = builtin(
        content.icons,
        content.loc,
        NAVAL_COMBAT_RESULT_SURVIVORS,
        &[],
    )?;
    let mut text = builtin(
        content.icons,
        content.loc,
        HEADER,
        &[("KEY", Argument::Text(&heading))],
    )?;
    text.push_str(&content.air_name(model)?);
    text.push_str(&air_origins(
        content.icons,
        content.loc,
        countries,
        |country| {
            let name = content_reader_localisation::localisation::crt_case_text(
                &content
                    .country_names
                    .name_with(order.slot(country)?, "_DEF")?,
                true,
                true,
            );
            Ok(format!(
                "{}{} {name}",
                super::INLINE_FLAG,
                country_flag(flags, country)?
            ))
        },
    )?);
    Ok(text)
}

fn air_origins(
    icons: &dyn TextIcons,
    loc: &Localisation,
    countries: &[(u32, i32)],
    mut marked: impl FnMut(u32) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = String::new();
    for &(country, count) in countries {
        let marked = marked(country)?;
        text.push('\n');
        text.push_str(&builtin(
            icons,
            loc,
            AIR_COUNT_ORIGIN,
            &[
                ("COUNTRY", Argument::Text(&marked)),
                ("COUNT", Argument::Int32(count)),
            ],
        )?);
    }
    Ok(text)
}

fn lost_ship_text(
    icons: &dyn TextIcons,
    loc: &Localisation,
    kind: &str,
    group: &[&NavalResultShip],
    line: &impl Fn(&NavalResultShip) -> Result<String, String>,
) -> Result<String, String> {
    let heading = builtin(
        icons,
        loc,
        SHIP_RESULT_SUNK,
        &[("SHIP", Argument::Text(kind))],
    )?;
    let mut text = builtin(icons, loc, HEADER, &[("KEY", Argument::Text(&heading))])?;
    for (index, ship) in group.iter().take(20).enumerate() {
        if index != 0 {
            text.push('\n');
        }
        text.push_str(&line(ship)?);
    }
    if group.len() > 20 {
        text.push_str(&builtin(
            icons,
            loc,
            AND_X_MORE_SUNK_SHIPS,
            &[("VAL", Argument::Int32((group.len() - 20) as i32))],
        )?);
    }
    Ok(text)
}

fn loss_text(
    content: &Content<'_>,
    kind: &str,
    group: &[&NavalResultShip],
) -> Result<String, String> {
    lost_ship_text(content.icons, content.loc, kind, group, &|ship| {
        let name = content.ship_name(ship)?;
        let killer = content.stored(&ship.sunk_by, "retained ship killer")?;
        builtin(
            content.icons,
            content.loc,
            NAVAL_COMBAT_RESULT_SUNK_BY,
            &[
                ("NAME", Argument::Text(&name)),
                ("KILLER", Argument::Text(&killer)),
            ],
        )
    })
}

fn lost_air_text(content: &Content<'_>, model: usize, count: i32) -> Result<String, String> {
    let kind = content.air_name(model)?;
    builtin(
        content.icons,
        content.loc,
        NAVAL_COMBAT_RESULT_SURVIVOR_TYPE,
        &[
            ("NUMBER", Argument::Int32(count)),
            ("TYPE", Argument::Text(&kind)),
        ],
    )
}

fn hits(
    icons: &dyn TextIcons,
    loc: &Localisation,
    air: &[Vec<NavalCombatAirHit>; 2],
    naval: &[NavalCombatHit],
    variant: &impl Fn(u64) -> Result<String, String>,
    stored: &impl Fn(&[u8]) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = String::new();
    for (index, lane) in air.iter().enumerate() {
        if lane.is_empty() {
            continue;
        }
        let key = if index == 0 {
            NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN
        } else {
            NAVAL_COMBAT_DEFENSE_AIRWING_TOOLTIP_SHOT_DOWN
        };
        text.push_str(&builtin(icons, loc, key, &[])?);
        for hit in lane {
            let name = variant(hit.variant)?;
            text.push_str(&builtin(
                icons,
                loc,
                NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY,
                &[
                    ("NUM", Argument::Int32(hit.count)),
                    ("MOD", Argument::Text(&name)),
                ],
            )?);
        }
    }
    if !naval.is_empty() {
        text.push_str(&builtin(
            icons,
            loc,
            NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED,
            &[],
        )?);
        for hit in naval {
            let name = stored(&hit.target_name)?;
            let damage = if hit.denominator < 1 {
                hit.damage
            } else if [i64::MAX, i64::MIN, i64::MIN + 1].contains(&hit.damage) {
                0xffff_ffff
            } else {
                hit.damage.wrapping_mul(100_000) / hit.denominator
            };
            text.push_str(&builtin(
                icons,
                loc,
                NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY,
                &[
                    ("NAVY", Argument::Text(&name)),
                    ("PERC", Argument::Fixed5(damage)),
                ],
            )?);
            if hit.last_hit != 0 {
                text.push_str(&builtin(
                    icons,
                    loc,
                    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT,
                    &[],
                )?);
            }
        }
    }
    Ok(text)
}
fn hit_score(air: &[Vec<NavalCombatAirHit>; 2], naval: &[NavalCombatHit]) -> i32 {
    let air = air
        .iter()
        .flatten()
        .fold(0_i32, |count, hit| count.wrapping_add(hit.count));
    let damage = naval
        .iter()
        .fold(0_i64, |damage, hit| damage.wrapping_add(hit.damage));
    air.wrapping_add((damage / 100_000) as i32)
}
fn detail_text(
    content: &Content<'_>,
    order: &Countries<'_>,
    side: &NavalResultSideDetails,
    flags: &Flags,
) -> Result<String, String> {
    let mut entries = Vec::new();
    let build = |air: &[Vec<NavalCombatAirHit>; 2], naval: &[NavalCombatHit]| {
        hits(
            content.icons,
            content.loc,
            air,
            naval,
            &|id| content.variant_name(id),
            &|name| content.stored(name, "retained naval hit target"),
        )
    };
    for plane in side.planes.iter().filter(|plane| plane.variant != 0) {
        let hits = build(&plane.air_hits, &plane.naval_hits)?;
        if hits.is_empty() {
            continue;
        }
        let name = content.variant_name(plane.variant)?;
        let flag = country_flag(flags, plane.country)?;
        entries.push((
            plane.country,
            0,
            hit_score(&plane.air_hits, &plane.naval_hits),
            format!("\u{11}Y{name}\u{11}! {}{flag}{hits}", super::INLINE_FLAG),
        ));
    }
    for ship in &side.ships {
        let hits = build(&ship.air_hits, &ship.naval_hits)?;
        if hits.is_empty() {
            continue;
        }
        let name = content.ship_name(ship)?;
        let flag = country_flag(flags, ship.owner)?;
        entries.push((
            ship.owner,
            1,
            hit_score(&ship.air_hits, &ship.naval_hits),
            format!("{name} {}{flag}{hits}", super::INLINE_FLAG),
        ));
    }
    join_details(order, entries)
}

fn join_details(
    order: &Countries<'_>,
    mut entries: Vec<(u32, u8, i32, String)>,
) -> Result<String, String> {
    for entry in &entries {
        order.slot(entry.0)?;
    }
    native_sort(&mut entries, &|a, b| {
        if order.same(a.0, b.0).expect("resolved countries") {
            if a.1 != b.1 {
                return a.1 < b.1;
            }
            return a.2 > b.2;
        }
        order.before(a.0, b.0)
    });
    Ok(entries
        .into_iter()
        .map(|entry| entry.3)
        .collect::<Vec<_>>()
        .join("\n\n"))
}

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
    flags_map: &Flags,
    vision: &Vision,
) -> io::Result<()> {
    let order = Countries::new(rows::<tables::CountryLinks>(frame)?, vision.ours());
    if let Some(result) = rows::<tables::NavalResults>(frame)?
        .rows()
        .iter()
        .find(|row| row.id == id)
    {
        let countries = rows::<tables::NavalResultCountries>(frame)?;
        match outcome(result, &countries, &order) {
            Ok(None) => hide(&mut out.values, "outcome"),
            value => out.bind(
                "outcome",
                TEXT,
                value
                    .and_then(|key| {
                        builtin(
                            static_content.assets.as_ref(),
                            &static_content.localisation,
                            key.expect("shown outcome"),
                            &[],
                        )
                    })
                    .map_err(|reason| format!("Naval result {id} outcome is hidden: {reason}.")),
            ),
        }
    } else {
        hide(&mut out.values, "outcome");
    }
    let details = rows::<tables::NavalCombatResultDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.result == id) else {
        out.refuse(format!(
            "Naval result {id} lists and hit details are withheld: no NavalCombatResultDetails row."
        ));
        out.values.entry("open_details".into()).or_default().enabled = Some(false);
        return Ok(());
    };
    let variants = rows::<tables::VariantDefinitions>(frame)?;
    let countries = rows::<tables::Countries>(frame)?;
    let viewer_tag = countries
        .rows()
        .iter()
        .find(|row| row.canonical_slot == order.viewer)
        .ok_or_else(|| "the viewing country has no Countries row".to_owned())
        .and_then(|row| {
            std::str::from_utf8(&row.tag).map_err(|_| "viewing country tag is not UTF-8".to_owned())
        });
    let content = Content {
        static_content,
        icons: static_content.assets.as_ref(),
        loc: &static_content.localisation,
        tokens: TokenNames::new(frame).map_err(|error| error.to_string()),
        variants: variants.rows().iter().map(|row| (row.id, row)).collect(),
        names: super::naval_variants::Names::new(static_content, frame),
        country_names: content_reader_country_names::country_names::CountryNames::read(
            frame,
            &static_content.localisation,
        ),
        viewer_tag,
    };
    out.values.entry("open_details".into()).or_default().enabled = Some(true);
    for (side, row) in detail.sides.iter().enumerate() {
        let label = if side == 0 { "attacker" } else { "defender" };
        let root = if side == 0 {
            "attacking_side"
        } else {
            "defending_side"
        };
        let detail_path = format!("{label}_details/text");
        out.bind(
            &detail_path,
            TEXT,
            detail_text(&content, &order, row, flags_map).map_err(|reason| {
                format!("Naval result {id} {label} hit details are hidden: {reason}.")
            }),
        );
        out.values.entry(detail_path).or_default().text_scroll_lines = Some(true);
        let (groups, refused) = classify(row, &|plane| content.model(plane));
        for (variant, reason) in refused {
            out.refuse(format!(
                "Naval result {id} aircraft {variant} is omitted: {reason}."
            ));
        }
        let mut survivors = Vec::<(i32, bool, (String, Values))>::new();
        for mut group in groups.ships {
            if let Err(reason) = group
                .iter()
                .try_for_each(|ship| order.slot(ship.owner).map(|_| ()))
            {
                out.refuse(format!(
                    "Naval result {id} ship survivor group is withheld: {reason}."
                ));
                continue;
            }
            native_sort(&mut group, &|a, b| ship_before(a, b, &order));
            let mut values = Values::new();
            out.bind_in(
                &mut values,
                "fleet",
                SPRITE,
                ship_sprite(static_content.assets.as_ref(), group[0], false),
            );
            text(&mut values, "number", group.len().to_string());
            let mut owners = Vec::new();
            for ship in &group {
                if owners.last().is_none_or(|&(owner, _)| {
                    !order.same(owner, ship.owner).expect("resolved countries")
                }) {
                    owners.push((ship.owner, 1));
                }
            }
            flags(out, &mut values, &owners, flags_map);
            hover(
                out,
                &mut values,
                &[
                    "",
                    "Background",
                    "fleet",
                    "number",
                    "country_flag_left",
                    "country_flag_right",
                    "country_flag_center",
                ],
                survivor_text(&content, &order, &group, flags_map),
            );
            survivors.push((
                priority_ships(&group),
                false,
                (
                    format!("naval_combat_results_{label}_survivor_entry"),
                    values,
                ),
            ));
        }
        for group in groups.planes {
            let owners = match order.counts(
                group
                    .members
                    .iter()
                    .map(|plane| (plane.country, plane.current_planes)),
            ) {
                Ok(owners) => owners,
                Err(reason) => {
                    out.refuse(format!(
                        "Naval result {id} air survivor group is withheld: {reason}."
                    ));
                    continue;
                }
            };
            let mut values = Values::new();
            out.bind_in(
                &mut values,
                "plane",
                SPRITE,
                content.air_sprite(group.model, true),
            );
            values.entry("plane".into()).or_default().frame = Some(side as u32);
            text(
                &mut values,
                "number",
                owners
                    .iter()
                    .fold(0_i32, |sum, owner| sum.wrapping_add(owner.1))
                    .to_string(),
            );
            flags(out, &mut values, &owners, flags_map);
            hover(
                out,
                &mut values,
                &[
                    "",
                    "Background",
                    "plane",
                    "number",
                    "country_flag_left",
                    "country_flag_right",
                    "country_flag_center",
                ],
                air_text(&content, &order, group.model, &owners, flags_map),
            );
            survivors.push((
                priority_planes(&group.members),
                true,
                (format!("naval_combat_results_{label}_air_entry"), values),
            ));
        }
        let columns = static_content
            .assets
            .native_windows
            .gui_type(b"navalcombatresultswindow")
            .and_then(|root_gui| root_gui.path_named(&format!("{root}/survivors/survivors_grid")))
            .and_then(|grid| grid.grid())
            .map(|grid| grid.max_slots[0]);
        order_survivors(&mut survivors, columns);
        let grid = format!("{root}/survivors/survivors_grid");
        repeat_mixed(
            &mut out.values,
            &grid,
            format!("naval_combat_results_{label}_survivor_entry"),
            survivors.into_iter().map(|entry| entry.2).collect(),
        );
        let mut losses = Vec::new();
        let mut convoy_losses = Vec::new();
        for group in groups.lost_ships {
            let owners = match order.counts(group.iter().map(|ship| (ship.owner, 1))) {
                Ok(owners) => owners,
                Err(reason) => {
                    out.refuse(format!(
                        "Naval result {id} ship loss group is withheld: {reason}."
                    ));
                    continue;
                }
            };
            let mut values = Values::new();
            out.bind_in(
                &mut values,
                "equipment",
                SPRITE,
                ship_sprite(static_content.assets.as_ref(), group[0], true),
            );
            values.entry("equipment".into()).or_default().frame = Some(side as u32);
            let pride = group.iter().any(|ship| ship.pride != 0);
            loss_color(
                out,
                &mut values,
                pride,
                &static_content.assets.naval_result_pride_color,
            );
            if !pride {
                hide(&mut values, "pride");
            }
            let kind = content.loss_type(group[0]);
            let description = kind
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|kind| loss_text(&content, kind, &group));
            hover(out, &mut values, &["", "equipment"], description);
            if pride {
                hover(
                    out,
                    &mut values,
                    &["pride"],
                    builtin(
                        content.icons,
                        content.loc,
                        NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME,
                        &[],
                    ),
                );
            }
            let mut counts = Vec::new();
            for &(owner, count) in &owners {
                let mut entry = Values::new();
                text(&mut entry, "number", count.to_string());
                out.bind_in(
                    &mut entry,
                    "country_flag",
                    FLAG,
                    country_flag(flags_map, owner),
                );
                let mut members: Vec<_> = group
                    .iter()
                    .copied()
                    .filter(|ship| order.same(ship.owner, owner).expect("resolved countries"))
                    .collect();
                native_sort(&mut members, &|a, b| a.build_cost_ic > b.build_cost_ic);
                hover(
                    out,
                    &mut entry,
                    &["", "country_flag", "number"],
                    kind.as_ref()
                        .map_err(Clone::clone)
                        .and_then(|kind| loss_text(&content, kind, &members)),
                );
                counts.push(entry);
            }
            repeat(
                &mut values,
                "lost_ships_grid",
                format!("naval_combat_results_{label}_lost_ship_entry"),
                counts,
            );
            let entry = (
                format!("naval_combat_results_{label}_lost_line_entry"),
                values,
            );
            if group[0].is_convoy != 0 {
                convoy_losses.push(entry);
            } else {
                losses.push(entry);
            }
        }
        for group in groups.lost_planes {
            let owners = match order.counts(
                group
                    .members
                    .iter()
                    .map(|plane| (plane.country, plane.lost_planes)),
            ) {
                Ok(owners) => owners,
                Err(reason) => {
                    out.refuse(format!(
                        "Naval result {id} air loss group is withheld: {reason}."
                    ));
                    continue;
                }
            };
            let mut values = Values::new();
            out.bind_in(
                &mut values,
                "equipment",
                SPRITE,
                content.air_sprite(group.model, false),
            );
            values.entry("equipment".into()).or_default().frame = Some(side as u32);
            hover(
                out,
                &mut values,
                &["", "equipment"],
                lost_air_text(
                    &content,
                    group.model,
                    owners
                        .iter()
                        .fold(0_i32, |sum, owner| sum.wrapping_add(owner.1)),
                ),
            );
            let mut counts = Vec::new();
            for &(owner, count) in &owners {
                let mut entry = Values::new();
                text(&mut entry, "number", count.to_string());
                out.bind_in(
                    &mut entry,
                    "country_flag",
                    FLAG,
                    country_flag(flags_map, owner),
                );
                hover(
                    out,
                    &mut entry,
                    &["", "country_flag", "number"],
                    lost_air_text(&content, group.model, count),
                );
                counts.push(entry);
            }
            repeat(
                &mut values,
                "lost_air_grid",
                format!("naval_combat_results_{label}_lost_air_entry"),
                counts,
            );
            losses.push((
                format!("naval_combat_results_{label}_air_lost_line_entry"),
                values,
            ));
        }
        losses.extend(convoy_losses);
        let grid = format!("{root}/lost/lost_line_grid");
        repeat_mixed(
            &mut out.values,
            &grid,
            format!("naval_combat_results_{label}_lost_line_entry"),
            losses,
        );
    }
    show_details(&mut out.values, false);
    Ok(())
}

#[cfg(test)]
mod tests;
