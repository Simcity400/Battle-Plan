import { DatedStore, type Lead } from '../dashboard/follow/datedStore.ts';
import type { Release } from '../dashboard/follow/decode';
import type { FollowEntry } from '../dashboard/follow/store';
import type { FollowItem, FollowedPanel, FollowedAnswer } from '../dashboard/follow/message';
import { rowKey } from '../dashboard/follow/message.ts';
import { panelStructure } from './panels/views.ts';
import { liveKey } from '../charts/live.ts';
import { sectionOf, sectionValues, sectionGroups } from '../dashboard/follow/liveCells.ts';

export interface PageReading {
  id: number; seek: number; hour: number; through: number; coverage?: { start: number; through: number };
  panel?: FollowedPanel | null; answer?: FollowedAnswer | null; values?: Map<string, number | string> | null;
}

export interface FollowUpdate { now: PageReading; ahead: PageReading[] }

export interface Answered { answer: FollowedAnswer; date: { since: number; through: number } }

export class Followed {
  private readonly pending = new Set<number>();
  private readonly branches = new Map<number, string>();
  private readonly seeks = new Map<string, number>();
  private readonly products = new Map<number, string>();
  private readonly sent = new Map<number, readonly unknown[]>();
  private readonly store: DatedStore;
  private readonly entries = new Map<number, FollowEntry>();
  private readonly shapes = new WeakMap<object, string>();
  private readonly leads = new WeakMap<Lead, PageReading>();

  constructor(release: (what: Release) => void, changed?: (id: number) => void) {
    this.store = new DatedStore(entry => { this.pending.add(entry.id); changed?.(entry.id); }, release, entry => this.pending.add(entry.id));
  }

  get size() { return this.entries.size; }

  add(id: number, branch: string, product = '') {
    this.products.set(id, product);
    const entry: FollowEntry = { id, status: 'followed', answer: undefined, hour: 0, through: 0, version: 0 };
    this.branches.set(id, branch);
    this.entries.set(id, entry);
    this.store.add(entry, branch);
  }
  remove(id: number) {
    this.pending.delete(id);
    this.products.delete(id); this.store.remove(id); this.entries.delete(id); this.sent.delete(id);
    const branch = this.branches.get(id);
    this.branches.delete(id);
    if (branch && ![...this.branches.values()].includes(branch)) this.seeks.delete(branch);
  }
  seek(branch: string, seek: number, retainThrough?: number) {
    for (const [id, owner] of this.branches) if (owner === branch) this.sent.delete(id);
    this.seeks.set(branch, seek); this.store.seek(branch, seek, retainThrough);
  }
  bind(branch: string, route: number, seek: number) {
    this.seeks.set(branch, seek); this.store.bind(branch, route, seek);
  }
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
  snapshot(id: number): PageReading | undefined {
    const entry = this.entries.get(id);
    if (!entry) return;
    const { hour, panel, answer } = entry;
    return { id, hour, through: this.store.eligibleThrough(entry), coverage: this.store.coverage(entry) ?? undefined, seek: this.seekOf(id),
      panel: panel ?? null, answer: answer ?? null, values: this.values(id, panel) };
  }

  private seekOf(id: number) { return this.seeks.get(this.branches.get(id)!) ?? 0; }

  private values(id: number, panel?: FollowedPanel) {
    const entry = this.entries.get(id)!;
    const section = sectionOf(panel?.data);
    const values = new Map<string, number | string>();
    if (section) {
      const live = sectionValues(section, sectionGroups(section, key => this.store.group(entry, key)));
      for (let i = 0; i < section.cells.length; i += 3 + section.cells[i + 2]! * 3) {
        const key = liveKey(section.rows[section.cells[i]!]!, section.kinds[section.cells[i + 1]!]!.id);
        values.set(key, live.get(key) ?? live.missing(key));
      }
    }
    return section ? values : null;
  }

  private shape(id: number, panel: FollowedPanel | undefined, answer: FollowedAnswer | undefined, hour: number) {
    const key = panel ?? (answer?.kind === 'notRecorded' ? undefined : answer);
    let shape = key && this.shapes.get(key);
    if (shape !== undefined) return shape;
    if (panel) {
      try { shape = panel.refused ?? panelStructure(this.products.get(id)!, panel.data, hour); }
      catch (error) { shape = String(error); }
    } else if (answer && answer.kind !== 'notRecorded') {
      shape = JSON.stringify(Array.from({ length: answer.rows }, (_, row) => String(rowKey(answer, row))).sort());
    } else return answer?.reason ?? 'pending';
    this.shapes.set(key!, shape);
    return shape;
  }

  private steps(entry: FollowEntry) {
    const now = this.shape(entry.id, entry.panel, entry.answer, entry.hour), shapes: unknown[] = [now], leads: Lead[] = [];
    let last = now;
    for (const lead of this.store.ahead(entry)) {
      if (lead.hour <= entry.hour) continue;
      const shape = this.shape(entry.id, lead.panel, undefined, lead.hour);
      if (shape === last) continue;
      shapes.push(lead.hour, shape); leads.push(lead);
      last = shape;
    }
    return { shapes, leads };
  }

  private update(id: number, leads: readonly Lead[]): FollowUpdate {
    const ahead = leads.map(lead => {
      let reading = this.leads.get(lead);
      if (!reading) this.leads.set(lead, reading = { id, seek: this.seekOf(id), hour: lead.hour, through: lead.hour,
        coverage: { start: lead.hour, through: lead.hour }, panel: lead.panel, values: this.values(id, lead.panel) });
      return reading;
    });
    return { now: this.snapshot(id)!, ahead };
  }

  *structures(playing: boolean): Iterable<FollowUpdate> {
    for (const id of this.pending) {
      const entry = this.entries.get(id);
      if (!entry) continue;
      const { shapes, leads } = this.steps(entry), last = this.sent.get(id);
      this.sent.set(id, shapes);
      if (playing && last?.length === shapes.length && last.every((shape, at) => shape === shapes[at])) continue;
      yield this.update(id, leads);
    }
    this.pending.clear();
  }

  inspect(id: number): FollowUpdate | undefined {
    const entry = this.entries.get(id);
    if (!entry) return;
    const { shapes, leads } = this.steps(entry);
    this.sent.set(id, shapes);
    return this.update(id, leads);
  }

  paused() { for (const id of this.entries.keys()) this.pending.add(id); }

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
