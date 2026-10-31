//! Packs and recipients read back exactly as written, every delta rebuilds the pack it was
//! written from whatever changed, and damaged bytes are refused.

use super::*;

fn unit(reference: u64, rate: Option<i64>) -> FuelDemandUnit {
    FuelDemandUnit {
        reference,
        parent: reference / 7,
        ordinal: (reference % 5) as u32,
        owners: (reference % 2 == 0).then_some([1, 2, u32::MAX]),
        rate_raw: rate,
        reason: if rate.is_none() {
            b"no rate".to_vec()
        } else {
            Vec::new()
        },
        name: (reference % 3 != 0).then(|| format!("Unit {reference}").into_bytes()),
        name_reason: if reference % 3 == 0 {
            b"unnamed".to_vec()
        } else {
            Vec::new()
        },
        group: (reference % 4 != 0).then_some(reference / 10),
        group_reason: Vec::new(),
    }
}

fn wing(reference: u64, rate: Result<i64, String>) -> Wing {
    Wing {
        reference,
        owner: (reference % 2 == 1).then_some(7),
        owner_reason: Vec::new(),
        pool_ordinal: 1,
        wing_ordinal: reference as u32,
        name: Some(vec![0xC3, 0xA9, b'w']),
        name_reason: Vec::new(),
        rate,
    }
}

/// A pack of `t`'s shape: a sliding window of divisions, one of whose rates moves with `t`,
/// task forces, sites and their wings, commands and a fleet.
fn shaped(t: u64) -> Pack {
    let divisions = (t / 3..t / 3 + 6 + t % 4)
        .map(|reference| {
            unit(
                reference,
                Some(reference as i64 * 3 + (reference == t) as i64),
            )
        })
        .collect();
    Pack {
        divisions,
        task_forces: if t % 5 == 0 {
            Vec::new()
        } else {
            vec![unit(900, None), unit(901, Some(-i64::MAX))]
        },
        sites: (0..3)
            .map(|site| Site {
                list: site as u8,
                ordinal: site,
                base: 5000 + u64::from(site),
                container_owner: (site != 1).then_some(3),
                capacity_raw: Some(i64::from(site) * 100 - 50),
                demilitarized: (site == 2).then_some(1),
                reason: Vec::new(),
                wings: (0..1 + (t + u64::from(site)) % 3)
                    .map(|index| {
                        let rate = if index == 1 {
                            Err("fuel current wing operands unavailable".to_owned())
                        } else {
                            Ok(index as i64 * 11 - (t % 2) as i64)
                        };
                        wing(60 + index, rate)
                    })
                    .collect(),
            })
            .collect(),
        groups: vec![
            FuelDemandGroup {
                reference: 70,
                country: 9,
                parent: 0,
                army_group: 1,
                name: None,
                name_reason: b"no name".to_vec(),
                members: (0..t % 4).collect(),
                children: vec![71],
            },
            FuelDemandGroup {
                reference: 71,
                country: 9,
                parent: 70,
                army_group: 0,
                name: Some(b"Second".to_vec()),
                name_reason: Vec::new(),
                members: vec![1, 2],
                children: Vec::new(),
            },
        ],
        fleets: vec![FuelDemandFleet {
            reference: u64::MAX,
            name: (t % 10 != 0).then(|| b"Fleet".to_vec()),
            name_reason: Vec::new(),
        }],
        group_reason: if t % 7 == 0 {
            b"topology refused".to_vec()
        } else {
            Vec::new()
        },
    }
}

fn rebuilt(bytes: Vec<u8>) -> Demand {
    let mut chain = Chain::default();
    chain.keyframe(bytes).unwrap();
    chain.pack().unwrap()
}

#[test]
fn every_pack_reads_back_whole_from_its_keyframe() {
    let demands = [
        Ok(Pack::default()),
        Ok(shaped(0)),
        Ok(shaped(13)),
        Err("Individual current fuel consumption was not recorded".to_owned()),
    ];
    for demand in demands {
        let encoded = Encoded::of(&demand).unwrap();
        let mut bytes = Vec::new();
        keyframe(&encoded, &mut bytes);
        assert_eq!(encoded_of(&bytes).unwrap(), encoded);
        assert_eq!(rebuilt(bytes), demand);
    }
}

