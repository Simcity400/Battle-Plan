import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import WidgetState from '../../components/WidgetState';
import CountryPicker from '../../components/CountryPicker';
import type { IconName } from '../../components/Icons';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { partsRaceAt, partsRaceLineRuns, partsRaceMixBands, partsRaceModel, partsRacePartName, partsRaceEntries, partsRaceSegmentAt,
  partsRaceTooltips, formatPartsRaceValue, hasPartsRaceValues, rankPartsRace } from '../../dashboard/partsRace';
import { countryMatchFilter, isMember, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { NumericReply } from '../../dashboard/numericReply';
import type { CountryGroupReply } from '../../generated/dashboard';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { dayLabel } from '../draw';
import DrillHost from '../drill/DrillHost';
import DrillParts from '../drill/DrillParts';
import type { Span } from '../series';
import { CHART_LAYOUT } from '../tokens';
import CountryRaceView from './CountryRaceView';
import type { CountryRaceViewProps } from './CountryRaceView';
import { partColours } from './countryRace';
import { linesTooltip } from './lines';
import { mixCountry } from './options';
import type { RaceView } from './options';
import { CLOSED_PARTS_RACE, nativePartsRaceNumbers, partsRaceDrillActors, partsRaceDrillData, reducePartsRaceDrill, UNATTRIBUTED_PART } from './partsRaceDrill';
import type { NativePartsRaceReply } from './partsRaceDrill';
import { bindCountryParts, countryPartsSelection, pinNativeCountryParts, savedCountryPartsSelection } from './countryPartsDrillData';
import { useChartTokens } from '../useChartTokens';

export interface PartsRaceViewConfig {
  product: string; title: string; subject: string; icon: IconName;
  options: readonly OptionField[];
  request: (branchId: string, countries: { tags: readonly string[]; top?: number }, options: Readonly<Record<string, OptionValue>>) => unknown;
  validate: (data: unknown) => NativePartsRaceReply;
  measure: (options: Readonly<Record<string, OptionValue>>) => string;
  ownership: (options: Readonly<Record<string, OptionValue>>) => string;
  partsLabel: (options: Readonly<Record<string, OptionValue>>) => string;
  resetKey: (options: Readonly<Record<string, OptionValue>>) => readonly OptionValue[];
  chartOptions?: (options: Readonly<Record<string, OptionValue>>) => Readonly<Record<string, OptionValue>>;
  zeroMix: (options: Readonly<Record<string, OptionValue>>) => string;
  formatValue?: (value: number, options: Readonly<Record<string, OptionValue>>) => string;
  formatTick?: (options: Readonly<Record<string, OptionValue>>) => ((value: number, ticks: readonly number[]) => string) | undefined;
  referenceLabel?: (options: Readonly<Record<string, OptionValue>>) => string | undefined;
  note?: (options: Readonly<Record<string, OptionValue>>) => string;
}
const NO_WARNINGS: string[] = [];

/** Shared parts-race playback and group handling; CountryRaceView owns presentation. */
export default function PartsRaceView({ widget, runtime, active, countrySlot = null, onDiagnostics, config }: WidgetProps & { config: PartsRaceViewConfig }) {
  const options = useMemo(() => resolveOptions(config.options, widget.options), [config.options, widget.options]);
  const chartOptions = useMemo(() => config.chartOptions?.(options) ?? options, [config.chartOptions, options]);
  const view = options.view as RaceView;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const [firstHour, lastExtentHour] = extent;
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const stores = useMemo(() => ({ main: new NumericReply<NativePartsRaceReply>(), drill: new NumericReply<NativePartsRaceReply>(),
    mainLabels: new NumericReply<Pick<NativePartsRaceReply, 'parts' | 'warnings'>>(), drillLabels: new NumericReply<Pick<NativePartsRaceReply, 'parts' | 'warnings'>>(),
    groups: new NumericReply<CountryGroupReply>(), groupWarnings: new NumericReply<string[]>() }), [runtime.client]);
  const sourceScope = JSON.stringify([track?.source.id, track?.branch.id]);
  const incomingGroups = useCountryGroups(runtime, track, active && grouped);
  const groupData = stores.groups.update(sourceScope, incomingGroups.data, groupNumbers);
  const groupWarnings = stores.groupWarnings.update(sourceScope, incomingGroups.data?.warnings, reply => reply);
  const groupReply = { ...incomingGroups, data: groupData };
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groupReply.data, lastHour]);
  const request = track && (resolved.top || resolved.tags.length) ? { sourceId: track.source.id, revision: track.source.revision,
    product: config.product, params: config.request(track.branch.id, resolved, options) } : null;
  const incoming = useProduct(runtime.client, request, active, config.validate);
  const requestScope = JSON.stringify([sourceScope, config.product, request?.params]);
  const numbers = stores.main.update(requestScope, incoming.data, nativePartsRaceNumbers);
  const decorations = useMemo(() => incoming.data ? { parts: incoming.data.parts, warnings: incoming.data.warnings } : undefined, [incoming.data]);
  const labels = stores.mainLabels.update(requestScope, decorations, reply => reply);
  const data = useMemo(() => numbers ? { ...numbers, ...labels } : undefined, [numbers, labels]);
  const { error, retry } = incoming;
  const numericModel = useMemo(() => numbers ? partsRaceModel(numbers) : null, [numbers]);
  const model = useMemo(() => numericModel && data ? { ...numericModel, reply: data } : null, [numericModel, data]);
  const identities = useMemo(() => numbers ? countryIdentityIndex(numbers.series) : null, [numbers]);
  const drawing = useMemo(() => resolved.top != null ? resolved : projectResolvedCountries(resolved,
    countryMatchFilter(widget.countries, fallback ? [fallback] : [], groupData, lastHour, identities, track?.branch.firstHour)),
    [resolved, widget.countries, fallback, groupData, lastHour, identities, track?.branch.firstHour]);
  const [storedDrill, dispatchDrill] = useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE);
  const pinned = options.show === 'breakdown';
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id, config.product, config.resetKey(options), widget.countries,
    track?.source.metadata.campaignKey, widget.options?.holder, widget.options?.drillOriginHour]);
  const contextOwner = useMemo(() => ({ client: runtime.client, resetKey, pinned }), [runtime.client, resetKey, pinned]);
  const drill = storedDrill.owner === contextOwner ? storedDrill : CLOSED_PARTS_RACE;
  usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown != null && drill.follow));
  const clockHour = track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : null;
  const liveHour = clockHour != null && track && clockHour >= track.branch.firstHour && clockHour <= track.branch.lastHour ? clockHour : null;
  const drillAnchor = useRef<HTMLDivElement>(null);
  const tokens = useChartTokens();
  useEffect(() => dispatchDrill({ type: 'close' }), [resetKey, options.show, runtime.client]);
  const available = useMemo(() => numericModel ? hasPartsRaceValues(numericModel) : false, [numericModel]);
  const slots = drawing.top ?? Math.max(1, drawing.tags.length);
  const membershipOf = useCallback((tag: string): readonly Span[] | null =>
    numericModel?.series.get(tag)?.series.membership ?? drawing.membership?.get(tag) ?? null, [numericModel, drawing.membership]);
  const cutTo = useCallback((tag: string): readonly Span[] | null => drawing.membership?.get(tag) ?? null, [drawing.membership]);
  const ranked = useMemo(() => numericModel ? rankPartsRace(numericModel, rankedCandidates(drawing, moment, numericModel.tags), moment, slots) : [],
    [numericModel, drawing, moment, slots]);
  const lineTags = useMemo(() => drawing.top != null ? numericModel?.tags ?? [] : drawing.tags, [drawing, numericModel]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const candidateOriginTag = pinned ? widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : mixTag : drill.breakdown;
  const restored = useMemo(() => savedCountryPartsSelection(widget, track),
    [contextOwner, track?.branch.firstHour, track?.branch.lastHour]); // eslint-disable-line react-hooks/exhaustive-deps
  const initialOrigin = useMemo(() => ({ hour: typeof widget.options?.drillOriginHour === 'number' ? widget.options.drillOriginHour : moment,
    selection: undefined as ReturnType<typeof countryPartsSelection> | undefined }), [contextOwner]); // eslint-disable-line react-hooks/exhaustive-deps
  const originTag = pinned ? restored?.tag ?? initialOrigin.selection?.tag ?? candidateOriginTag : candidateOriginTag;
  const drillFollowing = drill.breakdown != null ? drill.follow : pinned && widget.time.mode === 'follow';
  const missingPinnedDate = pinned && drill.breakdown == null && (widget.time.mode === 'pin' || widget.time.mode === 'range') && !window;
  const drillAt = Math.floor(drill.choices ? drill.hour ?? moment : drillFollowing ? liveHour ?? clockHour ?? moment : drill.hour ?? moment);
  // A country's drill follows its census even after it leaves the race's top N.
  const drillRequest = track && originTag ? { sourceId: track.source.id, revision: track.source.revision,
    product: config.product, params: config.request(track.branch.id, { tags: [originTag] }, options) } : null;
  const incomingDrill = useProduct(runtime.client, drillRequest, active, config.validate);
  const drillScope = JSON.stringify([sourceScope, config.product, drillRequest?.params]);
  const independentNumbers = stores.drill.update(drillScope, incomingDrill.data, nativePartsRaceNumbers);
  const drillDecorations = useMemo(() => incomingDrill.data ? { parts: incomingDrill.data.parts, warnings: incomingDrill.data.warnings } : undefined, [incomingDrill.data]);
  const drillLabels = stores.drillLabels.update(drillScope, drillDecorations, reply => reply);
  const independentData = useMemo(() => independentNumbers ? { ...independentNumbers, ...drillLabels } : undefined, [independentNumbers, drillLabels]);
  const drillNumbers = independentNumbers;
  const drillData = independentData;
  const drillReply = { ...incomingDrill, data: independentData };
  const drillNumericModel = useMemo(() => drillNumbers ? partsRaceModel(drillNumbers) : null, [drillNumbers]);
  const drillModel = useMemo(() => drillNumericModel && drillData ? { ...drillNumericModel, reply: drillData } : null, [drillNumericModel, drillData]);
  if (pinned && !missingPinnedDate && initialOrigin.selection === undefined && drillNumbers && originTag) {
    initialOrigin.selection = countryPartsSelection(drillNumbers.series, originTag, initialOrigin.hour);
  }
  const selection = pinned ? restored === undefined ? initialOrigin.selection : restored : drill.selection;
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groupWarnings ?? []), ...(drillReply.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groupWarnings, drillReply.data, onDiagnostics]);

  const split = options.split === true;
  const range = useMemo(() => ({ firstHour, lastHour: lastExtentHour }), [firstHour, lastExtentHour]);
  const lines = useMemo(() => !numericModel || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = partsRaceLineRuns(numericModel, tag, cutTo(tag), range);
    return runs.length ? [{ tag, runs }] : [];
  }), [numericModel, view, lineTags, cutTo, range]);
  const segmentAt = useCallback((tag: string, hour: number) => partsRaceSegmentAt(numericModel, tag, hour), [numericModel]);
  const parts = useMemo(() => (model?.reply.parts ?? []).map(part => ({ id: part.key, label: partsRacePartName(part) })), [model]);
  const formatValue = useCallback((value: number) => config.formatValue?.(value, options) ?? formatPartsRaceValue(value), [config.formatValue, options]);
  const entries = useMemo(() => model ? partsRaceEntries(model, ranked, split, formatValue).map(entry => ({ ...entry,
    entityKey: identities?.keyAt(entry.tag, moment) ?? JSON.stringify(['unknown', entry.tag]), hour: moment })) : [],
    [model, ranked, split, formatValue, identities, moment]);
  const mixCut = mixTag ? cutTo(mixTag) : null;
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const bands = useMemo(() => numericModel && view === 'mix' && mixTag ? partsRaceMixBands(numericModel, mixTag, mixCut, range) : null,
    [numericModel, view, mixTag, mixCut, range]);
  const drillParts = useMemo(() => [...(drillModel?.reply.parts ?? []).map(part => ({ id: part.key, label: partsRacePartName(part) })),
    UNATTRIBUTED_PART], [drillModel]);
  const detailExtent = useMemo<[number, number]>(() => track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1],
    [track?.branch.firstHour, track?.branch.lastHour]); // eslint-disable-line react-hooks/exhaustive-deps
  const detailRange = useMemo(() => ({ firstHour: detailExtent[0], lastHour: detailExtent[1] }), [detailExtent]);
  const nativeDetail = useMemo(() => drillModel && drillNumbers ? partsRaceDrillData(drillModel, drillNumbers.series, () => null) : null,
    [drillModel, drillNumbers]);
  const binding = useMemo(() => nativeDetail && originTag ? bindCountryParts(nativeDetail, originTag,
    selection?.hour ?? initialOrigin.hour, selection?.key ?? null) : null, [nativeDetail, originTag, selection, initialOrigin]);
  const drillTag = missingPinnedDate ? null : binding?.tagAt(drillAt) ?? null;
  const drillBands = useMemo(() => binding && originTag ? binding.data.bands(originTag, detailRange) : null,
    [binding, originTag, detailRange]);
  const colours = useMemo(() => partColours(drillParts, tokens.categories), [drillParts, tokens.categories]);
  const drillValue = drillModel && drillTag ? partsRaceAt(drillModel, drillTag, drillAt) : null;
  const remainder = drillValue?.total == null ? null : drillValue.total - drillValue.values.reduce<number>((sum, value) => sum + (value ?? 0), 0);
  const headerReason = !active ? 'The breakdown is inactive.' : missingPinnedDate ? 'The chosen moment is unavailable.' : pinned && restored === undefined
    && (Object.hasOwn(widget.options ?? {}, 'drillOriginHour') || widget.time.mode !== 'campaign')
    ? 'This older pin has no saved native holder.' : !selection ? 'The saved native holder selection is unavailable.'
      : !drillTag ? 'The selected native country identity is unknown or conflicting at this hour.' : null;
  const headerWanted = useMemo(() => pinned && countrySlot && headerReason === null && drillTag ? [{ tag: drillTag, hour: drillAt }] : [],
    [pinned, countrySlot, headerReason, drillTag, drillAt]);
  const headerLabels = useCountryLabelsAt(runtime.client, track, headerWanted, active && pinned && !!countrySlot);
  const header = pinned && countrySlot && createPortal(<DatedCountryChip label={headerReason === null && drillTag
    ? headerLabels.labelAt(drillAt)(drillTag) : null} hour={missingPinnedDate ? null : drillAt} reason={headerReason} />, countrySlot);
  const mix = useMemo(() => {
    if (!model || !mixTag || !bands) return null;
    const at = isMember(mixMembership ?? undefined, moment) ? partsRaceAt(model, mixTag, moment) : null;
    return { tag: mixTag, bands, compactStack: true, parts: parts.map((part, index) => ({ ...part,
      value: at?.values[index] == null ? 'Not known' : formatValue(at.values[index]!) })) };
  }, [model, mixTag, bands, mixMembership, moment, parts, formatValue]);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon={config.icon} message={`Load a campaign to see its ${config.subject}.`} />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable"
      message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
    if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
    if (grouped && groupReply.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon={config.icon} message={resolved.reason} />;
    if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label={`Loading ${config.subject}…`} />;
    if (!available) return <WidgetState kind="unavailable" message={`Not in this recording: no ${config.subject} counts were recorded for these countries in this branch.`} />;
    if (!pinned && view === 'mix' && mixTag && !parts.length && model.series.get(mixTag)?.series.runs.some(run =>
      run.total === 0 && run.lastHour >= extent[0] && run.firstHour <= extent[1])) {
      return <WidgetState kind="empty" icon={config.icon} message={config.zeroMix(options)} />;
    }
    return undefined;
  })();
  const measure = config.measure(options);
  const ownership = config.ownership(options);
  const partsLabel = config.partsLabel(options);
  const referenceLabel = config.referenceLabel?.(options);
  const membershipKey = useMemo(() => JSON.stringify([...(drawing.membership ?? [])]), [drawing.membership]);
  const geometryKey = JSON.stringify([extent, chartOptions.playback, chartOptions.windowDays, chartOptions.afterPlayhead,
    chartOptions.log, chartOptions.yMin, chartOptions.yMax]);
  const mainScope = JSON.stringify([sourceScope, config.product, config.resetKey(options), request?.params,
    widget.countries, membershipKey, view, view === 'mix' ? mixTag : null, geometryKey]);
  const mainPickOwner = useMemo(() => ({ client: runtime.client, scope: mainScope, numbers }), [runtime.client, mainScope, numbers]);
  const choices = track && !pinned && drill.choices?.key === resetKey && drill.pickOwner === mainPickOwner ? drill.choices : null;
  const detailScope = JSON.stringify([sourceScope, config.product, config.resetKey(options),
    drillRequest?.params, 'mix', originTag, selection, detailExtent, chartOptions.playback, chartOptions.afterPlayhead]);
  const detailPickOwner = useMemo(() => ({ client: runtime.client, scope: detailScope, numbers: drillNumbers }),
    [runtime.client, detailScope, drillNumbers]);
  const mainPending = !!numbers && (!incoming.data || grouped && !incomingGroups.data);
  const drillPending = !!independentNumbers && !incomingDrill.data;
  const controls = useRef({ active, contextOwner, mainPickOwner, detailPickOwner });
  controls.current = { active, contextOwner, mainPickOwner, detailPickOwner };
  useEffect(() => () => { controls.current.active = false; }, []);
  const currentMain = () => controls.current.active && controls.current.contextOwner === contextOwner && controls.current.mainPickOwner === mainPickOwner;
  const currentDetail = () => controls.current.active && controls.current.contextOwner === contextOwner && controls.current.detailPickOwner === detailPickOwner;
  const [pickedPart, setPickedPart] = useState<{ owner: unknown; id: string } | null>(null);
  const selectedPart = pickedPart?.owner === detailPickOwner ? drillParts.find(part => part.id === pickedPart.id) : undefined;
  const selectedIndex = selectedPart ? drillParts.indexOf(selectedPart) : -1;
  const historyParts = selectedPart ? [selectedPart] : drillParts;
  const historyBands = selectedIndex >= 0 && drillBands ? [drillBands[selectedIndex]] : drillBands;
  const common: CountryRaceViewProps = { runtime, track, active, options: chartOptions, view, extent, playhead, moment, slots,
    pickOwner: mainPickOwner,
    beforeChart: mainPending ? <WidgetState kind="loading" label={`Updating ${config.subject}…`} /> : undefined,
    ranked: entries, lines, parts, mix, segmentAt, countryTags: shownTags, label: config.title,
    empty: `No ${config.subject} recorded on ${dayLabel(moment)}.`, formatValue, formatTick: config.formatTick?.(options),
    partsLabel, raceLegendParts: split ? parts : [],
    labels: { race: `${config.title}, ${measure}, ${dayLabel(moment)}`, lines: `${config.title} over time, ${measure}`,
      mix: name => `${config.title} of ${name}, ${partsLabel.toLowerCase()}`, actions: `${config.title} actions` },
    presentation: { exact: false, held: false, lineColourHours: 'labels',
      tooltipKeep: 1, linePadding: CHART_LAYOUT.countryEndLabels, strongMixName: true, windowedLegend: true, mixKeyHours: 'starts',
      raceEmptyState: false, resetTimeView: true },
    lineEmpty: { icon: config.icon, message: 'No countries to draw. Choose countries in the widget settings.' },
    mixEmpty: { icon: config.icon, message: widget.countries.kind === 'group'
      ? `No country of this group was recorded on ${dayLabel(moment)}.` : 'Choose a country in the widget settings.',
      unavailable: name => `No ${config.subject} mix was recorded for ${name} in this branch.` },
    tooltip: (tag, hour, name, colours) => {
      if (!model) return null;
      const at = isMember(membershipOf(tag) ?? undefined, hour) ? partsRaceAt(model, tag, hour) : null;
      return at ? partsRaceTooltips({ model, name, at, date: dayLabel(hour), measure, ownership, colours,
        note: config.note?.(options), formatValue, referenceLabel: config.referenceLabel?.(options) }) : null;
    },
    lineTooltip: (hour, _tags, labelAt, country) => [linesTooltip(hour, lines, labelAt, country, dayLabel(hour), `${measure} · ${ownership}`, formatValue)],
    resetKey,
  };
  const open = (hour: number, tag?: string, candidates?: readonly string[]) => {
    if (!currentMain()) return;
    const actors = partsRaceDrillActors(tag, candidates, () => model ? mixCountry(widget.countries, fallback,
      rankPartsRace(model, rankedCandidates(drawing, hour, model.tags), hour, slots)) : null);
    if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour, owner: contextOwner, pickOwner: mainPickOwner });
    else if (actors.length) {
      const selected = numbers && countryPartsSelection(numbers.series, actors[0], hour);
      dispatchDrill({ type: 'open', breakdown: actors[0], hour, selection: selected ?? undefined, owner: contextOwner,
        playing: runtime.clock.getSnapshot().playing, playhead: liveHour });
    }
    else if (candidates !== undefined) dispatchDrill({ type: 'close' });
  };
  const follow = () => {
    if (currentDetail() && originTag && liveHour != null) dispatchDrill({ type: 'open', breakdown: originTag,
      hour: drillAt, playing: true, playhead: liveHour, selection: selection ?? undefined, owner: contextOwner });
  };
  const breakdown = <CountryRaceView {...common} pickOwner={detailPickOwner} view="mix" moment={drillAt} extent={detailExtent}
    playhead={drillAt} countryTags={drillTag ? [drillTag] : []}
    state={!track || missingPinnedDate ? state : drillReply.error ? <WidgetState kind="error" message={drillReply.error} onRetry={drillReply.retry} />
      : originTag && !drillModel ? <WidgetState kind="loading" label={`Loading ${partsLabel.toLowerCase()}…`} /> : undefined}
    parts={historyParts} selectedHour={drillAt} segmentAt={(_, hour) => binding?.data.segmentAt(originTag ?? '', hour) ?? null}
    countryLookHour={(tag, hour) => tag && binding?.tagAt(hour) === tag ? binding.data.readHour(tag, hour) : null}
    mix={originTag && historyBands ? { tag: drillTag ?? originTag, tagAt: hour => binding?.tagAt(hour) ?? null, bands: historyBands, compactStack: true,
      parts: historyParts.map(part => ({ ...part, value: part.id === ''
        ? remainder == null ? 'Not known' : formatValue(remainder)
        : drillValue?.values[drillParts.indexOf(part)] == null ? 'Not known' : formatValue(drillValue.values[drillParts.indexOf(part)]!) })) } : null}
    tooltip={(_tag, hour, name, colours) => {
      const alias = binding?.tagAt(hour);
      const at = drillModel && alias ? partsRaceAt(drillModel, alias, hour) : null;
      if (!drillModel || !at) return null;
      const remainder = at.total == null ? null : at.total - at.values.reduce<number>((sum, value) => sum + (value ?? 0), 0);
      return [...partsRaceTooltips({ model: drillModel, name, at, date: dayLabel(hour), measure, ownership, colours,
        note: config.note?.(options), formatValue, referenceLabel })].map(form => ({ ...form,
        rows: [...form.rows, ...(remainder ? [{ label: UNATTRIBUTED_PART.label, value: formatValue(remainder), swatch: colours.get('') }] : [])] }));
    }}
    beforeChart={<div className="parts-race-details">
      {drillPending && <WidgetState kind="loading" label={`Updating ${partsLabel.toLowerCase()}…`} />}
      {!drillTag && <p className="drill-empty">Selected country unavailable at this date.</p>}
      {pinned && <p className="drill-empty">{dayLabel(drillAt)} {!drillFollowing && liveHour != null
        && <button type="button" className="button small" onClick={follow}>Follow playback</button>}</p>}
      {!pinned && track && model && model.tags.length > 1 && <div className="parts-race-country">Country
        <CountryPicker countries={track.source.metadata.countries} allowedTags={model.tags} selectedTag={drillTag} playerCountryId={track.source.metadata.playerCountryId}
          source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: drillAt }} onChange={tag => {
            const selected = tag != null && numbers ? countryPartsSelection(numbers.series, tag, drillAt) : undefined;
            if (currentDetail() && selected) dispatchDrill({ type: 'open', breakdown: selected.tag, hour: drillAt,
              playing: drillFollowing, playhead: drillFollowing ? liveHour : null, selection: selected, owner: contextOwner });
          }} />
      </div>}
      <DrillParts virtual active={active} parts={drillParts.slice(0, -1).map((part, index) => ({ ...part,
        value: drillValue?.values[index] ?? null, color: colours.get(part.id), muted: drillModel?.reply.parts[index].unresolved }))}
        onPick={id => { if (currentDetail()) setPickedPart({ owner: detailPickOwner, id }); }}
        format={formatValue} total={drillValue?.total ?? null}
        unattributed={remainder == null ? undefined : { ...UNATTRIBUTED_PART, value: remainder, color: colours.get('') }}
        totalRow={{ label: measure, value: drillValue?.total == null ? 'Not recorded' : formatValue(drillValue.total) }} />
      {selectedPart && <p className="drill-empty">History: {selectedPart.label} <button type="button" className="button small"
        onClick={() => { if (currentDetail()) setPickedPart(null); }}>All parts</button></p>}
      {drillParts.slice(0, -1).some((_, index) => drillValue?.values[index] == null) && <p className="drill-empty">Not recorded: {drillParts.slice(0, -1)
        .filter((_, index) => drillValue?.values[index] == null).map(part => part.label).join(', ')}.</p>}
      {referenceLabel && <p className="drill-empty">{referenceLabel}: {drillValue?.run.referenceTotal == null
        ? 'Not known' : formatValue(drillValue.run.referenceTotal)}.</p>}
    </div>}
    onPick={originTag ? hour => { if (currentDetail()) dispatchDrill({ type: 'open', breakdown: originTag, hour, playing: false,
      playhead: null, selection: selection ?? undefined, owner: contextOwner }); } : undefined} />;
  if (pinned) return <div className="parts-race-drill">{header}{breakdown}</div>;
  const drillOpen = choices != null || drill.breakdown != null;
  return <div className="parts-race" ref={drillAnchor}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={drillAnchor}
      title={partsLabel} day={dayLabel(drillAt)} following={drillFollowing} canFollow={choices == null && liveHour != null}
      returnFocus=".country-race-chart canvas" onClose={() => { if (currentDetail()) dispatchDrill({ type: 'close' }); }}
      onFollow={follow}
      onPin={track && selection ? () => { if (currentDetail()) runtime.addWidgetBelow(widget.id,
        pinNativeCountryParts(widget, track, selection, drillAt, drillFollowing, partsLabel)); } : undefined}
      main={<CountryRaceView {...common} state={state} active={active && (!drillOpen || options.drill === 'popout')} onPick={open} />}>
      {choices && track ? <div className="parts-race-details">
        <p className="drill-empty">Choose a country for this moment.</p>
        <CountryPicker countries={track.source.metadata.countries} allowedTags={choices.tags} selectedTag={null} playerCountryId={null}
          source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: drillAt }}
          onChange={tag => {
            const selected = tag != null && numbers ? countryPartsSelection(numbers.series, tag, drillAt) : undefined;
            if (currentMain() && currentDetail() && tag != null) dispatchDrill({ type: 'choose', key: choices.key, hour: drillAt,
              tag, selection: selected ?? undefined, owner: contextOwner });
          }} />
      </div> : breakdown}
    </DrillHost>
  </div>;
}
