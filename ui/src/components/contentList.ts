import { storableToken, UNSTORABLE } from '../dashboard/options.ts';
import { visibleRange } from './countryList.ts';

export interface ContentChoice { value: string; label: string; icon?: string | null; hint?: string }
export interface ContentRow extends ContentChoice { search: string; unavailable?: string }

export function contentChoices(choices: readonly ContentChoice[], chosen: readonly string[]): ContentRow[] {
  const rows = new Map<string, ContentChoice>();
  for (const choice of choices) if (!rows.has(choice.value)) rows.set(choice.value, choice);
  for (const value of chosen) if (!rows.has(value)) rows.set(value, { value, label: 'Saved choice unavailable', hint: `Key: ${value}\nNot in this campaign.` });
  return [...rows.values()].map(choice => ({ ...choice, search: `${choice.label} ${choice.value}`.toLowerCase(),
    unavailable: storableToken(choice.value) ? undefined : UNSTORABLE }));
}

export function filterContent(choices: ContentRow[], query: string): ContentRow[] {
  const needle = query.trim().toLowerCase();
  return needle ? choices.filter(choice => choice.search.includes(needle)) : choices;
}

export function contentWindow(choices: ContentRow[], scrollTop: number, height: number) {
  const range = visibleRange(choices.length, scrollTop, height);
  return { ...range, rows: choices.slice(range.start, range.end) };
}
