import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { ResearchSlotHistory } from './types';
import { researchRateAt } from './researchData.ts';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import type { ResearchEntry, ResearchSlotState } from '../../generated/dashboard';

export interface ResearchPeriod {
  firstHour: number;
  endHour: number;
  state: ResearchSlotState | null;
  work: bigint | null;
  firstRate: bigint | null;
  lastRate: bigint | null;
}

const effectKey = (state: ResearchSlotState) => JSON.stringify([
  state.details?.modifiers, state.details?.bonuses, state.appliedDaysRaw, state.aheadReductionYearsRaw,
]);

export function researchWork(state: ResearchSlotState, from: number, to: number): bigint | null {
  let work = 0n;
  let steps = 0;
  for (let hour = from; hour < to;) {
    if (++steps > 100_000) return null;
    const nextDay = hour + 24 - ((hour - HOUR_EPOCH) % 24 + 24) % 24;
    const rate = researchRateAt(state, hour);
    if (!rate) return null;
    const end = rate.penaltyRaw === '0' ? to : Math.min(to, nextDay);
    work += BigInt(rate.rateRaw) * BigInt(end - hour);
    hour = end;
  }
  return work;
}

export function researchSplit(state: ResearchSlotState, from: number, to: number): { work: bigint; penalty: bigint; bonus: bigint; floored: boolean } | null {
  let work = 0n, penalty = 0n, bonus = 0n, floored = false;
  let steps = 0;
  for (let hour = from; hour < to;) {
    if (++steps > 100_000) return null;
    const nextDay = hour + 24 - ((hour - HOUR_EPOCH) % 24 + 24) % 24;
    const rate = researchRateAt(state, hour);
    if (!rate) return null;
    const end = rate.penaltyRaw === '0' ? to : Math.min(to, nextDay);
    floored ||= rate.floored;
    const earned = BigInt(rate.rateRaw) * BigInt(end - hour);
    const stretched = earned * (100_000n + BigInt(rate.penaltyRaw)) / 100_000n;
    work += earned;
    penalty += stretched - earned;
    bonus += stretched - 100_000n * BigInt(end - hour);
    hour = end;
  }
  return { work, penalty, bonus, floored };
}

export const MODIFIER_KINDS: Record<string, string> = {
  country: 'research speed', category: 'category bonus', designTeam: 'design team', experience: 'experience',
  license: 'licence', scientist: 'scientist', applied: 'research bonus', sharing: 'tech sharing',
};
const pretty = (token: string) => { const text = token.replace(/_/g, ' ').trim(); return text.charAt(0).toUpperCase() + text.slice(1); };

export interface ResearchModifierPart { key: string; kind: string; label: string; raw: bigint }

export function researchModifierParts(state: ResearchSlotState): ResearchModifierPart[] | null {
  const modifiers = state.details?.modifiers;
  if (!modifiers) return null;
  const parts: ResearchModifierPart[] = [];
  for (const component of modifiers.components) {
    const kind = MODIFIER_KINDS[component.kind] ?? component.kind;
    const sources = component.kind === 'scientist' ? []
      : component.kind === 'applied' ? (state.details?.bonuses ?? []).filter(b => b.speedBonusRaw && b.speedBonusRaw !== '0')
        .map(b => ({ id: `${b.id}`, label: contentTitle(b.name, b.token, 'Research bonus'), raw: BigInt(b.speedBonusRaw!) }))
        : component.sources.filter(s => s.raw !== '0')
          .map((s, i) => ({ id: s.token ?? `${i}`, label: contentTitle(s.name, s.token, pretty(kind)), raw: BigInt(s.raw) }));
    for (const source of sources) parts.push({ key: `${component.kind}:${source.id}`, kind, label: source.label, raw: source.raw });
    const rest = BigInt(component.raw) - sources.reduce((sum, s) => sum + s.raw, 0n);
    if (rest !== 0n) parts.push({ key: `${component.kind}:`, kind, label: sources.length ? `Other ${kind}` : pretty(kind), raw: rest });
  }
  return parts;
}

