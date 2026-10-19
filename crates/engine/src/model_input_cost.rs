//! Synthetic capture cost of the template and clone walks that scale with the
//! country count: the full template dictionary walk, the hourly template
//! counter and 3D-model inputs, and the local sub-unit clones. Each country owns `TEMPLATES` templates over `CLONES` clones of a
//! shared global database, and one country in eight has a clone whose
//! requirements differ. The budget test runs in every build; the timings are
//! `#[ignore]`d benches:
//! `cargo test -p engine --release model_input_capture_cost -- --ignored --nocapture`.
//! `unit_scale_templates` instead spreads 50,000 and 100,000 templates over
//! 1,500 countries.

use std::cell::Cell;
use std::time::Instant;

use crate::{
    Memory, ccountry,
    ccountry::synthetic::World,
    cdeployment_status::{self, LocalRequirement, LocalSubunitRow},
    cequipment_type,
    creferenced_division_template::{self as template, IconRow, MemoSlot, Row, SubunitCount},
    csub_unit_definition as subunit,
    walk::Outcome,
};

const MODULE: usize = 0x7_0000_0000;
const GLOBALS: usize = 64;
const CLONES: usize = 4;
const TEMPLATES: usize = 6;
const GROUPED: usize = 3;
/// Every field the walks read lies below this.
const DEFINITION: usize = 0x5c0;
const TEMPLATE: usize = (template::EQUIPMENT_NICHE + 1).next_multiple_of(8);

struct Bench {
    world: World,
    database: usize,
    copies: Cell<usize>,
}

impl Memory for Bench {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        if address == MODULE + subunit::DATABASE_RVA && output.len() == 8 {
            output.copy_from_slice(&(self.database as u64).to_le_bytes());
            return true;
        }
        self.world.memory().copy(address, output)
    }

    /// Hints the host bytes behind a synthetic address, as the game's memory
    /// adapter hints the game's own.
    fn prefetch(&self, address: usize, len: usize) {
        if let Some(offset) = address.checked_sub(crate::ccountry::synthetic::START)
            && offset < self.world.bytes.len()
        {
            crate::prefetch::span(self.world.bytes.as_ptr() as usize + offset, len);
        }
    }
}

fn inline_text(world: &mut World, at: usize, text: &[u8]) {
    world.put(at, text);
    world.u64(at + crate::msvc_string::LENGTH, text.len() as u64);
    world.u64(
        at + crate::msvc_string::CAPACITY,
        crate::msvc_string::INLINE_CAPACITY,
    );
}

