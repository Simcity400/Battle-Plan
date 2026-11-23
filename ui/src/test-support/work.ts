import assert from 'node:assert/strict';

export interface Work { reads: number; probes: number; visited: number; copied: number }

type Method = (this: unknown, ...args: unknown[]) => unknown;

const CALLBACKS = ['find', 'findIndex', 'findLast', 'findLastIndex', 'filter', 'some', 'every', 'map', 'forEach', 'reduce', 'reduceRight', 'flatMap'] as const;
const SEARCHES = ['indexOf', 'lastIndexOf', 'includes'] as const;
const COPIES = ['concat', 'slice', 'toSorted', 'toReversed', 'toSpliced', 'with', 'flat'] as const;
const WHOLE = ['sort', 'reverse'] as const;

export function measureWork<T>(input: T, run: (input: T) => void): Work {
  const work: Work = { reads: 0, probes: 0, visited: 0, copied: 0 };
  const seen = new WeakMap<object, object>();
  const counted = <V,>(value: V): V => {
    if (typeof value !== 'object' || value === null || ArrayBuffer.isView(value) || value instanceof Map || value instanceof Set) return value;
    let proxy = seen.get(value);
    if (!proxy) seen.set(value, proxy = new Proxy(value, { get: (target, key) => { work.reads++; return counted(Reflect.get(target, key)); } }));
    return proxy as V;
  };
  const wrapped = counted(input);
  const map = Map.prototype as unknown as Record<string, Method>;
  const array = Array.prototype as unknown as Record<string | symbol, Method>;
  const kept: [Record<string | symbol, Method>, string | symbol, Method][] = [];
  const { indexOf, lastIndexOf } = array;
  const patch = (owner: Record<string | symbol, Method>, name: string | symbol, make: (original: Method) => Method) => {
    const original = owner[name];
    kept.push([owner, name, original]);
    owner[name] = make(original);
  };
  for (const name of ['get', 'has', 'set', 'delete']) patch(map, name, original => function (...args) { work.probes++; return original.apply(this, args); });
  for (const name of CALLBACKS) patch(array, name, original => function (callback, ...rest) {
    const visit = function (this: unknown, ...args: unknown[]) { work.visited++; return (callback as Method).apply(this, args); };
    const given = typeof callback === 'function' ? visit : callback;
    return rest.length ? original.call(this, given, rest[0]) : original.call(this, given);
  });
  for (const name of SEARCHES) patch(array, name, original => function (...args) {
    const length = (this as { length: number }).length;
    const at = (name === 'lastIndexOf' ? lastIndexOf : indexOf).apply(this, args) as number;
    const from = Number(args[1] ?? (name === 'lastIndexOf' ? length - 1 : 0));
    const start = from < 0 ? Math.max(0, length + from) : Math.min(from, length);
    work.visited += name === 'lastIndexOf' ? (at < 0 ? start + 1 : start - at + 1) : (at < 0 ? length - start : at - start + 1);
    return original.apply(this, args);
  });
  for (const name of COPIES) patch(array, name, original => function (...args) {
    const out = original.apply(this, args) as { length: number };
    work.copied += out.length;
    return out;
  });
  for (const name of WHOLE) patch(array, name, original => function (...args) {
    work.copied += (this as { length: number }).length;
    return original.apply(this, args);
  });
  const at = (value: unknown, length: number, otherwise: number) => {
    if (value === undefined) return otherwise;
    const index = Math.trunc(Number(value)) || 0;
    return index < 0 ? Math.max(0, length + index) : Math.min(index, length);
  };
  patch(array, 'splice', original => function (...args) {
    const length = (this as { length: number }).length, start = args.length ? at(args[0], length, 0) : length;
    const removed = args.length < 2 ? length - start : Math.min(Math.max(0, Math.trunc(Number(args[1])) || 0), length - start);
    const added = Math.max(0, args.length - 2);
    work.copied += removed + added + (removed === added ? 0 : length - start - removed);
    return original.apply(this, args);
  });
  patch(array, 'unshift', original => function (...args) {
    work.copied += args.length ? (this as { length: number }).length + args.length : 0;
    return original.apply(this, args);
  });
  patch(array, 'shift', original => function (...args) {
    work.copied += Math.max(0, (this as { length: number }).length - 1);
    return original.apply(this, args);
  });
  patch(array, 'fill', original => function (...args) {
    const length = (this as { length: number }).length;
    work.copied += Math.max(0, at(args[2], length, length) - at(args[1], length, 0));
    return original.apply(this, args);
  });
  patch(array, 'copyWithin', original => function (...args) {
    const length = (this as { length: number }).length, target = at(args[0], length, 0), start = at(args[1], length, 0);
    work.copied += Math.max(0, Math.min(at(args[2], length, length) - start, length - target));
    return original.apply(this, args);
  });
  patch(array, Symbol.iterator, original => function () {
    const iterator = original.call(this) as Iterator<unknown>;
    return { next: () => { const step = iterator.next(); if (!step.done) work.visited++; return step; }, [Symbol.iterator]() { return this; } };
  });
  try {
    run(wrapped);
    return { ...work };
  } finally {
    for (let at = kept.length - 1; at >= 0; at--) { const [owner, name, original] = kept[at]!; owner[name] = original; }
  }
}

export function assertLinearWork(measure: (size: number) => Work, what: string, sizes: readonly [number, number, number] = [50, 500, 5000]): void {
  const [small, middle, large] = sizes.map(measure);
  for (const kind of ['reads', 'probes', 'visited', 'copied'] as const) {
    const early = (middle[kind] - small[kind]) / (sizes[1] - sizes[0]), late = (large[kind] - middle[kind]) / (sizes[2] - sizes[1]);
    assert.ok(late <= 2 * early + 0.1,
      `${what}: ${kind} at ${sizes.join(', ')} items were ${[small[kind], middle[kind], large[kind]].join(', ')}, ${late.toFixed(2)} an item late against ${early.toFixed(2)} early`);
  }
}
