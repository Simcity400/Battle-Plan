#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scores {
    pub first: u32,
    pub second: u32,
    pub equipment_damage: i64,
    pub province_capture: i64,
    pub air_damage: i64,
    pub strategic_bombing: i64,
    pub sunk_ships: i64,
    pub sunk_convoys: i64,
    pub casualties: i64,
    pub unknown_casualties: i64,
    pub lend_lease_sent: i64,
    pub lend_lease_received: i64,
}

impl Scores {
    pub fn total(self, exclude_casualties: bool) -> i64 {
        let mut total = [
            self.lend_lease_received,
            self.lend_lease_sent,
            self.sunk_convoys,
            self.sunk_ships,
            self.strategic_bombing,
            self.air_damage,
            self.province_capture,
            self.equipment_damage,
        ]
        .into_iter()
        .fold(0_i64, i64::wrapping_add);
        if !exclude_casualties {
            total = total.wrapping_add(self.unknown_casualties.wrapping_add(self.casualties));
        }
        total.max(0)
    }

    pub fn add(&mut self, other: Self) {
        macro_rules! sum { ($($field:ident),* $(,)?) => { $(self.$field = self.$field.wrapping_add(other.$field);)* }; }
        sum!(
            equipment_damage,
            province_capture,
            air_damage,
            strategic_bombing,
            sunk_ships,
            sunk_convoys,
            casualties,
            lend_lease_sent,
            lend_lease_received
        );
        self.unknown_casualties = self.unknown_casualties.max(other.unknown_casualties);
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryHistory {
    pub industry_sent: i64,
    pub fuel_sent: i64,
    pub industry_received: i64,
    pub fuel_received: i64,
}
