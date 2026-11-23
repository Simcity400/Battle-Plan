export type Channel = 'panel' | 'followed' | 'pinned';

export interface Fact<V> {
  readonly h: number;
  c: number;
  readonly value: V;
  readonly refused: string | null;
  readonly failed: string | null;
  readonly cells: readonly number[];
  readonly seeds: ReadonlyMap<number, Fact<V>>;
  readonly channel: Channel;
  readonly version: number;
  readonly bytes: number;
}

export interface Picture<V> {
  readonly h: number;
  readonly c: number;
  readonly parts: readonly Fact<V>[];
  readonly value: V;
  readonly refused: string | null;
  readonly failed: string | null;
  readonly carried?: true;
}

export interface CellSeed<V> { id: number; seedHour?: number; values?: V }

export type PanelRecord<V> =
  | { kind: 'data'; id: number; hour: number; value: V; stepped?: boolean; cells?: readonly CellSeed<V>[]; bytes?: number }
  | { kind: 'refused'; id: number; hour: number; reason: string; value: V; stepped?: boolean; bytes?: number }
  | { kind: 'failed'; id: number; hour: number; reason: string; stepped?: boolean }
  | { kind: 'stands'; id: number; hour: number; stepped?: boolean }
  | { kind: 'waiting'; id: number; hour: number };

export interface FollowAnswer<V> { id: number; hour: number; value: V; refused?: string | null; bytes?: number }

export interface FollowMessage<V> { hour: number; reset?: boolean; limited?: ReadonlyMap<number, number>; answers: readonly FollowAnswer<V>[] }

export interface Generation { seek: number; reset: number }

interface Owner<V> { id: number; route: number; channel: Channel; facts: Fact<V>[]; open: boolean }
interface Sub<V> extends Owner<V> { stepped: boolean; carried: Picture<V> | null; uses: Set<number>; nativeCells: Set<number> }
interface Group<V> extends Owner<V> { users: Set<number> }
interface Route { seek: number; reset: number; early: [number, () => void][] }

export type Window = readonly [lo: number, hi: number];

let versions = 0;

export const samePicture = <V>(a: Picture<V> | null | undefined, b: Picture<V> | null | undefined) => a === b
  || (!!a && !!b && a.parts.length === b.parts.length && a.parts.every((part, at) => part === b.parts[at]));


export const followsChannel = <V>(picture: Picture<V>) => picture.parts.some(part => part.channel === 'followed');

export const contentVersion = <V>(picture: Picture<V>) => picture.parts.map(part => part.version).join('.');

export class Facts<V> {
  private readonly subs = new Map<number, Sub<V>>();
  private readonly groups = new Map<number, Group<V>>();
  private readonly routes = new Map<number, Route>();
  private released: number[] = [];
  private freed = 0;
  private removed: Fact<V>[] = [];

  private route(id: number): Route {
    let route = this.routes.get(id);
    if (!route) this.routes.set(id, route = { seek: 0, reset: 0, early: [] });
    return route;
  }

  has(id: number) { return this.subs.has(id); }

  owns(id: number) { return this.subs.has(id) || this.groups.has(id); }

  usesOf(id: number): ReadonlySet<number> | undefined { return this.subs.get(id)?.uses; }

  usersOf(id: number): ReadonlySet<number> | undefined { return this.groups.get(id)?.users; }

  latest(id: number): V | undefined { return (this.subs.get(id) ?? this.groups.get(id))?.facts.at(-1)?.value; }

  activeCells(id: number, target: number): readonly number[] {
    const sub = this.subs.get(id);
    if (!sub) return [];
    for (let at = sub.facts.length - 1; at >= 0; at--) {
      const fact = sub.facts[at]!;
      if (fact.h <= target) return fact.cells;
    }
    return sub.carried && sub.carried.h <= target ? sub.carried.parts[0]?.cells ?? [] : [];
  }

  activePanelWindow(id: number, target: number): Window | null {
    const sub = this.subs.get(id);
    if (!sub) return null;
    for (let at = sub.facts.length - 1; at >= 0; at--) {
      const fact = sub.facts[at]!;
      if (fact.h <= target) {
        const next = sub.facts[at + 1];
        return [fact.h, next ? next.h - 1 : Infinity];
      }
    }
    if (sub.carried && sub.carried.h <= target) {
      const next = sub.facts[0];
      return [sub.carried.h, next ? next.h - 1 : Infinity];
    }
    return null;
  }

  isOpenSteppedPanel(id: number): boolean {
    const sub = this.subs.get(id);
    if (!sub || sub.channel !== 'panel' || !sub.stepped || !sub.open) return false;
    const last = sub.facts.at(-1);
    return !!last && last.failed === null;
  }

