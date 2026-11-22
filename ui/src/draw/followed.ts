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
}

export interface FollowUpdate {
  id: number; epoch: number; keep: number;
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

  constructor(release: (what: Release) => void, changed?: (id: number) => void) {
    this.store = new DatedStore(entry => changed?.(entry.id), release);
  }

  get size() { return this.entries.size; }

  add(id: number, branch: string) {
    const entry: FollowEntry = { id, status: 'followed', answer: undefined, hour: 0, through: 0, version: 0 };
    this.branches.set(id, branch);
    this.entries.set(id, entry);
    this.store.add(entry, branch);
  }
  remove(id: number) {
    this.store.remove(id); this.entries.delete(id); this.sent.delete(id);
    const branch = this.branches.get(id);
    this.branches.delete(id);
    if (branch && ![...this.branches.values()].includes(branch)) this.epochs.delete(branch);
  }
  seek(branch: string, seek: number, epoch: number, retainThrough?: number) {
    for (const [id, owner] of this.branches) if (owner === branch) this.sent.delete(id);
    this.epochs.set(branch, epoch); this.store.seek(branch, seek, retainThrough);
  }
  bind(branch: string, route: number, seek: number) { this.store.bind(branch, route, seek); }
  land(items: readonly FollowItem[]) {
    for (const item of items) {
      if (item.kind !== 'failure') { this.store.accept(item); continue; }
      this.sent.clear();
      this.store.fail();
    }
  }
  drain(head: number) { this.store.drain(head); }
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
    const out = new Map<number, Held>([[due, { panel: entry.panel ?? null, answer: entry.answer ?? null, values: this.values(entry, entry.panel),
      hour: entry.hour, start: coverage?.start, through: coverage?.through }]]);
    for (const lead of this.store.ahead(entry, aheadThrough)) if (lead.hour > due)
      out.set(lead.hour, { panel: lead.panel, answer: null, values: this.values(entry, lead.panel), hour: lead.hour, start: lead.hour, through: lead.hour });
    return out;
  }

  private reading(due: number, held: Held): TimedReading {
    return { due, hour: held.hour, ...(held.start !== undefined && { coverage: { start: held.start, through: held.through! } }),
      panel: this.wire(held.panel ?? undefined), answer: held.answer, values: held.values };
  }

  *updates(aheadThrough: number): Iterable<FollowUpdate> {
    for (const [id, entry] of this.entries) {
      const timeline = this.timeline(entry, aheadThrough);
      let mirror = this.sent.get(id);
      if (!mirror) this.sent.set(id, mirror = new Map());
      const keep = this.store.due(entry), put: TimedReading[] = [], cover: [number, number][] = [], drop: number[] = [];
      let kept = false;
      for (const due of mirror.keys()) if (due < keep) { mirror.delete(due); kept = true; }
      for (const [due, held] of timeline) {
        const was = mirror.get(due);
        if (was && was.panel === held.panel && was.answer === held.answer && was.hour === held.hour && was.start === held.start && sameValues(was.values, held.values)) {
          if (was.through !== held.through) { cover.push([due, held.through!]); was.through = held.through; }
          continue;
        }
        put.push(this.reading(due, held));
        mirror.set(due, held);
      }
      for (const due of mirror.keys()) if (!timeline.has(due)) { drop.push(due); mirror.delete(due); }
      if (!kept && !put.length && !cover.length && !drop.length) continue;
      yield { id, epoch: this.epochs.get(this.branches.get(id)!) ?? 0, keep, ...(put.length && { put }), ...(cover.length && { cover }), ...(drop.length && { drop }) };
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
