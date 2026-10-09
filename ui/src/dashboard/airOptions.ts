import { campaignToken } from './chosen.ts';
import { contentKey, setToken } from './options.ts';
import type { OptionField } from './options';
import type { DashboardClient } from './client';

/** Shared semantic settings; the two widgets keep independent saved choices. */
export const AIR_OPTIONS: readonly OptionField[] = [
  { key: 'mode', section: 'options', label: 'Comparison', kind: 'choice', default: 'sides', choices: [
    { value: 'sides', label: 'Per side' }, { value: 'viewpoint', label: 'Exact viewpoint' }] },
  { key: 'viewpoint', section: 'options', label: 'Viewpoint', kind: 'content-choice', automatic: 'Dashboard’s current country', default: '',
    hint: 'A country, or a side using its leader’s view', visibleWhen: o => o.mode === 'viewpoint' },
  { key: 'sides', section: 'options', label: 'Sides compared', kind: 'content-set', automatic: 'All recorded sides', default: '',
    visibleWhen: o => o.mode === 'sides' },
  { key: 'theatres', section: 'options', label: 'Air theatres', kind: 'content-set', automatic: 'All recorded theatres', default: '' },
];

/** Runtime faction, coalition and theatre identities belong to their recording. */
export const airChoiceToken = (campaign: string, key: string) => setToken(campaignToken(campaign, key));
export function airChoiceKey(campaign: string, value: string): string | null {
  const key = contentKey(value), prefix = campaignToken(campaign, '');
  return key.startsWith(prefix) ? key.slice(prefix.length) : null;
}

/** Follow requests change once per calendar day, not once per played hour. */
export function timelineRequestLast(hour: number, dayHours: number): number {
  return Math.floor(hour / dayHours) * dayHours;
}

/** Held replies are scoped to the same client, recording, comparison and content. */
export function heldAirReply<K extends 'timeline' | 'day'>(held: { client: DashboardClient; identity: string; timeline?: import('./airSuperiority').AirTimeline; day?: import('./airSuperiority').AirMoment } | undefined,
  client: DashboardClient, identity: string, kind: K): NonNullable<typeof held>[K] | undefined {
  return held?.client === client && held.identity === identity ? held[kind] : undefined;
}
