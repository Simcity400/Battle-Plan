import type { FollowItem, FollowedPanel } from './message';
import { CellGroup, isCellsId, sectionOf } from './liveCells.ts';
import type { FollowEntry } from './store';
import type { Release } from './decode';

export interface Coverage { start: number; through: number }
interface Picture extends Coverage { panel: FollowedPanel; cells: number[]; seeds: Map<number, CellGroup> }
interface Reading { entry: FollowEntry; branch: string; offset: number; seek?: number; candidate?: Picture; shown?: Coverage; stream?: Route; lane?: { hour: number }; cells: Map<number, CellGroup>;
  ends?: number; waiting?: number }
interface LaneClock { hour: number; limited?: Map<number, number> }
function coverageClock(clock: LaneClock, id: number) {
  return { get hour() { return clock.limited?.get(id) ?? clock.hour; } };
}
interface Route { id: number; seek: number; laneDue: number; lane: LaneClock; waiting: Map<Reading, number>; through: number; receivedThrough: number; stepHour?: number }
interface Packet { route: number; stamp: number; panel: boolean; hour: number; clock: number; apply: () => void }

const covers = (group: CellGroup | undefined, at: number) =>
  group?.start != null && group.start <= at && group.through != null && group.through >= at;

export class DatedStore {
  private readings = new Map<number, Reading>();
  private routes = new Map<string, Route>();
  private groups = new Map<number, CellGroup>();
  private dependents = new Map<number, Set<Reading>>();
  private byId = new Map<number, Route>();
  private members = new Map<string, Set<Reading>>();
  private unbound = new Set<string>();
  private early = new Map<number, Packet[]>();
  private packets: Packet[] = [];
  private first = 0;
  private head = -Infinity;

