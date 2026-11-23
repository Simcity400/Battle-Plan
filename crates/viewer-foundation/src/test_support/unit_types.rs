use content_reader_script::sub_units::SubUnits;

pub fn subunits() -> SubUnits {
    let mut subunits = SubUnits::default();
    subunits.read(
        b"sub_units={Foot={type=infantry} Horse={type=motorized} Boat={type=screen_ship}}",
        "synthetic.txt",
    );
    subunits
}
