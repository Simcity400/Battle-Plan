import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import WidgetState from '../../components/WidgetState';
import { logSettings } from '../../charts/log/logOptions';
import type { LogRow } from '../../charts/log/logData';
import { aheadEnd, useRevealClock } from '../../charts/reveal';
import { useAheadHour } from '../../dashboard/aheadHour';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { jumpItem } from '../../charts/jump';
import type { TooltipRow } from '../../charts/tooltip';
import { useAtHead } from '../../dashboard/atHead';
import { formatDateTime } from '../../dashboard/calendar';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetMeta, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { countryMatch, countryMatchChangeHours, countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useKeyedPainter, useKeyedReads } from '../../dashboard/keyedReads';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { placeKey, validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import type { BattleRow, BattleWarning, BattleWindow, PlaceName, PlaceNamesReply } from '../../generated/dashboard';
import type { Failure } from '../../dashboard/queryError';
import BattleLog from './BattleLog';
import type { BattleLogProps } from './BattleLog';
import { battleChangeHours, battleCountryRequests, battleJumpHour, battlePlaceRequests, battleStateRows, namedBattles, placesLeadingWith, battleWindowIndex, battleWindows, battleWarnings,
  heldAtHour, heldWindowsEnd, rankingHour, rowsById, shownBattleCensusHour } from './battleRows';
import type { BattleChanges, BattleLabels, BattleLossFormatter, BattleMeasure, BattleSideSummary } from './battleRows';
import { DEFAULT_SORT, DEFAULT_TOP_N } from './battleOptions';
import './battles.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { useDistinctState } from '../../dashboard/useDistinctState';
import { useWhyCommit } from '../../dashboard/widgetPerf';
import { productRequest } from '../../dashboard/request.ts';

interface ViewRow extends BattleRow { provinceId: number | null; windowIndex: number }
interface ManifestReply { windows: BattleWindow[]; available: boolean; unavailable: string | null; contentWarnings: string[] }
interface WindowReply { observed: [number, number][]; contentWarnings: string[]; warnings: BattleWarning[] }
interface RankingReply { censusHour: number | null; effectiveMeasure: string; warnings: BattleWarning[]; contentWarnings: string[] }
export interface BattleViewContext {
  runtime: WidgetRuntime; track: WidgetRuntime['tracks'][number]; active: boolean; widget: WidgetSpec;
  setOption?: WidgetProps['setOption'];
  shownHour: number;
  headHour: () => number | null;
  rowsAt: (hour: number) => ReadonlyArray<BattleRow>;
  rowAt: (id: string, hour: number) => BattleRow | null;
  rankedMeasure: string;
  placePending: (row: BattleRow & { provinceId: number | null }) => boolean;
  heldPlaces: readonly { provinceId: number; hour: number }[] | null;
}
export interface BattleDetailAsk { id: string; hour: number }
export interface BattleDetails<Row, Window> {
  product: 'naval-battle-details';
  validate: (data: unknown) => { rows: readonly BattleDetailAsk[]; warnings: string[] };
  ask: (row: Row, following: boolean) => BattleDetailAsk;
  held: (windows: ReadonlyMap<number, Window>) => BattleDetailAsk[];
  description: (detail: BattleDetailAsk | undefined, read: { pending: boolean; error: Failure | undefined }) => string | null;
  tooltipRows: (detail: BattleDetailAsk | undefined, error: Failure | undefined) => readonly TooltipRow[];
}
export interface BattleViewAdapter<Row extends ViewRow, Window extends WindowReply, Ranking extends RankingReply> {
  definition: WidgetMeta;
  product: 'land-battles' | 'naval-battles';
  manifestProduct: 'land-battles-manifest' | 'naval-battles-manifest';
  rankingProduct: 'land-battles-ranking' | 'naval-battles-ranking';
  landPlaceNames?: boolean;
  validateManifest: (data: unknown) => ManifestReply;
  validateWindow: (data: unknown) => Window;
  validateRanking: (data: unknown) => Ranking;
  rows: (windows: ReadonlyMap<number, Window>, hour: number, matches: (tag: string, hour: number) => boolean, all: boolean, from: number | null) => ReadonlyArray<Row>;
  row: (windows: ReadonlyMap<number, Window>, id: string, hour: number, matches: (tag: string, hour: number) => boolean, all: boolean, from: number | null) => Row | null;
  changes: (windows: ReadonlyMap<number, Window>) => BattleChanges;
  state?: (row: Row) => string;
  windowPlaces?: (windows: ReadonlyMap<number, Window>) => { provinceId: number; hour: number }[];
  rankedRows: (data: Ranking | undefined, hour: number) => ReadonlyArray<Row>;
  labels: BattleLabels;
  measures: readonly BattleMeasure<Row>[];
  resultMeasure?: BattleMeasure<Row>;
  compactSides?: BattleSideSummary;
  note?: string | ((row: Row) => string);
  formatLoss?: BattleLossFormatter<Row>;
  summary?: (rows: readonly Row[], ranking: Ranking | undefined, measure: string, view: 'log' | 'biggest') => string | null;
  description?: (row: Row) => string | null;
  tooltipRows?: (row: Row) => readonly TooltipRow[];
  details?: BattleDetails<Row, Window>;
  diagnostics?: readonly string[];
  view?: ComponentType<BattleLogProps<Row> & { context: BattleViewContext }>;
}
const NO_ROWS: readonly never[] = [];
type VisibleWindows = { identity: string | null; at: number; windows: number[] };
const NO_VISIBLE_WINDOWS: VisibleWindows = { identity: null, at: -1, windows: [] };
const sameWindows = (held: VisibleWindows, next: VisibleWindows) => held.identity === next.identity && held.at === next.at
  && held.windows.length === next.windows.length && held.windows.every((window, index) => window === next.windows[index]);
const sameRows = <R,>(held: readonly R[], next: readonly R[]) => held.length === next.length && held.every((row, index) => row === next[index]);
const NO_LOG: readonly LogRow<never>[] = [];
const NO_PLACES: readonly PlaceAt[] = [];
const NO_ASKS: readonly BattleDetailAsk[] = [];
const detailKey = (ask: BattleDetailAsk) => JSON.stringify([ask.id, ask.hour]);
type PlaceAt = { provinceId: number; hour: number };
const placeAtKey = (place: PlaceAt) => placeKey(place.provinceId, place.hour);
const NO_WARNINGS: string[] = [];

export default function BattleLogView<Row extends ViewRow, Window extends WindowReply, Ranking extends RankingReply>(
  { widget, runtime, active, onDiagnostics, setOption, adapter }: WidgetProps & { adapter: BattleViewAdapter<Row, Window, Ranking> }) {
  const live = widget.time.mode === 'follow';
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const options = resolveOptions(adapter.definition.options ?? [], widget.options);
  const view = options.view === 'biggest' ? 'biggest' : 'log';
  const measure = typeof options.measure === 'string' ? options.measure : adapter.measures[0].label;
  const topN = typeof options.topN === 'number' ? options.topN : DEFAULT_TOP_N;
  const following = active && live;
  const hourAt = useCallback((clockHour: number) => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    return window ? Math.floor(window.lastHour) : null;
  }, [widget, track, runtime.tracks]);
  const first = widget.time.mode === 'range' && track ? Math.floor(resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.firstHour ?? track.branch.firstHour)
    : track?.branch.firstHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind !== 'all');
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const manifestRequest = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: adapter.manifestProduct, params: { branchId: track.branch.id } }) : null;
  const manifest = useProduct<ManifestReply>(runtime.client, manifestRequest, active, adapter.validateManifest);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const windows = manifest.data?.windows;
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const browsing = view === 'log' && browse.identity === identity && browse.hour !== null;
  const ahead = following && view === 'log' && !browsing;
  const blockInputs = useRef<{ hourAt: typeof hourAt; windows: readonly BattleWindow[]; held: ReadonlyMap<number, unknown>; changes: readonly number[] }>(
    { hourAt, windows: [], held: new Map(), changes: [] });
  const end = useCallback((clockHour: number) => {
    const { hourAt: trackHour, windows: spans, held, changes: hours } = blockInputs.current;
    const at = trackHour(clockHour);
    if (at === null) return Math.floor(clockHour) + 1;
    const { hoursPerSecond, playing } = runtime.clock.getSnapshot();
    return Math.floor(clockHour) + Math.min(aheadEnd(hours, at, hoursPerSecond, playing), heldWindowsEnd(spans, held, at)) - at;
  }, [runtime.clock]);
  const block = useAheadHour(runtime.clock, ahead, end);
  const headWindow = useAtHead(runtime.clock, following && !ahead, clockHour => battleWindowIndex(windows ?? [], hourAt(clockHour) ?? first));
  const playheadWindow = ahead ? battleWindowIndex(windows ?? [], hourAt(block.hour) ?? first) : headWindow;
  const blockHour = windows?.[playheadWindow]?.lastHour ?? null;
  const browseIndex = browsing ? battleWindowIndex(windows ?? [], browse.hour!) : playheadWindow;
  const [visibleWindows, setVisibleWindows] = useDistinctState<VisibleWindows>(NO_VISIBLE_WINDOWS, sameWindows);
  const visibleIndices = visibleWindows.identity === identity ? visibleWindows.windows : [];
  const inBranch = !!track && playheadWindow >= 0;
  const wantedWindows = inBranch && view === 'log' ? battleWindows(manifest.data, windows?.[playheadWindow]?.firstHour ?? first, live,
    [...visibleIndices, ...(browsing ? [browseIndex] : [])], first) : [];
  const history = useProductWindows<Window>(runtime.client, identity, index => (productRequest({ sourceId: track!.source.id,
    revision: track!.source.revision, product: adapter.product, params: { branchId: track!.branch.id, window: index }, asksFrom: windows?.[index]?.firstHour })),
  wantedWindows, active && inBranch && manifest.data?.available === true, adapter.validateWindow, track?.source, true);
  const all = widget.countries.kind === 'all';
  const from = widget.time.mode === 'range' ? first : null;
  const battleChanges = useMemo(() => adapter.changes(history.data), [adapter, history.data]);
  const whole = useMemo(() => {
    const matching = countryMatchChangeHours(match.matches);
    return !all && matching === undefined ? null : new Set([...battleChanges.whole, ...matching ?? [], ...(from === null ? [] : [from])]);
  }, [battleChanges, match.matches, all, from]);
  const changes = useMemo(() => [...new Set([...battleChangeHours(battleChanges), ...whole ?? []])].sort((a, b) => a - b), [battleChanges, whole]);
  blockInputs.current = { hourAt, windows: windows ?? [], held: history.data, changes };
  const structureHour = ahead ? hourAt(block.hour) : null;
  const revealHour = useCallback((clockHour: number) => hourAt(clockHour) ?? -Infinity, [hourAt]);
  const reveal = useRevealClock(runtime.clock, ahead, revealHour);
  const headHour = useAtHead(runtime.clock, following && view === 'biggest', clockHour => {
    const at = hourAt(clockHour);
    return at === null || !track ? null : following ? rankingHour(at, first, track.branch.lastHour) : at;
  });
  const hour = browsing ? Math.min(browse.hour!, blockHour ?? browse.hour!) : view === 'biggest' ? headHour
    : following ? structureHour : hourAt(runtime.clock.getHour());
  const countries = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const rankingSpan = hour !== null ? gate.span(Math.min(first, hour), hour) : null;
  const rankingRequest = track && rankingSpan && inBranch ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: adapter.rankingProduct, params: { branchId: track.branch.id, ...rankingSpan, topN, measure,
      countries: { countries: countries.tags?.map(countryFromKey) ?? null,
        membership: countries.membership?.flatMap(run => { const held = gate.span(run.firstHour, run.lastHour); return held ? [{ ...run, ...held }] : []; }) ?? null,
        unobserved: countries.unobserved.flatMap(span => gate.span(span.firstHour, span.lastHour) ?? []) } } }) : null;
  const ranking = useProduct<Ranking>(runtime.client, rankingRequest,
    active && inBranch && view === 'biggest' && match.ready && !match.reason && manifest.data?.available === true, adapter.validateRanking);
  const shown = useMemo(() => !inBranch || hour === null || !match.ready ? NO_ROWS : view === 'biggest' ? adapter.rankedRows(ranking.data, hour)
    : adapter.rows(history.data, hour, match.matches, all, from),
  [adapter, inBranch, view, ranking.data, history.data, hour, match, all, from]);
  const rowsAt = useMemo(() => heldAtHour((at: number) => view === 'biggest' ? adapter.rankedRows(ranking.data, at) : adapter.rows(history.data, at, match.matches, all, from)),
    [adapter, view, ranking.data, history.data, match, all, from]);
  const rowAt = useMemo(() => view === 'biggest' ? (id: string, at: number) => rowsById(rowsAt(at)).get(id) ?? null
    : (id: string, at: number) => adapter.row(history.data, id, at, match.matches, all, from), [view, rowsAt, adapter, history.data, match, all, from]);
  const blockLast = structureHour === null ? null : structureHour + block.until - 1 - Math.floor(block.hour);
  const states = useMemo(() => {
    if (view !== 'log' || !inBranch || hour === null || !match.ready) return NO_LOG as readonly LogRow<Row>[];
    const hours = [hour, ...blockLast === null ? [] : changes.filter(change => change > hour && change <= blockLast)];
    return battleStateRows(hours, at => adapter.rows(history.data, at, match.matches, all, from), live && !browsing, adapter.state,
      at => whole === null || whole.has(at) ? null : battleChanges.battles.get(at) ?? [], (id, at) => adapter.row(history.data, id, at, match.matches, all, from));
  }, [view, inBranch, hour, blockLast, changes, whole, battleChanges, match, adapter, history.data, all, from, live, browsing]);
  const head = useCallback(() => following && !browsing && view === 'log' ? hourAt(runtime.clock.getHour()) : null, [following, browsing, view, hourAt, runtime.clock]);
  const paintHead = useMemo(() => following && !browsing
    ? { hour: () => hourAt(runtime.clock.getHour()), rowsAt, clock: runtime.clock } : undefined, [following, browsing, hourAt, rowsAt, runtime.clock]);
  const [visible, setVisible] = useDistinctState<readonly Row[]>(NO_ROWS, sameRows);
  const currentRows = useMemo(() => view === 'log' ? null : new Map(shown.map(row => [row.id, row])), [view, shown]);
  const drawn = useMemo(() => ahead ? NO_ROWS : currentRows ? visible.flatMap(row => { const current = currentRows.get(row.id); return current ? [current] : []; }) : visible,
    [ahead, visible, currentRows]);
  const wanted = useMemo(() => battleCountryRequests(ahead ? states.map(row => row.item) : drawn), [ahead, states, drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(country => country.hour)).size));
  const country = useCallback((tag: string, at: number) => labelAt(at)(tag), [labelAt]);
  const content = useContentRevision(runtime.client);
  const namesOwner = identity === null ? null : JSON.stringify([identity, content, adapter.landPlaceNames === true]);
  const leading = useMemo(() => {
    if (!ahead || hour === null) return NO_ROWS;
    const rows = states.map(row => row.item), shownAt = states.map(row => (row.revealedHour ?? row.hour) <= hour);
    return [...rows.filter((_, at) => shownAt[at]), ...rows.filter((_, at) => !shownAt[at])];
  }, [ahead, hour, states]);
  const windowPlaces = useMemo(() => ahead && adapter.windowPlaces ? placesLeadingWith(leading, adapter.windowPlaces(history.data)) : null,
    [ahead, adapter, history.data, leading]);
  const drawnPlaces = useMemo(() => ahead ? NO_PLACES : battlePlaceRequests(drawn), [ahead, drawn]);
  const recordedPlaces = useMemo(() => (windowPlaces ?? drawnPlaces).filter(place => gate.recorded(place.hour) !== null), [windowPlaces, drawnPlaces, gate.recorded]);
  const readPlaces = useCallback((places: readonly PlaceAt[], signal: AbortSignal) => loadProduct<PlaceNamesReply>(runtime.client, productRequest({ sourceId: track!.source.id,
    revision: track!.source.revision, product: 'place-names', params: { branchId: track!.branch.id,
      places: places.flatMap(place => { const hour = gate.recorded(place.hour); return hour === null ? [] : [{ ...place, hour }]; }), states: [], land: adapter.landPlaceNames === true } }),
  signal, validatePlaceNamesReply, track?.source).then(reply => ({ values: reply.places.map(place => [placeKey(place.provinceId, place.hour), place] as const), warnings: reply.warnings })),
  [runtime.client, track, adapter.landPlaceNames, gate.recorded]);
  const placeReads = useKeyedReads<PlaceAt, PlaceName>(track ? namesOwner : null, recordedPlaces, active, placeAtKey, readPlaces);
  const knownNames = placeReads.reads.values;
  const placeNamesHeld = useMemo(() => ({ places: [...knownNames.values()], states: [], warnings: [] }), [knownNames, placeReads.version]);
  const named = useMemo(() => namedBattles(shown, placeNamesHeld), [shown, placeNamesHeld]);
  const namedStates = useMemo(() => {
    const items = namedBattles(states.map(row => row.item), placeNamesHeld);
    return states.map((row, index) => items[index] === row.item ? row : { ...row, item: items[index] });
  }, [states, placeNamesHeld]);
  const placePending = useCallback((row: BattleRow & { provinceId: number | null }) => row.provinceId !== null && placeReads.reads.pending({ provinceId: row.provinceId, hour: row.place.namedAt ?? row.place.hour }),
  [placeReads.reads, placeReads.version]);
  const details = adapter.details;
  const detailWanted = useMemo(() => (!details ? NO_ASKS : ahead ? [...leading.map(row => details.ask(row, true)), ...details.held(history.data)]
    : drawn.map(row => details.ask(row, false))).filter(ask => gate.recorded(ask.hour) !== null), [details, ahead, history.data, drawn, leading, gate.recorded]);
  const readDetails = useCallback((battles: readonly BattleDetailAsk[], signal: AbortSignal) => loadProduct(runtime.client, productRequest({ sourceId: track!.source.id,
    revision: track!.source.revision, product: details!.product, params: { branchId: track!.branch.id,
      battles: battles.flatMap(battle => { const hour = gate.recorded(battle.hour); return hour === null ? [] : [{ ...battle, hour }]; }) } }), signal, details!.validate, track?.source)
    .then(reply => ({ values: reply.rows.map(row => [detailKey(row), row] as const), warnings: reply.warnings })), [runtime.client, track, details, gate.recorded]);
  const detailReads = useKeyedReads<BattleDetailAsk, BattleDetailAsk>(details && track && identity !== null ? JSON.stringify([identity, content, details.product]) : null,
    detailWanted, active, detailKey, readDetails);
  const described = useMemo(() => {
    if (!details) return null;
    const reads = detailReads.reads;
    return {
      description: (row: Row) => { const ask = details.ask(row, ahead); return details.description(reads.values.get(detailKey(ask)), { pending: reads.pending(ask), error: reads.failure(ask) }); },
      tooltipRows: (row: Row) => { const ask = details.ask(row, ahead); return details.tooltipRows(reads.values.get(detailKey(ask)), reads.failure(ask)); },
    };
  }, [details, ahead, detailReads.reads, detailReads.version]);
  const onVisibleRows = useCallback((drawnRows: readonly Row[]) => {
    if (!ahead) setVisible(currentRows ? drawnRows.map(row => currentRows.get(row.id) ?? row) : drawnRows);
    if (view === 'log') {
      const indices = [...new Set(drawnRows.map(row => row.windowIndex))].sort((a, b) => a - b);
      if (ahead && indices.every(index => wantedWindows.includes(index))) return;
      setVisibleWindows({ identity, at: playheadWindow, windows: indices });
    }
  }, [ahead, currentRows, view, identity, playheadWindow, wantedWindows, setVisible, setVisibleWindows]);
  const warnings = useMemo(() => view === 'biggest' ? [...(ranking.data?.contentWarnings ?? NO_WARNINGS),
    ...(ranking.data?.warnings ?? []).map(w => `Hour ${w.hour}: ${w.reason} (${w.count})`)]
    : battleWarnings(history.data, first, hour ?? first), [view, ranking.data, history.data, first, hour]);
  const failures = (reads: { failures: ReadonlyMap<string, Failure> }, what: string) => [...new Set([...reads.failures.values()].map(error => `${what}: ${error.message}`))];
  const diagnostics = useMemo(() => [...new Set([...(manifest.data?.contentWarnings ?? NO_WARNINGS), ...warnings,
    ...placeReads.reads.warnings, ...detailReads.reads.warnings, ...failures(placeReads.reads, 'Place names'), ...failures(detailReads.reads, 'Battle details'),
    ...(adapter.diagnostics ?? NO_WARNINGS)])],
  [warnings, manifest.data, placeReads.reads, placeReads.version, detailReads.reads, detailReads.version, adapter.diagnostics]);
  useDiagnostics(onDiagnostics, diagnostics);
  const root = useRef<HTMLDivElement>(null);
  useKeyedPainter(placeReads.reads as never, root, 'place', key => {
    const place = placeReads.reads.values.get(key), loading = !place && placeReads.reads.pendingKey(key);
    return { text: place?.name ?? (loading ? 'Loading…' : adapter.labels.unknownPlace), loading };
  });
  useKeyedPainter(detailReads.reads as never, root, 'detail', key => {
    const reads = detailReads.reads, pending = reads.pendingKey(key);
    return { text: details?.description(reads.values.get(key), { pending, error: reads.failures.get(key) }) ?? '', loading: pending && !reads.values.has(key) };
  });
  const keyed = useMemo(() => ({ place: (row: Row) => row.provinceId === null ? null : placeAtKey({ provinceId: row.provinceId, hour: row.place.namedAt ?? row.place.hour }),
    ...(details ? { detail: (row: Row) => detailKey(details.ask(row, ahead)) } : {}) }), [details, ahead]);
  useWhyCommit('BattleLogView', { block: block.hour, until: block.until, headWindow, windows: history.data, manifest: manifest.data, ranking: ranking.data, names: placeReads.version,
    details: detailReads.version, labels: labelAt, visible, visibleWindows, groups: groups.data, browse });

  if (!track) return <WidgetState kind="empty" icon={adapter.definition.icon} message={`Load a campaign to see its ${adapter.definition.title.toLowerCase()}.`} />;
  if (manifest.error) return <WidgetState kind="error" failure={manifest.error} onRetry={manifest.retry} />;
  if (manifest.data && !inBranch) return <WidgetState kind="outside" />;
  const groupFailure = widget.countries.kind !== 'all' && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!manifest.data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label={`Loading ${adapter.definition.title.toLowerCase()}…`} /></>;
  if (!manifest.data.available) return <>{groupFailure}<WidgetState kind="unavailable" message={manifest.data.unavailable ?? 'Battles were not recorded.'} /></>;
  const required = view === 'log' && browseIndex >= 0 ? [browseIndex] : [];
  const failed = view === 'biggest' ? ranking.error : required.map(index => history.errors.get(index)).find(Boolean);
  if (failed) return <>{groupFailure}<WidgetState kind="error" failure={failed} onRetry={view === 'biggest' ? ranking.retry : history.retry} /></>;
  const pending = view === 'biggest' ? !ranking.data : required.some(index => !history.data.has(index)) || hour === null;
  if (pending) return <>{groupFailure}<WidgetState kind="loading" label="Loading battles…" /></>;
  const censusHour = view === 'biggest' ? ranking.data?.censusHour ?? null : shownBattleCensusHour(history.data, hour!);
  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? preceding.reduce((low, index) => Math.min(low, index)) : 0;
  const earlier = view === 'log' && oldest > 0 && manifest.data.windows[oldest - 1].lastHour >= first;
  const earlierButton = earlier && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: manifest.data!.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier battles</button>;
  const navigation = view === 'log' && <div className="battle-navigation">{earlierButton}
    {browsing && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest battles</button>}
  </div>;
  if (!(view === 'log' ? states.length : shown.length) && !(adapter.view && censusHour !== null)) return <div className="battle-widget">{groupFailure}{navigation}<WidgetState kind={censusHour === null ? 'unavailable' : 'empty'}
    message={censusHour === null ? 'Not in this recording' : `No recorded ${adapter.definition.title.toLowerCase()} match these settings.`} /></div>;
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const summary = adapter.summary?.(shown, ranking.data, measure, view);
  const rankedMeasure = ranking.data?.effectiveMeasure ?? measure;
  const props: BattleLogProps<Row> = { rows: named, label: adapter.definition.title, labels: adapter.labels, country, live: live && !browsing, view,
    log: logSettings(options), measures: adapter.measures, sort: sort.column === 'measure' ? { ...sort, column: rankedMeasure } : sort,
    setOption, cellBars: options.cellBars === true, rankMarkers: options.rankMarkers === true, narrow: narrowTableOf(options), hoursPerSecond,
    menu: row => [jumpItem(runtime, track.id, battleJumpHour(row))], empty: `No recorded ${adapter.definition.title.toLowerCase()} match these settings.`,
    note: adapter.note, formatLoss: adapter.formatLoss, compactSides: adapter.compactSides, resultMeasure: adapter.resultMeasure,
    description: described?.description ?? adapter.description, tooltipRows: described?.tooltipRows ?? adapter.tooltipRows, onVisibleRows, placePending, paintHead,
    logRows: namedStates, reveal, keyed };
  const View = adapter.view;
  return <div ref={root} className="battle-widget">{groupFailure}
    {summary && <p className="battle-note">{summary}</p>}
    {view === 'biggest' && live && hour !== null && <p className="battle-note">Ranked as of {formatDateTime(hour)}.</p>}
    { }
    {widget.time.mode === 'range' && <p className="battle-note">Peak and duration cover the whole battle.</p>}
    {navigation}
    { }
    {view === 'log' && wantedWindows.some(index => index <= browseIndex && !history.data.has(index) && !history.errors.has(index)) && <p className="battle-note" role="status">Updating…</p>}
    {View ? <View {...props} context={{ runtime, track, active, widget, setOption, shownHour: hour!, headHour: head, rowsAt, rowAt, rankedMeasure, placePending, heldPlaces: windowPlaces }} /> : <BattleLog {...props} />}
  </div>;
}
