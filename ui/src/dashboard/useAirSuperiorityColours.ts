import { useMemo } from 'react';
import type { AirSide, CountryStyle } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { countryLineColours } from './countryLineColours.ts';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import { MAX_NAME_HOURS } from './countryFacts.ts';
import { STYLES } from './useCountriesAt.ts';
import { lookHour } from './countryLooks.ts';
import { airSegmentAt, sideAt } from './airSuperiority.ts';
import type { AirMoment, AirTimeline } from './airSuperiority';
import type { ChartTokens } from '../charts/tokens';

type Reply = AirMoment | AirTimeline;

/** Each displayed hour protects its own country reads; anchor tags may be reused. */
export function airColourPlan(data: Reply | undefined, day: AirMoment | undefined, hour: number, dayHour: number | null,
  branch: BoundTrack['branch'] | undefined, viewpointTag?: string) {
  const sources = new Map<string, { tag: string; hour: number }>();
  const wanted = new Map<string, { tag: string; hour: number }>();
  const add = (reply: Reply, at: number) => {
    const read = (tag: string, date: number) => {
      if (!branch || airSegmentAt(reply.segments, tag, date) == null || !lookHour(branch, date, undefined, true)) return null;
      const sample = { tag, hour: date };
      wanted.set(nameKey(date, tag), sample);
      return sample;
    };
    for (const base of reply.sides) {
      const side = sideAt(base, at);
      if (side.colourFaction) continue;
      const anchor = reply.anchorReads[side.key];
      const tag = side.tag ?? side.leader;
      const sample = anchor ? read(anchor[0], anchor[1]) : side.faction || side.key.startsWith('w:') || tag == null ? null : read(tag, at);
      if (sample) sources.set(JSON.stringify([side.key, at]), sample);
    }
    if (viewpointTag) {
      const sample = read(viewpointTag, at);
      if (sample) sources.set(JSON.stringify(['viewpoint', at]), sample);
    }
  };
  if (data?.view === 'timeline') for (const at of data.days) { if (at != null) add(data, at); }
  if (data) add(data, hour);
  if (day && dayHour != null) add(day, dayHour);
  return { sources, wanted: [...wanted.values()], maxHours: Math.max(MAX_NAME_HOURS, new Set([...wanted.values()].map(read => read.hour)).size) };
}

export function useAirCountryColours(client: DashboardClient, track: BoundTrack | undefined, data: Reply | undefined, day: AirMoment | undefined,
  hour: number, dayHour: number | null, viewpointTag: string | undefined, active: boolean, tokens: ChartTokens) {
  const plan = useMemo(() => airColourPlan(data, day, hour, dayHour, track?.branch, viewpointTag), [data, day, hour, dayHour, track?.branch, viewpointTag]);
  const styles = useCountryFactsRead(client, track, plan.wanted, active, STYLES, plan.maxHours);
  const colours = useMemo(() => {
    const byHour = new Map<number, Map<string, CountryStyle>>();
    for (const [key, sample] of plan.sources) {
      const style = styles.get(nameKey(sample.hour, sample.tag));
      if (!style || typeof style !== 'object' || style.color == null) continue;
      const [, at] = JSON.parse(key) as [string, number];
      const entries = byHour.get(at) ?? new Map<string, CountryStyle>();
      entries.set(nameKey(sample.hour, sample.tag), style);
      byHour.set(at, entries);
    }
    const dated = new Map<number, Map<string, string>>();
    for (const [at, entries] of byHour) dated.set(at, countryLineColours([...entries.keys()].sort(), entries, tokens.surface, tokens.countryColours === 'fixed'));
    const result = new Map<string, string>();
    for (const [key, sample] of plan.sources) {
      const [, at] = JSON.parse(key) as [string, number];
      const colour = dated.get(at)?.get(nameKey(sample.hour, sample.tag));
      if (colour) result.set(key, colour);
    }
    return result;
  }, [styles, plan, tokens.surface, tokens.countryColours]);
  return useMemo(() => ({
    color: (side: AirSide, at: number) => colours.get(JSON.stringify([side.key, at])) ?? tokens.muted,
    viewpoint: (at: number) => colours.get(JSON.stringify(['viewpoint', at])) ?? tokens.muted,
  }), [colours, tokens.muted]);
}
