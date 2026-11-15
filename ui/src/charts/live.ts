/** How a shared chart paints the playhead's values: its structure (rows, order, names, art, the keys) comes by props
 * and renders; its values come from a `Live` the chart reads in its own frame and repaints from, with no React commit. */

/** A value read as a chart paints: `read()` gives the same object until what it holds moves, and `subscribe` calls
 * `changed` whenever it may have (a frame of playback, a seek, an answer landing). */
export interface Live<T> { read(): T; subscribe(changed: () => void): () => void }

/** Values keyed by the keys a chart's structure names. A key the source does not hold reads undefined: the chart draws
 * it as missing, `missing(key)` saying why, never as zero and never as the value it had before. */
export interface LiveValues<V> { get(key: string): V | undefined; missing(key: string): string }

/** One key from the parts a structure names it by: a row and a column, a source and a target. */
export const liveKey = (...parts: readonly string[]) => parts.join('\u0000');

/** Values a map holds; a key it does not hold is missing for `reason`. */
export function valuesOf<V>(values: ReadonlyMap<string, V>, reason: string | ((key: string) => string)): LiveValues<V> {
  return { get: key => values.get(key), missing: typeof reason === 'string' ? () => reason : reason };
}

/** A table's values from one live source per column, each keyed by row: what `Table`'s `live` reads, keyed
 * `liveKey(row, column)`. A column without a source is missing. */
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

/** What a chart without a source of its own reads: nothing, every key missing for `reason`. */
export const noValues = (reason: string): LiveValues<never> => ({ get: () => undefined, missing: () => reason });

/** The parts of the dashboard clock a live value reads its hour from and wakes with. */
export interface LiveClock {
  getSnapshot(): { position: number };
  subscribeFrame(listener: () => void): () => void;
  /** Seeks and pauses, which move the hour without a frame of playback. */
  subscribe?(listener: () => void): () => void;
}

/** What `read` makes at the hour `at` gives for the clock's position: made again only when that hour moves, and kept
 * (the same object) while `same` holds. It wakes its chart at each frame of playback and at a seek. */
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

/** What a chart reads, made from `source`'s value and made again only when that value moves. */
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

/** Whether two lists hold the same entries by `key`, in order. */
export const sameBy = <T>(key: (item: T) => string) => (a: readonly T[], b: readonly T[]) =>
  a.length === b.length && a.every((item, i) => key(item) === key(b[i]));

/** A live value written from outside, such as a pushed panel's `paint`: its readers are told when it changes. */
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

/** Texts at the moment a frame paints: each key's text now, and the signal that they may have moved. */
export interface LiveTexts { text(key: string): string; subscribe(changed: () => void): () => void }

/** A clock's hour as a live text reads it. */
export interface TextClock { subscribeFrame(listener: () => void): () => void; subscribe(listener: () => void): () => void }

/** What texts read beside the hour (a followed answer, a reply landing): `version()` differs once it changed, entry by
 * entry, and `subscribe` tells when it may have, paused or playing. */
export interface TextSource { version(): readonly unknown[]; subscribe(changed: () => void): () => void }

const NO_VERSION: readonly unknown[] = [];

/** Texts read at `hour()`, made again only when it or `source`'s version moves: what `LiveText`s paint each frame. */
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

/** A text that follows another on its line after a separator, the separator with it: none when it is empty. */
export const separated = (text: string | null | undefined) => text ? ` · ${text}` : '';
