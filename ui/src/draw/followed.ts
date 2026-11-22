import { DatedStore, type Coverage, type Lead } from '../dashboard/follow/datedStore.ts';
import type { Release } from '../dashboard/follow/decode';
import type { FollowEntry } from '../dashboard/follow/store';
import type { FollowItem, FollowedPanel, FollowedAnswer } from '../dashboard/follow/message';
import { liveKey } from '../charts/live.ts';
import { sectionOf, sectionValues, sectionGroups } from '../dashboard/follow/liveCells.ts';

export type Values = Map<string, number | string>;

export interface WirePanel { refused: string | null; bytes?: Uint8Array; data?: unknown }

export interface TimedReading {
  due: number; hour: number; coverage?: Coverage;
  panel: WirePanel | null; answer: FollowedAnswer | null; values: Values | null;
  structure?: string; kept?: true;
}

export type Structure = (panel: FollowedPanel, hour: number) => string | undefined;

export interface FollowUpdate {
  id: number; epoch: number;
  put?: TimedReading[]; cover?: [due: number, through: number][]; drop?: number[];
}

export interface Answered { answer: FollowedAnswer; date: { since: number; through: number } }

interface Held { panel: FollowedPanel | null; answer: FollowedAnswer | null; values: Values | null; hour: number; start?: number; through?: number }

const sameValues = (a: Values | null, b: Values | null) => {
  if (a === b) return true;
  if (!a || !b || a.size !== b.size) return false;
  for (const [key, value] of a) if (b.get(key) !== value) return false;
  return true;
};

export class Followed {
  private readonly branches = new Map<number, string>();
  private readonly epochs = new Map<string, number>();
  private readonly store: DatedStore;
  private readonly entries = new Map<number, FollowEntry>();
  private readonly sent = new Map<number, Map<number, Held>>();
  private readonly wires = new WeakMap<FollowedPanel, WirePanel>();
  private readonly structures = new Map<number, Structure>();
  private readonly shapes = new WeakMap<FollowedPanel, Map<number, string | undefined>>();
  private readonly models = new Map<number, string>();
  private readonly routes = new Map<number, string>();
  private readonly failed = new Set<string>();
  private readonly consumed = new Map<string, { epoch: number; hour: number }>();
  private readonly passed = new Map<number, Map<number, Lead>>();

  constructor(release: (what: Release) => void, changed?: (id: number) => void) {
    this.store = new DatedStore(entry => changed?.(entry.id), release);
  }

  get size() { return this.entries.size; }

  add(id: number, branch: string, structure?: Structure) {
    const entry: FollowEntry = { id, status: 'followed', answer: undefined, hour: 0, through: 0, version: 0 };
    this.branches.set(id, branch);
    if (structure) this.structures.set(id, structure);
    this.entries.set(id, entry);
    this.store.add(entry, branch);
  }
  remove(id: number) {
    this.store.remove(id); this.entries.delete(id); this.sent.delete(id); this.models.delete(id); this.structures.delete(id); this.passed.delete(id);
    const branch = this.branches.get(id);
    this.branches.delete(id);
    if (branch && ![...this.branches.values()].includes(branch)) {
      this.epochs.delete(branch); this.failed.delete(branch);
      for (const [route, owner] of this.routes) if (owner === branch) this.routes.delete(route);
    }
  }
  seek(branch: string, seek: number, epoch: number, retainThrough?: number) {
    for (const [id, owner] of this.branches) if (owner === branch) { this.sent.delete(id); this.models.delete(id); this.passed.delete(id); }
    this.failed.delete(branch);
    this.epochs.set(branch, epoch); this.store.seek(branch, seek, retainThrough);
  }
  bind(branch: string, route: number, seek: number) {
    for (const [held, owner] of this.routes) if (owner === branch) this.routes.delete(held);
    this.routes.set(route, branch);
    this.store.bind(branch, route, seek);
  }
  land(items: readonly FollowItem[]) {
    for (const item of items) {
      if (item.kind !== 'failure') {
        const route = item.kind === 'panels' ? item.batch.route : item.message.route, branch = this.routes.get(route);
        if (branch === undefined ? this.failed.size > 0 : this.failed.has(branch)) continue;
        this.store.accept(item);
        continue;
      }
      for (const branch of new Set(this.branches.values())) this.failed.add(branch);
      this.sent.clear(); this.models.clear(); this.passed.clear();
      this.store.fail();
    }
  }
  consume(branch: string, epoch: number, hour: number) { this.consumed.set(branch, { epoch, hour }); }
  isFailed(id: number) { const branch = this.branches.get(id); return branch !== undefined && this.failed.has(branch); }
  drain(head: number) {
    for (const [id, entry] of this.entries) for (const lead of this.store.ahead(entry, head)) {
      let leads = this.passed.get(id);
      if (!leads) this.passed.set(id, leads = new Map());
      leads.set(lead.hour, lead);
    }
    this.store.drain(head);
  }
  get stepHour() { return this.store.stepHour; }
  receivedThrough(id: number) {
    const entry = this.entries.get(id);
    return entry && this.store.receivedThrough(entry);
  }


