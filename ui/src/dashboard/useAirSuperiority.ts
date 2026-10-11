import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import type { DashboardClient } from './client';
import { useChartTokens, useViewportHeight } from '../charts/useChartTokens';
import { fitTooltip } from '../charts/tooltip';
import type { AirSide } from '../generated/dashboard';
import type { OptionChoices, WidgetProps, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { resolveWindow } from './model';
import { contentKey, contentSet, setToken } from './options';
import type { OptionValue } from './options';
import { usePlaybackHour, usePlaybackSpeed } from './playback';
import { useProduct } from './products';
import { defaultCountryTag, selectedTags } from './countrySelection';
import { useProductWindows } from './productWindows';
import { useFactionColours } from './FactionColours';
import { useCountryLooks } from './countryLooks';
import { useAirCountryColours } from './useAirSuperiorityColours';
import type { AirColourLookup } from './useAirSuperiorityColours';
import { FOLLOW_MS, useCountriesAt } from './useCountriesAt';
import { useThrottled } from './useThrottled';
import { useHoverLabels } from './useHoverLabels';
import { useContentRevision } from './contentRevision.ts';
import { airCountryTags, airSegmentAt, airSideName, airSummaryTooltip, airTooltip, airTooltipTags, inSpans, lastRecordedAt, momentReadAt, momentReadHours, momentViews, momentWindows, sideAt, sideOrder, timelineViews, validateAirReply } from './airSuperiority';
import { airChoiceKey, airChoiceToken, heldAirReply, timelineRequestLast } from './airOptions';
import { easeTau } from '../charts/easing';
import type { AirChoices, AirLooks, AirMoment, AirTimeline, AirView } from './airSuperiority';

const NO_WARNINGS: string[] = [];
const NO_SIDES: AirSide[] = [];
const NO_COUNTRIES: string[] = [];
const timelineReply = (value: unknown) => { const reply = validateAirReply(value); if (reply.view !== 'timeline') throw new Error('Expected air timeline'); return reply; };
const momentReply = (value: unknown) => { const reply = validateAirReply(value); if (reply.view !== 'moment') throw new Error('Expected air moment'); return reply; };
const choicesReply = (value: unknown) => { const reply = validateAirReply(value); if (reply.view !== 'choices') throw new Error('Expected air choices'); return reply; };

export function useAirChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  usePlaybackHour(runtime.clock, true);
  const hour = Math.floor(track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? track.branch.lastHour : 0);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority', params: { branchId: track.branch.id, view: 'choices' } } : null;
  const product = useProduct<AirChoices>(runtime.client, request, true, choicesReply);
  const countries = product.data?.countries ?? NO_COUNTRIES;
  const tags = useMemo(() => [...new Set([...countries, ...(product.data?.sides.flatMap(side => side.tag ? [side.tag] : []) ?? [])])], [countries, product.data]);
  const names = useCountriesAt(runtime.client, track, [], hour, true, tags);
  return useMemo(() => {
    const sides = [...new Map(product.data?.sides.map(side => [side.key, { value: airChoiceToken(track?.source.metadata.campaignKey ?? '', side.key), label: airSideName(sideAt(side, hour), tag => names.country(tag).name) }]) ?? []).values()];
    return { viewpoint: [...sides, ...countries.map(tag => ({ value: setToken(`country:${tag}`), label: names.country(tag).name }))], sides,
      regions: product.data?.regions.map(region => ({ value: setToken(`r:${region.id}`), label: region.name })) ?? [],
      theatres: product.data?.theatres.map(theatre => ({ value: airChoiceToken(track?.source.metadata.campaignKey ?? '', theatre.id), label: theatre.name })) ?? [] };
  }, [product.data, countries, names, hour, track?.source.metadata.campaignKey]);
}

