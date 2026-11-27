export type Channel = 'panel' | 'followed' | 'pinned';

export interface Fact<V> {
  readonly h: number;
  c: number;
  readonly value: V;
  readonly refused: string | null;
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
  readonly carried?: true;
}

export interface CellSeed<V> { id: number; seedHour?: number; values?: V }

export type PanelRecord<V> =
  | { kind: 'data'; id: number; hour: number; value: V; stepped?: boolean; cells?: readonly CellSeed<V>[]; bytes?: number }
  | { kind: 'refused'; id: number; hour: number; reason: string; value: V; stepped?: boolean; bytes?: number }
  | { kind: 'stands'; id: number; hour: number; stepped?: boolean }
  | { kind: 'again'; id: number; hour: number; stepped?: boolean }
  | { kind: 'waiting'; id: number; hour: number };

export interface FollowAnswer<V> { id: number; hour: number; value: V; refused?: string | null; bytes?: number }

export interface FollowMessage<V> { hour: number; reset?: boolean; limited?: ReadonlyMap<number, number>; answers: readonly FollowAnswer<V>[] }

export interface Generation { seek: number; reset: number }

interface Owner<V> { id: number; route: number; channel: Channel; facts: Fact<V>[]; open: boolean }
interface Sub<V> extends Owner<V> { stepped: boolean; openSeq: number; carried: Picture<V> | null; uses: Set<number>; nativeCells: Set<number> }
interface Group<V> extends Owner<V> { users: Set<number> }
interface Route { seek: number; reset: number; completedPanel: number; completedSeq: number; openStepped: Set<number>; pendingConfirm: Set<number>; early: [number, () => void][] }

export type Window = readonly [lo: number, hi: number];

let versions = 0;

export const samePicture = <V>(a: Picture<V> | null | undefined, b: Picture<V> | null | undefined) => a === b
  || (!!a && !!b && a.parts.length === b.parts.length && a.parts.every((part, at) => part === b.parts[at]));

export const followsChannel = <V>(picture: Picture<V>) => picture.parts.some(part => part.channel === 'followed');

export class Facts<V> {
  readonly stats = { completionVisits: 0 };
  private readonly subs = new Map<number, Sub<V>>();
  private readonly groups = new Map<number, Group<V>>();
  private readonly routes = new Map<number, Route>();
  private released: number[] = [];
  private freed = 0;
  private removed: Fact<V>[] = [];

  private route(id: number): Route {
    let route = this.routes.get(id);
    if (!route) this.routes.set(id, route = { seek: 0, reset: 0, completedPanel: -Infinity, completedSeq: 0, openStepped: new Set(), pendingConfirm: new Set(), early: [] });
    return route;
  }

  private materializePanel(sub: Sub<V>) {
    if (sub.channel !== 'panel' || !sub.stepped || !sub.open) return;
    const at = this.routes.get(sub.route);
    if (!at || sub.openSeq === at.completedSeq) return;
    const last = sub.facts.at(-1);
    if (last && last.c < at.completedPanel) last.c = at.completedPanel;
    sub.openSeq = at.completedSeq;
  }

  has(id: number) { return this.subs.has(id); }

  owns(id: number) { return this.subs.has(id) || this.groups.has(id); }

  usesOf(id: number): ReadonlySet<number> | undefined { return this.subs.get(id)?.uses; }

  usersOf(id: number): ReadonlySet<number> | undefined { return this.groups.get(id)?.users; }

  latest(id: number): V | undefined { return (this.subs.get(id) ?? this.groups.get(id))?.facts.at(-1)?.value; }

  isOpenSteppedPanel(id: number): boolean {
    const sub = this.subs.get(id);
    return !!sub && sub.channel === 'panel' && sub.stepped && sub.open && sub.facts.length > 0;
  }

  confirmedHour(id: number): number {
    const sub = this.subs.get(id);
    if (!sub) return -Infinity;
    this.materializePanel(sub);
    const last = sub.facts.at(-1);
    if (!last) return -Infinity;
    let confirmed = last.c;
    for (let i = 0; i < last.cells.length; i++) {
      const cellId = last.cells[i]!;
      const groupLast = this.groups.get(cellId)?.facts.at(-1);
      const seed = last.seeds.get(cellId);
      const cellC = Math.max(groupLast ? groupLast.c : -Infinity, seed ? seed.c : -Infinity);
      if (cellC < confirmed) confirmed = cellC;
    }
    return confirmed;
  }

