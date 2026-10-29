// The requests some widgets issue, built from the widget and its track alone, so the
// readiness table's test checks exactly what the widgets send.
import * as requests from './batch5Requests.ts';
import type { ProductRequest } from './client';
import { selectedTags } from './countrySelection.ts';
import type { BoundTrack, HourWindow, WidgetSpec } from './model';
import { contentKey, resolveOptions } from './options.ts';
import { historyWindow, POLITICS_HISTORY } from './politicsHistory.ts';
import { OPTIONS as AIR_STATISTICS } from '../widgets/air-statistics/definition.ts';
import { DEFAULT_CHANNEL } from '../widgets/air-statistics/airStatisticsData.ts';
import { OPTIONS as INTEL } from '../widgets/intel-network-coverage/definition.ts';
import { OPTIONS as OCCUPATION } from '../widgets/occupation-reserve-manpower/definition.ts';
import { OPTIONS as WEATHER } from '../widgets/weather-by-region/definition.ts';
import type { WeatherShow } from '../widgets/weather-by-region/weatherData';
import { weatherWindow } from '../widgets/weather-by-region/weatherData.ts';

type Track = BoundTrack | undefined;
const request = <P>(track: BoundTrack, product: string, params: P): ProductRequest<P> =>
  ({ sourceId: track.source.id, revision: track.source.revision, product, params });
const firstTag = (widget: WidgetSpec, track: Track) => selectedTags(widget.countries, track)[0] ?? null;
const chosenTags = (widget: WidgetSpec) => widget.countries.kind === 'tags' ? widget.countries.tags : undefined;

/** The shown hours inside the branch. */
export const windowExtent = (track: BoundTrack, window: HourWindow): [number, number] =>
  [Math.max(track.branch.firstHour, window.firstHour), Math.min(track.branch.lastHour, window.lastHour)];

export function airMissionsRequest(widget: WidgetSpec, track: Track) {
  const tag = firstTag(widget, track);
  return track && tag ? request(track, 'air-missions', requests.airMissionsHistory(track.branch.id, tag)) : null;
}

export function airStatisticsChannel(widget: WidgetSpec): string {
  const options = resolveOptions(AIR_STATISTICS, widget.options);
  return options.channel ? contentKey(String(options.channel)) : DEFAULT_CHANNEL;
}
export function airStatisticsRequest(widget: WidgetSpec, track: Track) {
  const tag = firstTag(widget, track);
  const params = track && tag ? requests.airStatistics(track.branch.id, tag, airStatisticsChannel(widget)) : null;
  return track && params ? request(track, 'air-statistics', params) : null;
}

export const minefieldsRequest = (widget: WidgetSpec, track: Track) =>
  track ? request(track, 'minefields', requests.minefieldsHistory(track.branch.id, chosenTags(widget))) : null;

export function navalSupremacyRequest(widget: WidgetSpec, track: Track, extent: readonly [number, number] | null) {
  const tag = firstTag(widget, track);
  return track && tag && extent ? request(track, 'naval-supremacy', requests.navalSupremacy(track.branch.id, tag, ...requests.blockSpan(track.branch, extent))) : null;
}

/** The weather view and the modifier it shows. */
export function weatherShow(widget: WidgetSpec): { show: WeatherShow; chosen: string | null } {
  const options = resolveOptions(WEATHER, widget.options);
  const show: WeatherShow = options.show === 'modifiers' || options.show === 'temperature' ? options.show : 'conditions';
  return { show, chosen: options.modifier ? contentKey(String(options.modifier)) : null };
}
/** The shown days, folded into at most `pixels` columns where they do not fit one a column. */
export function weatherRequest(widget: WidgetSpec, track: Track, extent: readonly [number, number] | null, pixels: number) {
  const { show, chosen } = weatherShow(widget);
  const window = track && extent ? weatherWindow(track.branch, extent, pixels) : null;
  return track && window ? request(track, 'regional-weather', { ...requests.weatherHistory(track.branch.id, window.first, window.last,
    show === 'modifiers' ? 'modifier' : show, chosen), width: window.width }) : null;
}
/** One day of the weather history, as the whole history reads it. */
export function weatherDayRequest(widget: WidgetSpec, track: BoundTrack, day: number, dayHours: number) {
  const { show, chosen } = weatherShow(widget);
  return request(track, 'regional-weather', requests.weatherHistory(track.branch.id, Math.max(track.branch.firstHour, day * dayHours),
    Math.min(track.branch.lastHour, (day + 1) * dayHours - 1), show === 'modifiers' ? 'modifier' : show, chosen));
}

export function doctrineMasteryRequest(widget: WidgetSpec, track: Track) {
  const tag = firstTag(widget, track);
  return track && tag ? request(track, 'doctrine-mastery', requests.doctrineMastery(track.branch.id, tag)) : null;
}

/** `tags` are the holders' tags to ask for: by default the chosen literal tags. */
export const occupationReserveRequest = (widget: WidgetSpec, track: Track, tags: readonly string[] | undefined = chosenTags(widget)) =>
  track && resolveOptions(OCCUPATION, widget.options).show !== 'breakdown'
    ? request(track, 'occupation-reserve-manpower', requests.occupationRace(track.branch.id, tags)) : null;

/** How many of the most active countries the matrix shows: a Top countries group's count, else Countries shown. */
export function intelShown(widget: WidgetSpec): number {
  const countries = widget.countries;
  return countries.kind === 'group' && countries.group.type === 'top' ? countries.group.count : Number(resolveOptions(INTEL, widget.options).mostCountries) || 12;
}
/** Whether the matrix shows one country (the dashboard's, one picked, or the top one) with every
 * country it has a network in or that has one in it; a group's members stay among themselves. */
export function intelTouching(widget: WidgetSpec, tags: readonly string[] | undefined): boolean {
  const countries = widget.countries;
  return countries.kind === 'group' ? countries.group.type === 'top' && countries.group.count === 1 : tags?.length === 1;
}
/** `chosen`: the countries chosen, or a group's roots; none asks for the most active. `held`: the
 * tags asked for in their place (every tag they held), while `chosen` alone decides `intelTouching`. */
export function intelCoverageRequest(widget: WidgetSpec, track: Track, chosen: readonly string[] | undefined = chosenTags(widget),
  held: readonly string[] | undefined = chosen) {
  const options = resolveOptions(INTEL, widget.options);
  return track && options.show !== 'pair'
    ? request(track, 'intel-network-coverage', requests.intelMatrix(track.branch.id, intelShown(widget), held, intelTouching(widget, chosen))) : null;
}

/** The yard product each yard widget reads. */
export const YARD_PRODUCTS = { 'dockyards-and-repairs': 'naval-repairs', refits: 'naval-refits' } as const;
export function yardsRequest(widget: WidgetSpec, track: Track) {
  const tag = firstTag(widget, track);
  const product = YARD_PRODUCTS[widget.kind as keyof typeof YARD_PRODUCTS];
  return track && tag && product ? request(track, product, requests.navalYards(track.branch.id, tag)) : null;
}

/** The hour a moment widget shows. */
export const shownHour = (track: Track, window: HourWindow | null | undefined) => Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
/** One week of exiles holding `hour`; playback looks them up locally within it. */
export function exilesRequest(track: Track, hour: number) {
  return track ? request(track, POLITICS_HISTORY, { branchId: track.branch.id, mode: 'exiles', ...historyWindow(track.branch.firstHour, hour) }) : null;
}