/** Moment playback reads a held window locally; prefetch at 75%, seek directly. */
export function useAir(props: WidgetProps, options: Readonly<Record<string, OptionValue>>, moment: boolean) {
  const { widget, runtime, active, onDiagnostics } = props;
  const contentRevision = useContentRevision(runtime.client);
  usePlaybackHour(runtime.clock, active);
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const playhead = window ? moment || widget.time.mode === 'follow' || widget.time.mode === 'pin' ? window.lastHour : Math.floor(runtime.clock.getHour()) + (track?.axisOffset ?? 0) : null;
  const hour = Math.floor(playhead ?? track?.branch.lastHour ?? 0);
  const firstHour = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const lastHour = !moment && widget.time.mode === 'campaign' ? track?.branch.lastHour ?? 1 : widget.time.mode === 'range' && window ? window.lastHour : playhead ?? track?.branch.lastHour ?? 1;
  const beforeBranch = lastHour < firstHour;
  const campaign = track?.source.metadata.campaignKey ?? '';
  const currentCountry = selectedTags(widget.countries, track)[0] ?? defaultCountryTag(track);
  const mode = useMemo(() => ({ mode: options.mode === 'viewpoint' ? 'viewpoint' as const : 'sides' as const,
    viewpoint: options.mode === 'viewpoint' ? options.viewpoint ? contentKey(String(options.viewpoint)).startsWith('country:') ? contentKey(String(options.viewpoint)) : airChoiceKey(campaign, String(options.viewpoint)) ?? '' : currentCountry ? `country:${currentCountry}` : '' : '',
    sides: contentSet(options.sides).flatMap(value => airChoiceKey(campaign, value) ?? []), regions: contentSet(options.regions).map(value => Number(contentKey(value).slice(2))),
    theatres: contentSet(options.theatres).flatMap(value => airChoiceKey(campaign, value) ?? []) }), [options.mode, options.viewpoint, options.sides, options.regions, options.theatres, currentCountry, campaign]);
  const params = useMemo(() => ({ mode: mode.mode, ...(mode.viewpoint ? { viewpoint: mode.viewpoint.startsWith('country:') ? mode.viewpoint.slice(8) : mode.viewpoint,
    ...(mode.viewpoint.startsWith('country:') ? { viewpointCountry: true } : {}) } : {}),
    ...(mode.sides.length ? { sides: mode.sides } : {}), ...(mode.regions.length ? { regions: mode.regions } : {}),
    ...(mode.theatres.length ? { theatres: mode.theatres } : {}) }), [mode]);
  const needsViewpoint = mode.mode === 'viewpoint' && !mode.viewpoint;
  const follow = !moment && widget.time.mode === 'follow';
  const heldIdentity = useMemo(() => JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, contentRevision, firstHour, params, options.regionCount, follow]),
    [track?.source.id, track?.source.revision, track?.branch.id, contentRevision, firstHour, params, options.regionCount, follow]);
  const [held, setHeld] = useState<{ client: DashboardClient; identity: string; timeline?: AirTimeline; day?: AirMoment }>();
  const previousTimeline = heldAirReply(held, runtime.client, heldIdentity, 'timeline');
  // The first reply supplies cadence; subsequent requests change only by day.
  const timelineLast = follow ? previousTimeline ? Math.max(firstHour, timelineRequestLast(lastHour, previousTimeline.dayHours)) : firstHour : lastHour;
  const timelineRequest = track && window && !moment && !beforeBranch && !needsViewpoint ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority',
    params: { branchId: track.branch.id, view: 'timeline', firstHour, lastHour: timelineLast, ...params, ...(mode.regions.length ? {} : { top: Number(options.regionCount) }) } } : null;
  const timeline = useProduct<AirTimeline>(runtime.client, timelineRequest, active, timelineReply);
  const timelineData = timeline.data ?? (follow ? previousTimeline : undefined);
  const windowChoicesRequest = track && moment ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority', params: { branchId: track.branch.id, view: 'choices', windowsOnly: true } } : null;
  const windowChoices = useProduct<AirChoices>(runtime.client, windowChoicesRequest, active, choicesReply);
  const recordedWindows = windowChoices.data?.windows;
  const wanted = moment && recordedWindows ? momentWindows(recordedWindows, hour) : [];
  const shownWindow = recordedWindows?.[wanted[0]];
  const momentFirst = shownWindow?.firstHour ?? firstHour;
  const momentLast = shownWindow?.lastHour ?? lastHour;
  const choicesRequest = track && moment && shownWindow && options.order === 'name' ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority', params: {
    branchId: track.branch.id, view: 'choices', firstHour: momentFirst, lastHour: momentLast,
    ...(mode.regions.length ? { regions: mode.regions } : {}), ...(mode.theatres.length ? { theatres: mode.theatres } : {}),
  } } : null;
  const choices = useProduct<AirChoices>(runtime.client, choicesRequest, active, choicesReply);
  const identity = track && window && moment && !needsViewpoint ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision, params]) : null;
  const windows = useProductWindows<AirMoment>(runtime.client, identity, index => {
    const window = recordedWindows![index];
    return { sourceId: track!.source.id, revision: track!.source.revision, product: 'air-superiority', params: {
      branchId: track!.branch.id, view: 'moment', firstHour: window.firstHour, lastHour: window.lastHour, ...params,
    } };
  }, wanted, active, momentReply);
  const dayFirst = timelineData ? Math.max(firstHour, timelineRequestLast(lastHour, timelineData.dayHours)) : firstHour;
  const dayRequest = follow && track && timelineData && !beforeBranch && !needsViewpoint ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority', params: {
      branchId: track.branch.id, view: 'moment', firstHour: dayFirst,
      lastHour: Math.min(timelineRequestLast(lastHour, timelineData.dayHours) + timelineData.dayHours - 1, track.branch.lastHour), ...params, regions: timelineData.rows.map(row => row.region),
    } } : null;
  const day = useProduct<AirMoment>(runtime.client, dayRequest, active, momentReply);
  const catalogueRequest = track && !moment && !needsViewpoint ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-superiority',
    params: { branchId: track.branch.id, view: 'choices' } } : null;
  const catalogue = useProduct<AirChoices>(runtime.client, catalogueRequest, active, choicesReply);
  const labelCatalogue = useMemo(() => catalogue.data?.regions.map(region => region.name), [catalogue.data]);
  const dayData = day.data ?? (follow ? heldAirReply(held, runtime.client, heldIdentity, 'day') : undefined);
  useEffect(() => {
    if (!follow) { setHeld(undefined); return; }
    setHeld(previous => ({ client: runtime.client, identity: heldIdentity,
      timeline: timeline.data ?? heldAirReply(previous, runtime.client, heldIdentity, 'timeline'), day: day.data ?? heldAirReply(previous, runtime.client, heldIdentity, 'day') }));
  }, [follow, runtime.client, heldIdentity, timeline.data, day.data]);
  const dayReads = useMemo(() => dayData ? momentReadHours(dayData) : [], [dayData]);
  const dayRecorded = dayData ? lastRecordedAt(dayData.recorded, hour) : null;
  const dayRead = dayRecorded == null ? null : momentReadAt(dayReads, dayRecorded);
  const data = moment ? windows.data.get(wanted[0]) : timelineData;
  const timelineRead = useMemo(() => {
    if (data?.view !== 'timeline') return null;
    for (let index = data.days.length - 1; index >= 0; index--) if (data.days[index] != null) return data.days[index];
    return data.firstHour;
  }, [data]);
  const dataHour = data?.view === 'moment' ? hour : timelineRead ?? hour;
  const readHour = data?.view === 'moment' ? inSpans(data.recorded, hour) ? hour : null : dataHour;
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const factions = useFactionColours(runtime.client, track?.source);
  const setFactionColour = useRef(factions.setColour);
  useEffect(() => { setFactionColour.current = factions.setColour; }, [factions.setColour]);
  const changeFactionColour = useCallback((id: string, hex: string) => { setFactionColour.current(id, hex, 'campaign'); }, []);
  const sides = data?.sides ?? NO_SIDES;
  const viewpointSide = useMemo(() => {
    for (let index = sides.length - 1; index >= 0; index--) if (sides[index].key === mode.viewpoint) return sideAt(sides[index], dataHour);
    return undefined;
  }, [sides, mode.viewpoint, dataHour]);
  const viewpointTag = mode.viewpoint.startsWith('country:') ? mode.viewpoint.slice(8) : viewpointSide?.leader ?? undefined;
  const tags = useMemo(() => [...new Set([...(data ? airCountryTags(data) : []), ...(dayData ? airCountryTags(dayData) : []), ...(viewpointTag ? [viewpointTag] : [])])], [data, dayData, viewpointTag]);
  const lookHour = data?.view === 'moment' ? hour : follow ? dayRecorded ?? dataHour : dataHour;
  const segments = data?.segments;
  const daySegments = dayData?.segments;
  const segmentAt = useCallback((tag: string, hour: number) => {
    if (dayData && dayData.firstHour <= hour && hour <= dayData.lastHour) return daySegments ? airSegmentAt(daySegments, tag, hour) : null;
    return segments ? airSegmentAt(segments, tag, hour) : null;
  }, [segments, daySegments, dayData]);
  const countryColours = useAirCountryColours(runtime.client, track, data, dayData, dataHour, dayRecorded, viewpointTag, active, tokens);
  const nameHour = useThrottled(Math.floor(lookHour), FOLLOW_MS);
  const labelLooks = useCountryLooks(runtime.client, track, tags, nameHour, active, segmentAt, undefined, true);
  const names = useMemo(() => ({ country: (tag: string) => ({ name: labelLooks.name(tag, lookHour), flag: labelLooks.flag(tag, lookHour) }) }), [labelLooks, lookHour]);
  const { ask: askHover, labelAt: hoverLabels, revision: hoverRevision } = useHoverLabels(runtime.client, track, active, names.country, undefined, segmentAt);
  // Rebuilt only when a country or its name changes, not each played hour.
  const shownNames = tags.map(tag => names.country(tag).name);
  const namesHeld = useRef<{ tags: readonly string[]; names: readonly string[]; byTag: ReadonlyMap<string, string> } | null>(null);
  const named = namesHeld.current;
  const countryNames = named && named.tags.length === tags.length && tags.every((tag, index) => tag === named.tags[index] && shownNames[index] === named.names[index])
    ? named.byTag : (namesHeld.current = { tags, names: shownNames, byTag: new Map(tags.map((tag, index) => [tag, shownNames[index]])) }).byTag;
  const factionNames = factions.displayNames;
  const sideName = useCallback((side: AirSide) => airSideName(side, tag => countryNames.get(tag) ?? 'Country name not recorded', factionNames), [factionNames, countryNames]);
  const looksWith = useCallback((colours: AirColourLookup): AirLooks => {
    const color = (side: AirSide, at = dataHour) => side.colourFaction ? factions.colours.get(side.colourFaction) ?? tokens.muted : colours.color(side, at);
    return { name: sideName, color, lookAt: colours.at,
      viewpoint: { name: viewpointSide ? sideName(viewpointSide) : viewpointTag ? countryNames.get(viewpointTag) ?? 'Country name unavailable' : 'No country selected', tag: viewpointSide ? undefined : viewpointTag,
        color: viewpointSide ? color(viewpointSide) : colours.viewpoint(dataHour) },
      viewpointAt: at => ({ name: viewpointTag ? countryNames.get(viewpointTag) ?? 'Country name unavailable' : 'No country selected', tag: viewpointTag, color: colours.viewpoint(at) }), enemy: tokens.error,
      scale: data?.scale ?? 1, badBelow: data?.badBelow ?? 0, goodAbove: data?.goodAbove ?? 1, mode: mode.mode };
  }, [sideName, factions.colours, tokens.muted, viewpointSide, viewpointTag, countryNames, dataHour, tokens.error, data?.scale, data?.badBelow, data?.goodAbove, mode.mode]);
  // The reply's own hours keep their looks while the current day's read moves.
  const appearance = useMemo(() => looksWith(countryColours.reply), [looksWith, countryColours.reply]);
  const dayLooks = useMemo(() => looksWith(countryColours.latest), [looksWith, countryColours.latest]);
  const baseTimeline = useMemo(() => data?.view === 'timeline' ? timelineViews(data, appearance) : null, [data, appearance]);
  const projectedTimeline = useMemo(() => {
    if (!baseTimeline || !follow || !dayData || dayRead == null || data?.view !== 'timeline' || Math.floor(dayData.firstHour / data.dayHours) !== Math.floor(data.lastHour / data.dayHours)) return baseTimeline;
    const regions = data.rows.map(row => ({ id: row.region, name: row.name }));
    const liveRows = new Map(momentViews(dayData, dayRecorded ?? dayRead, dayLooks, { regions }).map(row => [row.region.id, row]));
    const periods = [...baseTimeline.periods];
    const column = periods.length - 1;
    periods[column] = { firstHour: periods[column].firstHour, lastHour: dayData.lastHour, readHour: dayRecorded };
    const gap: AirView = { ...appearance, state: 'gap', parts: [], total: 0n, unknown: [], noAirObject: [], excluded: 0, wings: null, hour: null };
    const rows = baseTimeline.rows.map(row => {
      const view = liveRows.get(Number(row.id))?.view;
      const cells = [...row.cells];
      cells[column] = { parts: view?.parts ?? [], unknown: view?.state === 'unknown', filtered: view?.reason === 'outside-theatres' };
      return { ...row, cells };
    });
    const shown = new Set(rows.map(row => Number(row.id)));
    return { rows, periods, viewAt: (region: number, day: number) => day === column && shown.has(region) ? liveRows.get(region)?.view ?? gap : baseTimeline.viewAt(region, day) };
  }, [baseTimeline, follow, dayData, dayRead, dayRecorded, appearance, dayLooks, data]);
  const allChoices = options.order === 'name' ? choices.data : undefined;
  const projectedMoment = useMemo(() => data?.view === 'moment' ? momentViews(data, dataHour, appearance, allChoices) : [], [data, dataHour, appearance, allChoices]);
  const regionMap = useMemo(() => new Map(projectedMoment.map(row => [row.region.id, row])), [projectedMoment]);
  const timelineRegions = useMemo(() => new Map(data?.view === 'timeline' ? data.rows.map(row => [row.region, row]) : []), [data]);
  const tooltip = useCallback((regionId: number, at: number | null) => {
    let region: { name: string } | undefined, view: AirView | undefined;
    if (data?.view === 'timeline' && projectedTimeline) {
      const index = at == null ? -1 : Math.floor((at - data.firstHour) / data.dayHours);
      region = timelineRegions.get(regionId);
      const cell = projectedTimeline.viewAt(regionId, index);
      if (cell) view = { ...cell, hour: projectedTimeline.periods[index]?.readHour ?? null,
        ...(projectedTimeline.periods[index]?.readHour == null ? { state: 'gap' as const } : {}) };
    } else { const row = regionMap.get(regionId); region = row?.region; view = row && { ...row.view, hour: at }; }
    if (!region || !view) return null;
    if (view.hour != null) askHover(view.hour, airTooltipTags(view));
    const full = airTooltip(region, view, mode.mode === 'viewpoint', view.hour == null ? names.country : hoverLabels(view.hour), !!mode.sides.length, factionNames);
    return fitTooltip([airSummaryTooltip(full, view)], viewport, { keep: view.wings == null ? 1 : 2, full });
  }, [data, projectedTimeline, regionMap, timelineRegions, askHover, hoverLabels, mode.mode, mode.sides.length, names.country, viewport, factionNames]);
  const legend = useMemo(() => mode.mode === 'viewpoint' ? [
    { id: 'friendly', label: `${appearance.viewpoint.name} (friendly)`, color: appearance.viewpoint.color },
    { id: 'enemy', label: `Enemies of ${appearance.viewpoint.name}`, color: tokens.error },
  ] : [...new Map(sides.map(side => sideAt(side, dataHour)).map(side => [side.key, { id: side.key, label: sideName(side), color: appearance.color(side),
    onColor: side.colourFaction ? (hex: string) => changeFactionColour(side.colourFaction!, hex) : undefined }])).values()],
    [mode.mode, appearance, tokens.error, sides, sideName, changeFactionColour, dataHour]);
  const orderedLegend = useMemo(() => {
    const order = sideOrder(sides);
    return [...legend].sort((a, b) => (order.get(a.id) ?? 0) - (order.get(b.id) ?? 0));
  }, [legend, sides]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(windowChoices.data?.warnings ?? []), ...(choices.data?.warnings ?? []), ...(catalogue.data?.warnings ?? []), ...factions.warnings, ...(factions.error ? [factions.error] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, windowChoices.data, choices.data, catalogue.data, factions.warnings, factions.error, onDiagnostics]);
  const error = moment ? windowChoices.error ?? windows.errors.get(wanted[0]) ?? (options.order === 'name' ? choices.error : null) : timeline.error ?? (follow ? day.error : null) ?? catalogue.error;
  const snapshot = runtime.clock.getSnapshot();
  const previous = useRef({ identity, hour, seek: snapshot.seekSequence });
  const glides = moment && data?.view === 'moment' && snapshot.playing && previous.current.identity === identity && previous.current.seek === snapshot.seekSequence
    && data.recorded.some(span => span.firstHour <= Math.min(previous.current.hour, hour) && span.lastHour >= Math.max(previous.current.hour, hour));
  useEffect(() => { previous.current = { identity, hour, seek: snapshot.seekSequence }; }, [identity, hour, snapshot.seekSequence]);
  const glideStyle = { '--air-glide': `${glides ? Math.round(3 * easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion)) : 0}ms` } as CSSProperties;
  return { data, error, needsViewpoint, retry: moment ? () => { windows.retry(); choices.retry(); windowChoices.retry(); } : () => { timeline.retry(); catalogue.retry(); if (follow) day.retry(); }, track, window, beforeBranch, playhead, hour,
    readHour, extent: [firstHour, widget.time.mode === 'range' ? lastHour : track?.branch.lastHour ?? lastHour] as [number, number], mode, tokens, projectedTimeline, projectedMoment, tooltip,
    legend: orderedLegend, labelCatalogue, hoverRevision, choices: choices.data, country: names.country, factionNames, glides, glideStyle, hoursPerSecond };
}
