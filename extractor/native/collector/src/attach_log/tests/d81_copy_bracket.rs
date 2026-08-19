use super::*;
use std::time::Instant;

pub(crate) mod shape {
    pub(crate) const PROVINCES: usize = 13_413;

    pub(crate) const RAILED: usize = 3_510;

    pub(crate) const NEIGHBORS_PER_RAILED: usize = 4;

    pub(crate) const SELECTOR_ENTRIES: usize = 16_384;

    pub(crate) const SELECTOR_STRIDE: usize = 0x18;

    pub(crate) const TABLE_SLOTS: usize = 16_384;

    pub(crate) const PROBE_SLACK_ENTRIES: usize = 258;

    pub(crate) const NON_SELF_KEYED_RAILED: usize = 0;

    pub(crate) const PROBE_STEPS: usize = 2;

    pub(crate) const RAIL_SLOT: usize = 8;
    pub(crate) const ACTIVE_STRIDE: usize = 0x20;
    pub(crate) const POINTER: usize = 8;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadFamily {
    site: &'static str,
    contiguous: bool,
    calls: usize,
    bytes: usize,
}

fn d81_live_read_census() -> Vec<ReadFamily> {
    use shape::*;
    let railed = RAILED;
    let pair_bytes = NEIGHBORS_PER_RAILED * 8;
    vec![
        ReadFamily {
            site: "prologue roots/selector header/map container",
            contiguous: false,
            calls: 30,
            bytes: 210,
        },
        ReadFamily {
            site: "rail-table slot",
            contiguous: true,
            calls: 2 * PROVINCES,
            bytes: 2 * PROVINCES * RAIL_SLOT,
        },
        ReadFamily {
            site: "rail-info header window (opening + closing)",
            contiguous: false,
            calls: 4 * railed,
            bytes: 4 * railed * 16,
        },
        ReadFamily {
            site: "rail pair blob",
            contiguous: false,
            calls: 2 * railed,
            bytes: 2 * railed * pair_bytes,
        },
        ReadFamily {
            site: "active-table entry window (opening + closing)",
            contiguous: true,
            calls: 4 * railed,
            bytes: 4 * railed * 7,
        },
        ReadFamily {
            site: "gamestate province-array slot (opening + closing)",
            contiguous: true,
            calls: 4 * railed + 4 * NON_SELF_KEYED_RAILED,
            bytes: (4 * railed + 4 * NON_SELF_KEYED_RAILED) * POINTER,
        },
        ReadFamily {
            site: "map container province-array slot (opening + closing)",
            contiguous: true,
            calls: 4 * railed,
            bytes: 4 * railed * POINTER,
        },
        ReadFamily {
            site: "map province +0x11B8 record pointer (opening + closing)",
            contiguous: false,
            calls: 4 * railed,
            bytes: 4 * railed * POINTER,
        },
        ReadFamily {
            site: "selector-key record +0xC4",
            contiguous: false,
            calls: 2 * railed,
            bytes: 2 * railed * 4,
        },
        ReadFamily {
            site: "CProvince vtable + id",
            contiguous: false,
            calls: 4 * railed,
            bytes: 2 * railed * 8 + 2 * railed * 4,
        },
        ReadFamily {
            site: "anchor-selector hash probe",
            contiguous: true,
            calls: (4 * PROBE_STEPS + 2) * railed,
            bytes: (2 * PROBE_STEPS + 2 * PROBE_STEPS * 4 + 2 * 8) * railed,
        },
        ReadFamily {
            site: "anchor triplet window",
            contiguous: false,
            calls: 2 * railed,
            bytes: 2 * railed * 12,
        },
        ReadFamily {
            site: "whole-snapshot closing gates",
            contiguous: false,
            calls: 32,
            bytes: 222,
        },
    ]
}

fn d81_copy_plan_census() -> Vec<ReadFamily> {
    use shape::*;
    let railed = RAILED;
    let mut plan: Vec<ReadFamily> = Vec::new();

    for (site, bytes) in [
        ("rail table (whole)", TABLE_SLOTS * RAIL_SLOT),
        ("active table (whole)", TABLE_SLOTS * ACTIVE_STRIDE),
        ("gamestate province array (whole)", TABLE_SLOTS * POINTER),
        (
            "map container province array (whole)",
            TABLE_SLOTS * POINTER,
        ),
        (
            "anchor-selector table (whole)",
            (SELECTOR_ENTRIES + PROBE_SLACK_ENTRIES) * SELECTOR_STRIDE,
        ),
    ] {
        plan.push(ReadFamily {
            site,
            contiguous: true,
            calls: 2,
            bytes: 2 * bytes,
        });
    }

    plan.push(ReadFamily {
        site: "rail-info header window (opening + closing)",
        contiguous: false,
        calls: 4 * railed,
        bytes: 4 * railed * 16,
    });
    plan.push(ReadFamily {
        site: "rail pair blob",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * NEIGHBORS_PER_RAILED * 8,
    });
    plan.push(ReadFamily {
        site: "map province +0x11B8 record pointer (opening + closing)",
        contiguous: false,
        calls: 4 * railed,
        bytes: 4 * railed * POINTER,
    });
    plan.push(ReadFamily {
        site: "anchor triplet window",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * 24,
    });
    plan.push(ReadFamily {
        site: "selector-key record +0xC4",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * 4,
    });
    plan.push(ReadFamily {
        site: "CProvince vtable + id",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * 168,
    });

    plan.push(ReadFamily {
        site: "gamestate province-array slot closes (vector)",
        contiguous: true,
        calls: 2 * railed,
        bytes: 2 * railed * POINTER,
    });
    plan.push(ReadFamily {
        site: "map container province-array slot closes (vector)",
        contiguous: true,
        calls: 2 * railed,
        bytes: 2 * railed * POINTER,
    });
    plan.push(ReadFamily {
        site: "active-table entry window closes (vector)",
        contiguous: true,
        calls: 3 * railed,
        bytes: 3 * railed * 7,
    });

    for family in d81_live_read_census() {
        if family.site == "prologue roots/selector header/map container"
            || family.site == "whole-snapshot closing gates"
        {
            plan.push(family);
        }
    }

    plan
}

fn totals(census: &[ReadFamily]) -> (usize, usize) {
    census.iter().fold((0, 0), |(calls, bytes), family| {
        (calls + family.calls, bytes + family.bytes)
    })
}

/// `TABLE_SLOTS * RAIL_SLOT / 2`. Observed in `--release`:
#[test]
fn the_copy_plan_covers_every_range_the_live_reader_reads() {
    use shape::*;
    let live = d81_live_read_census();
    let plan = d81_copy_plan_census();

    for family in live.iter().filter(|family| !family.contiguous) {
        let carried = plan
            .iter()
            .find(|entry| entry.site == family.site)
            .unwrap_or_else(|| panic!("the plan drops the scattered site {:?}", family.site));
        assert!(
            carried.calls <= family.calls,
            "the plan must not issue MORE calls than the live reader at {:?}: \
             plan {}, live {}",
            family.site,
            carried.calls,
            family.calls
        );
        assert!(
            carried.bytes >= family.bytes,
            "a merged window must not read FEWER bytes than the scalars it \
             replaces at {:?}",
            family.site
        );
    }

    for (plan_site, extent, live_site) in [
        (
            "rail table (whole)",
            TABLE_SLOTS * RAIL_SLOT,
            "rail-table slot",
        ),
        (
            "active table (whole)",
            TABLE_SLOTS * ACTIVE_STRIDE,
            "active-table entry window (opening + closing)",
        ),
        (
            "gamestate province array (whole)",
            TABLE_SLOTS * POINTER,
            "gamestate province-array slot (opening + closing)",
        ),
        (
            "map container province array (whole)",
            TABLE_SLOTS * POINTER,
            "map container province-array slot (opening + closing)",
        ),
        (
            "anchor-selector table (whole)",
            (SELECTOR_ENTRIES + PROBE_SLACK_ENTRIES) * SELECTOR_STRIDE,
            "anchor-selector hash probe",
        ),
    ] {
        let copy = plan
            .iter()
            .find(|entry| entry.site == plan_site)
            .unwrap_or_else(|| panic!("the plan has no {plan_site} copy"));
        let passes = 2;
        let span = copy.bytes / passes;
        assert!(
            span >= extent,
            "the plan's {plan_site:?} copy spans {span} bytes, short of the \
             {extent} the live reader indexes into",
        );
        assert!(
            live.iter().any(|family| family.site == live_site),
            "the live census must still contain {live_site:?}, or this row \
             covers nothing"
        );
    }

    for (site, calls) in [
        (
            "gamestate province-array slot closes (vector)",
            2 * shape::RAILED,
        ),
        (
            "map container province-array slot closes (vector)",
            2 * shape::RAILED,
        ),
        (
            "active-table entry window closes (vector)",
            3 * shape::RAILED,
        ),
    ] {
        let close = plan
            .iter()
            .find(|entry| entry.site == site)
            .unwrap_or_else(|| panic!("the plan has no {site} family"));
        assert_eq!(
            close.calls, calls,
            "{site} must observe at the walk's own closing count"
        );
    }

    let (live_calls, _) = totals(&live);
    let (plan_calls, _) = totals(&plan);
    assert!(
        plan_calls < live_calls,
        "the copy plan issues {plan_calls} guarded calls against the live \
         reader's {live_calls}: it is not a reduction"
    );
}

/// cargo test -p collector --release d81_copy_bracket_cost_model -- --ignored --nocapture
#[test]
#[ignore]
fn d81_copy_bracket_cost_model() {
    const POOL_LEN: usize = 256 << 20;
    const TRIALS: usize = 9;

    let pool = vec![0x5Au8; POOL_LEN];
    let base = pool.as_ptr() as usize;
    let mut seed = 0xD8_1C_0B_1Au64;
    let mut splitmix = move || {
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };

    let rail_table = base;
    let active_table = rail_table + shape::TABLE_SLOTS * shape::RAIL_SLOT;
    let gs_array = active_table + shape::TABLE_SLOTS * shape::ACTIVE_STRIDE;
    let container_array = gs_array + shape::TABLE_SLOTS * shape::POINTER;
    let selector_table = container_array + shape::TABLE_SLOTS * shape::POINTER;
    let tables_end = selector_table
        + (shape::SELECTOR_ENTRIES + shape::PROBE_SLACK_ENTRIES) * shape::SELECTOR_STRIDE;

    let scatter_lo = tables_end - base + 4096;
    let scatter_span = POOL_LEN - scatter_lo - 8192;
    let mut scatter = |count: usize| -> Vec<usize> {
        (0..count)
            .map(|_| base + scatter_lo + ((splitmix() as usize) % (scatter_span / 64)) * 64)
            .collect()
    };
    let rail_infos = scatter(shape::RAILED);
    let pair_blobs = scatter(shape::RAILED);
    let map_provinces = scatter(shape::RAILED);
    let records = scatter(shape::RAILED);
    let cprovinces = scatter(shape::RAILED);

    page_cache_new_frame();

    let pair_bytes = shape::NEIGHBORS_PER_RAILED * 8;

    fn scattered_half(
        rail_infos: &[usize],
        pair_blobs: &[usize],
        map_provinces: &[usize],
        records: &[usize],
        pair_bytes: usize,
    ) -> u64 {
        let mut acc = 0u64;
        let mut window = [0u8; 16];
        let mut blob = vec![0u8; pair_bytes];
        let mut anchor = [0u8; 12];
        for index in 0..rail_infos.len() {
            for _ in 0..4 {
                acc = acc.wrapping_add(u64::from(read_into(rail_infos[index], &mut window)));
            }
            for _ in 0..2 {
                acc = acc.wrapping_add(u64::from(read_into(pair_blobs[index], &mut blob)));
            }
            for _ in 0..2 {
                acc = acc.wrapping_add(read_u64_stable(map_provinces[index]).unwrap_or(0));
            }
            acc = acc.wrapping_add(u64::from(read_u32_stable(records[index]).unwrap_or(0)));
            for _ in 0..2 {
                acc = acc.wrapping_add(u64::from(read_into(
                    map_provinces[index] + 0x400,
                    &mut anchor,
                )));
            }
        }
        acc
    }

    fn prologue_and_closing_gates(roots: &[usize]) -> u64 {
        let mut acc = 0u64;
        for _ in 0..2 {
            for slot in roots.iter().take(12) {
                acc = acc.wrapping_add(read_u64_stable(*slot).unwrap_or(0));
            }
            acc = acc.wrapping_add(u64::from(read_u32_stable(roots[12]).unwrap_or(0)));
            acc = acc.wrapping_add(u64::from(read_u32_stable(roots[13]).unwrap_or(0)));
            acc = acc.wrapping_add(u64::from(read_u8_stable(roots[14]).unwrap_or(0)));
        }
        acc = acc.wrapping_add(read_u64(roots[15]).unwrap_or(0));
        acc = acc.wrapping_add(u64::from(read_u32(roots[16]).unwrap_or(0)));
        acc
    }
    let roots = scatter(17);
    let live_roots = roots.clone();

    let live_rail_infos = rail_infos.clone();
    let live_pair_blobs = pair_blobs.clone();
    let live_map_provinces = map_provinces.clone();
    let live_records = records.clone();
    let live_cprovinces = cprovinces.clone();

    #[allow(clippy::type_complexity)]
    let mut arms: Vec<(&'static str, Box<dyn FnMut() -> u64>)> = vec![
        (
            "LIVE (today's in-bracket read sequence)",
            Box::new(move || {
                let mut acc = 0u64;
                for id in 0..shape::PROVINCES {
                    acc = acc.wrapping_add(
                        read_u64_stable(rail_table + id * shape::RAIL_SLOT).unwrap_or(0),
                    );
                }
                let stride = shape::PROVINCES / shape::RAILED;
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    let window = active_table + id * shape::ACTIVE_STRIDE + 0x12;
                    for _ in 0..4 {
                        acc = acc
                            .wrapping_add(read_scalar::<7>(window).map_or(0, |w| u64::from(w[0])));
                    }
                    for _ in 0..2 {
                        acc = acc.wrapping_add(
                            read_u64_stable(gs_array + id * shape::POINTER).unwrap_or(0),
                        );
                    }
                    if index < shape::NON_SELF_KEYED_RAILED {
                        for _ in 0..2 {
                            acc = acc.wrapping_add(
                                read_u64_stable(gs_array + id * shape::POINTER).unwrap_or(0),
                            );
                        }
                    }
                    for _ in 0..2 {
                        acc = acc.wrapping_add(
                            read_u64_stable(container_array + id * shape::POINTER).unwrap_or(0),
                        );
                    }
                    acc = acc.wrapping_add(read_u64_stable(live_cprovinces[index]).unwrap_or(0));
                    acc = acc.wrapping_add(u64::from(
                        read_u32_stable(live_cprovinces[index] + 0xA4).unwrap_or(0),
                    ));
                    let mut entry = selector_table
                        + (index * 7919 % shape::SELECTOR_ENTRIES) * shape::SELECTOR_STRIDE;
                    for _ in 0..shape::PROBE_STEPS {
                        acc = acc.wrapping_add(u64::from(read_u8_stable(entry + 4).unwrap_or(0)));
                        acc = acc.wrapping_add(u64::from(read_u32_stable(entry + 8).unwrap_or(0)));
                        entry += shape::SELECTOR_STRIDE;
                    }
                    acc = acc.wrapping_add(read_u64_stable(entry + 0x10).unwrap_or(0));
                }
                acc = acc.wrapping_add(prologue_and_closing_gates(&live_roots));
                acc.wrapping_add(scattered_half(
                    &live_rail_infos,
                    &live_pair_blobs,
                    &live_map_provinces,
                    &live_records,
                    pair_bytes,
                ))
            }),
        ),
        (
            "PLAN (vector families, demand-driven images)",
            Box::new(move || {
                let mut acc = 0u64;
                let mut rail = vec![0u8; shape::TABLE_SLOTS * shape::RAIL_SLOT];
                let mut active = vec![0u8; shape::TABLE_SLOTS * shape::ACTIVE_STRIDE];
                let mut gs = vec![0u8; shape::TABLE_SLOTS * shape::POINTER];
                let mut container = vec![0u8; shape::TABLE_SLOTS * shape::POINTER];
                let mut selector = vec![
                    0u8;
                    (shape::SELECTOR_ENTRIES + shape::PROBE_SLACK_ENTRIES)
                        * shape::SELECTOR_STRIDE
                ];
                for _ in 0..2 {
                    acc = acc.wrapping_add(u64::from(read_into(rail_table, &mut rail)));
                    acc = acc.wrapping_add(u64::from(read_into(active_table, &mut active)));
                    acc = acc.wrapping_add(u64::from(read_into(gs_array, &mut gs)));
                    acc = acc.wrapping_add(u64::from(read_into(container_array, &mut container)));
                    acc = acc.wrapping_add(u64::from(read_into(selector_table, &mut selector)));
                }
                let mut window = [0u8; 16];
                let mut blob = vec![0u8; pair_bytes];
                let mut anchors = [0u8; 24];
                let mut slot = [0u8; 8];
                let mut key = [0u8; 4];
                let mut head = [0u8; 168];
                let mut active_window = [0u8; 7];
                for pass in 0..2 {
                    for &rail_info in rail_infos.iter() {
                        for _ in 0..2 {
                            acc = acc.wrapping_add(u64::from(read_into(rail_info, &mut window)));
                        }
                    }
                    let _ = pass;
                }
                for &pair_blob in pair_blobs.iter() {
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(pair_blob, &mut blob)));
                    }
                }
                for _ in 0..2 {
                    for &map_province in map_provinces.iter() {
                        for _ in 0..2 {
                            acc = acc.wrapping_add(u64::from(read_into(map_province, &mut slot)));
                        }
                    }
                }
                for &map_province in map_provinces.iter() {
                    for _ in 0..2 {
                        acc = acc
                            .wrapping_add(u64::from(read_into(map_province + 0x400, &mut anchors)));
                    }
                }
                for &record in records.iter() {
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(record, &mut key)));
                    }
                }
                for &cprovince in cprovinces.iter() {
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(cprovince, &mut head)));
                    }
                }
                let stride = shape::PROVINCES / shape::RAILED;
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(
                            gs_array + id * shape::POINTER,
                            &mut slot,
                        )));
                    }
                }
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(
                            container_array + id * shape::POINTER,
                            &mut slot,
                        )));
                    }
                }
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    for _ in 0..3 {
                        acc = acc.wrapping_add(u64::from(read_into(
                            active_table + id * shape::ACTIVE_STRIDE + 0x12,
                            &mut active_window,
                        )));
                    }
                }
                acc.wrapping_add(prologue_and_closing_gates(&roots))
            }),
        ),
    ];

    let mut samples: Vec<Vec<f64>> = vec![Vec::with_capacity(TRIALS); arms.len()];
    let mut order: Vec<usize> = (0..arms.len() * TRIALS).map(|k| k % arms.len()).collect();
    let mut shuffle_seed = 0x51_ED_C0_DEu64;
    let mut shuffle = move || {
        shuffle_seed = shuffle_seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = shuffle_seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for k in (1..order.len()).rev() {
        let j = (shuffle() as usize) % (k + 1);
        order.swap(k, j);
    }
    for &index in &order {
        let started = Instant::now();
        let acc = (arms[index].1)();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(acc);
        samples[index].push(ms);
    }

    let median = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        sorted[sorted.len() / 2]
    };

    let live = d81_live_read_census();
    let plan = d81_copy_plan_census();
    let (live_calls, live_bytes) = totals(&live);
    let (plan_calls, plan_bytes) = totals(&plan);

    println!(
        "\n# d81 copy-only bracket cost model ({TRIALS} interleaved trials, \
         whole-frame medians, {} provinces / {} railed)",
        shape::PROVINCES,
        shape::RAILED
    );
    println!("\n## census (derived from the reader's call sites)\n");
    println!("| arm | guarded calls | bytes moved | measured wall |");
    println!("|---|---:|---:|---:|");
    for (index, (label, _)) in arms.iter().enumerate() {
        let (calls, bytes) = if index == 0 {
            (live_calls, live_bytes)
        } else {
            (plan_calls, plan_bytes)
        };
        println!(
            "| {label} | {calls} | {:.2} MiB | {:.2} ms |",
            bytes as f64 / (1024.0 * 1024.0),
            median(&samples[index])
        );
    }
    let live_ms = median(&samples[0]);
    let plan_ms = median(&samples[1]);
    println!(
        "\ncall reduction {:.2}x, wall {:.2}x ({live_ms:.2} ms -> {plan_ms:.2} ms)",
        live_calls as f64 / plan_calls as f64,
        live_ms / plan_ms.max(f64::MIN_POSITIVE),
    );

    println!("\n## every trial, ms (shuffled order; the spread is the output)\n");
    for (index, (label, _)) in arms.iter().enumerate() {
        let rendered: Vec<String> = samples[index]
            .iter()
            .map(|value| format!("{value:.2}"))
            .collect();
        println!("| {label} | {} |", rendered.join(" | "));
    }

    println!("\n## sensitivity: probe steps per selector lookup\n");
    println!("| probe steps | live calls | plan calls | call reduction |");
    println!("|---:|---:|---:|---:|");
    for steps in [1usize, 2, 4, 8] {
        let delta = (4 * steps + 2) as isize - (4 * shape::PROBE_STEPS + 2) as isize;
        let live_at = (live_calls as isize + delta * shape::RAILED as isize) as usize;
        println!(
            "| {steps} | {live_at} | {plan_calls} | {:.2}x |",
            live_at as f64 / plan_calls as f64
        );
    }
    println!("\n## per-site census\n");
    println!("| site | batchable | calls | bytes |");
    println!("|---|---|---:|---:|");
    for family in &live {
        println!(
            "| {} | {} | {} | {} |",
            family.site,
            if family.contiguous {
                "table"
            } else {
                "scattered"
            },
            family.calls,
            family.bytes
        );
    }
    println!(
        "\nSELECTOR_ENTRIES is an assumption ({}); at twice that the plan moves \
         {:.2} MiB more and issues no more calls.",
        shape::SELECTOR_ENTRIES,
        (2 * shape::SELECTOR_ENTRIES * shape::SELECTOR_STRIDE) as f64 / (1024.0 * 1024.0)
    );
}