fn build(countries: u32, templates: usize) -> Bench {
    let mut world = World::new(MODULE, countries, ccountry::DEPLOYMENT_MANAGER + 8);
    let archetypes = [world.alloc(0x20), world.alloc(0x20)];
    for (index, &archetype) in archetypes.iter().enumerate() {
        world.u64(archetype, (MODULE + cequipment_type::VTABLE_RVA) as u64);
        world.u32(archetype + cequipment_type::TOKEN, 100 + index as u32);
        world.put(archetype + cequipment_type::ACTIVE, &[1]);
    }
    let definition = |world: &mut World, token: u32, index: i32, amount: i64| {
        let at = world.alloc(DEFINITION + archetypes.len() * 16);
        world.u64(at, (MODULE + subunit::VTABLE_RVA) as u64);
        world.u32(at + subunit::TOKEN, token);
        world.put(at + subunit::IS_REAL, &[1]);
        world.put(at + subunit::DATABASE_INDEX, &index.to_le_bytes());
        inline_text(world, at + subunit::NAME, b"infantry");
        let data = at + DEFINITION;
        world.array(
            at + subunit::ARCHETYPE_REQUIREMENTS_DATA,
            data,
            archetypes.len() as u32,
        );
        for (entry, &archetype) in archetypes.iter().enumerate() {
            world.u64(data + entry * 16, archetype as u64);
            world.put(data + entry * 16 + 8, &amount.to_le_bytes());
        }
        at
    };
    let database = world.alloc(subunit::DATABASE_VECTOR + 16);
    let vector = world.alloc((GLOBALS + 1) * 8);
    world.array(
        database + subunit::DATABASE_VECTOR,
        vector,
        GLOBALS as u32 + 1,
    );
    for index in 1..=GLOBALS {
        let global = definition(&mut world, index as u32, index as i32, 100_000);
        world.u64(vector + index * 8, global as u64);
    }
    for slot in 1..=countries as usize {
        let object = world.object(slot as u32);
        let manager = world.alloc(cdeployment_status::OWNER + 8);
        world.u64(manager, (MODULE + cdeployment_status::VTABLE_RVA) as u64);
        world.u64(manager + cdeployment_status::OWNER, object as u64);
        world.u64(object + ccountry::DEPLOYMENT_MANAGER, manager as u64);
        let mut clones = [0; CLONES];
        for (ordinal, clone) in clones.iter_mut().enumerate() {
            let index = (slot + ordinal) % GLOBALS + 1;
            let amount = if ordinal == 0 && slot % 8 == 0 {
                50_000
            } else {
                100_000
            };
            *clone = definition(&mut world, index as u32, index as i32, amount);
        }
        let list = world.alloc(CLONES * 8);
        world.array(
            manager + cdeployment_status::LOCAL_SUBUNITS,
            list,
            CLONES as u32,
        );
        for (ordinal, &clone) in clones.iter().enumerate() {
            world.u64(list + ordinal * 8, clone as u64);
        }
        let begin = (slot - 1) * templates / countries as usize;
        let end = slot * templates / countries as usize;
        let roster = world.alloc((end - begin) * 8);
        world.array(
            object + ccountry::DIVISION_TEMPLATES,
            roster,
            (end - begin) as u32,
        );
        for ordinal in 0..end - begin {
            let at = world.alloc(TEMPLATE + GROUPED * template::SUBUNIT_COUNT_STRIDE);
            world.u64(roster + ordinal * 8, at as u64);
            world.u64(at, (MODULE + template::VTABLE_RVA) as u64);
            let id = (begin + ordinal + 1) as u64;
            world.u64(
                at + template::TYPE,
                (id << 32) | u64::from(template::REFERENCE_TYPE),
            );
            inline_text(&mut world, at + template::NAME, b"Infantry");
            inline_text(&mut world, at + template::NAME_KEY, b"");
            inline_text(&mut world, at + template::MODEL_ENTITY, b"");
            world.u64(
                at + template::PRIORITY_UNIT,
                clones[ordinal % CLONES] as u64,
            );
            world.u64(
                at + template::VISUAL_REPRESENTATIVE,
                clones[(ordinal + 1) % CLONES] as u64,
            );
            world.u32(at + template::UNIT_TYPE_OVERRIDE, 1);
            let grouped = at + TEMPLATE;
            world.array(at + template::SUBUNIT_COUNTS, grouped, GROUPED as u32);
            for (entry, &clone) in clones.iter().take(GROUPED).enumerate() {
                world.u64(
                    grouped + entry * template::SUBUNIT_COUNT_STRIDE,
                    clone as u64,
                );
                world.u32(grouped + entry * template::SUBUNIT_COUNT_STRIDE + 8, 2);
            }
        }
    }
    Bench {
        world,
        database,
        copies: Cell::new(0),
    }
}

struct Cost {
    templates: usize,
    /// The full dictionary walk: names and categories.
    daily_copies: usize,
    daily_seconds: f64,
    /// The hourly walk: counter and 3D-model inputs.
    hourly_copies: usize,
    hourly_seconds: f64,
    clones: usize,
    clone_copies: usize,
    clone_seconds: f64,
}

/// The fastest of `RUNS` steady-state runs, and its copy count: the machine is
/// shared, so a single run can include another process's work.
const RUNS: usize = 5;

fn fastest<T>(bench: &Bench, mut run: impl FnMut() -> T) -> (T, f64, usize) {
    let mut best: Option<(T, f64, usize)> = None;
    for _ in 0..RUNS {
        bench.copies.set(0);
        let started = Instant::now();
        let value = run();
        let seconds = started.elapsed().as_secs_f64();
        if best
            .as_ref()
            .is_none_or(|(_, fastest, _)| seconds < *fastest)
        {
            best = Some((value, seconds, bench.copies.get()));
        }
    }
    best.unwrap()
}

fn measure(countries: u32) -> Cost {
    measure_templates(countries, countries as usize * TEMPLATES)
}