  private enqueue(packet: Packet) {
    const route = this.byId.get(packet.route);
    if (!route) {
      if (!this.unbound.size) return;
      let waiting = this.early.get(packet.route);
      if (!waiting) this.early.set(packet.route, waiting = []);
      waiting.push(packet);
      return;
    }
    if (route.seek !== packet.stamp) return;
    if (!packet.panel) route.laneDue = packet.hour = Math.max(packet.hour, route.laneDue);
    if (this.first && this.first * 2 >= this.packets.length) {
      this.packets = this.packets.slice(this.first);
      this.first = 0;
    }
    if (!this.packets.length || this.packets.at(-1)!.hour <= packet.hour) {
      this.packets.push(packet);
      return;
    }
    let low = this.first, high = this.packets.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (this.packets[mid]!.hour <= packet.hour) low = mid + 1;
      else high = mid;
    }
    this.packets.splice(low, 0, packet);
  }

  private applyThrough(hour: number, route: number) {
    const due = this.packets.slice(this.first).filter(p => p.route === route && p.hour + p.clock <= hour);
    if (!due.length) return;
    this.discard(p => due.includes(p));
    for (const p of due) p.apply();
  }

  private discard(matches: (packet: Packet) => boolean) {
    this.packets = this.packets.slice(this.first).filter(p => !matches(p));
    this.first = 0;
    for (const [id, packets] of this.early) this.early.set(id, packets.filter(p => !matches(p)));
  }
  private readonly changed: (entry: FollowEntry) => void;

  private readonly release: (what: Release) => void;

  constructor(changed: (entry: FollowEntry) => void, release: (what: Release) => void) {
    this.changed = changed; this.release = release;
  }

  add(entry: FollowEntry, branch: string) {
    const reading = { entry, branch, offset: 0, cells: new Map() };
    this.readings.set(entry.id, reading);
    let members = this.members.get(branch);
    if (!members) this.members.set(branch, members = new Set());
    members.add(reading);
    if (!this.routes.has(branch)) this.unbound.add(branch);
  }

  remove(id: number) {
    const r = this.readings.get(id);
    if (!r) return;
    this.routes.get(r.branch)?.waiting.delete(r);
    for (const group of new Set([...r.cells.keys(), ...r.candidate?.cells ?? []])) this.dropGroup(group, r);
    this.release(id);
    this.readings.delete(id);
    const members = this.members.get(r.branch)!;
    members.delete(r);
    if (!members.size) {
      this.members.delete(r.branch);
      this.unbound.delete(r.branch);
      if (!this.unbound.size) this.early.clear();
      const route = this.routes.get(r.branch);
      if (route) this.release({ route: route.id });
      this.clearRoute(r.branch);
      if (route) this.byId.delete(route.id);
      this.routes.delete(r.branch);
    }
    if (!this.readings.size) this.close();
  }

  private dropGroup(id: number, r: Reading) {
    const owners = this.dependents.get(id);
    owners?.delete(r);
    if (owners?.size) return;
    this.dependents.delete(id); this.groups.delete(id); this.release(id);
  }

  private clearRoute(branch: string, retainThrough?: number) {
    const route = this.routes.get(branch);
    if (retainThrough !== undefined && route) this.applyThrough(retainThrough, route.id);
    for (const r of this.members.get(branch) ?? []) {
      for (const id of r.candidate?.cells ?? []) this.dependents.delete(id);
      const interval = retainThrough === undefined ? undefined : this.coverage(r.entry);
      r.candidate = undefined; r.ends = undefined; r.waiting = undefined; r.seek = undefined; r.stream = undefined; r.lane = undefined;
      if (retainThrough !== undefined && interval && interval.start <= retainThrough && retainThrough <= interval.through) {
        r.shown = interval; r.entry.version++; this.changed(r.entry); continue;
      }
      r.shown = undefined; r.cells.clear();
      r.entry.panel = undefined; r.entry.answer = undefined; r.entry.version++;
      this.changed(r.entry);
    }
    if (route) {
      this.discard(p => p.route === route.id);
      route.laneDue = -Infinity;
      route.lane = { hour: -Infinity };
      route.waiting.clear();
      route.through = -Infinity;
      route.receivedThrough = -Infinity;
      route.stepHour = undefined;
      for (const [id, group] of this.groups) if (group.route === route.id) this.groups.delete(id);
    }
  }

  seek(branch: string, seek: number, retainThrough?: number) {
    this.clearRoute(branch, retainThrough);
    const route = this.routes.get(branch);
    if (route) route.seek = seek;
  }

  bind(branch: string, id: number, seek: number) {
    if (!this.members.has(branch)) return;
    const old = this.routes.get(branch);
    if (old?.id === id && old.seek === seek) return;
    if (old) { this.clearRoute(branch); this.byId.delete(old.id); }
    const route = { id, seek, laneDue: -Infinity, lane: { hour: -Infinity }, waiting: new Map<Reading, number>(), through: -Infinity, receivedThrough: -Infinity };
    this.routes.set(branch, route); this.byId.set(id, route);
    this.unbound.delete(branch);
    const early = this.early.get(id) ?? [];
    this.early.delete(id);
    for (const p of early) this.enqueue(p);
    if (!this.unbound.size) this.early.clear();
  }

  private reading(id: number, route: number) {
    const r = this.readings.get(id);
    return r && this.routes.get(r.branch)?.id === route ? r : undefined;
  }

  accept(item: Exclude<FollowItem, {kind: 'failure'}>) {
    const panel = item.kind === 'panels';
    const route = panel ? item.batch.route : item.message.route;
    if (!this.byId.has(route) && !this.unbound.size) return;
    if (panel) {
      const { seek, panels } = item.batch;
      for (const p of panels) {
        if (!this.readings.has(p.id)) continue;
        this.enqueue({ route, stamp: seek, panel, hour: p.hour - item.batch.clock, clock: item.batch.clock, apply: () => {
          const r = this.reading(p.id, route);
          if (!r) return;
          r.offset = item.batch.clock;
          if (p.stepped) r.stream = this.byId.get(route);
          if (p.waiting) { r.waiting ??= p.hour; this.present(r); return; }
          if (!p.panel) {
            if (r.candidate) r.candidate.through = Math.max(r.candidate.through, p.hour);
            r.waiting = undefined;
          } else {
            if (r.candidate && p.hour < r.candidate.start) return;
            const cells: number[] = [];
            const seeds = new Map<number, CellGroup>();
            for (const spec of sectionOf(p.panel.data)?.groups ?? []) {
              if (spec.follow === undefined) continue;
              cells.push(spec.follow);
              let owners = this.dependents.get(spec.follow);
              if (!owners) this.dependents.set(spec.follow, owners = new Set());
              owners.add(r);
              if (spec.seedHour === undefined || spec.seedHour > p.hour) continue;
              const seeded = new CellGroup();
              seeded.seed(spec);
              seeds.set(spec.follow, seeded);
            }
            for (const id of r.candidate?.cells ?? []) if (!cells.includes(id)) this.dropGroup(id, r);
            if (r.shown) r.ends ??= Math.min(p.hour, r.waiting ?? Infinity) - 1;
            r.waiting = undefined;
            r.candidate = { start: p.hour, through: p.hour, panel: p.panel, cells, seeds };
          }
          this.present(r);
        } });
      }
      const completed = item.batch.completed;
      const bound = this.byId.get(route);
      if (completed !== undefined && bound?.seek === seek) bound.receivedThrough = completed;
      if (completed !== undefined) this.enqueue({ route, stamp: seek, panel, hour: completed - item.batch.clock, clock: item.batch.clock, apply: () => {
        const stream = this.byId.get(route);
        if (stream) {
          stream.through = completed;
          stream.stepHour = completed - item.batch.clock;
          for (const r of this.readings.values()) if (r.stream === stream && r.candidate?.cells.length && r.candidate.through < Math.min(completed, (r.waiting ?? Infinity) - 1)) {
            r.candidate.through = Math.min(completed, (r.waiting ?? Infinity) - 1);
            this.present(r);
          }
        }
      } });
    } else {
      const message = item.message;
      const { hour, seek: stamp, reset } = message;
      const due = message.answers.reduce((latest, a) => Math.max(latest, a.hour), hour);
      const axis = due - message.clock;
      if (reset && (!this.byId.has(route) || this.byId.get(route)!.seek === stamp)) {
        this.discard(p => p.route === route && p.stamp === stamp && p.hour >= axis);
        const bound = this.byId.get(route);
        if (bound) bound.laneDue = axis;
      }
      this.enqueue({ route, stamp, panel, hour: axis, clock: message.clock, apply: () => {
        const stream = this.byId.get(route)!;
        if (reset) {
          stream.through = -Infinity;
          stream.stepHour = undefined;
          stream.lane = { hour: -Infinity };
          stream.waiting.clear();
          for (const [id, g] of this.groups) if (g.route === route) this.groups.delete(id);
          for (const r of this.readings.values()) if (this.reading(r.entry.id, route)) {
            for (const id of r.candidate?.cells ?? []) this.dependents.delete(id);
            r.candidate = undefined; r.ends = undefined; r.waiting = r.shown && r.shown.through + 1; r.seek = undefined; r.stream = undefined; r.lane = undefined;
          }
        }
        stream.lane.limited = message.limited;
        const touched = new Set<Reading>();
        for (const { id, answer, hour: start } of message.answers) {
          const lane = coverageClock(stream.lane, id);
          const through = Math.max(start, message.limited?.get(id) ?? hour);
          if (isCellsId(id)) {
            const existing = this.groups.get(id);
            if (existing?.start != null && existing.start > start) {
              existing.seek = stamp;
              continue;
            }
            const g = new CellGroup();
            existing?.follow();
            g.land(answer); g.start = start; g.through = through; g.route = route; g.seek = stamp;
            g.follow(lane);
            this.groups.set(id, g);
            for (const r of this.dependents.get(id) ?? []) touched.add(r);
          } else {
            const r = this.reading(id, route);
            if (r) {
              r.offset = message.clock;
              r.entry.answer = answer; r.entry.hour = start; r.entry.through = through;
              r.seek = stamp; r.lane = lane; r.shown = { start, through: r.entry.through };
              r.entry.version++; this.changed(r.entry);
            }
          }
        }
        stream.lane.hour = Math.max(stream.lane.hour, hour);
        for (const [r, when] of stream.waiting) if (when <= hour) touched.add(r);
        for (const r of touched) this.present(r);
      } });
    }
  }

  private present(r: Reading) {
    const candidate = r.candidate;
    if (!candidate) return;
    const route = this.routes.get(r.branch)!;
    route.waiting.delete(r);
    let start = candidate.start, through = candidate.through;
    const cells = new Map<number, CellGroup>();
    for (const id of candidate.cells) {
      const stream = this.groups.get(id);
      if (stream?.start == null || stream.through === null) { through = -Infinity; break; }
      start = Math.max(start, stream.start); through = Math.min(through, stream.through);
      cells.set(id, stream);
    }
    if (start > through) {
      if (start <= candidate.through && cells.size === candidate.cells.length) route.waiting.set(r, start);
      start = through = candidate.start;
      for (const id of candidate.cells) {
        const stream = this.groups.get(id), held = r.cells.get(id);
        const group = covers(stream, start) ? stream : covers(held, start) ? held : candidate.seeds.get(id);
        if (group?.through == null || group.through < start) return;
        cells.set(id, group);
      }
    }
    if (r.entry.panel === candidate.panel && r.shown && r.shown.through > through) return;
    if (r.entry.panel === candidate.panel && r.entry.through === candidate.through && r.shown?.through === through &&
      r.shown.start === start && candidate.cells.every(id => r.cells.get(id) === cells.get(id))) return;
    r.entry.panel = candidate.panel; r.entry.hour = candidate.start; r.entry.through = candidate.through; r.ends = undefined;
    r.shown = { start, through }; r.cells = cells;
    r.entry.version++; this.changed(r.entry);
  }

  drain(head: number) {
    this.head = Math.floor(head);
    while (this.first < this.packets.length && this.packets[this.first]!.hour <= head) {
      this.packets[this.first++]!.apply();
    }
    if (this.first === this.packets.length) { this.packets.length = 0; this.first = 0; }
  }

  coverage(entry: FollowEntry) {
    const r = this.readings.get(entry.id);
    if (!r?.shown) return null;
    const covered = this.covered(r as Reading & { shown: Coverage });
    if (!covered || r.waiting === undefined) return covered;
    return { start: covered.start, through: Math.max(r.shown.through, Math.min(covered.through, r.waiting - 1)) };
  }

  private covered(r: Reading & { shown: Coverage }) {
    if (r.lane) return { start: r.shown.start, through: Math.max(r.shown.through, r.lane.hour) };
    if (r.candidate && r.candidate.panel !== r.entry.panel) {
      let through = Math.min(r.ends ?? r.candidate.start - 1, r.stream?.through ?? r.shown.through);
      for (const g of r.cells.values()) through = Math.min(through, g.through ?? -Infinity);
      return { start: r.shown.start, through: Math.max(r.shown.through, through) };
    }
    if (r.candidate?.cells.length && r.candidate.panel === r.entry.panel) {
      let through = r.candidate.through;
      for (const g of r.cells.values()) through = Math.min(through, g.through ?? -Infinity);
      return { start: r.shown.start, through: Math.max(r.shown.through, through) };
    }
    return r.stream && r.stream.through > r.shown.through ? { start: r.shown.start, through: r.stream.through } : r.shown;
  }
  receivedThrough(entry: FollowEntry) { return this.readings.get(entry.id)?.stream?.receivedThrough; }
  get stepHour() { return this.byId.values().next().value?.stepHour; }
  eligibleThrough(entry: FollowEntry) {
    return Math.min(this.head + (this.readings.get(entry.id)?.offset ?? 0), this.coverage(entry)?.through ?? entry.hour);
  }
  group(entry: FollowEntry, id: number) { return this.readings.get(entry.id)?.cells.get(id); }
  close() { this.packets = []; this.first = 0; this.early.clear(); this.groups.clear(); this.dependents.clear(); }
}