  private values(entry: FollowEntry, panel?: FollowedPanel) {
    const section = sectionOf(panel?.data);
    if (!section) return null;
    const values: Values = new Map();
    const live = sectionValues(section, sectionGroups(section, key => this.store.group(entry, key)));
    for (let i = 0; i < section.cells.length; i += 3 + section.cells[i + 2]! * 3) {
      const key = liveKey(section.rows[section.cells[i]!]!, section.kinds[section.cells[i + 1]!]!.id);
      values.set(key, live.get(key) ?? live.missing(key));
    }
    return values;
  }

  private wire(panel: FollowedPanel | undefined): WirePanel | null {
    if (!panel) return null;
    let wire = this.wires.get(panel);
    if (!wire) this.wires.set(panel, wire = panel.bytes ? { refused: panel.refused, bytes: panel.bytes.slice() } : { refused: panel.refused, data: panel.data });
    return wire;
  }

  private timeline(entry: FollowEntry, aheadThrough: number): Map<number, Held> {
    const coverage = this.store.coverage(entry) ?? undefined, due = this.store.due(entry);
    const out = new Map<number, Held>();
    if (entry.panel || entry.answer) out.set(due, { panel: entry.panel ?? null, answer: entry.answer ?? null, values: this.values(entry, entry.panel),
      hour: entry.hour, start: coverage?.start, through: coverage?.through });
    for (const lead of this.store.ahead(entry, aheadThrough)) if (lead.hour > due)
      out.set(lead.hour, { panel: lead.panel, answer: null, values: this.values(entry, lead.panel), hour: lead.hour, start: lead.hour, through: lead.hour });
    return out;
  }

  private structure(id: number, panel: FollowedPanel | null, hour: number): string | undefined {
    const shape = this.structures.get(id);
    if (!shape || !panel) return undefined;
    let byHour = this.shapes.get(panel);
    if (!byHour) this.shapes.set(panel, byHour = new Map());
    if (!byHour.has(hour)) {
      let made: string | undefined;
      try { made = panel.refused !== null ? undefined : shape(panel, hour); } catch { made = undefined; }
      byHour.set(hour, made);
    }
    return byHour.get(hour);
  }

  private reading(id: number, due: number, held: Held): TimedReading {
    const structure = this.structure(id, held.panel, held.hour);
    const kept = structure !== undefined && this.models.get(id) === structure;
    if (structure !== undefined && !kept) this.models.set(id, structure);
    return { due, hour: held.hour, ...(held.start !== undefined && { coverage: { start: held.start, through: held.through! } }),
      panel: kept ? null : this.wire(held.panel ?? undefined), answer: held.answer, values: held.values,
      ...(structure !== undefined && !kept && { structure }), ...(kept && { kept: true as const }) };
  }

