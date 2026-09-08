use super::*;
use super::super::country_roster::{Cause, Kind, Observation};

fn rosters() -> (Observation, Observation) {
    (Observation::unavailable(Kind::Division, Cause::ExternalBridge),
     Observation::unavailable(Kind::Railway, Cause::ExternalBridge))
}

#[test]
fn input_size_covers_every_pool_and_append_preserves_funded_allocation() {
    let (division, railway) = rosters();
    let ordinary = [9, 7, 9];
    let alternate = [11];
    let routes = [(Some(ordinary.as_slice()), None), (None, Some(alternate.as_slice()))];
    let preference = TemplateModelPreference {
        template_id: 73, entity: "tank".into(), category: Some(0),
        excluded_variants: Some(vec![None, Some(11)]),
    };
    let acclimatization = Acclimatization {
        maximum_raw: -1,
        categories: vec![AcclimatizationCategory { category: "cold".into(), weight_raw: -2 }],
    };
    let preferences = [Some(&preference), None];
    let values = [Some(&acclimatization), None];
    let len = inputs_encoded_len(routes.into_iter(), preferences.into_iter(), values.into_iter(), &division, &railway).unwrap();
    assert_eq!(len, 32 + 2 * ROW_LEN + 16 + 4 + 14 + 16 + 8 + 64);
    let mut out = Vec::new();
    out.try_reserve_exact(len).unwrap();
    out.resize(32 + 2 * ROW_LEN, 0);
    out[8..12].copy_from_slice(&2_u32.to_le_bytes());
    write_current_template(&mut out, 0, Some(Some(73)));
    let allocation = (out.as_ptr(), out.capacity());
    append_inputs(&mut out, routes.into_iter(), preferences.into_iter(), values.into_iter(), &division, &railway).unwrap();
    assert_eq!(out.len(), len);
    assert_eq!((out.as_ptr(), out.capacity()), allocation);
    assert_eq!(word(&out, 28), FORMAT_MARKER);
    assert_eq!(word(&out, 32 + 124), 4);
    assert_eq!(word(&out, 32 + ROW_LEN + 184), 3);
    assert_eq!(word(&out, 32 + ROW_LEN + 188), 2);
    assert_eq!(Observation::parse(&out[len - 64..len - 32], Kind::Division).unwrap(), division);
    assert_eq!(Observation::parse(&out[len - 32..], Kind::Railway).unwrap(), railway);
}

#[test]
fn input_append_rejects_capacity_header_and_custody_before_mutation() {
    let (division, railway) = rosters();
    let mut out = vec![0; 32 + ROW_LEN];
    out[8..12].copy_from_slice(&1_u32.to_le_bytes());
    let before = out.clone();
    assert!(append_inputs(&mut out, [(None, None)].into_iter(), [None].into_iter(), [None].into_iter(), &division, &railway)
        .unwrap_err().contains("capacity"));
    assert_eq!(out, before);
    let len = inputs_encoded_len([(None, None)].into_iter(), [None].into_iter(), [None].into_iter(), &division, &railway).unwrap();
    out.try_reserve_exact(len - out.len()).unwrap();
    out[8..12].copy_from_slice(&0_u32.to_le_bytes());
    let before = out.clone();
    assert!(append_inputs(&mut out, [(None, None)].into_iter(), [None].into_iter(), [None].into_iter(), &division, &railway)
        .unwrap_err().contains("cardinality"));
    assert_eq!(out, before);
    out[8..12].copy_from_slice(&1_u32.to_le_bytes());
    out[40..44].copy_from_slice(&(1_u32 << 12).to_le_bytes());
    let before = out.clone();
    assert!(append_inputs(&mut out, [(Some([].as_slice()), None)].into_iter(), [None].into_iter(), [None].into_iter(), &division, &railway)
        .unwrap_err().contains("deleted"));
    assert_eq!(out, before);
}

#[test]
fn input_size_rejects_wrong_cardinality_roster_kinds_and_invalid_values() {
    let (division, railway) = rosters();
    assert!(inputs_encoded_len([(None, None)].into_iter(), [].into_iter(), [None].into_iter(), &division, &railway).is_err());
    assert!(inputs_encoded_len([].into_iter(), [].into_iter(), [].into_iter(), &railway, &division).is_err());
    for id in [0_u32, i32::MAX as u32, u32::MAX] {
        assert!(inputs_encoded_len([(Some([id].as_slice()), None)].into_iter(), [None].into_iter(), [None].into_iter(), &division, &railway).is_err());
    }
    let preference = TemplateModelPreference {
        template_id: 73, entity: "tank".into(), category: None, excluded_variants: Some(vec![Some(0)]),
    };
    assert!(inputs_encoded_len([(None, None)].into_iter(), [Some(&preference)].into_iter(), [None].into_iter(), &division, &railway).is_err());
    let value = Acclimatization {
        maximum_raw: 0,
        categories: vec![AcclimatizationCategory { category: "cold".into(), weight_raw: 1 }; 2],
    };
    let len = inputs_encoded_len([(None, None)].into_iter(), [None].into_iter(), [Some(&value)].into_iter(), &division, &railway).unwrap();
    let mut out = Vec::new();
    out.try_reserve_exact(len).unwrap();
    out.resize(32 + ROW_LEN, 0);
    out[8..12].copy_from_slice(&1_u32.to_le_bytes());
    let before = out.clone();
    assert!(append_inputs(&mut out, [(None, None)].into_iter(), [None].into_iter(), [Some(&value)].into_iter(), &division, &railway)
        .unwrap_err().contains("repeats"));
    assert_eq!(out, before);
}

