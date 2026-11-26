export type Revision = number | { readonly [key: string]: Revision };
export type Revisions = Readonly<Record<string, Revision>>;

export interface PayloadsMessage {
  readonly type: 'payloads';
  readonly put?: readonly (readonly [id: number, value: unknown])[];
  readonly drop?: readonly number[];
  readonly identity?: { id: number; base: number; changes: FieldChanges };
}

interface FieldChanges {
  set: [string, unknown][];
  drop: string[];
  nested?: [string, FieldChanges][];
  order?: string[];
}

const KEYED: Readonly<Record<string, number>> = { staticPanels: 1, staticAnswers: 1, hovers: 1, pagedRows: 2, tiledBySource: 2 };

const revisions = new WeakMap<object, number>();
let nextRevision = 1;

export function revisionOf(value: object): number {
  let revision = revisions.get(value);
  if (revision === undefined) revisions.set(value, revision = nextRevision++);
  return revision;
}

const isRecord = (value: unknown): value is Readonly<Record<string, unknown>> =>
  !!value && typeof value === 'object' && !Array.isArray(value) && Object.getPrototypeOf(value) === Object.prototype;

function changedFields(before: Readonly<Record<string, unknown>>, after: Readonly<Record<string, unknown>>, dictionaries: boolean): FieldChanges {
  const old = Object.keys(before), keys = Object.keys(after);
  const changes: FieldChanges = { set: [], drop: old.filter(key => !Object.hasOwn(after, key)) };
  let added = false;
  let kept = 0;
  for (const key of keys) {
    const value = after[key], previous = before[key];
    if (Object.hasOwn(before, key)) {
      while (kept < old.length && !Object.hasOwn(after, old[kept]!)) kept++;
      if (old[kept++] !== key || added) changes.order = keys;
      if (Object.is(value, previous)) continue;
      if (dictionaries && isRecord(previous) && isRecord(value)) {
        const child = changedFields(previous, value, false);
        if (child.drop.length || child.set.length || child.order) (changes.nested ??= []).push([key, child]);
        continue;
      }
    } else added = true;
    changes.set.push([key, value]);
  }
  return changes;
}

function applyFields(before: Readonly<Record<string, unknown>>, changes: FieldChanges): Record<string, unknown> {
  const result = { ...before, ...Object.fromEntries(changes.set) };
  for (const key of changes.drop) delete result[key];
  if (changes.nested) for (const [key, fields] of changes.nested) result[key] = applyFields(before[key] as Record<string, unknown>, fields);
  return changes.order ? Object.fromEntries(changes.order.map(key => [key, result[key]])) : result;
}

export function sameRevision(a: Revision | undefined, b: Revision | undefined): boolean {
  if (a === b) return true;
  if (a === undefined || b === undefined || typeof a === 'number' || typeof b === 'number') return false;
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  for (const key of keys) if (!sameRevision(a[key], b[key])) return false;
  return true;
}

export interface Encoded {
  readonly wire: unknown;
  readonly ids: ReadonlySet<number>;
}

export class PayloadSender {
  private readonly resident = new Map<number, { readonly value: object; users: number }>();
  private readonly post: (message: PayloadsMessage) => void;

  constructor(post: (message: PayloadsMessage) => void) { this.post = post; }

  get size(): number { return this.resident.size; }

  acquire(model: unknown, previousIdentity?: number): Encoded {
    if (!isRecord(model)) return { wire: model, ids: new Set() };
    const ids = new Set<number>();
    const put: [number, unknown][] = [];
    let identity: PayloadsMessage['identity'];
    const leaf = (value: object, base?: number): number => {
      const id = revisionOf(value);
      if (ids.has(id)) return id;
      ids.add(id);
      const held = this.resident.get(id);
      if (held) held.users++;
      else {
        this.resident.set(id, { value, users: 1 });
        const previous = base === undefined ? undefined : this.resident.get(base)?.value;
        if (base !== undefined && isRecord(previous) && isRecord(value) && previous.scope && previous.scope === value.scope) {
          identity = { id, base, changes: changedFields(previous, value, true) };
        } else put.push([id, value]);
      }
      return id;
    };
    const refer = (value: object, depth: number): Revision => {
      if (depth === 0 || !isRecord(value) || Object.values(value).some(inner => !inner || typeof inner !== 'object')) return leaf(value);
      const out: Record<string, Revision> = {};
      for (const [key, inner] of Object.entries(value)) if (inner && typeof inner === 'object') out[key] = refer(inner, depth - 1);
      return out;
    };
    const wire: Record<string, unknown> = {};
    const refs: Record<string, Revision> = {};
    for (const [field, value] of Object.entries(model)) {
      if (value && typeof value === 'object') refs[field] = field === 'identitySnapshot' ? leaf(value, previousIdentity) : refer(value, KEYED[field] ?? 0);
      else if (value !== undefined) wire[field] = value;
    }
    wire.revisions = refs;
    if (put.length || identity) this.post({ type: 'payloads', ...put.length ? { put } : {}, ...identity ? { identity } : {} });
    return { wire, ids };
  }

  release(ids: ReadonlySet<number>): void {
    const drop: number[] = [];
    for (const id of ids) {
      const held = this.resident.get(id);
      if (!held || --held.users > 0) continue;
      this.resident.delete(id);
      drop.push(id);
    }
    if (drop.length) this.post({ type: 'payloads', drop });
  }
}

export class PayloadStore {
  private readonly values = new Map<number, unknown>();

  get size(): number { return this.values.size; }

  apply(message: PayloadsMessage): void {
    if (message.put) for (const [id, value] of message.put) this.values.set(id, value);
    if (message.identity) {
      const { id, base, changes } = message.identity, before = this.values.get(base);
      if (!isRecord(before)) throw new Error(`Identity snapshot ${base} was referenced before it reached this lane`);
      this.values.set(id, applyFields(before, changes));
    }
    if (message.drop) for (const id of message.drop) this.values.delete(id);
  }

  resolve(wire: unknown): unknown {
    if (!isRecord(wire) || !isRecord(wire.revisions)) return wire;
    const out: Record<string, unknown> = { ...wire };
    for (const [field, revision] of Object.entries(wire.revisions as Revisions)) out[field] = this.value(revision);
    return out;
  }

  private value(revision: Revision): unknown {
    if (typeof revision !== 'number') {
      const out: Record<string, unknown> = {};
      for (const [key, inner] of Object.entries(revision)) out[key] = this.value(inner);
      return out;
    }
    if (!this.values.has(revision)) throw new Error(`Payload ${revision} was referenced before it reached this lane`);
    return this.values.get(revision);
  }
}