  *updates(aheadThrough: number): Iterable<FollowUpdate> {
    for (const [id, entry] of this.entries) {
      if (this.isFailed(id)) continue;
      const branch = this.branches.get(id)!, epoch = this.epochs.get(branch) ?? 0;
      const timeline = this.timeline(entry, aheadThrough);
      let mirror = this.sent.get(id);
      if (!mirror) this.sent.set(id, mirror = new Map());
      const keep = this.store.due(entry), told = this.consumed.get(branch), consumed = told?.epoch === epoch ? told.hour : -Infinity;
      let floor = -Infinity;
      for (const [due, held] of mirror) if (due > floor && Math.max(due, held.start ?? -Infinity) <= consumed) floor = due;
      for (const due of mirror.keys()) if (due < floor) mirror.delete(due);
      const put: TimedReading[] = [], cover: [number, number][] = [], drop: number[] = [];
      for (const due of [...mirror.keys()]) if (due >= keep && !timeline.has(due)) { drop.push(due); mirror.delete(due); }
      for (const [hour, lead] of this.passed.get(id) ?? []) if (hour < keep && hour >= floor && !mirror.has(hour))
        timeline.set(hour, { panel: lead.panel, answer: null, values: this.values(entry, lead.panel), hour, start: hour, through: hour });
      this.passed.delete(id);
      const confirmed = this.store.confirmed(entry), order = [...new Set([...mirror.keys(), ...timeline.keys()])].sort((a, b) => a - b);
      order.forEach((due, at) => {
        const next = order[at + 1];
        if (due >= keep || next === undefined) return;
        const after = timeline.get(next) ?? mirror.get(next)!, held = timeline.get(due) ?? mirror.get(due)!;
        const through = Math.max(held.through ?? held.hour, Math.min(Math.max(next, after.start ?? -Infinity) - 1, confirmed));
        if (timeline.has(due)) { held.through = through; return; }
        if (held.through !== through) { cover.push([due, through]); held.through = through; }
      });
      for (const [due, held] of timeline) {
        const was = mirror.get(due);
        if (was && was.panel === held.panel && was.answer === held.answer && was.hour === held.hour && was.start === held.start && sameValues(was.values, held.values)) {
          if (was.through !== held.through) { cover.push([due, held.through!]); was.through = held.through; }
          continue;
        }
        put.push(this.reading(id, due, held));
        mirror.set(due, held);
      }
      if (!put.length && !cover.length && !drop.length) continue;
      yield { id, epoch, ...(put.length && { put }), ...(cover.length && { cover }), ...(drop.length && { drop }) };
    }
  }

  ahead(id: number): readonly Lead[] {
    const entry = this.entries.get(id);
    return entry ? this.store.ahead(entry) : [];
  }

  current(id: number) {
    const entry = this.entries.get(id);
    if (!entry) return;
    const coverage = this.store.coverage(entry) ?? undefined;
    return { hour: entry.hour, coverage, panel: entry.panel ?? null, answer: entry.answer ?? null, values: this.values(entry, entry.panel) };
  }

  cellHour(id: number) {
    const entry = this.entries.get(id);
    if (!entry) return;
    let hour: number | undefined;
    for (const spec of sectionOf(entry?.panel?.data)?.groups ?? []) {
      const at = spec.follow !== undefined ? this.store.group(entry, spec.follow)?.landedHour : null;
      if (at != null && (hour === undefined || at > hour)) hour = at;
    }
    return hour;
  }
  answered(id: number): Answered | undefined {
    const entry = this.entries.get(id);
    if (!entry?.answer) return;
    const date = this.store.coverage(entry);
    return date ? { answer: entry.answer, date: { since: date.start, through: date.through } } : undefined;
  }
  sectionLive(id: number) {
    const entry = this.entries.get(id);
    const section = entry?.panel && sectionOf(entry.panel.data);
    return entry && section ? sectionValues(section, sectionGroups(section, key => this.store.group(entry, key))) : null;
  }
  covers(id: number, hour: number) {
    const entry = this.entries.get(id), date = entry && this.store.coverage(entry);
    return !!date && date.start <= hour && hour <= date.through;
  }
  read(id: number) {
    const entry = this.entries.get(id);
    if (!entry?.panel) return;
    const date = this.store.coverage(entry);
    if (!date) return;
    const section = sectionOf(entry.panel.data);
    const joined = section?.groups.some(g => g.follow !== undefined) ? section : null;
    return { reply: entry.panel.data, version: entry.version, hour: entry.hour, date: { since: date.start, through: date.through },
      hasFollowedCells: !!joined, live: joined && sectionValues(joined, sectionGroups(joined, key => this.store.group(entry, key))) };
  }
}
