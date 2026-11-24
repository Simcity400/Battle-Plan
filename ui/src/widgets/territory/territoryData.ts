import type { BreakdownLeafItem as SunburstItem, BreakdownLeafKey as SunburstKey } from '../../families/breakdown/tree';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { TerritoryShareReply } from '../../generated/dashboard';

export const WORLD_TOTAL = liveKey('world', 'total');
export const entryCell = (slot: number) => liveKey(String(slot), 'value');

const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);

export function validateShareReply(value: unknown): TerritoryShareReply {
  if (!object(value) || typeof value.measure !== 'string' || !Array.isArray(value.entries)
    || !value.entries.every(entry => object(entry) && typeof entry.tag === 'string' && Number.isSafeInteger(entry.slot))
    || typeof value.unnamed !== 'boolean' || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) throw new Error('The territory share reply is malformed.');
  return value as unknown as TerritoryShareReply;
}

export const shareStructure = (reply: TerritoryShareReply, count: number) =>
  JSON.stringify([reply.entries.slice(0, Math.max(0, count)), reply.unnamed, reply.missing, reply.warnings]);

export function shareItems(reply: TerritoryShareReply, count: number, values: LiveValues<number>, look: (tag: string) => SunburstKey, rest: SunburstKey):
  { items: SunburstItem[]; unavailableReason: string | null } {
  const total = values.get(WORLD_TOTAL);
  if (reply.missing !== null || total === undefined) return { items: [], unavailableReason: reply.missing ?? values.missing(WORLD_TOTAL) ?? 'Territory is not recorded at this hour.' };
  const shown = reply.entries.slice(0, Math.max(0, count)).map(entry => ({ entry, value: values.get(entryCell(entry.slot)) ?? null }));
  const items: SunburstItem[] = shown.map(({ entry, value }) => ({ path: [look(entry.tag)], value, exact: value === null ? null : BigInt(Math.trunc(value)),
    ...(value === null ? { note: values.missing(entryCell(entry.slot)) } : {}) }));
  if (shown.some(item => item.value === null)) return { items, unavailableReason: null };
  const remainder = total - shown.reduce((sum, item) => sum + item.value!, 0);
  if (remainder > 0) items.push({ path: [{ ...rest, last: true }], value: remainder, exact: BigInt(Math.trunc(remainder)) });
  return { items, unavailableReason: null };
}