fn measure_templates(countries: u32, templates: usize) -> Cost {
    let bench = build(countries, templates);
    let state = bench.world.game_state();
    let mut memo: Vec<MemoSlot> = vec![(0, 0); 1024];

    let mut rows = vec![Row::default(); templates];
    let mut bytes = vec![0; templates * 32];
    let (daily, daily_seconds, daily_copies) = fastest(&bench, || {
        template::walk(&bench, MODULE, state, &mut rows, &mut bytes, &mut memo).outcome
    });
    assert_eq!(daily, Outcome::Rows { len: templates });

    let mut icons = vec![IconRow::default(); templates];
    let mut counts = vec![SubunitCount::default(); templates * GROUPED];
    // The first walk sizes the definition slots off-thread; the runs are the
    // steady state.
    let mut database = vec![0_usize; 16];
    let mut globals = vec![template::GlobalSlot::default(); 16];
    let mut locals = vec![template::LocalSlot::default(); 16];
    for _ in 0..3 {
        let walk = template::walk_icons(
            &bench,
            MODULE,
            state,
            &mut icons,
            &mut bytes,
            &mut counts,
            &mut [],
            template::DefinitionSlots {
                database: &mut database,
                globals: &mut globals,
                locals: &mut locals,
            },
        );
        database.resize(walk.required_database.max(database.len()), 0);
        globals.resize(
            walk.required_database.max(globals.len()),
            Default::default(),
        );
        locals.resize(walk.required_locals.max(locals.len()), Default::default());
    }
    let (hourly, hourly_seconds, hourly_copies) = fastest(&bench, || {
        template::walk_icons(
            &bench,
            MODULE,
            state,
            &mut icons,
            &mut bytes,
            &mut counts,
            &mut [],
            template::DefinitionSlots {
                database: &mut database,
                globals: &mut globals,
                locals: &mut locals,
            },
        )
        .outcome
    });
    assert_eq!(hourly, Outcome::Rows { len: templates });
    for row in &icons {
        assert_eq!(row.model_failure, None);
        assert!(row.details_fresh);
        assert_eq!(row.details_failure, None);
        assert_eq!(row.equipment_failure, None);
        // The zeroed settings must not overlap the appended grouped entries.
        assert_eq!(row.settings.division_cap, None);
    }

    let clones = countries as usize * CLONES;
    let differing = countries as usize / 8;
    let mut local = vec![LocalSubunitRow::default(); differing];
    let mut requirements = vec![LocalRequirement::default(); differing * 2];
    let (walked, clone_seconds, clone_copies) = fastest(&bench, || {
        cdeployment_status::walk_local_subunits(
            &bench,
            MODULE,
            state,
            &mut local,
            &mut requirements,
        )
        .outcome
    });
    assert_eq!(walked, Outcome::Rows { len: differing });
    Cost {
        templates,
        daily_copies,
        daily_seconds,
        hourly_copies,
        hourly_seconds,
        clones,
        clone_copies,
        clone_seconds,
    }
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn unit_scale_templates() {
    for templates in [50_000, 100_000] {
        let cost = measure_templates(1_500, templates);
        println!(
            "UNIT_SCALE name=Templates countries=1500 templates={templates} daily_best_ms={:.6} hourly_best_ms={:.6} clones_best_ms={:.6} daily_calls={} hourly_calls={} clone_calls={}",
            cost.daily_seconds * 1e3,
            cost.hourly_seconds * 1e3,
            cost.clone_seconds * 1e3,
            cost.daily_copies,
            cost.hourly_copies,
            cost.clone_copies
        );
    }
}

/// Reads per template and per clone stay flat as countries grow. The daily
/// walk gathers three spans a template. The hourly walk gathers each template
/// in one batch gather (identity and one contiguous span), plus the priority
/// unit's name and the grouped entries in one read each; a definition costs one
/// gathered read (identity and index) once per country, or once per walk for a
/// database entry, however many templates name it. This walk also demands both
/// fresh extension spans per template, counted separately from its fixed record.
#[test]
fn model_input_capture_reads_stay_flat_per_template_and_clone() {
    for countries in [16, 128] {
        let cost = measure(countries);
        let per_country = 16;
        let overhead = countries as usize * per_country;
        let extension_copies = cost.templates * 2;
        assert!(
            cost.daily_copies <= cost.templates * 3 + overhead,
            "{countries} countries: {} daily copies for {} templates",
            cost.daily_copies,
            cost.templates
        );
        assert!(
            cost.hourly_copies
                <= cost.templates * 6 + extension_copies + cost.clones * 3 + overhead,
            "{countries} countries: {} hourly copies for {} templates",
            cost.hourly_copies,
            cost.templates
        );
        assert!(
            cost.clone_copies <= cost.clones * 7 + overhead,
            "{countries} countries: {} copies for {} clones",
            cost.clone_copies,
            cost.clones
        );
    }
}

#[test]
#[ignore = "timing bench; run in release with --ignored --nocapture"]
fn model_input_capture_cost() {
    for countries in [1_500, 5_000] {
        let cost = measure(countries);
        let per = |copies: usize, of: usize| copies as f64 / of as f64;
        println!(
            "{countries} countries, {} templates: daily {:.2} ms ({:.1} copies each), \
             hourly {:.2} ms ({:.1} copies each); {} clones: {:.2} ms ({:.1} copies each)",
            cost.templates,
            cost.daily_seconds * 1e3,
            per(cost.daily_copies, cost.templates),
            cost.hourly_seconds * 1e3,
            per(cost.hourly_copies, cost.templates),
            cost.clones,
            cost.clone_seconds * 1e3,
            per(cost.clone_copies, cost.clones),
        );
    }
}
