import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo,  useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import CountryPicker from '../../components/CountryPicker';
import { useContextMenu } from '../../components/ContextMenu';
import type { ChartSummary } from '../../charts/chartSummary';
import { dayLabel, font } from '../../charts/draw';
import { lineSummaryLayer, type EndLabelsResolver } from '../../charts/renderTimeChart';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { motionOf } from '../../charts/easing.ts';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { SECONDARY_DASH } from '../../charts/palette';
import { barStrip } from '../../charts/strip';
import type { ChartStrip } from '../../charts/strip';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer } from '../../charts/TimeChart';
import { CHART_LAYOUT } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useComparisonCountries } from '../../dashboard/comparisonCountries';
import GateState from '../../dashboard/GateState';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryLineColours } from '../../dashboard/countryLineColours';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { followStore } from '../../dashboard/follow/store.ts';
import { FollowedText } from '../../dashboard/follow/FollowedText.tsx';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { followFailure, followRefusal, followsHead, usePlaybackHour, useFollow } from '../../dashboard/follow/useFollow.ts';
import { isMember } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProducts } from '../../dashboard/products';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { useMeasureSeries } from '../country-measure/measureSeries';
import { summaryEnd } from '../country-measure/measureData';
import { BREAKDOWNS, OPTIONS } from './definition';
import FuelBreakdown from './FuelBreakdown';
import FuelConsumptionBreakdown from './FuelConsumptionBreakdown';
import { fuelConsumptionLevelToken, fuelConsumptionToken, readFuelConsumption, readFuelConsumptionLevel } from './fuelConsumption';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';
import { readDatedCountry } from '../../dashboard/datedCountry.ts';
import { COUNTRY_FUEL, FUEL_DAYS, QUANTITY_SCALE, formatFuel, fuelDecimals, fuelTooltips, stripBars, stripNote, validateCountryFuel } from './fuelData';
import { fuelExtentToken, fuelRecipientToken, readFuelExtent, readFuelRecipient } from './fuelDrill';
import type { FuelRecipientSelection } from './fuelDrill';
import type { CountryFuelReply } from '../../generated/dashboard';
import { axisAt, recordedHour, SPEC } from '../../dashboard/regionTime.ts';
import { useGate } from '../../dashboard/regionGate';
import { coverageOf, useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { UNRECORDED_MESSAGE } from '../../dashboard/regionTime.ts';

const STRIP_SHARE = 0.28;
const DAY_HOURS = 24;
const DETAIL_HOURS = 720;
const NO_PAINT = () => {};
const HOLDERS = { holders: true };

export default function FuelWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
  const savedShow = BREAKDOWNS.find(item => item.id === widget.options?.show)?.id ?? null;
  const storedCountry = widget.options?.fuelCountry;
  const drillNeeds = useMemo(() => savedShow ? { country: { stored: storedCountry }, holders: true } : HOLDERS, [savedShow, storedCountry]);
  const drillControl = useDrill(runtime, useRegion(widget), savedShow, undefined, undefined, drillNeeds);
  const storedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<object>();
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const digits = fuelDecimals(options);
  const showCapacity = options.capacity !== false;
  const { track, countries, pending, contentPending, error: groupError, refreshError: groupRefreshError, retry: retryGroups } = useComparisonCountries(widget, runtime, active);
  const tags = countries.tags;
  const single = tags.length === 1;
  const membership = countries.membership ?? undefined;
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const saved = BREAKDOWNS.find(item => item.id === options.show);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const shown = useShownHour();
  const headTags = following && active && track && !saved ? tags : null;
  const branchId = track?.branch.id;
  const fuelQuery = useMemo(() => headTags?.length ? gate.follow({ product: 'series-state', params: { branchId,
    family: { kind: COUNTRY_FUEL, measure: 'fuel' }, reads: headTags.map(tag => ({ country: countryFromKey(tag) })) } }) : null, [headTags, branchId, gate.follow]);
  const capacityQuery = useMemo(() => headTags?.length && showCapacity ? gate.follow({ product: 'series-state', params: { branchId,
    family: { kind: COUNTRY_FUEL, measure: 'capacity' }, reads: headTags.map(tag => ({ country: countryFromKey(tag) })) } }) : null, [headTags, branchId, showCapacity, gate.follow]);
  const fuelFollowed = useFollow(store, fuelQuery, root, useCallback((entry: FollowEntry) => shown(shownThrough(entry)), [shown]));
  const capacityFollowed = useFollow(store, capacityQuery, root, NO_PAINT);
  const fuelHead = followsHead(fuelFollowed), capacityHead = followsHead(capacityFollowed);
  const refused = followRefusal(fuelFollowed, capacityFollowed);
  const followErr = followFailure(fuelFollowed, capacityFollowed);
  const playing = usePlaybackMotion(runtime.clock) !== undefined;
  const drillOnClock = drillControl.following;
  const clockHour = usePlaybackHour(runtime.clock, active && drillOnClock && !playing);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const heldExtent = track && saved ? readFuelExtent(options.fuelExtent, track.source.metadata.campaignKey, track.branch.id) : null;
  const extent: [number, number] = heldExtent ?? (track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1]);
  const head = playhead ?? extent[1];
  const headAt = following ? gate.hourAt(axisAt(clockHour)) : gate.hour;
  const lastHour = track?.branch.lastHour ?? 0;
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const live = following && !saved;
  const hoverAt = gate.recorded(hoverHour);
  const fuel = useMeasureSeries(runtime.client, gate, track?.source, track?.branch, COUNTRY_FUEL, 'fuel', tags, live ? null : headAt, hoverAt, active && !saved,
    false, membership, lastHour, live);
  const capacity = useMeasureSeries(runtime.client, gate, showCapacity ? track?.source : undefined, track?.branch, COUNTRY_FUEL, 'capacity', tags,
    live ? null : headAt, hoverAt, active && !saved && showCapacity, false, membership, lastHour, live);
  const withStrip = single && options.strip !== false && !saved;
  const gain = useMeasureSeries(runtime.client, gate, withStrip ? track?.source : undefined, track?.branch, FUEL_DAYS, 'stripGain', tags, null, null,
    active && withStrip, false, membership, lastHour, live);
  const burn = useMeasureSeries(runtime.client, gate, withStrip ? track?.source : undefined, track?.branch, FUEL_DAYS, 'stripBurn', tags, null, null,
    active && withStrip, false, membership, lastHour, live);
  useEffect(() => { if (!fuelHead && !saved) shown(tags.length && tags.every(tag => fuel.hourLoaded(tag, Math.floor(head))) ? head : null); });
  const styles = useCountryLineStyles(runtime.client, gate, track, tags, Math.floor(head), active && !single && !saved);
  const colours = useMemo(() => countryLineColours(tags, styles.known, tokens.surface, tokens.countryColours === 'fixed'), [tags, styles.known, tokens]);
  const colourOf = useCallback((tag: string, of: 'fuel' | 'capacity') => single ? (of === 'fuel' ? tokens.accent : tokens.muted) : colours.get(tag) ?? tokens.muted,
    [single, tokens.accent, tokens.muted, colours]);
  const wantedNames = useMemo(() => tags.map(tag => ({ tag, hour: Math.floor(head) })), [tags, head]);
  const names = useCountryNamesAt(runtime.client, track, single ? [] : wantedNames, active && !saved);
  const nameOf = useCallback((tag: string) => { const read = names.get(nameKey(Math.floor(head), tag)); return read?.kind === 'value' ? read.value : tag; }, [names, head]);
  const fuelEntry = fuelHead ? fuelFollowed.entry : null, capacityEntry = capacityHead ? capacityFollowed.entry : null;
  useEffect(() => {
    const stops = [[fuelEntry, fuel.head], [capacityEntry, capacity.head]].flatMap(([entry, lineHead]) => entry
      ? tags.map((tag, row) => followAtHead((lineHead as typeof fuel.head)(tag), () => rowAmount((entry as FollowEntry).answer, row, 0))) : []);
    return () => stops.forEach(stop => stop());
  }, [fuelEntry, capacityEntry, tags, fuel.head, capacity.head]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const layers = useMemo<ChartLayer[]>(() => tags.flatMap(tag => [
    ...showCapacity ? [lineSummaryLayer({
      summary: () => capacity.lineOf(tag),
      color: colourOf(tag, 'capacity'),
      options: { dash: SECONDARY_DASH },
      until,
      yScale: QUANTITY_SCALE,
    })] : [],
    lineSummaryLayer({
      summary: () => fuel.lineOf(tag),
      color: colourOf(tag, 'fuel'),
      options: { area: single && options.areaFill !== false },
      until,
      yScale: QUANTITY_SCALE,
    }),
  ]), [tags, showCapacity, capacity.lineOf, fuel.lineOf, colourOf, single, options.areaFill, until]);
  const endLabels = useMemo<EndLabelsResolver | undefined>(() => {
    if (options.endLabels === false) return undefined;
    return (frame, drawnUntil) => {
      const end = Math.min(drawnUntil ?? extent[1], frame.x[1], extent[1]);
      return tags.flatMap(tag => (['fuel', ...showCapacity ? ['capacity'] as const : []] as const).flatMap(of => {
        const value = summaryEnd(of === 'fuel' ? fuel.lineOf(tag) : capacity.lineOf(tag), end);
        if (value == null) return [];
        const what = of === 'fuel' ? 'Fuel' : 'Capacity';
        return [{ id: `${tag}:${of}`, y: frame.toY(value / QUANTITY_SCALE), color: colourOf(tag, of),
          text: single ? what : `${nameOf(tag)} · ${what}`, suffix: formatFuel(BigInt(Math.round(value)), QUANTITY_SCALE, digits) }];
      }));
    };
  }, [options.endLabels, showCapacity, extent[1], tags, fuel.lineOf, capacity.lineOf, colourOf, single, nameOf, digits]);
  const gainFailure = gain.error?.message ?? null, burnFailure = burn.error?.message ?? null;
  const strip = useMemo<ChartStrip | undefined>(() => {
    if (!withStrip || !tags[0]) return undefined;
    let held: { gain?: ChartSummary; burn?: ChartSummary; strip: ChartStrip; bars: number } | null = null;
    const current = () => {
      const [up, down] = [gain.lineOf(tags[0]), burn.lineOf(tags[0])];
      if (!held || held.gain !== up || held.burn !== down) {
        const bars = stripBars(up, down, DAY_HOURS).filter(bar => isMember(membership?.get(tags[0]), bar.lastHour));
        held = { gain: up, burn: down, bars: bars.length, strip: barStrip(bars,
          { height: height => Math.max(CHART_LAYOUT.stripMin, (height - CHART_LAYOUT.stripInset) * STRIP_SHARE), upLabel: 'gain', downLabel: 'burn' }) };
      }
      return held;
    };
    const lines = (summary: ChartSummary | undefined, unrecorded: string | null, failure: string | null) =>
      ({ landed: summary !== undefined || unrecorded !== null, unrecorded, failure });
    return { height: height => current().strip.height(height),
      draw: (ctx, frame, chartTokens, state) => {
        const drawn = current();
        const note = stripNote(drawn.bars, lines(drawn.gain, gain.notRecorded, gainFailure), lines(drawn.burn, burn.notRecorded, burnFailure));
        if (!note) drawn.strip.draw(ctx, frame, chartTokens, state);
        if (!note || !frame.strip) return;
        ctx.save();
        ctx.fillStyle = chartTokens.muted;
        ctx.font = font(chartTokens, chartTokens.fontSize.small);
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(note.text, frame.plotLeft + frame.plotWidth / 2, frame.strip.top + frame.strip.height / 2);
        ctx.restore();
      },
      describe: (point, frame, through) => current().strip.describe?.(point, frame, through) ?? null,
      keyTargets: (view, through) => current().strip.keyTargets?.(view, through) ?? [] };
  }, [withStrip, tags, gain.lineOf, burn.lineOf, membership, gain.notRecorded, burn.notRecorded, gainFailure, burnFailure]);
  const chartData = useMemo<ChartData>(() => ({ revision: () => [fuel.revision(), capacity.revision(), gain.revision(), burn.revision()],
    subscribe: listener => { const stops = [fuel.subscribe(listener), capacity.subscribe(listener), gain.subscribe(listener), burn.subscribe(listener)];
      return () => stops.forEach(stop => stop()); } }), [fuel.revision, fuel.subscribe, capacity.revision, capacity.subscribe, gain.revision, gain.subscribe, burn.revision, burn.subscribe]);
  const hoverTags = useMemo(() => hoverAt === null ? [] : tags.filter(tag => isMember(membership?.get(tag), hoverAt)), [tags, membership, hoverAt]);
  const hoverRequests = useMemo(() => track && hoverAt !== null && !saved ? hoverTags.map(tag => (productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: COUNTRY_FUEL, params: { branchId: track.branch.id, country: countryFromKey(tag), hour: hoverAt } }))) : [], [track, hoverAt, hoverTags, saved]);
  const hovered = useProducts<CountryFuelReply>(runtime.client, hoverRequests, active, validateCountryFuel, track?.source);
  const hoverLabels = useCountryLabelsAt(runtime.client, track, hoverHour === null || single ? [] : tags.map(tag => ({ tag, hour: Math.floor(hoverHour) })), active);
  const colors = { fuel: tokens.accent, capacity: tokens.muted, gained: tokens.categories[0], burned: tokens.categories[1] };
  const menu = useContextMenu('Fuel actions');
  const anchor = useRef<HTMLDivElement>(null);

  const pickScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags, options.show, extent]);
  const pickOwner = useMemo(() => ({ scope: pickScope }), [pickScope]);
  const drill = saved || drillOwner === undefined || drillOwner === pickOwner ? storedDrill : CLOSED;
  const [measure, setMeasure] = useState<'gain' | 'burn' | 'consumption'>('consumption');
  const drillMeasure = (saved?.id ?? BREAKDOWNS.find(item => item.id === drill.breakdown)?.id ?? measure) as 'gain' | 'burn' | 'consumption';
  const actualHour = track ? recordedHour(track, clockHour + track.axisOffset) : null;
  const drillCoverage = coverageOf(drillControl.outside);
  const at = drill.breakdown !== null ? drillControl.hour : null;
  const drillFollows = drillControl.following;
  const [pickTags, setPickTags] = useState<readonly string[]>();
  const campaign = track?.source.metadata.campaignKey ?? '';
  const savedCountry = useMemo(() => track && saved ? readDatedCountry(options.fuelCountry, campaign, track.branch.id, widget.countries) : null,
    [track, saved, options.fuelCountry, widget.countries, campaign]);
  const drillCountry = drillControl.gate.country;
  const pickKey = saved ? tags[0] ?? null : drill.country?.tag ?? null;
  const identitySpans = drillCountry?.aliases ?? null;
  const drillTag = drillCountry?.tag ?? null;
  const [recipient, setRecipient] = useState<FuelRecipientSelection | null | undefined>(undefined);
  const savedRecipient = track && saved?.id === 'burn' ? readFuelRecipient(options.fuelRecipient, campaign, track.branch.id) : null;
  const [consumption, setConsumption] = useState<FuelConsumptionSelection | null | undefined>(undefined);
  const savedConsumption = track && saved?.id === 'consumption' ? readFuelConsumption(options.fuelConsumption, campaign, track.branch.id) : null;
  const [level, setLevel] = useState<FuelConsumptionLevel | null | undefined>(undefined);
  const savedLevel = track && saved?.id === 'consumption' ? readFuelConsumptionLevel(options.fuelConsumptionLevel, campaign, track.branch.id) : null;
  const heldRecipient = recipient === undefined ? savedRecipient : recipient;
  const heldConsumption = consumption === undefined ? savedConsumption : consumption;
  const heldLevel = level === undefined ? savedLevel : level;
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(undefined); setPickTags(undefined);
    setRecipient(undefined); setConsumption(undefined); setLevel(undefined); }, [pickOwner]);
  const invalidSaved = !!saved && (!!options.fuelExtent && !heldExtent || !!options.fuelCountry && !savedCountry
    || saved.id === 'burn' && !!options.fuelRecipient && !savedRecipient || saved.id === 'consumption' && !!options.fuelConsumption && !savedConsumption
    || saved.id === 'consumption' && !!options.fuelConsumptionLevel && !savedLevel);

  const state = !track ? <WidgetState kind="empty" icon="droplet" message="Load a campaign and choose countries to see their fuel." />
    : refused ? <WidgetState kind="unavailable" technical={refused} />
    : followErr ? <WidgetState kind="error" failure={followErr.error} onRetry={followErr.retry} />
    : playhead !== null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour) ? <WidgetState kind="outside" />
      : groupError ? <WidgetState kind="error" failure={groupError} onRetry={retryGroups} />
        : countries.reason ? <WidgetState kind="empty" message={countries.reason} />
          : (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window ? <WidgetState kind="unavailable" message="Choose another time in widget settings." />
            : pending ? <WidgetState kind="loading" label="Loading fuel…" />
              : fuel.error && !fuel.summaries && !saved ? <WidgetState kind="error" failure={fuel.error} onRetry={fuel.retry} /> : undefined;
  useDiagnostics(onDiagnostics, useMemo(() => [...(contentPending ? ['Group names are updating from the recorded content.'] : []),
    ...(styles.error ? [`Map colours could not be loaded: ${styles.error.message}`] : []),
    ...(showCapacity && capacity.error ? [`The capacity could not be loaded: ${capacity.error.message}`] : []),
    ...(withStrip && (gain.error ?? burn.error) ? [`The gain and burn bars could not be loaded: ${(gain.error ?? burn.error)!.message}`] : []),
    ...(withStrip && (gain.notRecorded ?? burn.notRecorded) ? [`Gain and burn not recorded: ${gain.notRecorded ?? burn.notRecorded}`] : []),
    ...(hovered.data?.flatMap(reply => reply.warnings.map(note => `${reply.tag}: ${note}`)) ?? [])], [contentPending, styles.error, showCapacity, capacity.error,
    withStrip, gain.error, burn.error, gain.notRecorded, burn.notRecorded, hovered.data]));

  const open = (hour: number, tag?: string, choices?: readonly string[]) => {
    if (!active || saved || state || !track) return;
    hour = Math.floor(hour);
    if (hour < extent[0] || hour > extent[1]) return;
    const choosing = !tag && choices && choices.length > 1;
    setRecipient(null); setConsumption(null); setLevel(null);
    setDrillOwner(pickOwner);
    if (choosing) { setPickTags(choices); drillControl.showAt(drillMeasure, hour); return; }
    const picked = tag ?? choices?.[0] ?? pickKey;
    if (!picked || !tags.includes(picked) || !isMember(membership?.get(picked), hour)) return;
    setPickTags(undefined);
    drillControl.show(drillMeasure, hour, { tag: picked, origin: hour });
  };
  const tooltip = (hour: number) => {
    const at = Math.floor(hour);
    const replies = hoverHour !== null && Math.floor(hoverHour) === at ? hovered.data : undefined;
    if (single) {
      if (!isMember(membership?.get(tags[0]), at)) return fitTooltip([{ title: dayLabel(at), rows: [{ label: 'Not in the group at this date', tone: 'muted' as const }] }], viewport);
      const reply = replies?.[hoverTags.indexOf(tags[0])];
      return fitTooltip(reply ? fuelTooltips(reply, dayLabel(at), colors, digits) : [{ title: dayLabel(at), rows: [{ label: hovered.error ? 'Could not load this day' : gate.recorded(at) === null ? UNRECORDED_MESSAGE : 'Loading…', tone: 'muted' as const }] }], viewport);
    }
    const rows: TooltipRow[] = tags.flatMap((tag): TooltipRow[] => {
      const label = hoverLabels.labelAt(at)(tag);
      const reply = replies?.[hoverTags.indexOf(tag)];
      if (!isMember(membership?.get(tag), at)) return [{ label: label.name, countryFlag: label, value: 'Not in the group', tone: 'muted' as const }];
      if (!reply) return [{ label: label.name, countryFlag: label, value: 'Loading…', tone: 'muted' as const }];
      const tip = fuelTooltips(reply, label.name, colors, digits)[0];
      return [{ label: label.name, countryFlag: label, swatch: colourOf(tag, 'fuel'), strong: true }, ...tip.rows.map(row => ({ ...row, indent: true })),
        ...(tip.note ? [{ label: tip.note, tone: 'muted' as const, indent: true }] : [])];
    });
    return fitTooltip([{ title: dayLabel(at), rows }], viewport);
  };
  const legend = single ? null : tags.map((tag, row) => ({ id: tag, label: nameOf(tag), color: colourOf(tag, 'fuel'),
    value: fuelEntry && store ? <FollowedText entry={fuelEntry} write={(entry, cell) => {
      const raw = rowAmount(entry.answer, row, 0);
      cell.put(raw == null ? raw === null ? 'Not recorded' : 'Loading…' : formatFuel(BigInt(Math.round(raw)), QUANTITY_SCALE, digits), '');
    }} /> : undefined }));

  const main = state ?? <div className="country-measure" ref={setRoot}><div className="country-measure-view">
    {legend && <Legend label="Countries" items={legend} windowed />}
    <p className="chart-line-key">Solid: fuel{showCapacity ? ' · dashed: capacity' : ''}</p>
    <button type="button" className="button small quiet" onClick={() => {
      const hour = Math.floor(playhead ?? extent[1]);
      const present = tags.filter(tag => isMember(membership?.get(tag), hour));
      if (present.length) open(hour, present.length === 1 ? present[0] : pickKey ?? undefined, present);
    }}>View fuel breakdown</button>
    <div className="country-measure-chart">
      <TimeChart label={`Fuel of ${tags.map(nameOf).join(', ')}`} extent={extent} playhead={playhead}
        liveHead={following && track ? { read: () => runtime.clock.getSnapshot().position + (track.axisOffset ?? 0), subscribe: runtime.clock.subscribeFrame } : undefined}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track!.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels !== false ? CHART_LAYOUT.measureEndLabels : CHART_LAYOUT.right }} layers={layers} strip={strip} active={active && !saved} notRecorded={fuel.notRecorded}
        revision={`${track!.branch.id}:${tags.length}:${viewport}:${digits}:${showCapacity}:${hovered.data ? 'hover' : ''}`} data={chartData}
        pickOwner={fuel.owner} onSummaryView={view => { fuel.onSummaryView(view); if (showCapacity) capacity.onSummaryView(view); if (withStrip) { gain.onSummaryView(view); burn.onSummaryView(view); } }}
        onNavigateHour={fuel.onNavigateHour} keyboardHour={fuel.keyboardHour} keyboardRevision={fuel.keyboardRevision} onHover={setHoverHour}
        formatValue="fuel"
        endLabels={endLabels}
        tooltip={hour => tooltip(hour)} onPick={hour => open(hour, single ? tags[0] : undefined, tags.filter(tag => isMember(membership?.get(tag), Math.floor(hour))))}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track!.id, hour)])} />
    </div>
  </div></div>;

  const drillStage = drillControl.gate.stage.stage;
  const actorState = drillCountry && drillStage !== 'open' && drillStage !== 'outside' && drillStage !== 'unbound' ? <GateState gate={drillControl.gate} loading="Loading country…" /> : undefined;
  const chart = track ? { active: active && (!!saved || drill.breakdown !== null), pickOwner, extent, branchStart: track.branch.firstHour,
    playhead: drillFollows ? actualHour : playhead, playback,
    liveHead: drillFollows ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: motionOf(runtime.clock.getSnapshot()), within: pickKey ? membership?.get(pickKey) ?? null : null, detailHours: DETAIL_HOURS,
    onPick: (hour: number) => { setDrillOwner(pickOwner); drillControl.pick(hour); },
    onContextMenu: (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]) } : null;
  const breakdown = at === null && track ? drillControl.gate.stage.stage === 'open' || drillControl.gate.stage.stage === 'outside' ? <WidgetState kind="outside" read={drillCoverage} /> : <GateState gate={drillControl.gate} loading="Loading country…" />
    : invalidSaved ? <WidgetState kind="unavailable" message="Choose a country again." /> : actorState ?? (track && chart && drillTag
    ? drillMeasure === 'consumption'
      ? <FuelConsumptionBreakdown runtime={runtime} gate={drillControl.gate} widget={widget} track={track} tag={drillTag} hour={drillFollows ? null : at} follows={drillFollows} digits={digits}
        campaign={campaign} aliases={identitySpans ?? undefined} history={drillCountry?.history ?? null} selection={heldConsumption} onSelection={setConsumption} level={heldLevel} onLevel={setLevel} {...chart} />
      : <FuelBreakdown runtime={runtime} gate={drillControl.gate} widget={widget} track={track} tag={drillTag} measure={drillMeasure} hour={drillFollows ? null : at} follows={drillFollows}
        digits={digits} campaign={campaign} aliases={identitySpans ?? undefined} history={drillCountry?.history ?? null} selection={heldRecipient} onSelection={setRecipient} {...chart} />
    : <p className="drill-empty">{drillCountry ? 'Country unavailable at this time.' : 'Choose a country from the drawn lines for this moment.'}</p>);
  const headerReason = at === null ? 'This time is outside the recording.' : invalidSaved ? 'Choose a country again.' : !drillCountry?.historyHeld ? 'Loading country…'
    : !drillTag ? 'Country unavailable at this time.' : null;
  const headerReady = !!saved && !!options.fuelCountry && !!countrySlot && active && headerReason === null && at !== null && !!drillTag;
  const headerWanted = useMemo(() => headerReady && drillTag && at !== null ? [{ tag: drillTag, hour: at }] : [], [headerReady, drillTag, at]);
  const { labelAt: headerLabelAt } = useCountryLabelsAt(runtime.client, track, headerWanted, headerReady);
  const pinnedHeader = saved && options.fuelCountry && countrySlot ? createPortal(<DatedCountryChip failure={drillCountry?.historyHeld ? undefined : drillCountry?.error}
    label={headerReady && drillTag && at !== null ? headerLabelAt(at)(drillTag) : null} hour={at} reason={headerReason} />, countrySlot) : null;
  const notices = groupRefreshError ? <WidgetState kind="error" failure={groupRefreshError} onRetry={retryGroups} /> : null;

  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinHeld = drillControl.gate.checkHold(drillCountry?.holder ?? null);
  const pinCountry = pinHeld.held;
  if (saved) return <div ref={anchor} className="drill-body">
    {pinnedHeader}{notices}
    {!drillFollows && <button type="button" className="button small quiet" onClick={() => {
      if (actorState || invalidSaved) return;
      setDrillOwner(pickOwner); drillControl.follow();
    }}>Follow playback</button>}
    {breakdown}{menu.menu}
  </div>;
  return <div ref={anchor} style={{ height: '100%', minHeight: 0 }}>{notices}<DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} title="Fuel consumption and daily accounting" day={drillFollows && playing ? 'Following playback' : at === null ? '' : dayLabel(at)}
    canFollow={actualHour !== null && !!drillTag} onClose={() => dispatch({ type: 'close' })}
    main={main}
    pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={drillTag && track && !actorState && pinTime && pinCountry ? () => {
      const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [pinCountry.tag] } }, BREAKDOWNS.find(item => item.id === drillMeasure)!, track.id, pinTime);
      pinned.options = { ...pinned.options, fuelCountry: pinCountry.token,
        fuelRecipient: heldRecipient ? fuelRecipientToken(heldRecipient) : '', fuelConsumption: heldConsumption ? fuelConsumptionToken(heldConsumption) : '',
        fuelConsumptionLevel: heldLevel ? fuelConsumptionLevelToken(heldLevel) : '', fuelExtent: fuelExtentToken(campaign, track.branch.id, extent) };
      runtime.addWidgetBelow(widget.id, pinned);
    } : undefined}>
    {!single && track && <CountryPicker countries={track.source.metadata.countries} playerCountryId={null} selectedTag={drillTag} source={track.source} client={runtime.client}
      allowedTags={pickTags ?? tags} flagAt={at !== null ? { branchId: track.branch.id, hour: at } : null}
      onChange={tag => {
        if (!tag || at === null || !(pickTags ?? tags).includes(tag) || !isMember(membership?.get(tag), at)) return;
        setDrillOwner(pickOwner); setPickTags(undefined);
        dispatch({ type: 'open', breakdown: drillMeasure, hour: at, follow: drillFollows, country: { tag, origin: at } });
        setRecipient(null); setConsumption(null); setLevel(null);
      }} />}
    <Segmented label="Breakdown of" value={drillMeasure} onChange={next => {
      setMeasure(next); setRecipient(null); setConsumption(null); setLevel(null);
      if (drill.breakdown !== null && at !== null) dispatch({ type: 'open', breakdown: next, hour: at, follow: drillControl.following, country: drill.country });
    }} options={BREAKDOWNS.map(item => ({ value: item.id, label: item.label }))} />
    {breakdown}
  </DrillHost>{menu.menu}</div>;
}
