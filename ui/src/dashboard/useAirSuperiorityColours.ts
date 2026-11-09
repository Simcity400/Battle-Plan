import { useMemo, useRef } from 'react';
import { COUNTRY_COLOURS_OVER_BUDGET } from '../generated/dashboard.ts';
import type { AirSide, CountryColourReply, CountryColourRun, CountryStyle } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { countryLineColours } from './countryLineColours.ts';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import { MAX_NAME_HOURS } from './countryFacts.ts';
import type { BatchFactSpec, FactPlan, NameAt } from './countryFacts';
import { STYLES } from './useCountriesAt.ts';
import { lookHour } from './countryLooks.ts';
import { airSegmentAt, sideAt } from './airSuperiority.ts';
import type { AirMoment, AirTimeline } from './airSuperiority';
import type { ChartTokens } from '../charts/tokens';
import { colourOf, coloursIn } from './countryTimeline.ts';
import type { HeldLookup } from './countryTimeline';

type Reply = AirMoment | AirTimeline;
/** A country's recorded map colour at an hour. */
export type CountryColour = Pick<CountryStyle, 'color'>;

/** The last of a tag's runs (ascending) starting at or before `hour`. */
function runAt(runs: readonly CountryColourRun[] | undefined, hour: number): CountryColourRun | undefined {
  if (!runs) return undefined;
  let low = 0, high = runs.length;
  while (low < high) { const middle = (low + high) >> 1; if (runs[middle].hour <= hour) low = middle + 1; else high = middle; }
  return runs[low - 1];
}

/** Each planned hour's colours from `country-colours` runs. An hour the reply could
 * not read is absent; a country with no recorded colour is absent from its hour, so
 * it is read with nothing. Hours in one run share one colour object. */
export function colourHours(reply: CountryColourReply, plan: FactPlan): Map<number, Map<string, CountryColour>> {
  const unread = new Set(reply.unread.map(item => item.hour));
  const runs = new Map<string, CountryColourRun[]>();
  for (const run of reply.runs) {
    const list = runs.get(run.tag);
    if (list) list.push(run); else runs.set(run.tag, [run]);
  }
  const shared = new Map<CountryColourRun, CountryColour>();
  const hours = new Map<number, Map<string, CountryColour>>();
  for (const [hour, tags] of plan) {
    if (unread.has(hour)) continue;
    const found = hours.get(hour) ?? new Map<string, CountryColour>();
    hours.set(hour, found);
    for (const tag of tags) {
      const run = runAt(runs.get(tag), hour);
      if (!run?.color) continue;
      let colour = shared.get(run);
      if (!colour) shared.set(run, colour = { color: run.color });
      found.set(tag, colour);
    }
  }
  return hours;
}

/** Colours the country timeline holds, as `colourHours` gives them, one object per recorded colour: a country with
 * no recorded colour is left out. */
const heldColours: HeldLookup<CountryColour> = (timeline, hour, tags) => {
  const packed = coloursIn(timeline, hour, tags);
  if (!packed) return undefined;
  const found = new Map<string, CountryColour>();
  for (const [tag, colour] of packed) if (colour >= 0) found.set(tag, colourOf(timeline, colour));
  return found;
};

/** Colours for every day of a timeline the country timeline does not hold, in one
 * `country-colours` request, not one request per day. A single hour (a day Follow adds)
 * is read on the interactive path; a reply too large for one request is read again in halves. */
