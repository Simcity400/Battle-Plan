import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC, type GatedHour } from '../../dashboard/regionTime';
import GateState from '../../dashboard/GateState';
import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import WidgetState from '../../components/WidgetState';
import ContentSetPicker from '../../components/ContentSetPicker';
import { dayLabel } from '../../charts/draw';
import LiveText, { liveTexts } from '../../charts/LiveText';
import CountryRaceView from '../../charts/race/CountryRaceView';
import CountryPartsDrill from '../../charts/race/CountryPartsDrill';
import { linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, noMembersAt, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { pausedKey, useHeadHour } from '../army-size/headHour.ts';
import { liveArmy } from '../army-size/liveArmy.ts';
import { definition, OPTIONS } from './definition';
import { countedSince, formatProduced, formatRaw, lineRuns, MEASURES, mergeProducedWindows, producedAt, producedModel, producedRequest, producedWindows,
  raceEntries, rankProduced, typeLabel, UNKNOWN_WORDS, validateProducedWindow, producedRankHour } from './producedData.ts';
import { WINDOW_HOURS } from '../../dashboard/heldWindows.ts';
import { useFrameWindows } from '../army-size/frameWindows.ts';
import type { Count, Measure, ProducedModel } from './producedData';
import { producedDrillData, validateCountryReply } from './producedDrill';
import { amountText, countDecimals, producedCaption, producedCaptionAt, producedWindowUnavailable } from './producedView';
import ProducedDrillTable from './ProducedDrillTable';
import { useProducedAt, useProducedTypes } from './useProduced';
import './equipmentProduced.css';
import type { EquipmentProducedReply, ProducedCountryReply } from '../../generated/dashboard';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const same = <T,>(reply: T) => reply;

export default function EquipmentProducedWidget(props: WidgetProps) {
  const { widget, runtime, active, setOption } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'lines' ? 'lines' : 'race';
  const measure = options.measure as Measure;
  const count: Count = options.count === 'ic' ? 'ic' : 'units';
  const included = useMemo(() => contentSet(options.included).map(contentKey), [options.included]);
  const refits = options.refits === true;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const offset = track?.axisOffset ?? 0;
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const structure = useRef<() => string>(() => '');
  const headHour = useHeadHour(runtime.clock, active && following,
    hour => pausedKey(structure.current(), hour, runtime.clock.getSnapshot().playing));
  useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const window = track && resolveWindow(widget, track, runtime.tracks, headHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);

  const types = useProducedTypes(runtime, track, refits, active);
  const pickingDefault = count === 'units' && !included.length;
  const mostType = types.data?.mostDelivered ?? null;
  const shownTypes = useMemo(() => pickingDefault ? mostType ? [mostType] : [] : included, [pickingDefault, mostType, included]);
  const rankedAt = useRef<GatedHour | null>(null);
  const rankHour = producedRankHour(following, following ? gate.hourAt(axisAt(headHour)) : gate.hour, runtime.clock.getSnapshot().playing, rankedAt.current);
  rankedAt.current = following ? rankHour : null;
  const params = track && (resolved.top != null || resolved.tags.length > 0) && !(pickingDefault && !mostType)
    ? producedRequest(track.branch.id, resolved, rankHour, count, measure, shownTypes, refits) : null;
  const identity = track && params ? JSON.stringify([track.source.id, track.source.revision, params]) : null;
  const momentAt = (hour: number) => {
    const at = track && resolveWindow(widget, track, runtime.tracks, hour);
    return Math.floor((at && (following || widget.time.mode === 'pin') ? at.lastHour : null) ?? extent[1]);
  };
  const viewKey = JSON.stringify([identity, view, extent]);
  const viewport = useRef<{ key: string; range: [number, number] } | null>(null);
  const windowsAt = (hour: number): number[] => {
    if (!track || !params) return [];
    if (view === 'lines') {
      const seen = viewport.current?.key === viewKey ? viewport.current.range : extent;
      return producedWindows(Math.max(extent[0], seen[0]), Math.min(extent[1], seen[1]));
    }
    const at = momentAt(hour);
    return producedWindows(at, following ? at + WINDOW_HOURS : at);
  };
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const windows = useFrameWindows<EquipmentProducedReply>(runtime.client, identity, index => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'equipment-produced', params: { ...params!, window: index } })), () => windowsAt(runtime.clock.getHour()), subscribeClock, active,
  validateProducedWindow, track?.source);
  const windowsCheck = useRef(windows.check);
  windowsCheck.current = windows.check;
  const onViewChange = useCallback((x: readonly [number, number]) => {
    viewport.current = { key: viewKey, range: [Math.floor(x[0]), Math.ceil(x[1])] };
    windowsCheck.current();
  }, [viewKey]);
  const merged = useMemo(() => mergeProducedWindows(windows.data), [windows.data]);
  const model = useMemo(() => merged ? producedModel(merged) : null, [merged]);
  const needed = view === 'lines' ? windowsAt(moment) : producedWindows(moment, moment);
  const failure = needed.map(index => windows.errors.get(index)).find(error => error !== undefined);
  const holding = view === 'lines' ? needed.some(index => windows.data.has(index)) : needed.every(index => windows.data.has(index));
  const decimals = countDecimals(count);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  const produced = resolved.top != null;
  const membershipOf = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  const entriesAt = useCallback((at: ProducedModel, hour: number) =>
    raceEntries(rankProduced(at, rankedCandidates(resolved, hour, at.tags), hour, slots, produced), decimals, hour), [resolved, slots, produced, decimals]);
  const ranked = useMemo(() => model ? entriesAt(model, moment) : [], [model, entriesAt, moment]);
  const live = useMemo(() => active && following && view === 'race' && model
    ? liveArmy(runtime.clock, offset, hour => entriesAt(model, hour), () => 0, undefined, true) : null,
  [active, following, view, model, runtime.clock, offset, entriesAt]);
  structure.current = () => live?.structure() ?? '';
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + offset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, offset]);
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = lineRuns(model, tag, membershipOf(tag), range);
    return runs.length || model.lines.has(tag) ? [{ tag, runs }] : [];
  }), [model, view, lineTags, resolved.membership, range]);
  const keyHours = useMemo(() => merged?.reads.filter(hour => hour >= extent[0] && hour <= extent[1]), [merged, extent[0], extent[1]]);

  const typeName = (key: string) => typeLabel(types.data?.types.find(type => type.key === key) ?? { key, name: null });
  const equipment = !shownTypes.length ? count === 'ic' ? 'All equipment' : 'Equipment'
    : shownTypes.length === 1 ? typeName(shownTypes[0]) : `${shownTypes.length.toLocaleString()} types`;
  const measureLabel = (MEASURES.find(item => item.value === measure) ?? MEASURES[0]).label;
  const reads = merged?.observations ?? null;
  const since = reads ? countedSince(reads, moment) : null;
  const caption = view === 'race' ? producedCaptionAt(count, measure, reads, moment, dayLabel)
    : producedCaption(count, measure, null, since == null ? null : dayLabel(since));
  const shownHour = useRef(() => moment);
  shownHour.current = () => (track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null) ?? moment;
  const paintedCaption = following && view === 'race';
  const captionTexts = useMemo(() => paintedCaption
    ? liveTexts(runtime.clock, () => shownHour.current(), hour => new Map([['caption', producedCaptionAt(count, measure, reads, hour, dayLabel)]])) : null,
  [paintedCaption, runtime.clock, count, measure, reads]);
  const raceLabel = paintedCaption ? `${equipment}: ${producedCaption(count, measure, null, null)}` : `${equipment}: ${caption}`;
  const what = `${count === 'ic' ? `IC, ${equipment === 'All equipment' ? 'all equipment' : equipment}` : equipment}, ${measureLabel.toLowerCase()}`;
  const typeChoices = useMemo(() => (types.data?.types ?? []).map(type => ({ value: setToken(type.key), label: typeLabel(type) })), [types.data]);
  const controls = <div className="produced-caption">
    {setOption ? <ContentSetPicker label="Equipment" automatic={count === 'ic' ? 'All equipment' : 'Most delivered'} choices={typeChoices}
      selected={contentSet(options.included)} single triggerClassName="produced-pill" trigger={<span>{equipment}</span>}
      onToggle={value => setOption('included', value)} onReset={() => setOption('included', '')} />
      : <span className="produced-pill"><span>{equipment}</span></span>}
    <span className="produced-caption-text">{captionTexts ? <LiveText live={captionTexts} at="caption" /> : caption}</span>
  </div>;

  const [hovered, setHovered] = useChangedState<{ tag: string; hour: number } | null>(null);
  const hoveredAt = useProducedAt(runtime, track, hovered?.tag ?? null, gate.recorded(hovered?.hour), measure, refits, active);
  const point = (tag: string, hour: number) => {
    if (hovered?.tag === tag && hovered.hour === hour) return;
    queueMicrotask(() => setHovered(held => held?.tag === tag && held.hour === hour ? held : { tag, hour }));
  };
  const tooltipAt = (tag: string, hour: number, name: string): TooltipContent[] | null => {
    if (!model) return null;
    const at = isMember(membershipOf(tag) ?? undefined, hour) || !resolved.membership ? producedAt(model, tag, hour) : null;
    if (!at) return null;
    const shown = Math.floor(hour);
    point(tag, shown);
    const detail = answersKey(hoveredAt.data?.asked, tag) && hoveredAt.data?.hour === shown ? hoveredAt.data : null;
    const counted = (key: string) => !shownTypes.length || shownTypes.includes(key);
    const rows: TooltipRow[] = !detail ? [{ label: hoveredAt.error ? `This hour could not be read: ${hoveredAt.error.message}` : 'Reading this hour…', tone: 'muted' }]
      : count === 'ic' && !shownTypes.length
        ? [...new Set(detail.models.map(model => model.typeKey))].map(type => {
          const models = detail.models.filter(model => model.typeKey === type);
          const sum = models.reduce<number | null>((total, model) => total == null || model.ic.value == null ? null : total + model.ic.value, 0);
          return { label: models[0].typeRecorded ? typeLabel({ key: type, name: models[0].typeName }) : 'Type not recorded',
            value: sum == null ? 'Not known' : formatRaw(sum, 5), ...(sum == null ? { tone: 'muted' as const } : {}) };
        })
        : detail.models.filter(model => counted(model.typeKey)).map(model => {
          const amount = count === 'ic' ? model.ic : model.units;
          return { label: model.name ?? `Design ${model.variant}`, value: amountText(amount, decimals), ...(amount.value == null ? { tone: 'muted' as const } : {}) };
        });
    const total: TooltipRow = { label: 'Total', value: at.value == null ? 'Not known' : formatRaw(at.value, decimals), strong: true, divider: true };
    const counting = measure === 'total' && merged ? countedSince(merged.observations, hour) : null;
    const read = detail?.read ?? null;
    const note = [at.reason ? UNKNOWN_WORDS[at.reason] : '', counting != null ? `Counted since ${dayLabel(counting)}.` : '',
      read != null ? `Read on ${dayLabel(read)}.` : ''].filter(Boolean).join(' ');
    return [{ title: name, subtitle: `${dayLabel(hour)} · ${what}`, rows: [...(rows.length ? rows : [{ label: 'Nothing delivered', tone: 'muted' as const }]), total],
      ...(note ? { note } : {}) }];
  };

  const warnings = useMemo(() => [...(merged?.warnings ?? []), ...(types.data?.warnings ?? []), ...(groups.data?.warnings ?? [])],
    [merged, types.data, groups.data]);
  const none = coverage.outside ? null : noMembersAt(widget.countries, resolved, groups.data, moment);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see the equipment it produced." />;
    if (gate.outside && (view === 'race' || widget.time.mode !== 'follow')) return <GateState gate={gate} loading="Loading equipment produced…" />;
    if (failure) return <WidgetState kind="error" failure={failure} onRetry={windows.retry} />;
    if (types.error && pickingDefault) return <WidgetState kind="error" failure={types.error} onRetry={types.retry} />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="layers" message={resolved.reason} />;
    if (none) return <NoMembersState reason={none.reason} icon="layers" />;
    if (pickingDefault && types.data && !mostType) return <WidgetState kind="unavailable" message="No production data in this recording." />;
    if (types.data && !types.data.types.length) return <WidgetState kind="unavailable" message="Production data is not in this recording." />;
    if (!model || !holding || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading equipment produced…" />;
    if (!produced && !model.tags.length) return <WidgetState kind="unavailable" message={producedWindowUnavailable(measure)} />;
    return null;
  })();
  const nothingYet = produced && model && ranked.length === 0 && model.tags.some(tag => producedAt(model, tag, moment)?.value != null)
    ? measure === 'total' ? `No production by ${dayLabel(moment)}.` : measure === 'day' ? `No production on ${dayLabel(moment)}.`
      : measure === 'week' ? 'No production in the previous week.' : 'No production in the previous month.'
    : producedWindowUnavailable(measure);

  const [drillType, setDrillType] = useState<{ scope: string; type: string } | null>(null);
  const drillScope = JSON.stringify([track?.source.id, track?.branch.id, count, measure, included, refits]);
  const chartType = drillType?.scope === drillScope ? drillType.type : shownTypes.length === 1 ? shownTypes[0] : null;
  const chartSpec = useMemo(() => ({ count, type: count === 'units' ? chartType ?? shownTypes[0] ?? null : chartType }), [count, chartType, shownTypes]);
  const drillParams = useCallback((tag: string) => ({ branchId: track!.branch.id, country: countryFromKey(tag), count, measure, refits }), [track, count, measure, refits]);
  const decodeDrill = useCallback((data: ProducedCountryReply) => producedDrillData(data, chartSpec), [chartSpec]);
  const pickDrillTag = (hour: number) => model ? mixCountry(widget.countries, fallback,
    rankProduced(model, rankedCandidates(resolved, hour, model.tags), hour, slots, produced)) : null;
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? (live?.entries() ?? ranked).map(entry => entry.tag) : lineTags;
  const format = (value: number) => formatProduced(value, decimals);

  return <CountryPartsDrill {...props} widget={widget} runtime={runtime} active={active} track={track}
    options={options} extent={extent} playhead={playhead} moment={moment} product="equipment-produced-country" params={drillParams}
    validate={validateCountryReply} decode={decodeDrill} numbers={same} defaultTag={mixTag} pickTag={pickDrillTag}
    title="Equipment types" measure={measureLabel} positiveGeometry warnings={warnings}
    resetKey={JSON.stringify([track?.source.id, track?.branch.id, measure, count, included, refits, widget.countries])}
    details={({ gate: drillGate, tag, hour, tagAt, key }) => track && <ProducedDrillTable runtime={runtime} gate={drillGate} track={track} active={active} tag={tag} hour={hour} tagAt={tagAt}
      narrow={narrowTableOf(options)} measure={measure} refits={refits} count={count} chartType={chartSpec.type} chartKey={key}
      chartTitle={chartSpec.type == null ? `IC put into equipment ${measureLabel.toLowerCase()}, by type`
        : `${typeName(chartSpec.type)}: ${count === 'ic' ? 'IC put in' : 'units delivered'} ${measureLabel.toLowerCase()}, by model`}
      onChart={type => setDrillType({ scope: drillScope, type })} />}
    main={(onPick, visible) => <div className="produced-main">
      {track && controls}
      <CountryRaceView runtime={runtime} track={track} active={active && visible} options={options} view={view} extent={extent} playhead={playhead} state={state}
        moment={moment} slots={slots} ranked={ranked} liveRanked={live ?? undefined} lines={lines} countryTags={shownTags} partsLabel="Equipment"
        label={raceLabel} empty={nothingYet} formatValue={format} tooltip={tooltipAt} liveHead={liveHead}
        lineTooltip={(hour, _tags, labelAt, country) => [linesTooltip(hour, lines, labelAt, country, dayLabel(hour), what, format)]}
        keyHours={keyHours} onViewChange={view === 'race' ? undefined : onViewChange}
        labels={{ race: paintedCaption ? undefined : raceLabel, lines: `${equipment}: ${caption}`, actions: 'Equipment produced actions' }}
        presentation={{ exact: false, held: false, raceEmptyState: false, linePadding: CHART_LAYOUT.countryEndLabels, resetTimeView: true }}
        lineEmpty={{ icon: 'layers', message: 'No countries to draw. Choose countries in the widget settings.' }}
        resetKey={`${track?.source.id ?? ''}:${track?.branch.id ?? ''}:${measure}:${count}:${shownTypes.join(' ')}`} onPick={onPick} />
    </div>} />;
}