  nextFaultStart(id: number, target: number, isFaulted: (cell: number) => boolean): number {
    const sub = this.subs.get(id);
    if (!sub) return Infinity;
    for (const fact of sub.facts) {
      if (fact.h > target && fact.cells.some(isFaulted)) return fact.h;
    }
    return Infinity;
  }

  nextStart(id: number, target: number): number {
    const sub = this.subs.get(id);
    if (!sub) return Infinity;
    let best = Infinity;
    for (const fact of sub.facts) {
      if (fact.h > target && fact.h < best) best = fact.h;
      for (const seed of fact.seeds.values()) if (seed.h > target && seed.h < best) best = seed.h;
    }
    if (sub.carried && sub.carried.h > target && sub.carried.h < best) best = sub.carried.h;
    for (const groupId of sub.uses) {
      for (const fact of this.groups.get(groupId)?.facts ?? []) {
        if (fact.h > target) {
          if (fact.h < best) best = fact.h;
          break;
        }
      }
    }
    return best;
  }

  prunable(id: number): boolean {
    const sub = this.subs.get(id);
    if (!sub) return false;
    if (sub.facts.length > 1 || sub.carried !== null) return true;
    for (const groupId of sub.uses) {
      const group = this.groups.get(groupId);
      if (group && group.facts.length > 1) return true;
    }
    return false;
  }

  follow(id: number, route: number, channel: Channel) {
    if (channel !== 'pinned') this.route(route);
    this.subs.set(id, { id, route, channel, facts: [], open: false, stepped: false, carried: null, uses: new Set(), nativeCells: new Set() });
  }

  unfollow(id: number): number[] {
    const sub = this.subs.get(id);
    if (!sub) return [];
    this.subs.delete(id);
    sub.nativeCells.clear();
    this.removed.push(...sub.facts);
    this.retain(sub, new Set());
    return this.drain();
  }

  drain(): number[] { const out = this.released; this.released = []; return out; }

  hasRemoved() { return this.removed.length > 0; }

  releasedBytes(): number { return this.release(() => false); }

  release(held: (fact: Fact<V>) => boolean): number {
    let bytes = this.freed;
    this.freed = 0;
    if (!this.removed.length) return bytes;
    const carried = new Set<Fact<V>>();
    for (const sub of this.subs.values()) for (const part of sub.carried?.parts ?? []) carried.add(part);
    for (const sub of this.subs.values()) for (const fact of sub.facts) carried.add(fact);
    for (const group of this.groups.values()) for (const fact of group.facts) carried.add(fact);
    const kept: Fact<V>[] = [];
    for (const fact of new Set(this.removed)) {
      if (carried.has(fact) || held(fact)) kept.push(fact);
      else bytes += fact.bytes;
    }
    this.removed = kept;
    return bytes;
  }

  generation(route: number): Generation { const at = this.route(route); return { seek: at.seek, reset: at.reset }; }

  private accept(route: number, seek: number, again: () => void): boolean {
    const at = this.route(route);
    if (seek < at.seek) return false;
    if (seek > at.seek) { at.early.push([seek, again]); return false; }
    return true;
  }

  stamp(route: number, seek: number) {
    const at = this.route(route);
    if (seek <= at.seek) return;
    for (const sub of this.subs.values()) if (sub.route === route && (sub.facts.length || sub.carried)) return;
    at.seek = seek;
    const due = at.early.filter(([stamp]) => stamp <= seek);
    at.early = at.early.filter(([stamp]) => stamp > seek);
    for (const [, again] of due) again();
  }

  seek(route: number, seek: number, target: number) {
    const at = this.route(route);
    if (seek <= at.seek) return;
    for (const sub of this.subs.values()) if (sub.route === route && sub.channel !== 'pinned') {
      const shown = this.select(sub.id, target);
      sub.carried = shown && shown.failed === null && shown.h <= target && target <= shown.c ? Object.freeze({ ...shown, carried: true as const }) : null;
      this.removed.push(...sub.facts);
      sub.facts = []; sub.open = false; sub.stepped = false;
    }
    for (const group of this.groups.values()) if (group.route === route) {
      this.removed.push(...group.facts);
      group.facts = []; group.open = false;
    }
    at.seek = seek;
    const due = at.early.filter(([stamp]) => stamp <= seek);
    at.early = at.early.filter(([stamp]) => stamp > seek);
    for (const [, again] of due) again();
  }

