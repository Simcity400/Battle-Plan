import FollowControl from '../drill/FollowControl';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import { useFactionColours } from '../../dashboard/FactionColours';
import { factionLabelAt, factionLines, factionMemberLabelLimit, factionNameAt, factionTooltipForms, factionTotalsModel,
  factionVisibleMemberTags } from '../../dashboard/factionTotals';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { usePreparedFactionTotals } from '../../dashboard/preparedFactionTotals';
import { useParityRows } from '../../dashboard/parityRows';
import { NumericReply } from '../../dashboard/numericReply';
import type { FactionTotalsParams, FactionTotalsReply } from '../../generated/dashboard';
import { dayLabel } from '../draw';
import { CHART_LAYOUT } from '../tokens';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import DrillHost from '../drill/DrillHost';
import { reduceDrill } from '../drill/drillState';
import WindowedRows from '../WindowedRows';
import { runAt } from '../series';
import CountryRaceView from './CountryRaceView';
import FactionPartsView from './FactionPartsView';
import { chooseFaction, factionNumbers, factionParts, factionPick, factionSelection, FACTION_SELECTION, pinFactionParts, savedFactionSelection } from './factionParts';
import type { FactionPick } from './factionParts';
import { factionRaceRequest, SMOOTH_TOTALS_PRESENTATION } from './factionRaceData';
import { raceSeriesIndex, seriesAt, seriesRaceEntries } from './series';

const NO_WARNINGS: string[] = [];

interface FactionRaceViewProps extends WidgetProps {
  options: Readonly<Record<string, OptionValue>>;
  source: FactionTotalsParams['source'];
  max: number;
  label: string;
  measureLabel: (unit: string | null) => string;
  formatValue: (value: number) => string;
  note?: string;
  unavailableReason?: string;
  /** Territory's existing Share keeps its own presentation and uses this drill. */
  renderMain?: (onPick: (hour: number, id?: string, choices?: readonly string[]) => void, pickOwner: unknown,
    onDiagnostics: (warnings: string[]) => void, active: boolean) => ReactNode;
}

/** Territory and Faction strength wire only their measure; this view owns
 * faction requests, identities, exact member tooltips and race/line presentation. */
