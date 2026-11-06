//! One ship as every port or line holding it shows it: its row's cells but its holder's, the
//! refusals it counts against each owner holding it, and what it reads beside its own rows.
use super::*;
use super::super::layout::ship;

/// The cells of a ship's row after its key and whether it is held.
pub(super) const WIDTH: usize = ship::CELLS - 2;

fn at(cell: usize) -> usize {
    cell - 2
}

pub(super) struct Ship {
    /// Its row's cells, its holder's Null: each owner's row names its own.
    pub(super) cells: Box<[Spelled]>,
    /// The refusals it counts against each owner, once for every port or line holding it.
    pub(super) refusals: Vec<(u32, u64)>,
    /// The task forces it reads: its own, its rejoin target and its children.
    pub(super) tasks: Vec<u64>,
    /// The variants whose definitions it reads.
    pub(super) variants: Vec<u64>,
    /// The class token whose spelling it reads.
    pub(super) class: Option<u32>,
    /// Its holders, ascending: owner and port province or refit line.
    pub(super) holders: Vec<(u32, u64)>,
}

impl Ship {
    /// The holder an owner's row of it names: the least it has there.
    pub(super) fn holder(&self, owner: u32) -> Option<u64> {
        let first = self.holders.partition_point(|&(held, _)| held < owner);
        self.holders
            .get(first)
            .filter(|&&(held, _)| held == owner)
            .map(|&(_, holder)| holder)
    }

    /// Its hull from the ship's row the hour wrote only the strength of.
    pub(super) fn strengthened<F: Tables + ?Sized>(&mut self, frame: &Frame<'_, F>, id: u64) {
        if let Some(row) = frame.get::<tables::Ships>(id) {
            self.cells[at(ship::HULL)] = Spelled::Integer(i128::from(row.strength));
        }
    }
}

/// What one projection of a ship reads into.
struct Out<'r> {
    cells: Vec<Spelled>,
    held: Option<&'r [Spelled]>,
    refusals: Vec<(u32, u64)>,
    tasks: Vec<u64>,
    variants: Vec<u64>,
}

impl Out<'_> {
    fn set(&mut self, cell: usize, value: Spelled) {
        self.cells[at(cell)] = value;
    }

    fn held(&self, cell: usize) -> Option<&Spelled> {
        self.held.map(|cells| &cells[at(cell)])
    }

    fn refuse(&mut self, reasons: &mut Reasons, why: &str) {
        self.refusals.push((reasons.number(why), 1));
    }

    /// A design handle's cell, and its name's or why it has none.
    fn design<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        (cell, name): (usize, usize),
        handle: u64,
    ) {
        self.set(cell, Spelled::Integer(i128::from(handle)));
        if handle == 0 {
            return;
        }
        self.variants.push(handle);
        match cx.shared.design(&mut cx.naming, handle) {
            Ok(label) => self.set(name, Spelled::Token(label)),
            Err(why) => self.refuse(cx.reasons, &why),
        }
    }
}