export const COLOURS: BatchFactSpec<CountryColour> = {
  product: 'country-colours', batch: true, single: STYLES, oversize: COUNTRY_COLOURS_OVER_BUDGET,
  params: (branchId, plan) => ({ branchId, hours: [...new Set(plan.map(([hour]) => hour))], tags: [...new Set(plan.flatMap(([, tags]) => tags))].sort() }),
  // The request reads every country it names at every hour it names.
  size: plan => new Set(plan.map(([hour]) => hour)).size * new Set(plan.flatMap(([, tags]) => tags)).size,
  extract: (data, plan) => colourHours(data as CountryColourReply, plan),
  held: heldColours,
};
/** The country reads colouring one displayed hour, by `nameKey`: each side's and the viewpoint's. */
export interface AirColourSources { sides: Map<string, string>; viewpoint?: string }
/** The colours shown at one hour, by side key and for the viewpoint. */
export interface AirColourGroup { sides: ReadonlyMap<string, string>; viewpoint?: string }
export type AirColourGroups = ReadonlyMap<number, { inputs: readonly unknown[]; group: AirColourGroup }>;
const NO_COLOUR_SOURCES: ReadonlyMap<number, AirColourSources> = new Map();
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
  if (data?.view === 'timeline') for (const at of new Set(data.days.map((at, index) => data.sampleHours?.[index] ?? at))) { if (at != null) add(data, at); }
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
  style: (key: string) => unknown, surface: string, fixed: boolean, previous: Pick<AirColourGroups, 'get'> = new Map()): AirColourGroups {
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
    const entries = new Map<string, CountryColour>();
    for (const read of viewpoint === undefined ? sides.values() : [...sides.values(), viewpoint]) {
      const found = style(read) as CountryColour | null | undefined;
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

function lookup(groups: AirColourGroups, muted: string, overlay?: AirColourGroups): AirColourLookup {
  const at = (hour: number) => (overlay?.get(hour) ?? groups.get(hour))?.group;
  return {
    color: (side, hour) => at(hour)?.sides.get(side.key) ?? muted,
    viewpoint: hour => at(hour)?.viewpoint ?? muted,
    at,
  };
}

/** Two colourings: `reply` for the reply's own hours, and `latest`, which also
 * colours the current day. `reply` changes only when one of its hours does, so
 * playing through the current day never redraws the history.
 *
 * A timeline's days are read in one request (`COLOURS`). A moment's played hour is
 * read alone, as is the current day, kept apart from the history for the same
 * reason. An hour both need is read once. */
export function useAirCountryColours(client: DashboardClient, track: BoundTrack | undefined, data: Reply | undefined, day: AirMoment | undefined,
  hour: number, dayHour: number | null, viewpointTag: string | undefined, active: boolean, tokens: ChartTokens) {
  const plan = useMemo(() => airColourPlan(data, undefined, hour, null, track?.branch, viewpointTag), [data, hour, track?.branch, viewpointTag]);
  const dayPlan = useMemo(() => airColourPlan(undefined, day, hour, dayHour, track?.branch, viewpointTag), [day, hour, dayHour, track?.branch, viewpointTag]);
  const dayWanted = useMemo(() => dayPlan.wanted.filter(item => !plan.keys.has(nameKey(item.hour, item.tag))), [plan, dayPlan]);
  const styles = useCountryFactsRead<CountryColour>(client, track, plan.wanted, active, data?.view === 'moment' ? STYLES : COLOURS, plan.maxHours);
  const dayStyles = useCountryFactsRead(client, track, dayWanted, active, STYLES, dayPlan.maxHours);
  const historyHeld = useRef<AirColourGroups>(new Map());
  const dayHeld = useRef<AirColourGroups>(new Map());
  const history = useMemo(() => {
    const groups = airColourGroups(plan.sources, NO_COLOUR_SOURCES, key => styles.get(key), tokens.surface, tokens.countryColours === 'fixed', historyHeld.current);
    historyHeld.current = groups;
    return groups;
  }, [plan, styles, tokens.surface, tokens.countryColours]);
  const overlay = useMemo(() => {
    const shared = new Map([...dayPlan.sources.keys()].flatMap(hour => {
      const source = plan.sources.get(hour);
      return source ? [[hour, source] as const] : [];
    }));
    const groups = airColourGroups(shared, dayPlan.sources, key => styles.get(key) ?? dayStyles.get(key), tokens.surface, tokens.countryColours === 'fixed',
      { get: hour => dayHeld.current.get(hour) ?? history.get(hour) });
    dayHeld.current = groups;
    return groups;
  }, [plan, dayPlan, history, styles, dayStyles, tokens.surface, tokens.countryColours]);
  const held = useRef<{ hours: number[]; shown: (AirColourGroup | undefined)[]; muted: string; reply: AirColourLookup } | null>(null);
  return useMemo(() => {
    const latest = lookup(history, tokens.muted, overlay);
    const hours = [...plan.sources.keys()], shown = hours.map(at => latest.at(at));
    const kept = held.current;
    const reply = kept && kept.muted === tokens.muted && kept.hours.length === hours.length && hours.every((at, index) => at === kept.hours[index] && shown[index] === kept.shown[index])
      ? kept.reply : lookup(history, tokens.muted, hours.some(at => overlay.has(at)) ? overlay : undefined);
    held.current = { hours, shown, muted: tokens.muted, reply };
    return { reply, latest };
  }, [history, overlay, plan, tokens.muted]);
}
