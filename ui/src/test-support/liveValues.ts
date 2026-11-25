import type { LiveValues } from '../charts/live';

export function valuesOf<V>(values: ReadonlyMap<string, V>, reason: string | ((key: string) => string)): LiveValues<V> {
  return { get: key => values.get(key), missing: typeof reason === 'string' ? () => reason : reason };
}
