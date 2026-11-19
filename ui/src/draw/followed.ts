import { DatedStore } from '../dashboard/follow/datedStore.ts';
import type { Release } from '../dashboard/follow/decode';
import type { FollowEntry } from '../dashboard/follow/store';
import type { FollowItem, FollowedPanel, FollowedAnswer } from '../dashboard/follow/message';
import { rowKey } from '../dashboard/follow/message.ts';
import { panelStructure } from './panels/views.ts';
import { liveKey } from '../charts/live.ts';
import { sectionOf, sectionValues, sectionGroups } from '../dashboard/follow/liveCells.ts';
import type { FromWorker } from './kit';

export interface PageReading {
  id: number; seek: number; hour: number; through: number; coverage?: { start: number; through: number };
  panel?: FollowedPanel | null; answer?: FollowedAnswer | null; values?: Map<string, number | string> | null;
}

export class Followed {
  private readonly dirty = new Set<FollowEntry>();
  private readonly branches = new Map<number, string>();
  private readonly seeks = new Map<string, number>();
  private readonly products = new Map<number, string>();
  private readonly sent = new Map<number, { panel?: FollowedPanel; answer?: FollowedAnswer; shape: string }>();
  private readonly store: DatedStore;
  private readonly entries = new Map<number, FollowEntry>();
  private readonly routes = new Map<string, number>();
  private readonly upcoming = new Map<number, string>();
  private staged = new WeakMap<FollowedPanel, string>();

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
    this.upcoming.delete(id);
    if (branch && ![...this.branches.values()].includes(branch)) { this.seeks.delete(branch); this.routes.delete(branch); }
  }
  private resetLead(branch: string) {
    for (const [id, key] of this.branches) if (key === branch) this.upcoming.delete(id);
    this.staged = new WeakMap();
  }
  seek(branch: string, seek: number, retainThrough?: number) {
    for (const [id, key] of this.branches) if (key === branch) this.sent.delete(id);
    this.resetLead(branch); this.seeks.set(branch, seek); this.store.seek(branch, seek, retainThrough);
  }
  bind(branch: string, route: number, seek: number) {
    if (this.routes.get(branch) !== route) this.resetLead(branch);
    this.routes.set(branch, route);
    this.seeks.set(branch, seek); this.store.bind(branch, route, seek);
  }
  land(items: readonly FollowItem[], lead?: (message: FromWorker) => void) {
    for (const item of items) {
      if (lead && item.kind === 'followed' && item.message.reset) {
        const { route, seek, hour } = item.message;
        for (const [name, id] of this.routes) if (id === route && this.seeks.get(name) === seek) this.resetLead(name);
        lead({ type: 'resetStructures', route, seek, from: hour });
      }
      if (lead && item.kind === 'panels') {
        const updates: PageReading[] = [];
        const { route, seek } = item.batch;
        for (const { panel, stepped, id, hour, through } of item.batch.panels) {
          const branch = this.branches.get(id);
          if (!stepped || !panel || branch === undefined || this.routes.get(branch) !== route || this.seeks.get(branch) !== seek) continue;
          const section = sectionOf(panel.data);
          if (section?.groups.some(g => g.follow !== undefined)) continue;
          const shape = this.panelShape(id, panel, hour);
          this.staged.set(panel, shape);
          if ((this.upcoming.get(id) ?? this.sent.get(id)?.shape) !== shape)
            updates.push({ id, seek, hour, through: hour, coverage: { start: hour, through: through ?? hour }, panel, values: this.values(id, panel) });
          this.upcoming.set(id, shape);
        }
        if (updates.length) lead({ type: 'structureLead', updates });
      }
      if (item.kind !== 'failure') this.store.accept(item);
      else for (const branch of new Set(this.branches.values())) this.store.seek(branch, this.seeks.get(branch) ?? 0);
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
    return { id, hour, through: this.store.eligibleThrough(entry), coverage: this.store.coverage(entry) ?? undefined, seek: this.seeks.get(this.branches.get(id)!) ?? 0,
      panel: panel ?? null, answer: answer ?? null, values: this.values(id, panel) };
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

  *structures(playing: boolean): Iterable<PageReading> {
    for (const entry of this.dirty) {
      const { id, panel, answer, hour } = entry;
      const last = this.sent.get(id);
      if (playing && last && last.panel === panel && last.answer === answer) continue;
      let shape: string;
      if (panel) {
        shape = this.staged.get(panel) ?? this.panelShape(id, panel, hour);
      } else if (answer && answer.kind !== 'notRecorded') {
        shape = JSON.stringify(Array.from({ length: answer.rows }, (_, row) => String(rowKey(answer, row))).sort());
      } else shape = answer?.reason ?? 'pending';
      this.sent.set(id, { shape, panel, answer });
      if (!playing || last?.shape !== shape && !(panel && this.staged.has(panel))) yield this.snapshot(id)!;
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
