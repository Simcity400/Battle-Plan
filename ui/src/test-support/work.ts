import assert from 'node:assert/strict';

export interface Work { reads: number; probes: number; scanned: number }

const SCANS = ['find', 'findIndex', 'findLast', 'findLastIndex', 'filter', 'some', 'every', 'indexOf', 'lastIndexOf', 'includes'] as const;

export function measureWork<T, R>(input: T, run: (input: T) => R): Work {
  const work: Work = { reads: 0, probes: 0, scanned: 0 };
  const seen = new WeakMap<object, object>();
  const counted = <V,>(value: V): V => {
    if (typeof value !== 'object' || value === null || ArrayBuffer.isView(value) || value instanceof Map || value instanceof Set) return value;
    let proxy = seen.get(value);
    if (!proxy) seen.set(value, proxy = new Proxy(value, { get: (target, key) => { work.reads++; return counted(Reflect.get(target, key)); } }));
    return proxy as V;
  };
  const wrapped = counted(input);
  const map = Map.prototype as unknown as Record<'get' | 'has' | 'set', (...args: unknown[]) => unknown>;
  const array = Array.prototype as unknown as Record<string | symbol, (...args: unknown[]) => unknown>;
  const kept = { get: map.get, has: map.has, set: map.set, iterator: array[Symbol.iterator], scans: SCANS.map(name => array[name]) };
  for (const name of ['get', 'has', 'set'] as const) {
    const original = kept[name];
    map[name] = function (this: unknown, ...args: unknown[]) { work.probes++; return original.apply(this, args); };
  }
  SCANS.forEach((name, at) => {
    const original = kept.scans[at];
    array[name] = function (this: { length: number }, ...args: unknown[]) { work.scanned += this.length; return original.apply(this, args); };
  });
  array[Symbol.iterator] = function (this: unknown) {
    const iterator = kept.iterator.call(this) as Iterator<unknown>;
    return { next: () => { const step = iterator.next(); if (!step.done) work.scanned++; return step; }, [Symbol.iterator]() { return this; } };
  };
  try {
    run(wrapped);
    return { ...work };
  } finally {
    map.get = kept.get; map.has = kept.has; map.set = kept.set;
    array[Symbol.iterator] = kept.iterator;
    SCANS.forEach((name, at) => { array[name] = kept.scans[at]; });
  }
}

export function assertLinearWork(measure: (size: number) => Work, what: string, sizes: readonly [number, number, number] = [50, 500, 5000]): void {
  const [small, middle, large] = sizes.map(measure);
  const ratio = (sizes[2] - sizes[1]) / (sizes[1] - sizes[0]);
  for (const kind of ['reads', 'probes', 'scanned'] as const)
    assert.equal(large[kind] - middle[kind], (middle[kind] - small[kind]) * ratio,
      `${what}: ${kind} at ${sizes.join(', ')} items were ${[small[kind], middle[kind], large[kind]].join(', ')}, not a fixed cost and the same per item`);
}