  hasInterruptionBetween(id: number, fromHour: number, toHour: number, isBad: (value: V) => boolean): boolean {
    if (toHour <= fromHour) return false;
    const sub = this.subs.get(id);
    if (!sub) return true;
    this.materializePanel(sub);
    const checkOwner = (facts: readonly Fact<V>[]): boolean => {
      let covered = fromHour;
      for (let i = 0; i < facts.length; i++) {
        const fact = facts[i]!;
        if (fact.c <= fromHour) continue;
        if (fact.h > toHour) break;
        if (fact.h > covered + 1) return true;
        if (fact.refused !== null || isBad(fact.value)) return true;
        if (fact.c > covered) covered = fact.c;
        if (covered >= toHour) return false;
      }
      return covered < toHour;
    };
    if (checkOwner(sub.facts)) return true;
    for (const groupId of sub.uses) {
      const group = this.groups.get(groupId);
      if (!group) continue;
      for (let i = 0; i < group.facts.length; i++) {
        const fact = group.facts[i]!;
        if (fact.c <= fromHour) continue;
        if (fact.h > toHour) break;
        if (fact.refused !== null || isBad(fact.value)) return true;
      }
    }
    return false;
  }

  fail(id: number) {
    const sub = this.subs.get(id);
    if (!sub) return;
    this.removed.push(...sub.facts);
    sub.facts = [];
    sub.open = false;
    sub.stepped = false;
    sub.carried = null;
    sub.nativeCells.clear();
    const at = this.routes.get(sub.route);
    at?.openStepped.delete(id);
    at?.pendingConfirm.delete(id);
    this.retain(sub, new Set());
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
    const seq = channel !== 'pinned' ? this.route(route).completedSeq : 0;
    this.subs.set(id, { id, route, channel, facts: [], open: false, stepped: false, openSeq: seq, carried: null, uses: new Set(), nativeCells: new Set() });
  }

  unfollow(id: number): number[] {
    const sub = this.subs.get(id);
    if (!sub) return [];
    this.subs.delete(id);
    const at = this.routes.get(sub.route);
    at?.openStepped.delete(id);
    at?.pendingConfirm.delete(id);
    sub.nativeCells.clear();
    this.removed.push(...sub.facts);
    this.retain(sub, new Set());
    return this.drain();
  }

  drain(): number[] { const out = this.released; this.released = []; return out; }

  hasRemoved() { return this.removed.length > 0; }

  retireRoute(route: number) { this.routes.delete(route); }