  private add(owner: Owner<V>, fact: Fact<V>) {
    const last = owner.facts.at(-1);
    if (last && last.h === fact.h) { this.removed.push(last); owner.facts[owner.facts.length - 1] = fact; }
    else {
      if (last && last.c >= fact.h) last.c = fact.h - 1;
      owner.facts.push(fact);
    }
    owner.open = true;
  }

  private group(id: number, route: number, sub: Sub<V>): Group<V> {
    let group = this.groups.get(id);
    if (!group) this.groups.set(id, group = { id, route, channel: 'followed', facts: [], open: false, users: new Set() });
    group.users.add(sub.id); sub.uses.add(id);
    return group;
  }

  private fact(h: number, c: number, value: V, refused: string | null, channel: Channel, bytes: number,
    cells: readonly number[] = [], seeds: ReadonlyMap<number, Fact<V>> = new Map(), failed: string | null = null): Fact<V> {
    return { h, c, value, refused, failed, cells, seeds, channel, version: ++versions, bytes };
  }

  exact(id: number, hour: number, value: V, refused: string | null = null, bytes = 0) {
    const sub = this.subs.get(id);
    if (!sub || sub.channel !== 'pinned') return;
    this.removed.push(...sub.facts);
    sub.facts = [this.fact(hour, hour, value, refused, 'pinned', bytes)];
  }

  panels(route: number, seek: number, records: readonly PanelRecord<V>[], completed?: number, only?: ReadonlySet<number>) {
    if (!this.accept(route, seek, () => this.panels(route, seek, records, completed, only))) {
      if (seek < this.route(route).seek) for (const record of records) if ('bytes' in record) this.freed += record.bytes ?? 0;
      return;
    }
    for (const record of records) {
      const sub = this.subs.get(record.id);
      if (!sub || sub.route !== route || sub.channel !== 'panel') { if ('bytes' in record) this.freed += record.bytes ?? 0; continue; }
      if ((record.kind === 'data' || record.kind === 'refused' || record.kind === 'failed' || record.kind === 'stands') && record.stepped) sub.stepped = true;
      if (record.kind === 'data' || record.kind === 'refused') {
        const seeds = new Map<number, Fact<V>>(), cells = record.kind === 'data' ? record.cells ?? [] : [];
        sub.nativeCells = new Set(cells.map(cell => cell.id));
        for (const cell of cells) {
          this.group(cell.id, route, sub);
          if (cell.seedHour !== undefined && cell.seedHour <= record.hour && cell.values !== undefined)
            seeds.set(cell.id, this.fact(cell.seedHour, cell.seedHour, cell.values, null, 'panel', 0));
        }
        this.add(sub, this.fact(record.hour, record.hour, record.value, record.kind === 'refused' ? record.reason : null, 'panel', record.bytes ?? 0,
          cells.map(cell => cell.id), seeds));
      } else if (record.kind === 'failed') {
        this.add(sub, this.fact(record.hour, record.hour, undefined as V, null, 'panel', 0, [], new Map(), record.reason));
      } else if (record.kind === 'stands') {
        const last = sub.facts.at(-1);
        if (last) { last.c = Math.max(last.c, record.hour); sub.open = true; }
      } else {
        const last = sub.facts.at(-1);
        if (last && sub.open) last.c = Math.max(last.c, record.hour - 1);
        sub.open = false;
      }
    }
    if (completed !== undefined) for (const sub of this.subs.values()) if (sub.route === route && sub.stepped && sub.open && (!only || only.has(sub.id))) {
      const last = sub.facts.at(-1);
      if (last && last.c < completed) last.c = completed;
    }
  }

  followed(route: number, seek: number, message: FollowMessage<V>, only?: ReadonlySet<number>) {
    if (!this.accept(route, seek, () => this.followed(route, seek, message, only))) {
      if (seek < this.route(route).seek) for (const answer of message.answers) this.freed += answer.bytes ?? 0;
      return;
    }
    const at = this.route(route);
    if (message.reset) {
      at.reset++;
      for (const owner of this.owners(route)) if (owner.channel === 'followed') {
        this.removed.push(...owner.facts);
        owner.facts = []; owner.open = false;
      }
      for (const sub of this.subs.values()) if (sub.route === route && sub.carried && followsChannel(sub.carried)) sub.carried = null;
    }
    const answered = new Set<number>();
    for (const answer of message.answers) {
      const owner = this.subs.get(answer.id) ?? this.groups.get(answer.id);
      if (!owner || owner.route !== route || owner.channel !== 'followed') { this.freed += answer.bytes ?? 0; continue; }
      answered.add(answer.id);
      this.add(owner, this.fact(answer.hour, Math.max(answer.hour, message.limited?.get(answer.id) ?? message.hour), answer.value,
        answer.refused ?? null, 'followed', answer.bytes ?? 0));
    }
    for (const owner of this.owners(route)) {
      if (owner.channel !== 'followed' || answered.has(owner.id) || !owner.open || only && !only.has(owner.id)) continue;
      const last = owner.facts.at(-1), confirmed = message.limited?.get(owner.id) ?? message.hour;
      if (last && confirmed > last.c) last.c = confirmed;
    }
  }

