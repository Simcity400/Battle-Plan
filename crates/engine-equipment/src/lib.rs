
#[cfg(test)]
use engine_native::read_census;
use engine_native::{
    Memory,
    allocation,
    cequipment_module,
    cequipment_upgrade,
    cequipment_variant,
    cpdx_array,
    memory,
    read_slice,
    walk,
};
use engine_native_politics::{equipment_module_entry, equipment_upgrade_entry};

pub mod cequipment_upgrades_instance;
pub mod equipment_module_container;
