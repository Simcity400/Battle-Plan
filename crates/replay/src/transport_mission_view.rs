//! Borrowed same-hour view of one transport mission source packet.
//! No selection or route state is retained. Nested spotting/damage adapters
//! compose the independently admitted numerical source domains.
use crate::{
    rows::{
        Recorded, TransportMissionInputs, TransportMissionReceiver, TransportNavalSearch,
        TransportNavy,
    },
    transport_spotting::{self, Defines},
};

pub type Fact<T> = Result<T, &'static str>;

pub const RISK_REFUSAL_NAMES: [&str; 22] = [
    "transport original embedded mission owner refused",
    "transport original receiver risk graph refused",
    "transport original contributor detection stat refused",
    "transport original contributor carrier detection stat refused",
    "transport original contributor SHIPS header refused",
    "transport original coordination modifier refused",
    "transport original spotting modifier refused",
    "transport original night spotting modifier refused",
    "transport original visibility modifier refused",
    "transport original speed base refused",
    "transport original speed modifier refused",
    "transport original terrain class refused",
    "transport original primary visibility stat refused",
    "transport original secondary visibility stat refused",
    "transport original roster owner refused",
    "transport original receiver location refused",
    "transport original receiver region refused",
    "transport original location controller refused",
    "transport original detached activity refused",
    "transport original targeted spotting modifier refused",
    "transport original class terrain speed refused",
    "transport original contributor detection modifier refused",
];

/// Current capture admission is independent of any viewer-local selection.
/// Every ledger receiver supplies contribution stats; its own original gate
/// alone demands the later risk graph. Missing fields never stand for zero.
pub fn risk_refusals(receiver: &TransportMissionReceiver) -> [bool; RISK_REFUSAL_NAMES.len()] {
    let mut missing = [false; RISK_REFUSAL_NAMES.len()];
    let own = receiver.in_regions.is_some_and(|flag| flag != 0)
        && receiver.combat_references.is_some_and(|count| count <= 0);
    missing[0] = own && receiver.mission_owner.is_none();
    missing[1] = receiver.risk.is_none();
    missing[21] = receiver.combined_detection.is_none();
    let Some(risk) = receiver.risk.as_ref() else {
        return missing;
    };
    missing[2] = risk
        .stats
        .as_ref()
        .is_none_or(|stats| stats.detection_138.is_none());
    missing[3] = risk
        .stats
        .as_ref()
        .is_none_or(|stats| stats.carrier_detection_160.is_none());
    missing[4] = risk.ship_count.is_none_or(|count| count < 0);
    if own {
        missing[5..21].copy_from_slice(&[
            risk.coordination.is_none(),
            risk.spotting_chance.is_none(),
            risk.night_spotting_chance.is_none(),
            risk.visibility.is_none(),
            risk.speed_base.is_none(),
            risk.speed_modifier.is_none(),
            risk.terrain_class.is_none(),
            risk.stats
                .as_ref()
                .is_none_or(|stats| stats.visibility_148.is_none()),
            risk.stats
                .as_ref()
                .is_none_or(|stats| stats.visibility_150.is_none()),
            risk.roster_owner.is_none(),
            risk.location.is_none(),
            risk.region.is_none(),
            risk.location_controller.is_none(),
            risk.detached_activity.is_none(),
            risk.targeted_spotting_modifier.is_none(),
            risk.actual_terrain_speed.is_none(),
        ]);
    }
    missing
}

pub struct MissionView<'a> {
    navies: Fact<&'a [TransportNavy]>,
    receivers: Fact<&'a [TransportMissionReceiver]>,
    naval_search: Fact<&'a TransportNavalSearch>,
}

impl<'a> MissionView<'a> {
    /// The caller borrows the current frame's complete required singleton.
    /// A missing/partial source table must be rejected before constructing
    /// this view. The view cannot outlive that frame's packet.
    pub fn new(input: &'a Recorded<Option<TransportMissionInputs>>, frame_hour: i32) -> Fact<Self> {
        let packet = match input {
            Recorded::NotRecorded => return Err("transport mission packet not recorded"),
            Recorded::Value(None) => return Err("transport mission packet refused"),
            Recorded::Value(Some(packet)) => packet,
        };
        if packet.sample_hour != frame_hour {
            return Err("transport mission packet has wrong sample hour");
        }
        // Validate each borrowed key domain once per query, not once per
        // repeated native occurrence. Refusals remain domain-independent.
        Ok(Self {
            navies: navy_domain(packet),
            receivers: receiver_domain(packet),
            naval_search: packet
                .naval_search
                .as_ref()
                .ok_or("transport naval search graph not captured"),
        })
    }

