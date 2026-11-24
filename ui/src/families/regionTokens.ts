import { contentKey, contentSet, setToken } from '../dashboard/options.ts';
import type { OptionValue } from '../dashboard/options.ts';

export function regionToken(id: number): string {
  return setToken(`r:${id}`);
}

export function regionIds(value: OptionValue | undefined): number[] {
  const out: number[] = [];
  for (const token of contentSet(value ?? '')) {
    const key = contentKey(token);
    const id = Number(key.startsWith('r:') ? key.slice(2) : key);
    if (Number.isSafeInteger(id) && id >= 0 && !out.includes(id)) out.push(id);
  }
  return out;
}
