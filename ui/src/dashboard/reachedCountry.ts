import type { CountryIdentityHistory, CountryIdentityRun } from '../generated/dashboard';
import type { OptionField } from './options';
import { runAt } from '../charts/series.ts';
import { countryDrillTagAt, countryIdentityKey } from './countryIdentity.ts';
import { datedCountrySpans, type DatedCountrySelection } from './datedCountry.ts';

export const REACHED_COUNTRY_OPTION: OptionField = { key: 'country', storeAs: 'country', section: 'options', label: 'Chosen country', kind: 'content-choice',
  automatic: 'Selected country', default: '', titleChoice: false, visibleWhen: () => false };

export interface CountryHolder { origin: number; firstHour: number; lastHour: number }

export function choiceAt(campaign: string, branch: string, branchFirst: number, tag: string, hour: number): DatedCountrySelection {
  return { campaign, branch, tag, originOffset: Math.max(0, hour - branchFirst) };
}

export type CountryBinding =
  | { kind: 'held' }
  | { kind: 'bound'; selection: DatedCountrySelection }
  | { kind: 'choose'; holders: CountryHolder[] }
  | { kind: 'none' };

export function settleChoice(histories: readonly CountryIdentityHistory[], selection: DatedCountrySelection, branchFirst: number): CountryBinding {
  const runs = histories.find(history => history.tag === selection.tag)?.identities ?? [];
  if (runAt(runs, branchFirst + selection.originOffset)) return { kind: 'held' };
  const holders = new Map<string, CountryHolder>();
  for (const run of runs.filter((run: CountryIdentityRun) => run.lastHour >= branchFirst)) {
    const key = countryIdentityKey(run), held = holders.get(key);
    if (held) held.lastHour = run.lastHour;
    else holders.set(key, { origin: Math.max(run.firstHour, branchFirst), firstHour: run.firstHour, lastHour: run.lastHour });
  }
  const [only] = holders.values();
  if (holders.size === 1) return { kind: 'bound', selection: { ...selection, originOffset: only.origin - branchFirst } };
  return holders.size ? { kind: 'choose', holders: [...holders.values()] } : { kind: 'none' };
}

export function resolveChoice(histories: readonly CountryIdentityHistory[], complete: boolean, selection: DatedCountrySelection,
  branchFirst: number, branchLast: number, hour: number)
  : { country: ReachedCountry; holders: CountryHolder[] | null; rebind: DatedCountrySelection | null } {
  const waiting = { country: { chosen: selection.tag, tag: null, absent: false, reason: null }, holders: null, rebind: null };
  if (hour < branchFirst || hour > branchLast) return { ...waiting, country: outsideCountry(selection.tag) };
  const settled = settleChoice(histories, selection, branchFirst);
  if (settled.kind === 'held') return { ...waiting, country: reachedCountryAt(histories, selection, branchFirst, branchLast, hour) };
  if (!complete) return waiting;
  if (settled.kind === 'bound') return { ...waiting, rebind: settled.selection };
  if (settled.kind === 'choose') return { ...waiting, holders: settled.holders };
  return { ...waiting, country: { ...waiting.country, reason: `${selection.tag} is not in this campaign.` } };
}

export interface ReachedCountry { chosen: string | null; tag: string | null; absent: boolean; reason: string | null }

export const outsideCountry = (chosen: string): ReachedCountry => ({ chosen, tag: null, absent: false, reason: 'This time is outside the recording.' });

export function reachedCountryAt(histories: readonly CountryIdentityHistory[], selection: DatedCountrySelection,
  branchFirst: number, branchLast: number, hour: number): ReachedCountry {
  const none = { chosen: selection.tag, tag: null, absent: false, reason: null };
  const spans = datedCountrySpans(histories, selection, branchFirst, branchLast);
  if (!spans) return { ...none, reason: 'The chosen country’s recorded identity is unavailable.' };
  const tag = countryDrillTagAt(spans, hour);
  return tag ? { ...none, tag } : { ...none, absent: true };
}
