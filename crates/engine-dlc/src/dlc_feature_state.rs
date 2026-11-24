
use engine_sites::Rva;
pub const MASK_RVA: Rva = Rva(0x033d_d698);
#[cfg(test)]
pub const EQUIPMENT_LOSSES_FEATURE: u32 = 3;
pub const EQUIPMENT_LOSSES_MASK: u32 = 0x0000_0002;
pub const DEEPER_FACTIONS_MASK: u32 = 0x0000_8000;
pub const THUNDER_AT_OUR_GATES: u32 = 0x0002_0000;
pub const NAME_MASKS: &[(&str, u32)] = &[
    ("Poland: United and Ready", 0x0000_0001),
    ("Together for Victory", EQUIPMENT_LOSSES_MASK),
    ("Death or Dishonor", 0x0000_0004),
    ("Waking the Tiger", 0x0000_0008),
    ("Man the Guns", 0x0000_0010),
    ("La Resistance", 0x0000_0020),
    ("Battle for the Bosporus", 0x0000_0040),
    ("No Step Back", 0x0000_0080),
    ("By Blood Alone", 0x0000_0100),
    ("Axis Armor Pack", 0x0000_0200),
    ("Arms Against Tyranny", 0x0000_0400),
    ("Trial of Allegiance", 0x0000_0800),
    ("Expansion Pass 1: Supporter Pack", 0x0000_1000),
    ("Gotterdammerung", 0x0000_2000),
    ("Graveyard of Empires", 0x0000_4000),
    ("No Compromise, No Surrender", 0x0000_8000),
    ("Peace For Our Time", 0x0001_0000),
    ("Thunder at Our Gates", 0x0002_0000),
];

