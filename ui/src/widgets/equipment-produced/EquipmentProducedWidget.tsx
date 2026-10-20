import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import WidgetState from '../../components/WidgetState';
import ContentSetPicker from '../../components/ContentSetPicker';
import { Segmented } from '../../components/Segmented';
import { dayLabel } from '../../charts/draw';
import CountryRaceView from '../../charts/race/CountryRaceView';
import CountryPartsDrill from '../../charts/race/CountryPartsDrill';
import { linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens } from '../../charts/useChartTokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, MEMBERSHIP_PREPARING, noMembersAt, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import type { CountryGroupReply, EquipmentProducedReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition';
import { countedSince, emptyRace, formatProduced, MEASURES, producedAt, producedModel, producedRequest, producedTooltips,
  raceEntries, rankProduced, requestParams, totalRuns, typeLabel, validateProducedReply } from './producedData';
import type { Measure } from './producedData';
import { producedDrillData, producedDrillNumbers } from './producedDrill';
import { COUNTS, countDecimals, mostDelivered, producedCaption, producedWindowUnavailable } from './producedView';
import type { Count } from './producedView';
import ProducedDrillTable from './ProducedDrillTable';
import { useProducedTypeList } from './useProducedTypes';
import './equipmentProduced.css';
import PreparedEquipmentProducedWidget from './PreparedEquipmentProducedWidget';
import { hasPreparedProduction, usePreparedProduction } from './usePreparedProduction.ts';
import { mostDeliveredPrefix } from './preparedProducedData.ts';
import { approximateAt, preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import { usePreparedProducedView } from './preparedProducedView';
import { historyProductionParity, preparedProductionParity } from './productionParity';
import { useParityRows } from '../../dashboard/parityRows';

/** Clients whose dashboard bridge predates the per-day measure and IC count: their
 * widgets show per month instead of per day (the caption says so) until it is rebuilt. */
const olderBridges = new WeakSet<object>();
const olderBridge = (error: string | undefined) => !!error && /unknown (variant|field) `(day|count)`/.test(error);

/** Equipment each country delivered: one type at a time in units, or the factory IC
 * put into equipment, as a race re-ranked every hour or as lines over time; a country's
 * breakdown lists every type it made with its models. */
export default function EquipmentProducedWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  return hasPreparedProduction(props.runtime.client) && options.view === 'race' && options.show !== 'breakdown'
    ? <PreparedEquipmentProducedWidget {...props} /> : <HistoryEquipmentProducedWidget {...props} />;
}

