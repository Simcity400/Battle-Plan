import { useMemo, useRef } from 'react';
import type { AirSide, CountryStyle } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { countryLineColours } from './countryLineColours.ts';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import { MAX_NAME_HOURS } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { STYLES } from './useCountriesAt.ts';
import { lookHour } from './countryLooks.ts';
import { airSegmentAt, sideAt } from './airSuperiority.ts';
import type { AirMoment, AirTimeline } from './airSuperiority';
import type { ChartTokens } from '../charts/tokens';

type Reply = AirMoment | AirTimeline;
/** The country reads colouring one displayed hour, by `nameKey`: each side's and the viewpoint's. */
export interface AirColourSources { sides: Map<string, string>; viewpoint?: string }
/** The colours shown at one hour, by side key and for the viewpoint. */
export interface AirColourGroup { sides: ReadonlyMap<string, string>; viewpoint?: string }
export type AirColourGroups = ReadonlyMap<number, { inputs: readonly unknown[]; group: AirColourGroup }>;
/** Colours by side and hour; `at` gives an hour's group, so hours colouring alike compare cheaply. */
export interface AirColourLookup { color(side: AirSide, at: number): string; viewpoint(at: number): string; at(hour: number): AirColourGroup | undefined }

/** Each displayed hour protects its own country reads; anchor tags may be reused. */
export function airColourPlan(data: Reply | undefined, day: AirMoment | undefined, hour: number, dayHour: number | null,
  branch: BoundTrack['branch'] | undefined, viewpointTag?: string) {
  const sources = new Map<number, AirColourSources>();
  const wanted = new Map<string, NameAt>();
  const add = (reply: Reply, at: number) => {
    const read = (tag: string, date: number) => {
      if (!branch || airSegmentAt(reply.segments, tag, date) == null || !lookHour(branch, date, undefined, true)) return null;
      const key = nameKey(date, tag);
      wanted.set(key, { tag, hour: date });
      return key;
    };
    const group = sources.get(at) ?? { sides: new Map<string, string>() };
    sources.set(at, group);
    for (const base of reply.sides) {
      const side = sideAt(base, at);
      if (side.colourFaction) continue;
      const anchor = reply.anchorReads[side.key];
      const tag = side.tag ?? side.leader;
      const key = anchor ? read(anchor[0], anchor[1]) : side.faction || side.key.startsWith('w:') || tag == null ? null : read(tag, at);
      if (key) group.sides.set(side.key, key);
    }
    if (viewpointTag) {
      const key = read(viewpointTag, at);
      if (key) group.viewpoint = key;
    }
  };
  if (data?.view === 'timeline') for (const at of data.days) { if (at != null) add(data, at); }
  if (data) add(data, hour);
  if (day && dayHour != null) add(day, dayHour);
  return { sources, wanted: [...wanted.values()], keys: new Set(wanted.keys()) as ReadonlySet<string>,
    maxHours: Math.max(MAX_NAME_HOURS, new Set([...wanted.values()].map(read => read.hour)).size) };
}

const sameGroup = (a: AirColourGroup, b: AirColourGroup) => a.viewpoint === b.viewpoint && a.sides.size === b.sides.size
  && [...a.sides].every(([key, colour]) => b.sides.get(key) === colour);

/** Colours at every planned hour, as the country charts colour them (`countryLineColours`);
 * the day's sources replace the history's at an hour both plan. An hour whose reads are
 * unchanged keeps its group from `previous`, and one colouring alike the hour before
 * shares that group, so only hours whose reads arrived are coloured again. */