  private *owners(route: number): Iterable<Owner<V>> {
    for (const sub of this.subs.values()) if (sub.route === route) yield sub;
    for (const group of this.groups.values()) if (group.route === route) yield group;
  }

  private join(fact: Fact<V>, target: number, cover: boolean): Picture<V> | null {
    if (cover && !(fact.h <= target && target <= fact.c)) return null;
    if (!fact.cells.length) return { h: fact.h, c: fact.c, parts: [fact], value: fact.value, refused: fact.refused, failed: fact.failed };
    const lists = fact.cells.map(id => {
      const own = this.groups.get(id)?.facts ?? [], seed = fact.seeds.get(id);
      return seed ? [...own, seed] : own;
    });
    const starts = new Set([fact.h]);
    for (const list of lists) for (const part of list) if (part.h > fact.h && part.h <= fact.c && part.h <= target) starts.add(part.h);
    for (const start of [...starts].sort((a, b) => b - a)) {
      const need = cover ? target : start;
      let h = fact.h, c = fact.c, complete = true;
      const parts = [fact];
      for (const list of lists) {
        let best: Fact<V> | null = null;
        for (const part of list) if (part.h <= start && part.c >= need && (!best || part.c > best.c || (part.c === best.c && part.h > best.h))) best = part;
        if (!best) { complete = false; break; }
        h = Math.max(h, best.h); c = Math.min(c, best.c); parts.push(best);
      }
      if (complete) return { h, c, parts, value: fact.value, refused: fact.refused, failed: fact.failed };
    }
    return null;
  }

  select(id: number, target: number): Picture<V> | null {
    const sub = this.subs.get(id);
    if (!sub) return null;
    let first = true;
    for (let at = sub.facts.length - 1; at >= 0; at--) {
      const fact = sub.facts[at]!;
      if (fact.h > target) continue;
      if (first) {
        first = false;
        const covering = this.join(fact, target, true);
        if (covering) return covering;
      }
      const latest = this.join(fact, target, false);
      if (latest) return latest;
    }
    return sub.carried && sub.carried.h <= target ? sub.carried : null;
  }

  window(id: number, target: number, picture: Picture<V> | null = this.select(id, target)): Window | null {
    const sub = this.subs.get(id);
    if (!sub || !picture) return null;
    const points = new Set<number>();
    const add = (fact: Pick<Fact<V>, 'h' | 'c'>) => { points.add(fact.h); points.add(fact.c + 1); };
    for (const fact of sub.facts) { add(fact); for (const seed of fact.seeds.values()) add(seed); }
    for (const id of sub.uses) for (const fact of this.groups.get(id)?.facts ?? []) add(fact);
    if (sub.carried) add(sub.carried);
    const sorted = [...points].sort((a, b) => a - b);
    let at = 0;
    while (at < sorted.length && sorted[at]! <= target) at++;
    let lo = at > 0 ? sorted[at - 1]! : -Infinity, hi = at < sorted.length ? sorted[at]! - 1 : Infinity;
    for (let i = at - 1; i > 0 && samePicture(this.select(id, sorted[i - 1]!), picture); i--) lo = sorted[i - 1]!;
    if (lo === sorted[0] && samePicture(this.select(id, lo - 1), picture)) lo = -Infinity;
    for (let i = at; i < sorted.length && samePicture(this.select(id, sorted[i]!), picture); i++) hi = i + 1 < sorted.length ? sorted[i + 1]! - 1 : Infinity;
    return [Math.max(lo, picture.h), hi];
  }

  private before(sub: Sub<V>, index: number, target: number): Picture<V> | null {
    for (let at = index - 1; at >= 0; at--) {
      const fact = sub.facts[at]!;
      if (fact.h > target) continue;
      const latest = this.join(fact, target, false);
      if (latest) return latest;
    }
    return null;
  }