export function HistoryEquipmentProducedWidget(props: WidgetProps & { prepared?: boolean }) {
  const { widget, runtime, active, onDiagnostics, setOption } = props;
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'lines' ? 'lines' : 'race';
  const older = olderBridges.has(runtime.client as object);
  const measure: Measure = older && options.measure === 'day' ? 'month' : options.measure as Measure;
  const count: Count = options.count === 'ic' ? 'ic' : 'units';
  const included = useMemo(() => contentSet(options.included).map(contentKey), [options.included]);
  const refits = options.refits === true;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  // The moment a race shows: the playhead, the end of a range, or the branch end.
  const wantedMoment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groupReply = useCountryGroups(runtime, track, active && grouped);
  const retainedGroups = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const groupScope = JSON.stringify([track?.source.id, track?.branch.id, widget.countries]);
  const numericGroups = retainedGroups.update(groupScope, groupReply.data, groupNumbers);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], numericGroups, lastHour, definition.countries.max),
    [widget.countries, fallback, numericGroups, lastHour]);
  const asks = (params: { top: number | null; tags: readonly string[] } | null) => !!params && (params.top != null || params.tags.length > 0);
  const productRequest = (params: ReturnType<typeof producedRequest> | null) => track && params && asks(params)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced', params: requestParams(params) } : null;

  // In units, until a type is chosen, the type the countries shown delivered most.
  const pickingDefault = count === 'units' && !included.length;
  const defaults = useProduct<EquipmentProducedReply>(runtime.client,
    productRequest(pickingDefault && track ? producedRequest(track.branch.id, resolved, 'total', 'type', [], refits) : null), active, validateProducedReply);
  // Kept while a refresh of the same countries is pending, so the race stays drawn.
  const retainedDefaults = useMemo(() => new NumericReply<EquipmentProducedReply>(), [runtime.client]);
  const defaultScope = JSON.stringify([track?.source.id, track?.branch.id, widget.countries, refits, pickingDefault]);
  const defaultNumbers = retainedDefaults.update(defaultScope, defaults.data, producedDrillNumbers);
  const defaultReply = defaults.data ?? defaultNumbers;
  const defaultParams = track && pickingDefault && props.prepared ? producedRequest(track.branch.id, resolved, 'total', 'type', [], refits) : null;
  const prefixDefault = usePreparedProduction(runtime.client, track?.source, defaultParams && track ? {
    hour: track.branch.lastHour, selection: defaultParams, offset: '0', limit: 0, ranked: false,
  } : null, active && !defaultReply);
  const mostType = useMemo(() => defaultReply ? mostDelivered(producedModel(defaultReply)) : mostDeliveredPrefix(prefixDefault.data), [defaultReply, prefixDefault.data]);
  const provisionalChoice = pickingDefault && !defaultReply && !!mostType;
  const shownTypes = useMemo(() => pickingDefault ? mostType ? [mostType] : [] : included, [pickingDefault, mostType, included]);
  // A type's own models name its tooltip rows; all equipment in IC is split by type.
  const grouping = count === 'ic' && !shownTypes.length ? 'type' : 'variant';
  const params = track && !(pickingDefault && !mostType) ? producedRequest(track.branch.id, resolved, measure, grouping, shownTypes, refits, count) : null;
  const request = productRequest(params);
  const endpointParams = useMemo(() => params && props.prepared ? { ...params,
    tags: params.top === null ? rankedCandidates(resolved, wantedMoment, params.tags) : params.tags } : null,
  [params, props.prepared, resolved, wantedMoment]);
  const endpoint = usePreparedProducedView(runtime.client, track?.source, endpointParams, wantedMoment, active);
  // An unknown empty endpoint cannot tell an absent history from an unread
  // current value. The existing history product keeps that distinction.
  const preparedView = props.prepared === true && !endpoint.unavailable && !(endpoint.page?.totalCountries === 0 && !endpoint.page.known);
  const history = useProduct<EquipmentProducedReply>(runtime.client, preparedView ? null : request, active, validateProducedReply);
  const moment = preparedView ? endpoint.hour ?? wantedMoment : wantedMoment;
  const data = preparedView ? endpoint.reply : history.data;
  const error = preparedView ? endpoint.error : history.error;
  const retry = preparedView ? endpoint.retry : history.retry;
  const [, recheck] = useReducer((value: number) => value + 1, 0);
  useEffect(() => {
    if (!olderBridge(error) || measure !== 'day' || olderBridges.has(runtime.client as object)) return;
    olderBridges.add(runtime.client as object);
    recheck();
  }, [error, measure, runtime.client]);
  const numeric = useMemo(() => new NumericReply<EquipmentProducedReply>(), [runtime.client]);
  const pickScope = JSON.stringify([track?.source.id, track?.branch.id, view, widget.countries, params]);
  const pickNumbers = numeric.update(pickScope, data, producedDrillNumbers);
  const shownReply = data ?? pickNumbers;
  const model = useMemo(() => shownReply ? producedModel(shownReply) : null, [shownReply]);
  const decimals = countDecimals(count, shownReply);
  const pickGroups = grouped ? numericGroups : undefined;
  const pickOwner = useMemo(() => ({ scope: pickScope, numbers: pickNumbers, membership: pickGroups }), [pickScope, pickNumbers, pickGroups]);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  // The top N rank only countries that have produced something.
  const produced = resolved.top != null;
  // The hours a country is shown: in the top N (from the product), or a member of a
  // group counted at each hour; null shows it whenever recorded.
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;

  // The countries at the moment, ranked by the value shown.
  const candidates = useMemo(() => model ? rankedCandidates(resolved, moment, model.tags) : [], [model, resolved, moment]);
  const ranked = useMemo(() => model ? preparedView && endpoint.page ? endpoint.page.countries.flatMap(country => {
    const at = producedAt(model, country.tag, moment);
    return at?.total !== null && at ? [{ tag: country.tag, at: { ...at, total: at.total } }] : [];
  }) : rankProduced(model, candidates, moment, slots, produced) : [], [model, candidates, moment, slots, produced, preparedView, endpoint.page]);
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : lineTags;
  const entries = useMemo(() => preparedView ? endpoint.entries : model ? raceEntries(model, ranked, { split: false, bands: () => [], unknown: tokens.muted, decimals }) : [],
    [model, ranked, tokens.muted, decimals, preparedView, endpoint.entries]);
  // A top-N reply's samples are already cut to its hours; a group's are cut here.
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = totalRuns(model, tag, cutTo(tag), range, decimals);
    return runs.length ? [{ tag, runs }] : [];
  }), [model, view, lineTags, resolved.membership, range, decimals]); // eslint-disable-line react-hooks/exhaustive-deps

  // What is counted, in words: the pill names the equipment, the caption the window and date.
  const types = useProducedTypeList(runtime, track, active);
  const typeName = (key: string) => typeLabel(types.find(type => type.key === key) ?? { key, name: shownReply?.types.find(type => type.key === key)?.name
    ?? defaultReply?.keys.find(item => item.key === key)?.name ?? prefixDefault.data?.keys.find(item => item.key === key)?.name ?? null });
  const equipment = !shownTypes.length ? count === 'ic' ? 'All equipment' : 'Equipment'
    : shownTypes.length === 1 ? typeName(shownTypes[0]) : `${shownTypes.length.toLocaleString()} types`;
  const measureLabel = (MEASURES.find(item => item.value === measure) ?? MEASURES[0]).label;
  const captionRead = useProduct<EquipmentProducedReply>(runtime.client, track && preparedView && measure === 'total'
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced',
      params: requestParams(producedRequest(track.branch.id, { tags: [] }, 'total', 'type', [], true)) } : null, active, validateProducedReply);
  const since = captionRead.data ? countedSince(captionRead.data, moment) : shownReply ? countedSince(shownReply, moment) : null;
  // Before preparation reaches the moment asked, the race shows its prepared
  // prefix's last hour; the caption says the later days are being prepared.
  const preparing = preparedView && !!endpoint.page?.pending?.length && moment < wantedMoment;
  const caption = `${producedCaption(count, measure, view === 'race' ? dayLabel(moment) : null, since == null ? null : dayLabel(since))}${
    preparing ? '; later days are still being prepared' : ''}${provisionalChoice
      ? `; approximate type choice: ${equipment}, most delivered in the prepared totals` : ''}${preparedView && approximateAt(preparedPrefix(endpoint.page), moment)
      ? '; approximate while production joins and types are still being prepared' : ''}`;
  const what = `${count === 'ic' ? `IC, ${equipment === 'All equipment' ? 'all equipment' : equipment}` : equipment}, ${measureLabel.toLowerCase()}`;
  const typeChoices = useMemo(() => types.map(type => ({ value: setToken(type.key), label: typeLabel(type) })), [types]);
  const controls = <div className="produced-caption">
    {setOption ? <ContentSetPicker label="Equipment" automatic={count === 'ic' ? 'All equipment' : 'Most delivered'} choices={typeChoices}
      selected={contentSet(options.included)} single triggerClassName="produced-pill" trigger={<span>{equipment}</span>}
      onToggle={value => setOption('included', value)} onReset={() => setOption('included', '')} />
      : <span className="produced-pill"><span>{equipment}</span></span>}
    <span className="produced-caption-text">{caption}</span>
    {setOption && <Segmented value={count} options={COUNTS} label="Count" onChange={value => setOption('count', value)} />}
  </div>;

  const warnings = useMemo(() => [...(data?.warnings ?? []), ...(groupReply.data?.warnings ?? [])], [data, groupReply.data]);
  const none = noMembersAt(widget.countries, resolved, numericGroups, wantedMoment, lastHour);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see the equipment it produced." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (error && olderBridge(error) && count === 'ic') return <WidgetState kind="unavailable" message="Counting in IC needs the updated dashboard bridge." />;
    if (error && !(olderBridge(error) && measure === 'day')) return <WidgetState kind="error" detail={error} onRetry={retry} />;
    if (defaults.error && !mostType) return <WidgetState kind="error" detail={defaults.error} onRetry={defaults.retry} />;
    if (grouped && groupReply.error) return <WidgetState kind="error" detail={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="layers" message={resolved.reason} />;
    // A group with no member found yet asks for nothing, so no reply will come.
    if (none) return <NoMembersState reason={none.reason} icon="layers" />;
    // A group's members at hours still being prepared are not known yet: no data is not proof of none.
    const branchKnown = resolved.known(track.branch.firstHour, track.branch.lastHour);
    if (pickingDefault && defaultReply && !mostType) return branchKnown ? <WidgetState kind="unavailable" message="No production data for these countries." />
      : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
    if (pickingDefault && prefixDefault.data && !mostType) return <WidgetState kind="loading" label="Most delivered type is still being prepared." />;
    // A group's members are not known until its history arrives; nothing is shown as empty meanwhile.
    if (!shownReply || !model || (grouped && !numericGroups)) return <WidgetState kind="loading" label="Loading equipment produced…" />;
    if (!shownReply.reads.length && !preparedView) return <WidgetState kind="unavailable" message="Production data is not in this recording." />;
    if (!produced && !model.tags.length) return branchKnown ? <WidgetState kind="unavailable" message={producedWindowUnavailable(measure)} />
      : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
    return null;
  })();
  // `name` names the country as recorded at the hour the tooltip is about.
  const tooltipAt = (tag: string, hour: number, name: string) => {
    if (preparedView) return endpoint.tooltip(tag, hour, name, what, since);
    if (!model) return null;
    const at = isMember(membershipOf(tag) ?? undefined, hour) ? producedAt(model, tag, hour) : null;
    return at ? producedTooltips(model, tag, name, at, hour, dayLabel(hour), [], what, decimals) : null;
  };
  const nothingYet = produced && model && emptyRace(model, moment) === 'nothing'
    ? measure === 'total' ? `No production by ${dayLabel(moment)}.` : measure === 'day' ? `No production on ${dayLabel(moment)}.` : 'No production in the previous month.'
    : resolved.known(moment) ? producedWindowUnavailable(measure) : MEMBERSHIP_PREPARING;

  // The breakdown asks for the country's every model, in the count shown; its chart
  // stacks the selected type's models, or every type in IC until one is chosen.
  const [drillType, setDrillType] = useState<{ scope: string; type: string } | null>(null);
  const drillScope = JSON.stringify([track?.source.id, track?.branch.id, count, measure, included, refits]);
  const chartType = drillType?.scope === drillScope ? drillType.type : shownTypes.length === 1 ? shownTypes[0] : null;
  const chartSpec = useMemo(() => ({ count, decimals: countDecimals(count), type: count === 'units' ? chartType ?? shownTypes[0] ?? null : chartType }),
    [count, chartType, shownTypes]);
  const drillParams = useCallback((tag: string) => requestParams(producedRequest(track!.branch.id, { tags: [tag] }, measure, 'variant', [], refits, count)),
    [track, measure, refits, count]);
  const decodeDrill = useCallback((reply: EquipmentProducedReply) => producedDrillData(producedModel(reply), { ...chartSpec, decimals: countDecimals(count, reply) }),
    [chartSpec, count]);
  const pickDrillTag = (hour: number) => model ? mixCountry(widget.countries, fallback,
    rankProduced(model, rankedCandidates(resolved, hour, model.tags), hour, slots, produced)) : null;

  const parityRoot = import.meta.env.MODE === 'parity' ? useRef<HTMLDivElement>(null) : null;
  if (import.meta.env.MODE === 'parity') {
    useParityRows(parityRoot!, () => preparedView ? preparedProductionParity(endpoint.fullPage, endpoint.parityComplete)
      : historyProductionParity(model, ranked, !!model && state === null), preparedView ? endpoint.parityPreparation : undefined);
  }
  return <CountryPartsDrill {...props} widget={widget} runtime={runtime} active={active} onDiagnostics={onDiagnostics} track={track}
    options={options} extent={extent} playhead={playhead} moment={moment} product="equipment-produced" params={drillParams}
    validate={validateProducedReply} decode={decodeDrill} numbers={producedDrillNumbers} defaultTag={mixTag} pickTag={pickDrillTag}
    title="Equipment types" measure={measureLabel} positiveGeometry warnings={warnings} state={state}
    resetKey={JSON.stringify([track?.source.id, track?.branch.id, measure, count, included, refits, widget.countries])}
    details={({ tag, hour, tagAt, key }) => track && <ProducedDrillTable runtime={runtime} track={track} active={active} tag={tag} hour={hour} tagAt={tagAt} narrow={narrowTableOf(options)}
      measure={measure} refits={refits} count={count} chartType={chartSpec.type} chartKey={key}
      chartTitle={chartSpec.type == null ? `IC put into equipment ${measureLabel.toLowerCase()}, by type`
        : `${typeName(chartSpec.type)}: ${count === 'ic' ? 'IC put in' : 'units delivered'} ${measureLabel.toLowerCase()}, by model`}
      onChart={type => setDrillType({ scope: drillScope, type })} />}
    main={(onPick, visible) => <div className="produced-main" ref={parityRoot}>
      {track && controls}
      <CountryRaceView runtime={runtime} track={track} active={active && visible} options={options} view={view} extent={extent} playhead={playhead} state={state}
        remoteRows={preparedView ? endpoint.remoteRows : undefined}
        moment={moment} slots={slots} ranked={entries} lines={lines} countryTags={shownTags} partsLabel="Equipment"
        label={`${equipment}: ${caption}`} empty={nothingYet} formatValue={value => formatProduced(value, decimals)} tooltip={tooltipAt}
        lineTooltip={(hour, _tags, labelAt, country) => [linesTooltip(hour, lines, labelAt, country, dayLabel(hour), what, value => formatProduced(value, decimals))]}
        labels={{ race: `${equipment}: ${caption}`, lines: `${equipment}: ${caption}`, actions: 'Equipment produced actions' }}
        presentation={{ exact: false, held: false, raceEmptyState: false, linePadding: CHART_LAYOUT.countryEndLabels, resetTimeView: true }}
        pickOwner={pickOwner} lineEmpty={{ icon: 'layers', message: resolved.known(extent[0], extent[1]) ? 'No countries to draw. Choose countries in the widget settings.' : MEMBERSHIP_PREPARING }}
        resetKey={`${track?.source.id ?? ''}:${track?.branch.id ?? ''}:${measure}:${count}:${shownTypes.join(' ')}`} onPick={onPick} />
    </div>} />;
}