/// The ship `id` at the hour, `held` its cells as last projected.
pub(super) fn project<F: Tables + ?Sized>(
    cx: &mut Cx<'_, '_, F>,
    id: u64,
    held: Option<&[Spelled]>,
) -> Ship {
    let frame = cx.frame;
    let mut out = Out {
        cells: vec![Spelled::Null; WIDTH],
        held,
        refusals: Vec::new(),
        tasks: Vec::new(),
        variants: Vec::new(),
    };
    let row = frame.get::<tables::Ships>(id);
    let model = frame.get::<tables::ShipModels>(id);
    let (row, model) = (row.as_deref(), model.as_deref());
    let class = row.map(|row| row.class_id as u32);
    let hull = dashboard_forces::products::naval_management::projection::hull(row, model);
    match hull.current_raw {
        Some(hp) => out.set(ship::HULL, Spelled::Integer(i128::from(hp))),
        None => out.refuse(cx.reasons, "Current ship hull not recorded"),
    }
    match hull.maximum_raw {
        Some(hp) => out.set(ship::MAXIMUM, Spelled::Integer(i128::from(hp))),
        None => out.refuse(cx.reasons, "Maximum ship hull not recorded"),
    }
    if let Some(model) = model {
        out.design(cx, (ship::DESIGN, ship::DESIGN_NAME), model.variant);
    }
    if let Some(row) = row {
        let index = model
            .and_then(|model| model.subunit_index.require("ShipModels.subunit_index").ok())
            .copied()
            .flatten();
        let class = cx.shared.class(frame, row.class_id as u32, index);
        out.set(ship::CLASS, Spelled::Token(class));
        out.set(ship::TASK, Spelled::Integer(i128::from(row.task_force)));
        out.tasks.push(row.task_force);
        let counter = frame.get::<tables::TaskForceCounterInputs>(row.task_force);
        match counter
            .as_deref()
            .and_then(|row| row.name.require("TaskForceCounterInputs.name").ok())
            .and_then(|bytes| bytes.as_ref())
        {
            Some(name) => {
                let operand = Operand::Task {
                    handle: row.task_force,
                    bytes: name.clone(),
                };
                let label = label(out.held(ship::TASK_NAME), operand);
                out.set(ship::TASK_NAME, Spelled::Token(label));
            }
            None => out.refuse(cx.reasons, "Task force name not recorded"),
        }
        if let Some(order) = frame.get::<tables::NavalRepairTaskForces>(row.task_force) {
            out.set(ship::REPAIR_MODE, Spelled::Integer(i128::from(order.repair_mode)));
            let policy = cx.shared.policy(false, order.repair_mode);
            out.set(ship::POLICY, Spelled::Token(policy));
            out.set(ship::REPAIR_SPLIT, Spelled::Integer(i128::from(order.repair_split)));
            out.set(ship::REPAIR_TARGET, Spelled::Integer(i128::from(order.repair_target)));
            out.set(ship::REJOIN, Spelled::Integer(i128::from(order.merge_after_repair)));
            out.tasks.push(order.merge_after_repair);
            let tasks = frame.complete::<tables::TaskForces>();
            if let Some(tasks) = tasks {
                let resolves = tasks.contains(u128::from(order.merge_after_repair));
                out.set(ship::REJOIN_RESOLVES, Spelled::Integer(i128::from(resolves)));
            }
            out.design(cx, (ship::REFIT_INTENT, ship::REFIT_INTENT_NAME), order.refit_variant);
            out.design(
                cx,
                (ship::AFTER_REPAIR_INTENT, ship::AFTER_REPAIR_INTENT_NAME),
                order.refit_after_repair,
            );
            if let Some(children) = frame.complete::<tables::NavalRepairChildren>() {
                let first = u128::from(row.task_force) << 32;
                let children = frame.each(children.range(first..=first | u128::from(u32::MAX)));
                let mut list = Vec::new();
                for child in &children {
                    out.tasks.push(child.child);
                    list.push(Spelled::Integer(i128::from(child.ordinal)));
                    list.push(Spelled::Integer(i128::from(child.child)));
                    list.push(match tasks {
                        Some(tasks) => {
                            Spelled::Integer(i128::from(tasks.contains(u128::from(child.child))))
                        }
                        None => Spelled::Null,
                    });
                }
                if !list.is_empty() {
                    out.set(ship::CHILDREN, Spelled::List(list));
                }
            }
        }
    }
    name(cx, &mut out, id, row, model);
    out.tasks.sort_unstable();
    out.tasks.dedup();
    out.variants.sort_unstable();
    out.variants.dedup();
    Ship {
        cells: out.cells.into_boxed_slice(),
        refusals: merged(out.refusals),
        tasks: out.tasks,
        variants: out.variants,
        class,
        holders: Vec::new(),
    }
}

/// The name the game holds for the ship, else one rebuilt from its naming operands.
fn name<F: Tables + ?Sized>(
    cx: &mut Cx<'_, '_, F>,
    out: &mut Out<'_>,
    id: u64,
    row: Option<&rows::ShipRow>,
    model: Option<&rows::ShipModelRow>,
) {
    if let Some(row) = row {
        if let Ok(Some(bytes)) = row.name.require("Ships.name") {
            let operand = Operand::ShipCache {
                handle: id,
                bytes: bytes.clone(),
            };
            let label = label(out.held(ship::NAME), operand);
            out.set(ship::NAME, Spelled::Token(label));
            return;
        }
        if let Ok(Some(naming)) = row.naming.require("Ships.naming") {
            if let Some(variant) = ship_names::variant(naming, model) {
                out.variants.push(variant);
            }
            let operand = Operand::ShipRebuilt {
                handle: id,
                naming: naming.clone(),
                group: ship_names::group(cx.frame, naming, model),
            };
            let label = label(out.held(ship::NAME_REBUILT), operand);
            out.set(ship::NAME_REBUILT, Spelled::Token(label));
            return;
        }
    }
    out.refuse(
        cx.reasons,
        "Ship name unavailable: Ships cache and naming operands not recorded",
    );
}

/// `refusals` with each reason once, its counts summed, by reason.
pub(super) fn merged(mut refusals: Vec<(u32, u64)>) -> Vec<(u32, u64)> {
    refusals.sort_unstable_by_key(|&(reason, _)| reason);
    refusals.dedup_by(|(reason, count), (kept, total)| {
        let same = reason == kept;
        if same {
            *total += *count;
        }
        same
    });
    refusals
}
