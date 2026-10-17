import { preparedPrefix, pendingReplyAt } from '../../dashboard/preparedPrefix';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import WidgetState from '../../components/WidgetState';
import { logSettings } from '../../charts/log/logOptions';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import type { TooltipRow } from '../../charts/tooltip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetMeta, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { BattleRow, BattleWindow, BattleWarning, PlaceName, PlaceNamesReply } from '../../generated/dashboard';
import BattleLog from './BattleLog';
import type { BattleLogProps } from './BattleLog';
import { battleCountryRequests, battleJumpHour, battlePlaceRequests, namedBattles, battleWindowIndex, battleWindows, battleWarnings, shownBattleCensusHour } from './battleRows';
import { placeKey, validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import type { BattleLabels, BattleLossFormatter, BattleMeasure, BattleSideSummary } from './battleRows';
import { DEFAULT_SORT, DEFAULT_TOP_N } from './battleOptions';
import { parityWindowCoverage, parityWindowIndices, useParityPreparation } from '../../dashboard/parityPreparation';
import { useParityRows } from '../../dashboard/parityRows';
import type { BattleSemantic } from './BattleLog';
import './battles.css';

interface ViewRow extends BattleRow { provinceId: number; windowIndex: number }
interface ManifestReply { windows: BattleWindow[]; available: boolean; unavailable: string | null; contentWarnings: string[] }
interface WindowReply { observed: [number, number][]; contentWarnings: string[]; warnings: BattleWarning[] }
interface RankingReply { censusHour: number | null; effectiveMeasure: string; warnings: BattleWarning[]; contentWarnings: string[] }
/** What an adapter's own view needs beyond the shared log props. */
export interface BattleViewContext {
  runtime: WidgetRuntime; track: WidgetRuntime['tracks'][number]; active: boolean; widget: WidgetSpec;
  setOption?: WidgetProps['setOption'];
  /** The hour the rows show. */
  shownHour: number;
  /** What Biggest ranks by: the ranking's effective measure, which falls back
   * from the chosen one where the recording lacks it. */
  rankedMeasure: string;
  /** A place name still loading, as opposed to a read that named nothing. */
  placePending: (row: BattleRow & { provinceId: number }) => boolean;
}
export interface BattleViewAdapter<Row extends ViewRow, Window extends WindowReply, Ranking extends RankingReply> {
  definition: WidgetMeta;
  product: string;
  landPlaceNames?: boolean;
  validateManifest: (data: unknown) => ManifestReply;
  validateWindow: (data: unknown) => Window;
  validateRanking: (data: unknown) => Ranking;
  rows: (windows: ReadonlyMap<number, Window>, hour: number, matches: (tag: string, hour: number) => boolean, all: boolean, from: number | null) => Row[];
  rankedRows: (data: Ranking | undefined, hour: number) => Row[];
  labels: BattleLabels;
  measures: readonly BattleMeasure<Row>[];
  resultMeasure?: BattleMeasure<Row>;
  compactSides?: BattleSideSummary;
  note?: string | ((row: Row) => string);
  formatLoss?: BattleLossFormatter<Row>;
  summary?: (rows: readonly Row[], ranking: Ranking | undefined, measure: string, view: 'log' | 'biggest') => string | null;
  description?: (row: Row) => string | null;
  tooltipRows?: (row: Row) => readonly TooltipRow[];
  onVisibleRows?: (rows: readonly Row[]) => void;
  diagnostics?: readonly string[];
  /** Replaces the shared log and table with the widget's own view of the same rows. */
  view?: ComponentType<BattleLogProps<Row> & { context: BattleViewContext }>;
}
const NO_ROWS: readonly never[] = [];
const NO_WARNINGS: string[] = [];

export default function BattleLogView<Row extends ViewRow, Window extends WindowReply, Ranking extends RankingReply>(
  { widget, runtime, active, onDiagnostics, setOption, adapter }: WidgetProps & { adapter: BattleViewAdapter<Row, Window, Ranking> }) {
  const live = widget.time.mode === 'follow';
  const playbackHour = useTableHour(runtime.clock, active && live);
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(adapter.definition.options ?? [], widget.options);
  const view = options.view === 'biggest' ? 'biggest' : 'log';
  const measure = typeof options.measure === 'string' ? options.measure : adapter.measures[0].label;
  const topN = typeof options.topN === 'number' ? options.topN : DEFAULT_TOP_N;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window ? Math.floor(window.lastHour) : null;
  const inBranch = track !== undefined && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind !== 'all');
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const manifestRequest = track ? { sourceId: track.source.id, revision: track.source.revision,
    product: `${adapter.product}-manifest`, params: { branchId: track.branch.id } } : null;
  const manifest = useProduct<ManifestReply>(runtime.client, manifestRequest, active, adapter.validateManifest);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const playheadWindow = battleWindowIndex(manifest.data?.windows ?? [], hour ?? first);
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const parityId = import.meta.env.MODE === 'parity' ? useId() : undefined;
  const parityEmptyRoot = import.meta.env.MODE === 'parity' ? useRef<HTMLDivElement>(null) : null;
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const reachedHour = hour !== null && pendingReplyAt(manifest.data, hour)
    ? Math.min(hour, preparedPrefix(manifest.data)!.coverage.lastHour) : hour;
  const shownHour = !parity?.requested && view === 'log' && browse.identity === identity && browse.hour !== null ? Math.min(browse.hour, hour ?? browse.hour) : reachedHour;
  const browseIndex = battleWindowIndex(manifest.data?.windows ?? [], shownHour ?? first);
  const [visibleWindows, setVisibleWindows] = useState<{ identity: string | null; at: number; windows: number[] }>({ identity: null, at: -1, windows: [] });
  const visibleIndices = visibleWindows.identity === identity && visibleWindows.at === playheadWindow ? visibleWindows.windows : [];
  const fullWindows = import.meta.env.MODE === 'parity' ? parityWindowIndices(manifest.data?.windows, first, hour ?? first) : [];
  const wantedWindows = inBranch && view === 'log' ? parity?.requested ? fullWindows
    : battleWindows(manifest.data, reachedHour ?? first, live, [...visibleIndices, ...(browse.hour !== null && browse.identity === identity ? [browseIndex] : [])], first) : [];
  const history = useProductWindows<Window>(runtime.client, identity, index => ({ sourceId: track!.source.id,
    revision: track!.source.revision, product: adapter.product, params: { branchId: track!.branch.id, window: index } }),
  wantedWindows, active && inBranch && manifest.data?.available === true, adapter.validateWindow, track?.source, true);
  const countries = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const rankingRequest = track && hour !== null && inBranch ? { sourceId: track.source.id, revision: track.source.revision,
    product: `${adapter.product}-ranking`, params: { branchId: track.branch.id, firstHour: first, lastHour: hour, topN, measure, countries } } : null;
  const ranking = useProduct<Ranking>(runtime.client, rankingRequest,
    active && inBranch && view === 'biggest' && match.ready && !match.reason && manifest.data?.available === true, adapter.validateRanking);
  const shown = useMemo(() => !inBranch || shownHour === null || !match.ready ? NO_ROWS : view === 'biggest' ? adapter.rankedRows(ranking.data, shownHour)
    : adapter.rows(history.data, shownHour, match.matches, widget.countries.kind === 'all', widget.time.mode === 'range' ? first : null),
  [adapter, inBranch, view, ranking.data, history.data, shownHour, match, widget.countries.kind, widget.time.mode, first]);
  const fullHistory = import.meta.env.MODE === 'parity' ? parityWindowCoverage(fullWindows, history.data, history.errors) : null;
  const historyComplete = !!manifest.data && (view === 'log' ? fullHistory?.complete === true : !!ranking.data);
  const [visible, setVisible] = useState<readonly Row[]>(NO_ROWS);
  const currentRows = useMemo(() => new Map(shown.map(row => [row.id, row])), [shown]);
  const drawn = useMemo(() => visible.flatMap(row => { const current = currentRows.get(row.id); return current ? [current] : []; }), [visible, currentRows]);
  useEffect(() => { adapter.onVisibleRows?.(drawn); }, [drawn, adapter.onVisibleRows]);
  const wanted = useMemo(() => battleCountryRequests(drawn), [drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(country => country.hour)).size));
  const country = useCallback((tag: string, at: number) => labelAt(at)(tag), [labelAt]);
  const places = useMemo(() => battlePlaceRequests(drawn), [drawn]);
  const placeRequest = track && places.length ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'place-names', params: { branchId: track.branch.id, places, states: [], land: adapter.landPlaceNames === true } } : null;
  const placeData = useProduct<PlaceNamesReply>(runtime.client, placeRequest, active, validatePlaceNamesReply);
  let paritySemantic: BattleSemantic<Row> | undefined;
  if (import.meta.env.MODE === 'parity') {
    const complete = parity!.requested && historyComplete;
    const preparation = { id: parityId, prepare: parity!.prepare, ready: () => complete,
      error: () => manifest.error ?? (view === 'log' ? fullHistory?.error : ranking.error) ?? null };
    const reads = { windows: fullHistory?.outcomes ?? [] };
    paritySemantic = { rows: shown, complete, historyComplete, preparation, reads };
    // An empty newest block still prepares earlier blocks. Its stable identity
    // follows the existing empty state as it gains the ordinary log or table.
    useParityRows(shown.length ? null : parityEmptyRoot!, () => ({ kind: 'log', label: adapter.definition.title,
      complete, historyComplete, total: 0, rows: [], readOutcomes: reads }), preparation);
  }
  // Names already read stay while the next drawn rows load, so scrolling never blanks them.
  const [placeNames, setPlaceNames] = useState<{ identity: string | null; names: ReadonlyMap<string, PlaceName> }>({ identity: null, names: new Map() });
  useEffect(() => {
    const places = placeData.data?.places;
    if (!places?.length) return;
    setPlaceNames(previous => {
      const names = new Map(previous.identity === identity ? previous.names : []);
      for (const place of places) names.set(placeKey(place.provinceId, place.hour), place);
      return { identity, names };
    });
  }, [placeData.data, identity]);
  const knownNames = placeNames.identity === identity ? placeNames.names : null;
  const named = useMemo(() => namedBattles(shown, knownNames ? { places: [...knownNames.values()], states: [], warnings: [] } : placeData.data),
    [shown, knownNames, placeData.data]);
  const answered = useMemo(() => new Set(placeData.data?.places.map(place => placeKey(place.provinceId, place.hour))), [placeData.data]);
  const placePending = useCallback((row: BattleRow & { provinceId: number }) => {
    const key = placeKey(row.provinceId, row.place.namedAt ?? row.place.hour);
    return placeData.error === undefined && !knownNames?.has(key) && !answered.has(key);
  }, [placeData.error, knownNames, answered]);
  const onVisibleRows = useCallback((drawn: readonly Row[]) => {
    setVisible(previous => {
      const next = drawn.map(row => currentRows.get(row.id) ?? row);
      return previous.length === next.length && previous.every((row, index) => row === next[index]) ? previous : next;
    });
    if (view === 'log') {
      const windows = [...new Set(drawn.map(row => row.windowIndex))].sort((a, b) => a - b);
      setVisibleWindows(previous => previous.identity === identity && previous.at === playheadWindow && JSON.stringify(previous.windows) === JSON.stringify(windows)
        ? previous : { identity, at: playheadWindow, windows });
    }
  }, [currentRows, view, identity, playheadWindow]);
  const warnings = useMemo(() => view === 'biggest' ? [...(ranking.data?.contentWarnings ?? NO_WARNINGS),
    ...(ranking.data?.warnings ?? []).map(w => `Hour ${w.hour}: ${w.reason} (${w.count})`)]
    : battleWarnings(history.data, first, shownHour ?? first), [view, ranking.data, history.data, first, shownHour]);
  const diagnostics = useMemo(() => [...new Set([...(manifest.data?.contentWarnings ?? NO_WARNINGS), ...warnings,
    ...(placeData.data?.warnings ?? NO_WARNINGS), ...(placeData.error ? [`Place names: ${placeData.error}`] : []), ...(adapter.diagnostics ?? NO_WARNINGS)])],
  [warnings, manifest.data, placeData.data, placeData.error, adapter.diagnostics]);
  useDiagnostics(onDiagnostics, diagnostics);

  if (!track) return <WidgetState kind="empty" icon={adapter.definition.icon} message={`Load a campaign to see its ${adapter.definition.title.toLowerCase()}.`} />;
  if (!window || !inBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (manifest.error) return <WidgetState kind="error" detail={manifest.error} onRetry={manifest.retry} />;
  if (widget.countries.kind !== 'all' && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!manifest.data || !match.ready) return <WidgetState kind="loading" label={`Loading ${adapter.definition.title.toLowerCase()}…`} />;
  const prefix = preparedPrefix(manifest.data);
  const coverageNote = <PreparedLogCoverage coverage={prefix?.coverage} exact={(manifest.data as { exact?: number }).exact}
    approximate={!!prefix?.approximate?.length} />;
  if (!manifest.data.available && prefix?.pending.length) return <><WidgetState kind="loading" label="This range is still being prepared." />{coverageNote}</>;
  if (!manifest.data.available) return <WidgetState kind="unavailable" message="Battles were not recorded." />;
  const required = view === 'log' && browseIndex >= 0 ? [browseIndex] : [];
  const failed = view === 'biggest' ? ranking.error : required.map(index => history.errors.get(index)).find(Boolean);
  if (failed) return <WidgetState kind="error" detail={failed} onRetry={view === 'biggest' ? ranking.retry : history.retry} />;
  const pending = view === 'biggest' ? !ranking.data : required.some(index => !history.data.has(index));
  if (pending) return <>{coverageNote}<WidgetState kind="loading" label={prefix?.pending.length ? 'This range is still being prepared.' : 'Loading battles...'} /></>;
  const censusHour = view === 'biggest' ? ranking.data?.censusHour ?? null : shownBattleCensusHour(history.data, shownHour!);
  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? preceding.reduce((oldest, index) => Math.min(oldest, index)) : 0;
  const earlier = view === 'log' && oldest > 0 && manifest.data.windows[oldest - 1].lastHour >= first;
  const earlierButton = earlier && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: manifest.data!.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier battles</button>;
  const navigation = view === 'log' && <div className="battle-navigation">{earlierButton}
    {shownHour !== hour && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest battles</button>}
  </div>;
  // An adapter's own view draws its empty list, keeping its header controls.
  if (!shown.length && !(adapter.view && censusHour !== null) && prefix?.pending.length && censusHour === null) return <>{coverageNote}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!shown.length && !(adapter.view && censusHour !== null)) return <div ref={parityEmptyRoot} className="battle-widget">{coverageNote}{navigation}<WidgetState kind={censusHour === null ? 'unavailable' : 'empty'}
    message={censusHour === null ? 'Not in this recording' : `No recorded ${adapter.definition.title.toLowerCase()} match these settings.`} /></div>;
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const summary = adapter.summary?.(shown, ranking.data, measure, view);
  const rankedMeasure = ranking.data?.effectiveMeasure ?? measure;
  return <div className="battle-widget">
    {coverageNote}
    {summary && <p className="battle-note">{summary}</p>}
    {/* A range cuts battles short; their peak and duration are still the whole battle's. */}
    {widget.time.mode === 'range' && <p className="battle-note">Peak and duration cover the whole battle.</p>}
    {navigation}
    {view === 'log' && wantedWindows.some(index => !history.data.has(index) && !history.errors.has(index)) && <p className="battle-note" role="status">Updating…</p>}
    {(() => {
      const props: BattleLogProps<Row> = { rows: named, label: adapter.definition.title, labels: adapter.labels, country, live: live && shownHour === hour, view,
        log: logSettings(options), measures: adapter.measures, sort: sort.column === 'measure' ? { ...sort, column: rankedMeasure } : sort,
        setOption, cellBars: options.cellBars === true, rankMarkers: options.rankMarkers === true, hoursPerSecond,
        menu: row => [jumpItem(runtime, track.id, battleJumpHour(row))], empty: `No recorded ${adapter.definition.title.toLowerCase()} match these settings.`,
        note: adapter.note, formatLoss: adapter.formatLoss, compactSides: adapter.compactSides, resultMeasure: adapter.resultMeasure,
        description: adapter.description, tooltipRows: adapter.tooltipRows, onVisibleRows, placePending,
        ...(import.meta.env.MODE === 'parity' ? { paritySemantic } : {}) };
      const View = adapter.view;
      return View ? <View {...props} context={{ runtime, track, active, widget, setOption, shownHour: shownHour!, rankedMeasure, placePending }} /> : <BattleLog {...props} />;
    })()}
  </div>;
}