  prune(id: number, actual: number, keep: readonly (Picture<V> | null | undefined)[]): number {
    const sub = this.subs.get(id);
    if (!sub || (!sub.facts.length && !sub.carried)) return Infinity;
    const current = this.select(id, actual), held = new Set<Fact<V>>();
    const from = current && !current.carried ? sub.facts.indexOf(current.parts[0]!) : -1;
    const fallback = from > 0 ? this.before(sub, from, actual) : null;
    keep = [...keep, fallback];
    for (const picture of [current, ...keep]) for (const part of picture?.parts ?? []) held.add(part);
    if (from > 0) sub.facts = sub.facts.filter((fact, at) => {
      const kept = at >= from || held.has(fact);
      if (!kept) this.removed.push(fact);
      return kept;
    });
    if (sub.carried && sub.facts.length && current !== sub.carried && !keep.some(picture => picture === sub.carried)) sub.carried = null;
    this.retain(sub, undefined, actual, keep);
    if (!this.prunable(id)) return Infinity;
    let next = Infinity;
    for (const fact of sub.facts) if (fact.h > actual && fact.h < next) next = fact.h;
    if (sub.carried && sub.carried.h > actual && sub.carried.h < next) next = sub.carried.h;
    for (const groupId of sub.uses) {
      const group = this.groups.get(groupId);
      if (!group) continue;
      for (let i = 0; i < group.facts.length; i++) {
        const fact = group.facts[i]!;
        if (fact.h > actual && fact.h < next) next = fact.h;
        if (!current && i < group.facts.length - 1 && fact.c >= actual && fact.c + 1 < next) next = fact.c + 1;
      }
    }
    return next;
  }

  private floor(sub: Sub<V>, actual: number, keep: readonly (Picture<V> | null | undefined)[]) {
    const current = this.select(sub.id, actual);
    let floor = Math.min(actual, current?.h ?? actual);
    if (!current || current.c < actual) {
      for (let at = sub.facts.length - 1; at >= 0; at--) if (sub.facts[at]!.h <= actual) { floor = Math.min(floor, sub.facts[at]!.h); break; }
    }
    return { floor, parts: new Set([...current?.parts ?? [], ...keep.flatMap(picture => picture?.parts ?? [])]) };
  }

  private retain(sub: Sub<V>, uses = new Set([...sub.facts.flatMap(fact => fact.cells), ...sub.carried?.parts[0]?.cells ?? [], ...sub.nativeCells]),
    actual?: number, keep: readonly (Picture<V> | null | undefined)[] = []) {
    for (const id of [...sub.uses]) if (!uses.has(id)) {
      sub.uses.delete(id);
      const group = this.groups.get(id);
      if (!group) continue;
      group.users.delete(sub.id);
      if (!group.users.size) {
        this.removed.push(...group.facts);
        this.groups.delete(id); this.released.push(id);
      }
    }
    if (actual === undefined) return;
    const { floor, parts } = this.floor(sub, actual, keep);
    for (const id of sub.uses) {
      const group = this.groups.get(id);
      if (!group) continue;
      let lowest = floor;
      const needed = new Set(parts);
      for (const user of group.users) if (user !== sub.id) {
        const other = this.subs.get(user);
        if (!other) continue;
        const theirs = this.floor(other, actual, []);
        lowest = Math.min(lowest, theirs.floor);
        for (const part of theirs.parts) needed.add(part);
      }
      group.facts = group.facts.filter((fact, at) => {
        const kept = fact.c >= lowest || needed.has(fact) || at === group.facts.length - 1;
        if (!kept) this.removed.push(fact);
        return kept;
      });
    }
  }

  retained(id: number): { facts: number; cells: number } {
    const sub = this.subs.get(id);
    if (!sub) return { facts: 0, cells: 0 };
    let cells = 0;
    for (const group of sub.uses) cells += this.groups.get(group)?.facts.length ?? 0;
    return { facts: sub.facts.length, cells };
  }
}

export function changeWindow(window: Window, target: number, next: (hour: number) => number, from?: number): Window {
  const [lo, hi] = window;
  let start = from !== undefined && from >= lo && from <= target ? from : lo;
  if (!Number.isFinite(start)) return window;
  for (let after = next(start); after <= target; after = next(start)) {
    if (!(after > start)) throw new Error(`A change hour ${after} does not follow ${start}`);
    start = after;
  }
  return [start, Math.min(hi, next(start) - 1)];
}

export interface Component<V> { picture: Picture<V>; window: Window }

export function joinComponents<V>(components: readonly Component<V>[]): { h: number; c: number; window: Window } | null {
  if (!components.length) return null;
  let h = -Infinity, c = Infinity, lo = -Infinity, hi = Infinity;
  for (const { picture, window } of components) {
    h = Math.max(h, picture.h); c = Math.min(c, picture.c);
    lo = Math.max(lo, window[0]); hi = Math.min(hi, window[1]);
  }
  return h <= c && lo <= hi ? { h, c, window: [lo, hi] } : null;
}
