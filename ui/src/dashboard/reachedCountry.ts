import type { CountryIdentityHistory, CountryIdentityRun } from '../generated/dashboard';
import type { OptionField } from './options';
import { runAt } from '../charts/series.ts';
import { countryDrillTagAt, countryIdentityKey } from './countryIdentity.ts';
import { datedCountrySpans, type DatedCountrySelection } from './datedCountry.ts';

/** The hidden option keeping a single-country reached-hour widget's choice bound to the
 * one country identity it named when it was made ([`DatedCountrySelection`]). */
export const REACHED_COUNTRY_OPTION: OptionField = { key: 'country', section: 'options', label: 'Chosen country', kind: 'content-choice',
  automatic: 'Selected country', default: '', titleChoice: false, visibleWhen: () => false };

/** One identity a tag has named, for a choice the tag alone does not settle. */
export interface CountryHolder { origin: number; firstHour: number; lastHour: number }

/** A new choice: the tag and the hour it is made at. Which identity that hour names is
 * read from the identity history whenever the choice is resolved, so the binding never
 * depends on how much of the history had loaded when it was made. */
export function choiceAt(campaign: string, branch: string, branchFirst: number, tag: string, hour: number): DatedCountrySelection {
  return { campaign, branch, tag, originOffset: Math.max(0, hour - branchFirst) };
}

/** What a choice's hour settles: the identity holding its tag then (`held`); when the
 * tag named no country then, the only identity it ever named (`bound`, a new origin),
 * the identities to choose from, or none. */
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

/** What a widget shows for a stored choice: the country at `hour` when the choice's hour
 * names one in the history so far; otherwise, only once the history is `complete`, the
 * choice's rebinding to its tag's only identity, the identities to choose from, or that
 * it names none. A provisional history never rebinds a choice: it keeps its hour. */
export function resolveChoice(histories: readonly CountryIdentityHistory[], complete: boolean, selection: DatedCountrySelection,
  branchFirst: number, branchLast: number, hour: number)
  : { country: ReachedCountry; holders: CountryHolder[] | null; rebind: DatedCountrySelection | null } {
  const waiting = { country: { chosen: selection.tag, tag: null, absent: false, reason: null }, holders: null, rebind: null };
  const settled = settleChoice(histories, selection, branchFirst);
  if (settled.kind === 'held') return { ...waiting, country: reachedCountryAt(histories, selection, branchFirst, branchLast, hour) };
  if (!complete) return waiting;
  if (settled.kind === 'bound') return { ...waiting, rebind: settled.selection };
  if (settled.kind === 'choose') return { ...waiting, holders: settled.holders };
  return { ...waiting, country: { ...waiting.country, reason: `${selection.tag} is not in this campaign.` } };
}

/** The country a single-country reached-hour widget asks about at an hour. `chosen` is
 * the widget's literal choice and `tag` the tag its bound identity holds at the hour. */
export interface ReachedCountry { chosen: string | null; tag: string | null; absent: boolean; reason: string | null }

/** The tag a bound identity holds at `hour`, following only that identity's dated
 * aliases: a renamed country keeps the choice, and a later holder of the tag never
 * takes it. No alias at the hour is absence. */
export function reachedCountryAt(histories: readonly CountryIdentityHistory[], selection: DatedCountrySelection,
  branchFirst: number, branchLast: number, hour: number): ReachedCountry {
  const none = { chosen: selection.tag, tag: null, absent: false, reason: null };
  const spans = datedCountrySpans(histories, selection, branchFirst, branchLast);
  if (!spans) return { ...none, reason: 'The chosen country’s recorded identity is unavailable.' };
  const tag = countryDrillTagAt(spans, hour);
  return tag ? { ...none, tag } : { ...none, absent: true };
}
