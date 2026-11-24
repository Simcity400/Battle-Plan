import { contentKey, contentSet, setLimit, setToken, storableToken, toggleContentSet, UNSTORABLE } from './options.ts';


export interface ChoiceSource {
  lines: (token: string) => number;
  name: (token: string) => string;
  noun: string;
}

export const campaignToken = (campaignKey: string, token: string) => `${campaignKey}:${token}`;

export const subjectToken = (campaignKey: string, handle: string) => setToken(campaignToken(campaignKey, handle));

export function subjectHandle(campaignKey: string, value: string): string | null {
  const key = contentKey(value), prefix = campaignToken(campaignKey, '');
  return key.startsWith(prefix) ? key.slice(prefix.length) : null;
}

export type ChipAction = { value: string } | { reason: string };

export function largestToken(items: readonly { token: string; size: bigint; recorded: boolean }[]): string | null {
  let best: { token: string; size: bigint } | null = null;
  for (const item of items) if (item.recorded && (!best || item.size > best.size)) best = item;
  return best?.token ?? items[0]?.token ?? null;
}

export function chosenShown(source: Pick<ChoiceSource, 'lines'>, chosen: readonly string[], automatic: string | readonly string[] | null): string[] {
  const known = chosen.filter(token => source.lines(token) > 0);
  if (known.length) return known;
  return automatic == null ? [] : typeof automatic === 'string' ? [automatic] : [...automatic];
}

export function toggleChosen(source: ChoiceSource, stored: readonly string[], shown: readonly string[], token: string, max: number): ChipAction {
  const storedHere = stored.some(item => source.lines(item) > 0);
  const base = storedHere ? stored : [...stored, ...shown.filter(item => storableToken(item) && !stored.includes(item))];
  if (!base.includes(token) && !storableToken(token)) return { reason: UNSTORABLE };
  const next = toggleContentSet(base.join(' '), token, max);
  if (next == null) return { reason: `At most ${setLimit(max)} can be chosen` };
  const lines = contentSet(next).reduce((sum, item) => sum + source.lines(item), 0);
  if (!lines) return { reason: `One ${source.noun} always stays chosen` };
  if (!base.includes(token) && lines > max) {
    return { reason: `At most ${max} lines can be drawn; ${source.name(token)} adds ${source.lines(token)}` };
  }
  return { value: next };
}
