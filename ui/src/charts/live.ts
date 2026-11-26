
export interface Live<T> { read(): T; subscribe(changed: () => void): () => void }

export interface LiveValues<V> { get(key: string): V | undefined; missing(key: string): string; raw?(key: string): bigint | undefined }

export const liveKey = (...parts: readonly string[]) => parts.join('\u0000');

export const noValues = (reason: string): LiveValues<never> => ({ get: () => undefined, missing: () => reason });

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

