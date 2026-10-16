//! Synthetic unit-type definitions shared by composition and glyph-inventory tests.
use installed_content::unit_leader::SubUnits;

pub fn subunits() -> SubUnits {
    let mut subunits = SubUnits::default();
    subunits.read(
        b"sub_units={Foot={type=infantry} Horse={type=motorized} Boat={type=screen_ship}}",
        "synthetic.txt",
    );
    subunits
}
