import { useMemo } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { CountryGroupReply, CountryIdentityRun, ExperienceReply, HourSpan } from '../../generated/dashboard';
import { countryIdentityIndex, countryIdentityKey } from '../../dashboard/countryIdentity';
import { clipToSpans } from '../../charts/series';
import { usePreparedMeasure } from '../country-measure/preparedMeasure';
import { scaleSummary } from '../country-measure/preparedMeasureData';
import { BRANCHES, validateExperienceReply } from './experienceData';
import type { ExperienceMeasure } from './experienceData';

export const experienceKey = (tag: string, identity?: Pick<CountryIdentityRun, 'slot' | 'countryId'>) => JSON.stringify([tag, identity?.slot ?? null, identity?.countryId ?? null]);
/** Shared native identity history selects holders; only reached XP reads and the visible summaries load. */
export function usePreparedExperience(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  tags: readonly string[], groups: CountryGroupReply | undefined, grouped: boolean, membership: ReadonlyMap<string, readonly HourSpan[]> | null,
  measure: ExperienceMeasure, head: number, hover: number | null, active: boolean) {
  const selection = useMemo(() => {
    if (!groups) return { roster: [...tags], rows: [], within: new Map<string, readonly HourSpan[]>() };
    const histories = groups.countryIdentities, selected = countryIdentityIndex(histories).selected(tags);
    const roster = grouped ? [...tags] : [...tags, ...histories.filter(history => !tags.includes(history.tag)
      && history.identities.some(identity => selected.has(countryIdentityKey(identity)))).map(history => history.tag)];
    const rows = roster.map(tag => ({ tag, identities: histories.find(history => history.tag === tag)?.identities.filter(identity => selected.has(countryIdentityKey(identity))) ?? [] }));
    const within = new Map<string, readonly HourSpan[]>();
    for (const row of rows) for (const identity of row.identities) {
      const key = experienceKey(row.tag, identity), runs = [...(within.get(key) ?? []), identity];
      within.set(key, membership ? clipToSpans(runs, membership.get(row.tag) ?? []) : runs);
    }
    for (const tag of roster) if (!rows.find(row => row.tag === tag)?.identities.length) within.set(experienceKey(tag), []);
    return { roster, rows, within };
  }, [groups, tags, grouped, membership]);
  const keys = [...selection.within.keys()];
  const useBranch = (branch: string) => usePreparedMeasure(client, groups ? source : undefined, branchId, `${branch}-${measure}`,
    keys, head, hover, active, false, 'country-experience', ['experienceRead'], selection.within);
  const army = useBranch('army'), navy = useBranch('navy'), air = useBranch('air'), channels = [army, navy, air];
  const states = army.states;
  const data = useMemo<ExperienceReply | undefined>(() => states?.scale && states.dayHours ? validateExperienceReply({ branchId, scale: states.scale,
    dayHours: states.dayHours, warnings: [...new Set(channels.flatMap(channel => channel.states?.warnings ?? []))], series: selection.rows.map(row => ({ ...row,
      reads: [...new Map(states.states.filter(state => state.experience && JSON.parse(state.tag)[0] === row.tag)
        .map(state => [state.experience!.firstHour, state.experience!])).values()].sort((a,b) => a.firstHour-b.firstHour) })) }) : undefined,
    [states, navy.states, air.states, selection]);
  const summaries = useMemo(() => channels.map(channel => new Map([...channel.summaryByTag].map(([key, row]) => [key,
    data ? scaleSummary(row.summary, 1 / data.scale) : row.summary]))), [army.summaryByTag, navy.summaryByTag, air.summaryByTag, data?.scale]);
  return { ...army, enabled: !!client.preparedQuery, data, keys, within: selection.within,
    error: channels.find(channel => channel.error)?.error, retry: () => channels.forEach(channel => channel.retry()),
    segmentAt: (tag: string, hour: number) => states?.states.find(state => JSON.parse(state.tag)[0] === tag
      && state.experience && state.experience.firstHour <= hour && hour <= state.experience.lastHour)?.segmentFirstHour ?? null,
    preparing: channels.some(channel => channel.preparing), summary: (tag: string, identity: CountryIdentityRun | undefined, branch: typeof BRANCHES[number]) => summaries[BRANCHES.indexOf(branch)].get(experienceKey(tag, identity)),
    recorded: (tag: string) => states?.states.some(state => JSON.parse(state.tag)[0] === tag && state.recorded) ?? false,
    gainKnown: channels.some(channel => channel.states?.states.some(state => state.available)),
    onSummaryView: (view: Parameters<typeof army.onSummaryView>[0]) => channels.forEach(channel => channel.onSummaryView(view)) };
}
