
export interface Live<T> { read(): T; subscribe(changed: () => void): () => void }

export interface LiveValues<V> { get(key: string): V | undefined; missing(key: string): string; raw?(key: string): bigint | undefined }

export const liveKey = (...parts: readonly string[]) => parts.join('\u0000');

export function valuesOf<V>(values: ReadonlyMap<string, V>, reason: string | ((key: string) => string)): LiveValues<V> {
  return { get: key => values.get(key), missing: typeof reason === 'string' ? () => reason : reason };
}

export function liveByColumn<V>(columns: Readonly<Record<string, Live<LiveValues<V>>>>): Live<LiveValues<V>> {
  const sources = Object.entries(columns);
  const split = (key: string) => { const at = key.lastIndexOf('\u0000'); return [key.slice(0, at), key.slice(at + 1)] as const; };
  let made: { from: LiveValues<V>[]; value: LiveValues<V> } | null = null;
  return {
    read: () => {
      const from = sources.map(([, live]) => live.read());
      if (made && from.every((values, at) => values === made!.from[at])) return made.value;
      const byColumn = new Map(sources.map(([column], at) => [column, from[at]!]));
      made = { from, value: {
        get: key => { const [row, column] = split(key); return byColumn.get(column)?.get(row); },
        missing: key => { const [row, column] = split(key); return byColumn.get(column)?.missing(row) ?? 'Not followed'; },
      } };
      return made.value;
    },
    subscribe: changed => {
      const stops = sources.map(([, live]) => live.subscribe(changed));
      return () => { for (const stop of stops) stop(); };
    },
  };
}

export const noValues = (reason: string): LiveValues<never> => ({ get: () => undefined, missing: () => reason });

export interface LiveClock {
  getSnapshot(): { position: number };
  subscribeFrame(listener: () => void): () => void;
  subscribe?(listener: () => void): () => void;
}

export function liveAt<T>(clock: LiveClock, at: (position: number) => number, read: (hour: number) => T,
  same: (a: T, b: T) => boolean = Object.is): Live<T> & { hour(): number } {
  let made: { hour: number; value: T } | null = null;
  const hour = () => at(clock.getSnapshot().position);
  return {
    hour,
    read: () => {
      const now = hour();
      if (!made || made.hour !== now) {
        const value = read(now);
        made = { hour: now, value: made && same(made.value, value) ? made.value : value };
      }
      return made.value;
    },
    subscribe: changed => {
      const stops = [clock.subscribeFrame(changed), ...clock.subscribe ? [clock.subscribe(changed)] : []];
      return () => { for (const stop of stops) stop(); };
    },
  };
}

export function mapLive<T, U>(source: Live<T>, make: (value: T) => U): Live<U> {
  let made: { from: T; value: U } | null = null;
  return {
    read: () => {
      const from = source.read();
      if (!made || !Object.is(made.from, from)) made = { from, value: make(from) };
      return made.value;
    },
    subscribe: source.subscribe,
  };
}

export const sameBy = <T>(key: (item: T) => string) => (a: readonly T[], b: readonly T[]) =>
  a.length === b.length && a.every((item, i) => key(item) === key(b[i]));

export class LiveCell<T> implements Live<T> {
  private value: T;
  private readonly listeners = new Set<() => void>();

  constructor(value: T) { this.value = value; }

  read = (): T => this.value;

  set(value: T): void {
    if (Object.is(value, this.value)) return;
    this.value = value;
    for (const changed of [...this.listeners]) changed();
  }

  subscribe = (changed: () => void): (() => void) => {
    this.listeners.add(changed);
    return () => { this.listeners.delete(changed); };
  };
}

export interface LiveTexts { text(key: string): string; subscribe(changed: () => void): () => void }

export interface TextClock { subscribeFrame(listener: () => void): () => void; subscribe(listener: () => void): () => void }

export interface TextSource { version(): readonly unknown[]; subscribe(changed: () => void): () => void }

const NO_VERSION: readonly unknown[] = [];

export function liveTexts(clock: TextClock, hour: () => number, read: (hour: number) => ReadonlyMap<string, string>, source?: TextSource): LiveTexts {
  let made: { hour: number; version: readonly unknown[]; texts: ReadonlyMap<string, string> } | null = null;
  return {
    text: key => {
      const at = hour(), version = source?.version() ?? NO_VERSION;
      if (made?.hour !== at || made.version.length !== version.length || made.version.some((held, index) => !Object.is(held, version[index])))
        made = { hour: at, version, texts: read(at) };
      return made.texts.get(key) ?? '';
    },
    subscribe: changed => {
      const stops = [clock.subscribeFrame(changed), clock.subscribe(changed), ...(source ? [source.subscribe(changed)] : [])];
      return () => { for (const stop of stops) stop(); };
    },
  };
}

export const separated = (text: string | null | undefined) => text ? ` · ${text}` : '';