#[test]
fn deltas_rebuild_every_pack_whatever_changed() {
    let mut demands: Vec<Pack> = (0..40).map(shaped).collect();
    // A list reordered, an element repeated, every list emptied, then all new.
    let mut reordered = shaped(40);
    reordered.divisions.reverse();
    reordered.divisions.push(reordered.divisions[0].clone());
    demands.push(reordered);
    demands.push(Pack::default());
    demands.push(shaped(1000));
    let mut chain = Chain::default();
    let mut before = Encoded::of(&Ok(demands[0].clone())).unwrap();
    let mut bytes = Vec::new();
    keyframe(&before, &mut bytes);
    chain.keyframe(bytes).unwrap();
    for pack in &demands[1..] {
        let after = Encoded::of(&Ok(pack.clone())).unwrap();
        let mut bytes = Vec::new();
        delta(&before, &after, &mut bytes).unwrap();
        assert_eq!(apply(&before, &bytes).unwrap(), after);
        chain.delta(bytes).unwrap();
        assert_eq!(chain.pack().unwrap(), Ok(pack.clone()));
        before = after;
    }
}

#[test]
fn a_pack_changing_one_element_writes_that_element_alone() {
    let mut big = shaped(0);
    big.divisions = (0..2000)
        .map(|reference| unit(reference, Some(1)))
        .collect();
    let mut changed = big.clone();
    changed.divisions[1000].rate_raw = Some(2);
    let (before, after) = (
        Encoded::of(&Ok(big)).unwrap(),
        Encoded::of(&Ok(changed)).unwrap(),
    );
    let (mut whole, mut changes) = (Vec::new(), Vec::new());
    keyframe(&after, &mut whole);
    delta(&before, &after, &mut changes).unwrap();
    assert!(
        changes.len() * 100 < whole.len(),
        "{} of {}",
        changes.len(),
        whole.len()
    );
}

#[test]
fn damaged_bytes_are_refused() {
    let encoded = Encoded::of(&Ok(shaped(5))).unwrap();
    let mut bytes = Vec::new();
    keyframe(&encoded, &mut bytes);
    for end in [0, 1, bytes.len() / 2, bytes.len() - 1] {
        assert!(Chain::default().keyframe(bytes[..end].to_vec()).is_err());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Chain::default().keyframe(longer).is_err());
    let after = Encoded::of(&Ok(shaped(6))).unwrap();
    let mut changes = Vec::new();
    delta(&encoded, &after, &mut changes).unwrap();
    let mut chain = Chain::default();
    assert!(
        chain.delta(changes.clone()).is_err(),
        "a delta needs a keyframe"
    );
    chain.keyframe(bytes).unwrap();
    // A copy past the pack before it.
    let mut beyond = Vec::new();
    super::bytes(&mut beyond, &[]);
    var(&mut beyond, 1);
    var(&mut beyond, 1000 << 1);
    var(&mut beyond, 1);
    for _ in 1..LISTS {
        var(&mut beyond, 0);
    }
    assert!(chain.delta(beyond).is_err());
    let refused = Encoded::of(&Err("refused".to_owned())).unwrap();
    assert!(delta(&refused, &after, &mut Vec::new()).is_err());
}

#[test]
fn recipients_read_back() {
    let parts = vec![
        FuelAwardDayRecipient {
            kind: 2,
            reference: u64::MAX,
            owner: 9,
            host: 4,
            name: Some(b"Wing".to_vec()),
            name_reason: Vec::new(),
            first_hour: -5,
            last_hour: i32::MAX,
            amount_raw: i64::MIN,
        },
        FuelAwardDayRecipient {
            kind: 3,
            name: None,
            name_reason: b"not read".to_vec(),
            ..Default::default()
        },
    ];
    let mut bytes = Vec::new();
    recipients(&mut bytes, &parts);
    assert_eq!(take_recipients(&bytes).unwrap(), parts);
    assert_eq!(take_recipients(&[]).unwrap(), Vec::new());
    assert!(take_recipients(&bytes[..bytes.len() - 1]).is_err());
}