export function researchBreakdown(slot: ResearchSlotHistory, entry: ResearchEntry) {
  const firstHour = entry.firstObservedHour;
  const endHour = entry.outcome !== 'unknown' && entry.endedByHour !== null
    ? entry.endedByHour : entry.lastObservedHour;
  const periods: ResearchPeriod[] = [];
  let cursor = firstHour;
  let lastKey: string | null = null;
  for (const state of slot.states) {
    if (state.entryFirstObservedHour !== firstHour || state.token !== entry.token || state.databaseIndex !== entry.databaseIndex) continue;
    const from = Math.max(firstHour, state.firstHour);
    const to = Math.min(endHour, state.lastHour + 1);
    if (to <= from) continue;
    if (from > cursor) {
      periods.push({ firstHour: cursor, endHour: from, state: null, work: null, firstRate: null, lastRate: null });
      lastKey = null;
    }
    const key = effectKey(state);
    const work = researchWork(state, from, to);
    const firstRate = researchRateAt(state, from);
    const lastRate = researchRateAt(state, to - 1);
    const previous = periods.at(-1);
    if (previous && previous.endHour === from && key === lastKey) {
      previous.endHour = to;
      previous.work = previous.work !== null && work !== null ? previous.work + work : null;
      previous.lastRate = lastRate ? BigInt(lastRate.rateRaw) : null;
    } else {
      periods.push({ firstHour: from, endHour: to, state, work,
        firstRate: firstRate ? BigInt(firstRate.rateRaw) : null,
        lastRate: lastRate ? BigInt(lastRate.rateRaw) : null });
    }
    cursor = to;
    lastKey = key;
  }
  if (cursor < endHour) periods.push({ firstHour: cursor, endHour, state: null, work: null, firstRate: null, lastRate: null });

  const samples = (slot.progress ?? []).filter(p => p.entryFirstObservedHour === firstHour
    && p.firstHour <= endHour && p.lastHour >= firstHour);
  const first = samples[0], last = samples.at(-1);
  const base = first?.baseDaysRaw != null ? BigInt(first.baseDaysRaw) : null;
  const observed = last && BigInt(last.progressRaw) >= 0n ? BigInt(last.progressRaw) : null;
  const initial = first && BigInt(first.progressRaw) >= 0n ? BigInt(first.progressRaw) : null;
  const costChanged = samples.some(p => p.baseDaysRaw !== first?.baseDaysRaw);
  const continuousProgress = samples.every((p, i) => i === 0 ||
    (samples[i - 1].lastHour + 1 === p.firstHour && BigInt(p.progressRaw) >= BigInt(samples[i - 1].progressRaw)));
  const compareEnd = last ? Math.min(last.lastHour, endHour) : firstHour;
  let calculated = 0n;
  let covered = 0;
  let complete = true;
  for (const period of periods) {
    const from = Math.max(period.firstHour, first?.firstHour ?? firstHour);
    const to = Math.min(period.endHour, compareEnd);
    if (to <= from) continue;
    const work = period.state ? researchWork(period.state, from, to) : null;
    if (work === null) complete = false;
    else { calculated += work; covered += to - from; }
  }
  const reconciled = first && last && initial !== null && observed !== null && continuousProgress && complete
    && covered === compareEnd - first.firstHour;
  return {
    periods, base, observed, initial, costChanged, endHour,
    latestBase: last?.baseDaysRaw != null ? BigInt(last.baseDaysRaw) : null,
    comparison: reconciled && observed !== null && initial !== null ? { observed: observed - initial, calculated: calculated / 24n,
      difference: observed - initial - calculated / 24n } : null,
    complete: entry.startedAfterHour !== null && entry.outcome === 'completed' && !costChanged
      && first?.firstHour === firstHour,
  };
}

export function researchNumber(raw: bigint, precision = 1): string {
  const scale = 10n ** BigInt(precision);
  const magnitude = raw < 0n ? -raw : raw;
  const rounded = (magnitude * scale + 50_000n) / 100_000n;
  const whole = rounded / scale;
  const fraction = (rounded % scale).toString().padStart(precision, '0').replace(/0+$/, '');
  return `${raw < 0n && rounded !== 0n ? '−' : ''}${whole}${fraction ? `.${fraction}` : ''}`;
}
