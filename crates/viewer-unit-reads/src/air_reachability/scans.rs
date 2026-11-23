use super::*;
use replay_data::rows::AirMissionInputRow;

pub struct Scans<'a> {
    pub tactical: Option<TacticalTargetSources<'a>>,
    pub port: Option<PortTargetSources<'a>>,
    pub strategic: Option<StrategicTargetSources<'a>>,
    pub scout: Option<ScoutTargetSources<'a>>,
    pub naval_combat: Option<NavalCombatTargetSources<'a>>,
    pub occupants: Option<NavalOccupantSources<'a>>,
    pub mine: Option<MineTargetSources<'a>>,
    pub fractions: &'a BTreeMap<u32, RegionFractions>,
    pub dlc_mask: Option<u32>,
    pub mine_maximum_raw: i64,
}

impl Scans<'_> {
    pub fn admission(
        &self,
        wing: &AirWingRow,
        country: MissionCountry,
        region: u32,
        inputs: &AirMissionInputRow,
    ) -> TargetAdmission {
        target_admission(
            wing.mission_mask,
            inputs.reached_count,
            self.fractions.get(&region).copied(),
            self.dlc_mask,
            self.predicates(wing, country, region, inputs),
        )
    }

    pub fn predicates(
        &self,
        wing: &AirWingRow,
        country: MissionCountry,
        region: u32,
        inputs: &AirMissionInputRow,
    ) -> TargetPredicates {
        let mask = scanned_bits(wing.mission_mask);
        let reached = &inputs.reached_provinces;
        let mut targets = TargetPredicates::default();
        if inputs.reached_count == 0 {
            return targets;
        }
        if mask & (1 << 1) != 0 {
            targets.tactical = self
                .tactical
                .as_ref()
                .and_then(|scan| scan.admit(wing, country, reached));
        }
        if mask & (1 << 8) != 0 {
            targets.port = self
                .port
                .as_ref()
                .and_then(|scan| scan.admit(country, reached));
        }
        if mask & 0x1_0008 != 0
            && let Some(scan) = &self.strategic
        {
            let strategic = scan.admit(country, region, reached);
            targets.strategic = strategic.found;
            targets.strategic_railway_gun = strategic.railway_gun;
        }
        if mask & (1 << 14) != 0 {
            targets.scout = self
                .scout
                .as_ref()
                .and_then(|scan| scan.admit(wing, region, inputs.definition_map_flags));
        }
        if mask & 0x90 != 0 {
            targets.naval_combat = self
                .naval_combat
                .as_ref()
                .and_then(|scan| scan.admit_combat(country, reached));
            targets.naval_occupants = self
                .occupants
                .as_ref()
                .map(|scan| scan.admit(country, reached))
                .unwrap_or_default();
        }
        if mask & AIR_MISSION_LAY_MINES != 0 {
            targets.mine = self
                .mine
                .as_ref()
                .and_then(|scan| scan.admit(wing, region, self.mine_maximum_raw));
        }
        targets
    }
}