export function airColourGroups(history: ReadonlyMap<number, AirColourSources>, day: ReadonlyMap<number, AirColourSources>,
  style: (key: string) => unknown, surface: string, fixed: boolean, previous: AirColourGroups = new Map()): AirColourGroups {
  const groups = new Map<number, { inputs: readonly unknown[]; group: AirColourGroup }>();
  let before: AirColourGroup | undefined;
  for (const at of new Set([...history.keys(), ...day.keys()])) {
    const shown = history.get(at), current = day.get(at);
    const sides = current ? new Map([...shown?.sides ?? [], ...current.sides]) : shown!.sides;
    const viewpoint = current?.viewpoint ?? shown?.viewpoint;
    const inputs = [surface, fixed, viewpoint, viewpoint === undefined ? undefined : style(viewpoint), ...[...sides].flatMap(([key, read]) => [key, read, style(read)])];
    const held = previous.get(at);
    if (held && held.inputs.length === inputs.length && held.inputs.every((input, index) => Object.is(input, inputs[index]))) {
      groups.set(at, held);
      before = held.group;
      continue;
    }
    const entries = new Map<string, CountryStyle>();
    for (const read of viewpoint === undefined ? sides.values() : [...sides.values(), viewpoint]) {
      const found = style(read) as CountryStyle | null | undefined;
      if (found && typeof found === 'object' && found.color != null) entries.set(read, found);
    }
    const colours = countryLineColours([...entries.keys()].sort(), entries, surface, fixed);
    let group: AirColourGroup = { sides: new Map([...sides].flatMap(([key, read]) => colours.has(read) ? [[key, colours.get(read)!] as const] : [])),
      viewpoint: viewpoint === undefined ? undefined : colours.get(viewpoint) };
    if (held && sameGroup(held.group, group)) group = held.group;
    else if (before && sameGroup(before, group)) group = before;
    groups.set(at, { inputs, group });
    before = group;
  }
  return groups;
}

function lookup(groups: AirColourGroups, muted: string): AirColourLookup {
  return {
    color: (side, at) => groups.get(at)?.group.sides.get(side.key) ?? muted,
    viewpoint: at => groups.get(at)?.group.viewpoint ?? muted,
    at: hour => groups.get(hour)?.group,
  };
}

/** `reply` colours the reply's own hours and changes only when one of them does, so a
 * played hour moving the current day's read never projects the history again; `latest`
 * also colours the current day. That day is read apart from the history for the same
 * reason; an hour both plan is read once. */
export function useAirCountryColours(client: DashboardClient, track: BoundTrack | undefined, data: Reply | undefined, day: AirMoment | undefined,
  hour: number, dayHour: number | null, viewpointTag: string | undefined, active: boolean, tokens: ChartTokens) {
  const plan = useMemo(() => airColourPlan(data, undefined, hour, null, track?.branch, viewpointTag), [data, hour, track?.branch, viewpointTag]);
  const dayPlan = useMemo(() => airColourPlan(undefined, day, hour, dayHour, track?.branch, viewpointTag), [day, hour, dayHour, track?.branch, viewpointTag]);
  const dayWanted = useMemo(() => dayPlan.wanted.filter(item => !plan.keys.has(nameKey(item.hour, item.tag))), [plan, dayPlan]);
  const styles = useCountryFactsRead(client, track, plan.wanted, active, STYLES, plan.maxHours);
  const dayStyles = useCountryFactsRead(client, track, dayWanted, active, STYLES, dayPlan.maxHours);
  const held = useRef<{ groups: AirColourGroups; hours: number[]; shown: (AirColourGroup | undefined)[]; muted: string; reply: AirColourLookup } | null>(null);
  const groups = useMemo(() => airColourGroups(plan.sources, dayPlan.sources, key => styles.get(key) ?? dayStyles.get(key),
    tokens.surface, tokens.countryColours === 'fixed', held.current?.groups), [plan, dayPlan, styles, dayStyles, tokens.surface, tokens.countryColours]);
  return useMemo(() => {
    const latest = lookup(groups, tokens.muted);
    const hours = [...plan.sources.keys()], shown = hours.map(at => groups.get(at)?.group);
    const kept = held.current;
    const reply = kept && kept.muted === tokens.muted && kept.hours.length === hours.length && hours.every((at, index) => at === kept.hours[index] && shown[index] === kept.shown[index])
      ? kept.reply : latest;
    held.current = { groups, hours, shown, muted: tokens.muted, reply };
    return { reply, latest };
  }, [groups, plan, tokens.muted]);
}