    /// The physical slot is already selected by the native country/navy
    /// adapter. It is never a raw country reference or viewer-selected tag.
    pub fn ledger(&self, slot: u32, region: u32) -> Fact<&'a [u64]> {
        let navies = self.navies?;
        // A null manager or empty original array supplies null for every
        // requested strategic navy slot.
        if navies.is_empty() {
            return Ok(&[]);
        }
        let navy = navies
            .get(slot as usize)
            .ok_or("transport physical navy slot outside original domain")?;
        if !navy.present {
            return Ok(&[]);
        }
        let count = navy
            .region_slots
            .ok_or("transport navy region header refused")?;
        if region >= count {
            return Err("transport region outside original navy domain");
        }
        let ledgers = navy
            .ledgers
            .as_deref()
            .ok_or("transport navy ledger headers refused")?;
        match ledgers.binary_search_by_key(&region, |ledger| ledger.region) {
            Ok(index) => ledgers[index]
                .occurrences
                .as_deref()
                .ok_or("transport region ledger refused"),
            Err(_) => Ok(&[]),
        }
    }

    /// Call only for a nonzero original ledger occurrence. Zero skips before
    /// demanding this dictionary; a missing nonzero receiver is a refusal.
    pub fn receiver(&self, full_handle: u64) -> Fact<&'a TransportMissionReceiver> {
        if full_handle == 0 {
            return Err("transport null occurrence has no receiver");
        }
        let receivers = self.receivers?;
        receivers
            .binary_search_by_key(&full_handle, |receiver| receiver.task_force)
            .map(|index| &receivers[index])
            .map_err(|_| "transport mission receiver missing")
    }

    pub fn physical_navies(&self) -> Fact<&'a [TransportNavy]> {
        self.navies
    }

    pub fn naval_search(&self) -> Fact<&'a TransportNavalSearch> {
        self.naval_search
    }

    /// Naval access queries use the current physical navy's dense byte array.
    /// Null or empty managers supply empty operands.
    pub fn access_modes(&self, slot: u32) -> Fact<&'a [u8]> {
        let navies = self.navies?;
        if navies.is_empty() {
            return Ok(&[]);
        }
        let navy = navies
            .get(slot as usize)
            .ok_or("transport physical navy access slot outside original domain")?;
        if !navy.present {
            return Ok(&[]);
        }
        navy.access_modes
            .as_deref()
            .ok_or("transport original navy access modes refused")
    }

    pub fn receivers(&self) -> Fact<&'a [TransportMissionReceiver]> {
        self.receivers
    }
}

fn navy_domain(packet: &TransportMissionInputs) -> Fact<&[TransportNavy]> {
    let navies = packet
        .navies
        .as_deref()
        .ok_or("transport physical navy domain refused")?;
    if navies.first().is_some_and(|navy| navy.present) {
        return Err("transport physical navy domain lacks native null slot zero");
    }
    for (index, navy) in navies.iter().enumerate() {
        if u32::try_from(index).ok() != Some(navy.slot) {
            return Err("transport physical navy domain is incomplete");
        }
        if !navy.present {
            if navy.region_slots != Some(0)
                || navy.ledgers.as_deref() != Some(&[])
                || navy.access_modes.as_deref() != Some(&[])
            {
                return Err("transport null navy has nonnull operands");
            }
        } else if let Some(ledgers) = navy.ledgers.as_deref() {
            let count = navy
                .region_slots
                .ok_or("transport navy ledgers lack original region count")?;
            if ledgers.iter().any(|ledger| ledger.region >= count)
                || ledgers
                    .windows(2)
                    .any(|pair| pair[0].region >= pair[1].region)
            {
                return Err("transport navy ledger keys invalid");
            }
        }
    }
    Ok(navies)
}

fn receiver_domain(packet: &TransportMissionInputs) -> Fact<&[TransportMissionReceiver]> {
    let receivers = packet
        .receivers
        .as_deref()
        .ok_or("transport mission receiver dictionary refused")?;
    if receivers
        .iter()
        .any(|receiver| receiver.task_force >> 32 == 0)
        || receivers
            .windows(2)
            .any(|pair| pair[0].task_force >= pair[1].task_force)
    {
        return Err("transport mission receiver dictionary keys invalid");
    }
    Ok(receivers)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectionOperands {
    pub combined_detection: i64,
    pub multiplier: i64,
}

/// Lazy mission gate. Earlier zero/positive values make later refused
/// fields irrelevant; negative combat counts still admit.
pub fn demanded(receiver: &TransportMissionReceiver) -> Fact<bool> {
    if receiver
        .in_regions
        .ok_or("transport mission in-regions refused")?
        == 0
    {
        return Ok(false);
    }
    Ok(receiver
        .combat_references
        .ok_or("transport mission combat header refused")?
        <= 0)
}

/// Content defines are supplied only after `demanded` returns true. This
/// preserves the native unreached branch when later graph/content fails.
pub fn detection(receiver: &TransportMissionReceiver, defines: Defines) -> Fact<DetectionOperands> {
    if !demanded(receiver)? {
        return Err("transport detection branch not reached");
    }
    let combined_detection = receiver
        .combined_detection
        .ok_or("transport combined detection refused")?;
    let cache = receiver
        .detection_cache
        .ok_or("transport detection cache refused")?;
    let supply = receiver
        .received_supply_ratio
        .ok_or("transport received supply refused")?;
    Ok(DetectionOperands {
        combined_detection,
        multiplier: transport_spotting::detection_multiplier(
            cache,
            defines,
            true,
            Ok(Some(supply)),
        )?,
    })
}

#[cfg(test)]
#[path = "transport_mission_view_tests.rs"]
mod tests;
