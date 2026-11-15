import { useCallback, useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import WidgetState from '../../components/WidgetState';
import ContentSetPicker from '../../components/ContentSetPicker';
import { Segmented } from '../../components/Segmented';
import { dayLabel } from '../../charts/draw';
import LiveText, { liveTexts } from '../../charts/LiveText';
import CountryRaceView from '../../charts/race/CountryRaceView';
import CountryPartsDrill from '../../charts/race/CountryPartsDrill';
import { linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { defaultCountryTag } from '../../dashboard/countrySelection';
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
import { amountText, COUNTS, countDecimals, producedCaption, producedCaptionAt, producedWindowUnavailable } from './producedView';
import ProducedDrillTable from './ProducedDrillTable';
import { useProducedAt, useProducedTypes } from './useProduced';
import './equipmentProduced.css';
import type { EquipmentProducedReply, ProducedCountryReply } from '../../generated/dashboard';
import { useChangedState } from '../../dashboard/changedState.ts';

const same = <T,>(reply: T) => reply;

/** Equipment each country delivered: one type at a time in units, or the factory IC put into equipment, as a race
 * or as lines over time; a country's breakdown lists every type it made with its models. A reply holds every shown
 * country's values over one window of hours: the race asks the window it shows and, following, the next ahead from
 * the frame; the lines the windows in view. Following the playhead paints the values held and asks nothing per hour. */
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
  // Following, the widget renders only when the rows of the race change: their values and order are painted.
  const structure = useRef<() => string>(() => '');
  // Paused, the widget's hour is the clock's: a seek shows its hour whether or not the rows change.
  const headHour = useHeadHour(runtime.clock, active && following,
    hour => pausedKey(structure.current(), hour, runtime.clock.getSnapshot().playing));
  useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const window = track && resolveWindow(widget, track, runtime.tracks, headHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);

  // In units, until a type is chosen, the type delivered most; IC counts every type until one is chosen.
  const types = useProducedTypes(runtime, track, refits, active);
  const pickingDefault = count === 'units' && !included.length;
  const mostType = types.data?.mostDelivered ?? null;
  const shownTypes = useMemo(() => pickingDefault ? mostType ? [mostType] : [] : included, [pickingDefault, mostType, included]);
  // A top N is ranked at the hour shown when paused or sought, kept while playing: never an hour that moves with playback.
  const rankedAt = useRef<number | null>(null);
  const rankHour = producedRankHour(widget.time.mode, playhead, runtime.clock.getSnapshot().playing, rankedAt.current, extent[1]);
  rankedAt.current = following ? rankHour : null;
  const params = track && (resolved.top != null || resolved.tags.length > 0) && !(pickingDefault && !mostType)
    ? producedRequest(track.branch.id, resolved, rankHour, count, measure, shownTypes, refits) : null;
  const identity = track && params ? JSON.stringify([track.source.id, track.source.revision, params]) : null;
  // The moment shown at a clock hour, as a render works it out.
  const momentAt = (hour: number) => {
    const at = track && resolveWindow(widget, track, runtime.tracks, hour);
    return Math.floor((at && (following || widget.time.mode === 'pin') ? at.lastHour : null) ?? extent[1]);
  };
  // The windows a clock hour wants: the race's at the moment and, following, the next ahead; the lines' in view.
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
  const windows = useFrameWindows<EquipmentProducedReply>(runtime.client, identity, index => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'equipment-produced', params: { ...params!, window: index } }), () => windowsAt(runtime.clock.getHour()), subscribeClock, active,
  validateProducedWindow, track?.source);
  const windowsCheck = useRef(windows.check);
  windowsCheck.current = windows.check;
  const onViewChange = useCallback((x: readonly [number, number]) => {
    viewport.current = { key: viewKey, range: [Math.floor(x[0]), Math.ceil(x[1])] };
    windowsCheck.current();
  }, [viewKey]);
  const merged = useMemo(() => mergeProducedWindows(windows.data), [windows.data]);
  const model = useMemo(() => merged ? producedModel(merged) : null, [merged]);
  // What this render shows: the race's window at the moment, or every window in view.
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
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = lineRuns(model, tag, membershipOf(tag), range);
    return runs.length || model.lines.has(tag) ? [{ tag, runs }] : [];
  }), [model, view, lineTags, resolved.membership, range]); // eslint-disable-line react-hooks/exhaustive-deps
  const keyHours = useMemo(() => merged?.reads.filter(hour => hour >= extent[0] && hour <= extent[1]), [merged, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps

  // What is counted, in words: the pill names the equipment, the caption the window and date.
  const typeName = (key: string) => typeLabel(types.data?.types.find(type => type.key === key) ?? { key, name: null });
  const equipment = !shownTypes.length ? count === 'ic' ? 'All equipment' : 'Equipment'
    : shownTypes.length === 1 ? typeName(shownTypes[0]) : `${shownTypes.length.toLocaleString()} types`;
  const measureLabel = (MEASURES.find(item => item.value === measure) ?? MEASURES[0]).label;
  const reads = merged?.observations ?? null;
  const since = reads ? countedSince(reads, moment) : null;
  const caption = view === 'race' ? producedCaptionAt(count, measure, reads, moment, dayLabel)
    : producedCaption(count, measure, null, since == null ? null : dayLabel(since));
  // Following, the race's date moves with the head: painted each frame at the hour shown, never rendered for.
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
    {setOption && <Segmented value={count} options={COUNTS} label="Count" onChange={value => setOption('count', value)} />}
  </div>;

  // A tooltip's breakdown is read for the country and hour pointed at, never for the hour playback moves through.
  const [hovered, setHovered] = useChangedState<{ tag: string; hour: number } | null>(null);
  const hoveredAt = useProducedAt(runtime, track, hovered?.tag ?? null, hovered?.hour ?? null, measure, refits, active);
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
    const detail = hoveredAt.data?.tag === tag && hoveredAt.data.hour === shown ? hoveredAt.data : null;
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
  const none = noMembersAt(widget.countries, resolved, groups.data, moment, lastHour);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see the equipment it produced." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
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
  // An empty top N says nothing was produced only where a shown country's value is known then.
  const nothingYet = produced && model && ranked.length === 0 && model.tags.some(tag => producedAt(model, tag, moment)?.value != null)
    ? measure === 'total' ? `No production by ${dayLabel(moment)}.` : measure === 'day' ? `No production on ${dayLabel(moment)}.`
      : measure === 'week' ? 'No production in the previous week.' : 'No production in the previous month.'
    : producedWindowUnavailable(measure);

  // The breakdown asks for the country's every model in the count shown; its chart stacks the chosen type's models,
  // or every type in IC until one is chosen.
  const [drillType, setDrillType] = useState<{ scope: string; type: string } | null>(null);
  const drillScope = JSON.stringify([track?.source.id, track?.branch.id, count, measure, included, refits]);
  const chartType = drillType?.scope === drillScope ? drillType.type : shownTypes.length === 1 ? shownTypes[0] : null;
  const chartSpec = useMemo(() => ({ count, type: count === 'units' ? chartType ?? shownTypes[0] ?? null : chartType }), [count, chartType, shownTypes]);
  const drillParams = useCallback((tag: string) => ({ branchId: track!.branch.id, tag, count, measure, refits }), [track, count, measure, refits]);
  const decodeDrill = useCallback((data: ProducedCountryReply) => producedDrillData(data, chartSpec), [chartSpec]);
  const pickDrillTag = (hour: number) => model ? mixCountry(widget.countries, fallback,
    rankProduced(model, rankedCandidates(resolved, hour, model.tags), hour, slots, produced)) : null;
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? (live?.entries() ?? ranked).map(entry => entry.tag) : lineTags;
  const format = (value: number) => formatProduced(value, decimals);

  return <CountryPartsDrill {...props} widget={widget} runtime={runtime} active={active} track={track}
    options={options} extent={extent} playhead={playhead} moment={moment} product="equipment-produced-country" params={drillParams}
    validate={validateCountryReply} decode={decodeDrill} numbers={same} defaultTag={mixTag} pickTag={pickDrillTag}
    title="Equipment types" measure={measureLabel} positiveGeometry warnings={warnings} state={state}
    resetKey={JSON.stringify([track?.source.id, track?.branch.id, measure, count, included, refits, widget.countries])}
    details={({ tag, hour, tagAt, key }) => track && <ProducedDrillTable runtime={runtime} track={track} active={active} tag={tag} hour={hour} tagAt={tagAt}
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