export default function FactionRaceView({ widget, runtime, active, onDiagnostics, setOption, options, source, max, label, measureLabel,
  formatValue, note, unavailableReason, renderMain }: FactionRaceViewProps) {
  const [pick, setPick] = useState<FactionPick | null>(null);
  const [query, setQuery] = useState('');
  const [mainWarnings, setMainWarnings] = useState<readonly string[]>([]);
  const reportMain = useCallback((warnings: string[]) => setMainWarnings(before => before.length === warnings.length
    && before.every((warning, index) => warning === warnings[index]) ? before : warnings), []);
  const anchor = useRef<HTMLDivElement>(null);
  const pinned = options.show === 'members';
  const playbackHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || (pick?.follow ?? false)));
  const viewport = useViewportHeight();
  const view = options.view === 'lines' ? 'lines' : 'race';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window
    ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const looks = useFactionColours(runtime.client, track?.source);
  const selectionScope = JSON.stringify([track?.source.id, track?.source.metadata.campaignKey, track?.branch.id,
    widget.countries, source, options.view, pinned]);
  const selectionKey = useMemo(() => ({ scope: selectionScope }), [runtime.client, selectionScope]);
  const scope = JSON.stringify([selectionScope, extent]);
  const requests = useMemo(() => ({ key: '', params: null as FactionTotalsParams | null }), [runtime.client]);
  if (requests.key !== scope) { requests.key = scope; requests.params = null; }
  if (track) {
    const params = factionRaceRequest(track.branch.id, widget.countries, source, looks.loading || looks.error ? max : looks.factions.length, max);
    requests.params = pinned && params ? { ...params, inventory: true } : params;
  }
  const params = requests.params;
  const prepared = usePreparedFactionTotals(runtime.client, track, params, moment, active, extent, undefined, !renderMain);
  // Totals kept while playback's newest loads are ranked and named at the hour they answer.
  const shownMoment = view === 'lines' ? moment : prepared.headHour ?? moment;
  const { data, error, retry } = prepared;
  const numeric = useMemo(() => new NumericReply<FactionTotalsReply>(), [runtime.client]);
  const numbers = numeric.update(JSON.stringify([scope, params]), data, factionNumbers);
  const model = useMemo(() => numbers ? factionTotalsModel(numbers) : null, [numbers]);
  const namedModel = useMemo(() => data ? factionTotalsModel(data) : model, [data, model]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const drawn = useMemo(() => view === 'lines' && prepared.geometryData ? factionTotalsModel(prepared.geometryData) : model, [view, prepared.geometryData, model]);
  const lines = useMemo(() => {
    const lines = factionLines(drawn, range);
    const keys = new Set(lines.map(line => line.tag));
    for (const [tag, summary] of prepared.lineSummaries) if (!keys.has(tag) && summary.buckets.some(Boolean)) lines.push({ tag, runs: [] });
    return lines;
  }, [drawn, range, prepared.lineSummaries]);
  const series = useMemo(() => raceSeriesIndex(lines), [lines]);
  const reachedSeries = useMemo(() => raceSeriesIndex(factionLines(model, range)), [model, range]);
  const slots = Math.max(1, Math.min(params?.top ?? 1, lines.length));
  const entries = useMemo(() => view === 'race' ? seriesRaceEntries(series, shownMoment, slots, formatValue) : [],
    [view, series, shownMoment, slots, formatValue]);
  const recordedFactions = useMemo(() => new Map(looks.factions.map(faction => [faction.id, faction])), [looks.factions]);
  const names = useMemo(() => ({ factions: recordedFactions, names: looks.names }), [recordedFactions, looks.names]);
  const colourState = useMemo(() => ({ key: '', colours: new Map<string, string>() as ReadonlyMap<string, string> }), [runtime.client]);
  if (colourState.key !== selectionScope) { colourState.key = selectionScope; colourState.colours = new Map(); }
  if (!looks.loading) colourState.colours = looks.colours;
  const tokens = useChartTokens();
  const colours = useMemo(() => new Map(lines.map(line => [line.tag, colourState.colours.get(line.tag) ?? tokens.muted])), [lines, colourState.colours, tokens.muted]);
  const entityLooks = useMemo(() => ({
    name: (id: string, hour = shownMoment) => factionLabelAt(namedModel, id, hour, names),
    color: (id: string) => colours.get(id)!,
  }), [namedModel, shownMoment, names, colours]);
  const owner = prepared.owner;
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const open = useCallback((hour: number, id?: string, choices?: readonly string[]) => {
    if (owner !== currentOwner.current) return;
    setQuery(''); setPick(factionPick(selectionKey, hour, id, choices,
      widget.time.mode === 'follow' ? { playing: runtime.clock.getSnapshot().playing, playhead } : undefined));
  }, [selectionKey, owner, widget.time.mode, runtime.clock, playhead]);
  const current = pick?.key === selectionKey ? pick : null;
  const saved = track ? savedFactionSelection(options[FACTION_SELECTION], track) : null;
  const selected = pinned ? current?.selected ?? saved?.id ?? (widget.countries.kind === 'group' && widget.countries.group.type === 'faction'
    ? widget.countries.group.id : null) : current?.selected ?? null;
  const origin = (pinned ? saved?.origin : null) ?? current?.origin ?? moment;
  const follow = current?.follow ?? (pinned && widget.time.mode === 'follow');
  const drillHour = follow && track ? Math.floor(playbackHour + track.axisOffset) : current?.hour ?? moment;
  const selectionValid = !pinned || !options[FACTION_SELECTION] || saved !== null;
  const selectedParams = params && selected && selectionValid ? { ...params, top: 1, factionId: selected, parts: true } : null;
  const partsProduct = usePreparedFactionTotals(runtime.client, track, selectedParams, drillHour, active && (pinned || current !== null), extent, origin);
  const partsNumeric = useMemo(() => new NumericReply<FactionTotalsReply>(), [runtime.client]);
  const partNumbers = partsNumeric.update(JSON.stringify([scope, selectedParams]), partsProduct.data, factionNumbers);
  const partsModel = useMemo(() => partNumbers && selected ? factionParts(partNumbers, selected, range) : null, [partNumbers, selected, range]);
  const partsOwner = partsProduct.owner;
  useParityRows(anchor, () => ({ kind: 'chart', label: 'Faction totals', complete: !!data, hour: moment, total: data?.series.length ?? 0,
    rows: data?.series.map(series => ({ id: series.id, value: runAt(series.runs, moment)?.value ?? null })) ?? [] }));
  const controls = useRef({ active, selectionKey, partsOwner });
  controls.current = { active, selectionKey, partsOwner };
  useEffect(() => () => { controls.current.active = false; }, []);
  const setFollowing = (following: boolean) => {
    if (!controls.current.active || controls.current.selectionKey !== selectionKey || controls.current.partsOwner !== partsOwner) return;
    setPick(before => {
      const current = before?.key === selectionKey ? before : pinned && selected ? factionPick(selectionKey, origin, selected) : null;
      return current ? { ...current, ...reduceDrill(current, following ? { type: 'follow' } : { type: 'pick', hour: drillHour }) } : before;
    });
  };
  const selectedSeries = partNumbers?.series.find(series => series.id === selected);
  const proven = selectedSeries?.membership.some(span => span.firstHour <= origin && origin <= span.lastHour) ?? false;
  const choices = useMemo(() => (current?.ids ?? (pinned ? [...(model?.series.keys() ?? [])] : [])).filter(id => {
    const faction = model?.series.get(id);
    return faction && (pinned || runAt(faction.runs, origin)) && runAt(faction.membership, origin);
  })
    .map(id => ({ key: id, item: { id, name: factionNameAt(namedModel, id, origin, names) } }))
    .filter(row => !query || `${row.item.name} ${row.item.id}`.toLocaleLowerCase().includes(query.toLocaleLowerCase())),
  [current, pinned, model, namedModel, origin, names, query]);
  const diagnosticKey = JSON.stringify([...(data?.warnings ?? []), ...looks.warnings, ...mainWarnings,
      ...(partsProduct.data?.warnings ?? []), ...(looks.loading ? ['Faction names and colours are loading.'] : []),
      ...(error ? [`Faction totals refresh is unavailable: ${error}`] : []),
      ...(partsProduct.error ? [`Faction members refresh is unavailable: ${partsProduct.error}`] : []),
      ...(looks.error ? [`Faction colours are unavailable: ${looks.error}`] : [])]);
  const diagnostics = useMemo(() => JSON.parse(diagnosticKey) as string[], [diagnosticKey]);
  useEffect(() => {
    onDiagnostics(diagnostics.length ? diagnostics : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnostics, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message={`Load a campaign to see its ${label.toLowerCase()}.`} />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (unavailableReason) return <WidgetState kind="unavailable" message="Faction totals unavailable." />;
  if (error && !numbers) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!params) return <WidgetState kind="empty" icon="shield" message="Choose Factions, or a faction group shown as one total, in the widget settings." />;
  if (!numbers || !model) return <WidgetState kind="loading" label={`Loading ${label.toLowerCase()}…`} />;
  if (numbers.unavailableReason && !pinned && !renderMain) return <WidgetState kind="unavailable" message="Faction totals unavailable." />;
  if (!lines.length && !pinned && !renderMain) return <WidgetState kind="unavailable" message="Faction totals unavailable." />;
  const countName = measureLabel(numbers.unit);
  const nameOf = (id: string, hour: number) => factionNameAt(namedModel, id, hour, names);
  const colourOf = entityLooks.color;
  const mainActive = active && !pinned && (current === null || options.drill === 'popout');
  const main = renderMain ? renderMain(open, owner, reportMain, mainActive) : <CountryRaceView runtime={runtime} track={track} active={mainActive} options={options} view={view} extent={extent} playhead={playhead}
    moment={shownMoment} slots={slots} ranked={entries} lines={lines} entityLooks={entityLooks} lineLegendLabel="Factions"
    label={label} empty={`Not recorded on ${dayLabel(shownMoment)}.`} formatValue={formatValue}
    labels={{ race: `${label}, ${countName}, ${dayLabel(shownMoment)}`, lines: `${label} over time, ${countName}`, actions: `${label} actions` }}
    presentation={{ ...SMOOTH_TOTALS_PRESENTATION, countryLooks: false, linePadding: CHART_LAYOUT.countryEndLabels }}
    onPick={open} pickOwner={owner} lineSummaries={prepared.lineSummaries} onSummaryView={prepared.onSummaryView}
    onReachHour={prepared.onReachHour} exactLineTags={prepared.exactLineTags} navigation={prepared.navigation}
    tooltipTags={(hour, ids) => factionVisibleMemberTags(model, hour, ids)}
    tooltip={(id, hour, _name, _colours, labelAt) => factionTooltipForms(model, [id], hour, labelAt(hour), colourOf, formatValue,
      nameOf(id, hour), `${dayLabel(hour)} · ${countName}`, note, factionMemberLabelLimit(viewport, 1, nameOf(id, hour)), names)}
    lineTooltip={(hour, _tags, labelAt) => {
      const ids = seriesAt(reachedSeries, hour);
      return factionTooltipForms(model, ids, hour, labelAt(hour), colourOf, formatValue,
        dayLabel(hour), countName, note, factionMemberLabelLimit(viewport, ids.length, dayLabel(hour)), names);
    }} />;
  const breakdown = !selectionValid ? <WidgetState kind="unavailable" message="Saved faction unavailable." />
    : !selected ? <div className="faction-member-chooser"><p className="drill-empty">Choose the faction at {dayLabel(origin)}.</p>
      <input type="search" aria-label="Search factions" value={query} onChange={event => setQuery(event.currentTarget.value)} />
      <WindowedRows items={choices} rowHeight={36} label="Factions at the chosen hour" empty={<p className="drill-empty">No eligible faction was drawn at this hour.</p>}>
        {row => <button type="button" tabIndex={0} disabled={row.phase === 'leaving'}
          onClick={() => {
            setPick(before => chooseFaction(before?.key === selectionKey ? before : factionPick(selectionKey, origin, undefined, choices.map(row => row.item.id)), selectionKey, row.item.id));
            if (pinned) setOption?.(FACTION_SELECTION, factionSelection(track, row.item.id, origin));
          }}>{row.item.name} · {row.item.id}</button>}
      </WindowedRows></div>
      : partsProduct.error && !partNumbers ? <WidgetState kind="error" what="faction members" detail={partsProduct.error} onRetry={partsProduct.retry} />
        : !partNumbers ? <WidgetState kind="loading" label="Loading faction members…" />
          : !proven ? <WidgetState kind="unavailable" message="Faction unavailable at the selected time." />
            : !partsModel ? <WidgetState kind="unavailable" message="Member details are unavailable." />
              : <FactionPartsView model={partsModel} runtime={runtime} track={track} hour={drillHour} following={follow}
                extent={extent} active={active} pickOwner={partsOwner} label={`${nameOf(selected, drillHour)} · ${countName}`} formatValue={formatValue}
                summaries={partsProduct.data?.memberSummaries} totalSummary={selected ? partsProduct.lineSummaries.get(selected) : undefined}
                onSummaryView={partsProduct.onSummaryView} onReachHour={partsProduct.onReachHour} navigation={partsProduct.navigation}
                onPick={hour => setPick(before => {
                  const current = before?.key === selectionKey ? before : factionPick(selectionKey, origin, selected);
                  return { ...current, ...reduceDrill(current, { type: 'pick', hour }) };
                })} />;
  if (pinned) return <div ref={anchor} className="faction-race-drill"><div className="drill-header"><span className="drill-day">{dayLabel(drillHour)}</span>
    {selected && proven && <FollowControl following={follow} onPause={() => setFollowing(false)} onFollow={() => setFollowing(true)} />}</div>
    {breakdown}</div>;
  return <div ref={anchor} className="faction-race-drill"><DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={current !== null}
    title={selected ? `${nameOf(selected, drillHour)}: members` : 'Faction members'} day={dayLabel(drillHour)} following={follow}
    canFollow={selected !== null && proven} onClose={() => setPick(null)} onFollow={() => setFollowing(true)}
    onPause={() => setFollowing(false)}
    onPin={selected && proven ? () => runtime.addWidgetBelow(widget.id, pinFactionParts(widget, track, selected, origin, drillHour, follow, params.membership)) : undefined}
    anchor={anchor} main={main}>{breakdown}</DrillHost></div>;
}
