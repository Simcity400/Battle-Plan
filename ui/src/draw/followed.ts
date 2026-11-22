import { DatedStore } from '../dashboard/follow/datedStore.ts';
import type { Release } from '../dashboard/follow/decode';
import type { FollowEntry } from '../dashboard/follow/store';
import type { FollowItem, FollowedPanel, FollowedAnswer } from '../dashboard/follow/message';
import { rowKey } from '../dashboard/follow/message.ts';
import { panelStructure } from './panels/views.ts';
import { liveKey } from '../charts/live.ts';
import { sectionOf, sectionValues, sectionGroups } from '../dashboard/follow/liveCells.ts';
import type { Version } from '../dashboard/follow/version';

export interface PageReading {
  id: number; seek: number; hour: number; through: number; coverage?: { start: number; through: number };
  panel?: FollowedPanel | null; answer?: FollowedAnswer | null; values?: Map<string, number | string> | null;
}

export type OrderedReading = PageReading & { version: Version };

export interface Answered { answer: FollowedAnswer; date: { since: number; through: number } }

export interface Notices {
  reset(ids: ReadonlySet<number>, route: number, seek: number, from: number): void;
  lead?(updates: OrderedReading[]): void;
}

export class Followed {
  private readonly dirty = new Set<FollowEntry>();
  private readonly branches = new Map<number, string>();
  private readonly seeks = new Map<string, number>();
  private readonly products = new Map<number, string>();
  private readonly sent = new Map<number, { generation: number; panel?: FollowedPanel; answer?: FollowedAnswer; shape: string }>();
  private readonly store: DatedStore;
  private readonly entries = new Map<number, FollowEntry>();
  private readonly routes = new Map<string, number>();
  private readonly staged = new WeakMap<FollowedPanel, string>();

  constructor(release: (what: Release) => void, changed?: (id: number) => void) {
    this.store = new DatedStore(entry => { this.dirty.add(entry); changed?.(entry.id); }, release);
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
    const entry = this.entries.get(id);
    if (entry) this.dirty.delete(entry);
    this.products.delete(id); this.store.remove(id); this.entries.delete(id); this.sent.delete(id);
    const branch = this.branches.get(id);
    this.branches.delete(id);
    if (branch && ![...this.branches.values()].includes(branch)) { this.seeks.delete(branch); this.routes.delete(branch); }
  }
  seek(branch: string, seek: number, retainThrough?: number) {
    this.seeks.set(branch, seek); this.store.seek(branch, seek, retainThrough);
  }
  bind(branch: string, route: number, seek: number) {
    this.routes.set(branch, route);
    this.seeks.set(branch, seek); this.store.bind(branch, route, seek);
  }
  land(items: readonly FollowItem[], notices?: Notices) {
    for (const item of items) {
      if (item.kind === 'failure') {
        for (const branch of new Set(this.branches.values())) this.store.seek(branch, this.seeks.get(branch) ?? 0);
        continue;
      }
      if (notices && item.kind === 'followed' && item.message.reset) {
        const { route, seek, hour } = item.message, ids = new Set<number>();
        for (const [id, branch] of this.branches) if (this.routes.get(branch) === route && this.seeks.get(branch) === seek) ids.add(id);
        notices.reset(ids, route, seek, hour);
      }
      const updates: OrderedReading[] = [], lead = notices?.lead;
      this.store.accept(item, lead && item.kind === 'panels' ? ({ id, hour, panel, stepped }, version) => {
        if (!stepped || !panel || sectionOf(panel.data)?.groups.some(g => g.follow !== undefined)) return;
        const shape = this.panelShape(id, panel, hour), last = this.sent.get(id);
        this.staged.set(panel, shape);
        if (last?.generation !== version.generation || last.shape !== shape)
          updates.push({ id, seek: item.batch.seek, hour, through: hour, coverage: { start: hour, through: hour }, panel, values: this.values(id, panel), version });
        this.sent.set(id, { generation: version.generation, shape, panel });
      } : undefined);
      if (updates.length) lead!(updates);
    }
  }
  drain(head: number) { this.store.drain(head); }
  get stepHour() { return this.store.stepHour; }
  receivedThrough(id: number) {
    const entry = this.entries.get(id);
    return entry && this.store.receivedThrough(entry);
  }
  snapshot(id: number): OrderedReading | undefined {
    const entry = this.entries.get(id);
    if (!entry) return;
    const { hour, panel, answer } = entry;
    return { id, hour, through: this.store.eligibleThrough(entry), coverage: this.store.coverage(entry) ?? undefined, seek: this.seeks.get(this.branches.get(id)!) ?? 0,
      panel: panel ?? null, answer: answer ?? null, values: this.values(id, panel), version: this.store.version(entry) };
  }

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

  private panelShape(id: number, panel: FollowedPanel, hour: number) {
    try { return panel.refused ?? panelStructure(this.products.get(id)!, panel.data, hour); }
    catch (error) { return String(error); }
  }

  *structures(playing: boolean): Iterable<OrderedReading> {
    for (const entry of this.dirty) {
      const { id, panel, answer, hour } = entry;
      const { generation } = this.store.version(entry), last = this.sent.get(id), same = last?.generation === generation;
      if (playing && same && last.panel === panel && last.answer === answer) continue;
      let shape: string;
      if (panel) {
        shape = this.staged.get(panel) ?? this.panelShape(id, panel, hour);
      } else if (answer && answer.kind !== 'notRecorded') {
        shape = JSON.stringify(Array.from({ length: answer.rows }, (_, row) => String(rowKey(answer, row))).sort());
      } else shape = answer?.reason ?? 'pending';
      this.sent.set(id, { generation, shape, panel, answer });
      if (!playing || !same || last.shape !== shape) yield this.snapshot(id)!;
    }
    this.dirty.clear();
  }

  paused() { for (const entry of this.entries.values()) this.dirty.add(entry); }

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
