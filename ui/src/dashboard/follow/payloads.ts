export type Revision = number | { readonly [key: string]: Revision };
export type Revisions = Readonly<Record<string, Revision>>;

export interface PayloadsMessage {
  readonly type: 'payloads';
  readonly put?: readonly (readonly [id: number, value: unknown])[];
  readonly drop?: readonly number[];
}

const KEYED: Readonly<Record<string, number>> = { staticPanels: 1, staticAnswers: 1, hovers: 1, pagedRows: 1, tiledBySource: 2 };

const revisions = new WeakMap<object, number>();
let nextRevision = 1;

export function revisionOf(value: object): number {
  let revision = revisions.get(value);
  if (revision === undefined) revisions.set(value, revision = nextRevision++);
  return revision;
}

const isRecord = (value: unknown): value is Readonly<Record<string, unknown>> =>
  !!value && typeof value === 'object' && !Array.isArray(value) && Object.getPrototypeOf(value) === Object.prototype;

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

  acquire(model: unknown): Encoded {
    if (!isRecord(model)) return { wire: model, ids: new Set() };
    const ids = new Set<number>();
    const put: [number, unknown][] = [];
    const leaf = (value: object): number => {
      const id = revisionOf(value);
      if (ids.has(id)) return id;
      ids.add(id);
      const held = this.resident.get(id);
      if (held) held.users++;
      else {
        this.resident.set(id, { value, users: 1 });
        put.push([id, value]);
      }
      return id;
    };
    const refer = (value: object, depth: number): Revision => {
      if (depth === 0 || !isRecord(value)) return leaf(value);
      const out: Record<string, Revision> = {};
      for (const [key, inner] of Object.entries(value)) if (inner && typeof inner === 'object') out[key] = refer(inner, depth - 1);
      return out;
    };
    const wire: Record<string, unknown> = {};
    const refs: Record<string, Revision> = {};
    for (const [field, value] of Object.entries(model)) {
      if (value && typeof value === 'object') refs[field] = refer(value, KEYED[field] ?? 0);
      else if (value !== undefined) wire[field] = value;
    }
    wire.revisions = refs;
    if (put.length) this.post({ type: 'payloads', put });
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
