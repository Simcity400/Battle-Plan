import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import { useFactionColours } from '../../dashboard/FactionColours';
import { factionLabelAt, factionLines, factionMemberLabelLimit, factionNameAt, factionTooltipForms, factionTotalsModel,
  factionVisibleMemberTags, validateFactionTotalsReply } from '../../dashboard/factionTotals';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import type { FactionTotalsParams, FactionTotalsReply } from '../../generated/dashboard';
import { dayLabel } from '../draw';
import { CHART_LAYOUT } from '../tokens';
import { useViewportHeight } from '../useChartTokens';
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
  usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || (pick?.follow ?? false)));
  const viewport = useViewportHeight();
  const view = options.view === 'lines' ? 'lines' : 'race';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
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
  if (track && !looks.loading && !looks.error) {
    const params = factionRaceRequest(track.branch.id, widget.countries, source, looks.factions.length, max);
    requests.params = pinned && params ? { ...params, inventory: true } : params;
  }
  const params = requests.params;
  const request = track && params ? { sourceId: track.source.id, revision: track.source.revision, product: 'faction-totals', params } : null;
  const { data, error, retry } = useProduct<FactionTotalsReply>(runtime.client, request, active, validateFactionTotalsReply);
  const numeric = useMemo(() => new NumericReply<FactionTotalsReply>(), [runtime.client]);
  const numbers = numeric.update(JSON.stringify([scope, params]), data, factionNumbers);
  const model = useMemo(() => numbers ? factionTotalsModel(numbers) : null, [numbers]);
  const namedModel = useMemo(() => data ? factionTotalsModel(data) : model, [data, model]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => factionLines(model, range), [model, range]);
  const series = useMemo(() => raceSeriesIndex(lines), [lines]);
  const slots = Math.max(1, Math.min(params?.top ?? 1, lines.length));
  const entries = useMemo(() => view === 'race' ? seriesRaceEntries(series, moment, slots, formatValue) : [],
    [view, series, moment, slots, formatValue]);
  const recordedFactions = useMemo(() => new Map(looks.factions.map(faction => [faction.id, faction])), [looks.factions]);
  const names = useMemo(() => ({ factions: recordedFactions, names: looks.names }), [recordedFactions, looks.names]);
  const colourState = useMemo(() => ({ key: '', colours: new Map<string, string>() as ReadonlyMap<string, string> }), [runtime.client]);
  if (colourState.key !== selectionScope) { colourState.key = selectionScope; colourState.colours = new Map(); }
  if (!looks.loading && !looks.error) colourState.colours = looks.colours;
  const colours = colourState.colours;
  const entityLooks = useMemo(() => ({
    name: (id: string, hour = moment) => factionLabelAt(namedModel, id, hour, names),
    color: (id: string) => colours.get(id)!,
  }), [namedModel, moment, names, colours]);
  const owner = useMemo(() => ({ scope, numbers }), [scope, numbers]);
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
  const drillHour = follow && track ? Math.floor(runtime.clock.getHour() + track.axisOffset) : current?.hour ?? moment;
  const selectionValid = !pinned || !options[FACTION_SELECTION] || saved !== null;
  const selectedParams = params && selected && selectionValid ? { ...params, top: 1, factionId: selected, parts: true } : null;
  const partsRequest = track && selectedParams ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'faction-totals', params: selectedParams } : null;
  const partsProduct = useProduct<FactionTotalsReply>(runtime.client, partsRequest, active && (pinned || current !== null), validateFactionTotalsReply);
  const partsNumeric = useMemo(() => new NumericReply<FactionTotalsReply>(), [runtime.client]);
  const partNumbers = partsNumeric.update(JSON.stringify([scope, selectedParams]), partsProduct.data, factionNumbers);
  const partsModel = useMemo(() => partNumbers && selected ? factionParts(partNumbers, selected, range) : null, [partNumbers, selected, range]);
  const partsOwner = useMemo(() => ({ scope, selected, partNumbers }), [scope, selected, partNumbers]);
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
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (unavailableReason) return <WidgetState kind="unavailable" message={unavailableReason} />;
  if (error && !numbers) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (looks.error && !numbers) return <WidgetState kind="unavailable" message={`Faction colours are unavailable: ${looks.error}`} />;
  if (looks.loading && !numbers) return <WidgetState kind="loading" label="Loading faction colours…" />;
  if (!params) return <WidgetState kind="empty" icon="shield" message="Choose Factions, or a faction group shown as one total, in the widget settings." />;
  if (!numbers || !model) return <WidgetState kind="loading" label={`Loading ${label.toLowerCase()}…`} />;
  if (numbers.unavailableReason && !pinned && !renderMain) return <WidgetState kind="unavailable" message={numbers.unavailableReason} />;
  if (!lines.length && !pinned && !renderMain) return <WidgetState kind="unavailable" message={numbers.series.find(series => series.unavailableReason)?.unavailableReason
    ?? `Not in this recording: no ${label.toLowerCase()} totals for these factions in this branch.`} />;
  if (!lines.every(line => colours.has(line.tag))) return <WidgetState kind="unavailable" message="Faction colours could not be read." />;
  const countName = measureLabel(numbers.unit);
  const nameOf = (id: string, hour: number) => factionNameAt(namedModel, id, hour, names);
  const colourOf = entityLooks.color;
  const mainActive = active && !pinned && (current === null || options.drill === 'popout');
  const main = renderMain ? renderMain(open, owner, reportMain, mainActive) : <CountryRaceView runtime={runtime} track={track} active={mainActive} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={entries} lines={lines} entityLooks={entityLooks} lineLegendLabel="Factions"
    label={label} empty={`Not recorded on ${dayLabel(moment)}.`} formatValue={formatValue}
    labels={{ race: `${label}, ${countName}, ${dayLabel(moment)}`, lines: `${label} over time, ${countName}`, actions: `${label} actions` }}
    presentation={{ ...SMOOTH_TOTALS_PRESENTATION, countryLooks: false, linePadding: CHART_LAYOUT.countryEndLabels }}
    onPick={open} pickOwner={owner}
    tooltipTags={(hour, ids) => factionVisibleMemberTags(model, hour, ids)}
    tooltip={(id, hour, _name, _colours, labelAt) => factionTooltipForms(model, [id], hour, labelAt(hour), colourOf, formatValue,
      nameOf(id, hour), `${dayLabel(hour)} · ${countName}`, note, factionMemberLabelLimit(viewport, 1, nameOf(id, hour)), names)}
    lineTooltip={(hour, _tags, labelAt) => {
      const ids = seriesAt(series, hour);
      return factionTooltipForms(model, ids, hour, labelAt(hour), colourOf, formatValue,
        dayLabel(hour), countName, note, factionMemberLabelLimit(viewport, ids.length, dayLabel(hour)), names);
    }} />;
  const breakdown = !selectionValid ? <WidgetState kind="unavailable" message="This saved faction belongs to another campaign or branch." />
    : !selected ? <div className="faction-member-chooser"><p className="drill-empty">Choose the faction at {dayLabel(origin)}.</p>
      <input type="search" aria-label="Search factions" value={query} onChange={event => setQuery(event.currentTarget.value)} />
      <WindowedRows items={choices} rowHeight={36} label="Factions at the chosen hour" empty={<p className="drill-empty">No eligible faction was drawn at this hour.</p>}>
        {row => <button type="button" tabIndex={0} disabled={row.phase === 'leaving'}
          onClick={() => {
            setPick(before => chooseFaction(before?.key === selectionKey ? before : factionPick(selectionKey, origin, undefined, choices.map(row => row.item.id)), selectionKey, row.item.id));
            if (pinned) setOption?.(FACTION_SELECTION, factionSelection(track, row.item.id, origin));
          }}>{row.item.name} · {row.item.id}</button>}
      </WindowedRows></div>
      : partsProduct.error && !partNumbers ? <WidgetState kind="error" message={partsProduct.error} onRetry={partsProduct.retry} />
        : !partNumbers ? <WidgetState kind="loading" label="Loading faction members…" />
          : !proven ? <WidgetState kind="unavailable" message="This faction was not recorded at the chosen origin hour in this branch." />
            : !partsModel ? <WidgetState kind="unavailable" message="Full member values and native country identities are not in this reply." />
              : <FactionPartsView model={partsModel} runtime={runtime} track={track} hour={drillHour} following={follow}
                extent={extent} active={active} pickOwner={partsOwner} label={`${nameOf(selected, drillHour)} · ${countName}`} formatValue={formatValue}
                onPick={hour => setPick(before => {
                  const current = before?.key === selectionKey ? before : factionPick(selectionKey, origin, selected);
                  return { ...current, ...reduceDrill(current, { type: 'pick', hour }) };
                })} />;
  if (pinned) return <div className="faction-race-drill"><div className="drill-header"><span className="drill-day">{dayLabel(drillHour)}</span>
    {selected && proven && (follow ? <span className="pill">Following playback</span> : <button className="button small" type="button"
      onClick={() => setPick(before => {
        const current = before?.key === selectionKey ? before : factionPick(selectionKey, origin, selected);
        return { ...current, ...reduceDrill(current, { type: 'follow' }) };
      })}>Follow playback</button>)}</div>
    {breakdown}</div>;
  return <div ref={anchor} className="faction-race-drill"><DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={current !== null}
    title={selected ? `${nameOf(selected, drillHour)}: members` : 'Faction members'} day={dayLabel(drillHour)} following={follow}
    canFollow={selected !== null && proven} onClose={() => setPick(null)} onFollow={() => setPick(before => before ? { ...before, ...reduceDrill(before, { type: 'follow' }) } : before)}
    onPin={selected && proven ? () => runtime.addWidgetBelow(widget.id, pinFactionParts(widget, track, selected, origin, drillHour, follow, params.membership)) : undefined}
    anchor={anchor} main={main}>{breakdown}</DrillHost></div>;
}
