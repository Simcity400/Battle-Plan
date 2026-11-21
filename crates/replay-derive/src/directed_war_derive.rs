
use crate::{
    frame::DecodedFrame,
    rows::{CountryLinkRow, WarRelationRow, tables},
};

type Fact<T> = Result<T, String>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiverWars {
    receiver_slot: u32,
    targets: Vec<u32>,
}

impl ReceiverWars {
    pub fn read(frame: &DecodedFrame, receiver_slot: u32) -> Fact<Self> {
        let wars = frame
            .complete::<tables::WarRelations>()
            .map_err(|error| error.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|error| error.to_string())?;
        Self::from_rows(wars, links, receiver_slot)
    }

    pub fn from_rows(
        wars: &[WarRelationRow],
        links: &[CountryLinkRow],
        receiver_slot: u32,
    ) -> Fact<Self> {
        if !links.windows(2).all(|pair| pair[0].id < pair[1].id) {
            return Err("CountryLinks rows are not in ascending raw-id order".into());
        }
        let slot = |raw: u32| {
            links
                .binary_search_by_key(&raw, |row| row.id)
                .ok()
                .map(|index| links[index].canonical_slot)
        };
        let receiver = links.iter().filter(|row| row.canonical_slot == receiver_slot).map(|row| row.id);
        Self::from_pairs(wars.iter().map(|row| (row.first_country, row.second_country)), receiver, slot, receiver_slot)
    }

    pub fn from_pairs(
        wars: impl IntoIterator<Item = (u32, u32)>,
        receiver: impl IntoIterator<Item = u32>,
        slot_of: impl Fn(u32) -> Option<u32>,
        receiver_slot: u32,
    ) -> Fact<Self> {
        if receiver_slot == 0 {
            return Err("derived direct wars need a positive receiver".into());
        }
        let slot = |raw: u32| slot_of(raw).ok_or_else(|| format!("WarRelations names country {raw}, which CountryLinks lacks"));
        let receiver: Vec<u32> = receiver.into_iter().collect();
        let mut targets = Vec::new();
        for (first_country, second_country) in wars {
            let other = match (
                receiver.contains(&first_country),
                receiver.contains(&second_country),
            ) {
                (true, false) => second_country,
                (false, true) => first_country,
                (false, false) => continue,
                (true, true) => {
                    return Err(format!(
                        "WarRelations joins country slot {receiver_slot} to itself"
                    ));
                }
            };
            match slot(other)? {
                0 => {
                    return Err(format!(
                        "WarRelations links country {other} to the reserved slot"
                    ));
                }
                target => targets.push(target),
            }
        }
        targets.sort_unstable();
        targets.dedup();
        Ok(Self {
            receiver_slot,
            targets,
        })
    }

    pub fn targets(&self) -> &[u32] {
        &self.targets
    }

    pub fn pointer_active(&self, target_slot: u32) -> bool {
        self.targets.binary_search(&target_slot).is_ok()
    }
}

#[cfg(test)]
mod tests;
