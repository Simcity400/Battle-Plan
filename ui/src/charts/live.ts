export interface Live<T> { read(): T; subscribe(changed: () => void): () => void }

export interface LiveValues<V> { get(key: string): V | undefined; missing(key: string): string; raw?(key: string): bigint | undefined }

export const liveKey = (...parts: readonly string[]) => parts.join('\u0000');

export const noValues = (reason: string): LiveValues<never> => ({ get: () => undefined, missing: () => reason });