#[test]
fn input_append_checks_exact_category_keys_after_large_permuted_census() {
    let (division, railway) = rosters();
    let mut value = Acclimatization {
        maximum_raw: 0,
        categories: (0..4097).map(|index| AcclimatizationCategory {
            category: format!("category-{:04}", (index * 257) % 4097), weight_raw: 0,
        }).collect(),
    };
    let len = inputs_encoded_len([(None, None)].into_iter(), [None].into_iter(), [Some(&value)].into_iter(), &division, &railway).unwrap();
    let mut out = Vec::new();
    out.try_reserve_exact(len).unwrap();
    out.resize(32 + ROW_LEN, 0);
    out[8..12].copy_from_slice(&1_u32.to_le_bytes());
    let before = out.clone();
    append_inputs(&mut out, [(None, None)].into_iter(), [None].into_iter(), [Some(&value)].into_iter(), &division, &railway).unwrap();
    assert_eq!(out.len(), len);
    out.truncate(before.len());
    out.copy_from_slice(&before);
    value.categories[4096].category = value.categories[0].category.clone();
    assert!(append_inputs(&mut out, [(None, None)].into_iter(), [None].into_iter(), [Some(&value)].into_iter(), &division, &railway)
        .unwrap_err().contains("repeats"));
    assert_eq!(out, before);
}

#[test]
fn input_append_rejects_duplicate_rosters_before_publishing_header_or_rows() {
    use super::super::country_roster::{Acquisition, Country, MemberRef, Slot};
    for kind in [Kind::Division, Kind::Railway] {
        let duplicate = Observation {
            kind, country_count: Some(1), failure: None,
            countries: vec![Country {
                physical_slot: 0, owner_tag_db_id: Some(7), acquisition: Acquisition::Bracketed,
                observed_count: Some(2), failure: None,
                slots: (0..2).map(|ordinal| Slot {
                    ordinal, reference: Ok(MemberRef { native_type: 4713, id: 9 }),
                }).collect(),
            }],
        };
        let (mut division, mut railway) = rosters();
        if kind == Kind::Division { division = duplicate; } else { railway = duplicate; }
        let len = inputs_encoded_len([].into_iter(), [].into_iter(), [].into_iter(), &division, &railway).unwrap();
        let mut out = Vec::new();
        out.try_reserve_exact(len).unwrap();
        out.resize(32, 0);
        out[28..32].copy_from_slice(b"WAIT");
        let before = out.clone();
        let allocation = (out.as_ptr(), out.capacity());
        assert!(append_inputs(&mut out, [].into_iter(), [].into_iter(), [].into_iter(), &division, &railway)
            .unwrap_err().contains("ambiguous member identity"));
        assert_eq!(out, before);
        assert_eq!((out.as_ptr(), out.capacity()), allocation);
    }
}

#[derive(Clone)]
struct WrongLen<I> { inner: I, announced: usize }
impl<I: Iterator> Iterator for WrongLen<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> { self.inner.next() }
    fn size_hint(&self) -> (usize, Option<usize>) { (self.announced, Some(self.announced)) }
}
impl<I: Iterator> ExactSizeIterator for WrongLen<I> {}

#[test]
fn input_size_checks_iterator_cardinality_and_u32_row_width() {
    let (division, railway) = rosters();
    for announced in [0, 2] {
        let routes = WrongLen { inner: [(None, None)].into_iter(), announced };
        assert!(inputs_encoded_len(routes, std::iter::repeat_n(None, announced), std::iter::repeat_n(None, announced), &division, &railway)
            .unwrap_err().contains("cardinality"));
    }
    if let Some(announced) = (u32::MAX as usize).checked_add(1) {
        let routes = WrongLen { inner: std::iter::empty(), announced };
        let preferences = WrongLen { inner: std::iter::empty(), announced };
        let values = WrongLen { inner: std::iter::empty(), announced };
        assert!(inputs_encoded_len(routes, preferences, values, &division, &railway).unwrap_err().contains("overflow"));
    }
}