  retainedBytes(): number {
    const held = new Set<Fact<V>>();
    for (const sub of this.subs.values()) {
      for (const fact of sub.facts) held.add(fact);
      for (const part of sub.carried?.parts ?? []) held.add(part);
    }
    for (const group of this.groups.values()) for (const fact of group.facts) held.add(fact);
    for (const fact of this.removed) held.add(fact);
    let bytes = this.freed;
    for (const fact of held) bytes += fact.bytes;
    return bytes;
  }

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
      sub.carried = shown && shown.h <= target && target <= shown.c ? Object.freeze({ ...shown, carried: true as const }) : null;
      this.removed.push(...sub.facts);
      sub.facts = []; sub.open = false; sub.stepped = false;
    }
    for (const group of this.groups.values()) if (group.route === route) {
      this.removed.push(...group.facts);
      group.facts = []; group.open = false;
    }
    at.completedPanel = -Infinity;
    at.completedSeq++;
    at.openStepped.clear();
    at.pendingConfirm.clear();
    for (const sub of this.subs.values()) if (sub.route === route && sub.channel !== 'pinned') sub.openSeq = at.completedSeq;
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
    cells: readonly number[] = [], seeds: ReadonlyMap<number, Fact<V>> = new Map()): Fact<V> {
    return { h, c, value, refused, cells, seeds, channel, version: ++versions, bytes };
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
    const at = this.route(route);
    for (const record of records) {
      const sub = this.subs.get(record.id);
      if (!sub || sub.route !== route || sub.channel !== 'panel') { if ('bytes' in record) this.freed += record.bytes ?? 0; continue; }
      this.materializePanel(sub);
      if ((record.kind === 'data' || record.kind === 'refused' || record.kind === 'stands' || record.kind === 'again') && record.stepped) {
        if (!sub.stepped) { sub.stepped = true; sub.openSeq = at.completedSeq; }
      }
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
        sub.openSeq = at.completedSeq;
        if (sub.stepped) { at.openStepped.add(sub.id); at.pendingConfirm.add(sub.id); }
      } else if (record.kind === 'again') {
        const last = sub.facts.at(-1);
        if (last) {
          this.add(sub, this.fact(record.hour, record.hour, last.value, last.refused, 'panel', 0, last.cells, last.seeds));
          sub.openSeq = at.completedSeq;
          if (sub.stepped) { at.openStepped.add(sub.id); at.pendingConfirm.add(sub.id); }
        }
      } else if (record.kind === 'stands') {
        const last = sub.facts.at(-1);
        if (last) {
          last.c = Math.max(last.c, record.hour);
          sub.open = true;
          sub.openSeq = at.completedSeq;
          if (sub.stepped) { at.openStepped.add(sub.id); at.pendingConfirm.add(sub.id); }
        }
      } else {
        sub.open = false;
        at.openStepped.delete(sub.id);
        at.pendingConfirm.delete(sub.id);
      }
    }
    if (completed !== undefined) {
      if (only) {
        for (const id of only) {
          const sub = this.subs.get(id);
          if (!sub || sub.route !== route || !sub.stepped || !sub.open) continue;
          this.stats.completionVisits++;
          this.materializePanel(sub);
          const last = sub.facts.at(-1);
          if (last && last.c < completed) last.c = completed;
          if (completed >= at.completedPanel) at.pendingConfirm.delete(id);
        }
      } else if (completed > at.completedPanel) {
        at.completedPanel = completed;
        at.completedSeq++;
        at.pendingConfirm.clear();
      } else if (completed === at.completedPanel) {
        for (const id of at.pendingConfirm) {
          const sub = this.subs.get(id);
          if (!sub || sub.route !== route || !sub.stepped || !sub.open) continue;
          this.stats.completionVisits++;
          const last = sub.facts.at(-1);
          if (last && last.c < completed) last.c = completed;
        }
        at.pendingConfirm.clear();
      } else {
        for (const id of at.openStepped) {
          const sub = this.subs.get(id);
          if (!sub || !sub.stepped || !sub.open) continue;
          this.stats.completionVisits++;
          const last = sub.facts.at(-1);
          if (last && last.c < completed) last.c = completed;
        }
      }
    }
  }

  resetFollowed(route: number, seek: number) {
    if (!this.accept(route, seek, () => this.resetFollowed(route, seek))) return;
    const at = this.route(route);
    at.reset++;
    for (const owner of this.owners(route)) if (owner.channel === 'followed') {
      this.removed.push(...owner.facts);
      owner.facts = []; owner.open = false;
    }
    for (const sub of this.subs.values()) if (sub.route === route && sub.carried && followsChannel(sub.carried)) sub.carried = null;
  }

  addFollowedAnswer(route: number, seek: number, answer: FollowAnswer<V>, extendPrevious = false) {
    if (!this.accept(route, seek, () => this.addFollowedAnswer(route, seek, answer, extendPrevious))) {
      if (seek < this.route(route).seek) this.freed += answer.bytes ?? 0;
      return;
    }
    const owner = this.subs.get(answer.id) ?? this.groups.get(answer.id);
    if (!owner || owner.route !== route || owner.channel !== 'followed') { this.freed += answer.bytes ?? 0; return; }
    if (extendPrevious && owner.open) {
      const last = owner.facts.at(-1);
      if (last && last.h < answer.hour && last.c < answer.hour - 1) last.c = answer.hour - 1;
    }
    this.add(owner, this.fact(answer.hour, answer.hour, answer.value, answer.refused ?? null, 'followed', answer.bytes ?? 0));
  }

  confirmFollowed(route: number, seek: number, hour: number, limited?: ReadonlyMap<number, number>, only?: ReadonlySet<number>) {
    if (!this.accept(route, seek, () => this.confirmFollowed(route, seek, hour, limited, only))) return;
    for (const owner of this.owners(route)) {
      if (owner.channel !== 'followed' || !owner.open || only && !only.has(owner.id)) continue;
      const last = owner.facts.at(-1), confirmed = limited?.get(owner.id) ?? hour;
      if (last && confirmed > last.c) last.c = confirmed;
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
    const cells = fact.cells;
    if (!cells.length) return { h: fact.h, c: fact.c, parts: [fact], value: fact.value, refused: fact.refused };
    if (cover) {
      let h = fact.h, c = fact.c;
      const parts: Fact<V>[] = [fact];
      for (let i = 0; i < cells.length; i++) {
        const id = cells[i]!;
        const own = this.groups.get(id)?.facts;
        const seed = fact.seeds.get(id);
        let best: Fact<V> | null = null;
        if (own) {
          for (let j = 0; j < own.length; j++) {
            const part = own[j]!;
            if (part.h <= target && part.c >= target && (!best || part.c > best.c || (part.c === best.c && part.h > best.h))) best = part;
          }
        }
        if (seed && seed.h <= target && seed.c >= target && (!best || seed.c > best.c || (seed.c === best.c && seed.h > best.h))) best = seed;
        if (!best) return null;
        if (best.h > h) h = best.h;
        if (best.c < c) c = best.c;
        parts.push(best);
      }
      return { h, c, parts, value: fact.value, refused: fact.refused };
    }
    let extraStarts: Set<number> | null = null;
    for (let i = 0; i < cells.length; i++) {
      const id = cells[i]!;
      const own = this.groups.get(id)?.facts;
      if (own) {
        for (let j = 0; j < own.length; j++) {
          const ph = own[j]!.h;
          if (ph > fact.h && ph <= fact.c && ph <= target) (extraStarts ??= new Set([fact.h])).add(ph);
        }
      }
      const seed = fact.seeds.get(id);
      if (seed && seed.h > fact.h && seed.h <= fact.c && seed.h <= target) (extraStarts ??= new Set([fact.h])).add(seed.h);
    }
    const ordered = extraStarts ? [...extraStarts].sort((a, b) => b - a) : [fact.h];
    for (let si = 0; si < ordered.length; si++) {
      const start = ordered[si]!;
      let h = fact.h, c = fact.c, complete = true;
      const parts: Fact<V>[] = [fact];
      for (let i = 0; i < cells.length; i++) {
        const id = cells[i]!;
        const own = this.groups.get(id)?.facts;
        const seed = fact.seeds.get(id);
        let best: Fact<V> | null = null;
        if (own) {
          for (let j = 0; j < own.length; j++) {
            const part = own[j]!;
            if (part.h <= start && part.c >= start && (!best || part.c > best.c || (part.c === best.c && part.h > best.h))) best = part;
          }
        }
        if (seed && seed.h <= start && seed.c >= start && (!best || seed.c > best.c || (seed.c === best.c && seed.h > best.h))) best = seed;
        if (!best) { complete = false; break; }
        if (best.h > h) h = best.h;
        if (best.c < c) c = best.c;
        parts.push(best);
      }
      if (complete) return { h, c, parts, value: fact.value, refused: fact.refused };
    }
    return null;
  }

  select(id: number, target: number): Picture<V> | null {
    const sub = this.subs.get(id);
    if (!sub) return null;
    this.materializePanel(sub);
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
    this.materializePanel(sub);
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
    const parts = new Set<Fact<V>>();
    if (current) for (let i = 0; i < current.parts.length; i++) parts.add(current.parts[i]!);
    for (let i = 0; i < keep.length; i++) {
      const pic = keep[i];
      if (pic) for (let j = 0; j < pic.parts.length; j++) parts.add(pic.parts[j]!);
    }
    return { floor, parts };
  }

  private retain(sub: Sub<V>, uses?: Set<number>,
    actual?: number, keep: readonly (Picture<V> | null | undefined)[] = []) {
    if (!uses) {
      uses = new Set<number>(sub.nativeCells);
      for (let i = 0; i < sub.facts.length; i++) {
        const cells = sub.facts[i]!.cells;
        for (let j = 0; j < cells.length; j++) uses.add(cells[j]!);
      }
      const carriedCells = sub.carried?.parts[0]?.cells;
      if (carriedCells) for (let i = 0; i < carriedCells.length; i++) uses.add(carriedCells[i]!);
    }
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

export function changeWindow(window: Window, target: number, next: (hour: number) => number | null, from?: number): Window {
  const [lo, hi] = window;
  let start = from !== undefined && from >= lo && from <= target ? from : lo;
  if (!Number.isFinite(start)) return window;
  let after = next(start);
  while (after !== null && after <= target) {
    if (!(after > start)) throw new Error(`A change hour ${after} does not follow ${start}`);
    start = after;
    after = next(start);
  }
  return after === null ? [target, hi] : [start, Math.min(hi, after - 1)];
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
