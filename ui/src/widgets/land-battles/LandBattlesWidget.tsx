import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { logSettings } from '../../charts/log/logOptions';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { selectedTags } from '../../dashboard/countrySelection';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { LandBattlesReply, LandBattlesManifestReply, LandBattlesRankingReply, PlaceNamesReply } from '../../generated/dashboard';
import BattleLog from '../battles/BattleLog';
import { battleCountryRequests, battleJumpHour, battleValueText } from '../battles/battleRows';
import type { BattleLabels, BattleMeasure, BattleSideSummary } from '../battles/battleRows';
import { DEFAULT_SORT, DEFAULT_TOP_N, OPTIONS } from './definition';
import { battleWarnings, battleWindowIndex, battleWindows, rankedLandBattleRows, landBattlePlaceRequests, landBattleRows, namedLandBattles,
  shownCensusHour, validatePlaceNamesReply, validateLandBattlesReply, validateLandBattlesManifest, validateLandBattlesRanking } from './landBattlesData';
import { casualtyRankingNote, landCasualtiesMissing, landCasualtyText, landDivisionText, landLossText } from './landBattlesData';
import type { LandBattleRow } from './landBattlesData';
import './landBattles.css';

const NO_ROWS: readonly LandBattleRow[] = [];
const NO_WARNINGS: string[] = [];
const LABELS: BattleLabels = { place: 'Battle', date: 'Date', start: 'Started', end: 'Ended', winner: 'Winner',
  ongoing: 'Ongoing', unknownPlace: 'Unnamed province', unknownStart: 'Start not recorded' };
const SIDES: BattleSideSummary = { label: 'Sides', countLabel: 'Divisions (incl. reserves)', separator: 'v',
  unknown: 'Not in this recording', formatCount: landDivisionText };
const MEASURES: readonly BattleMeasure<LandBattleRow>[] = [
  { label: 'Casualties', title: 'Casualties', value: row => row.casualties,
    format: (value, row) => landCasualtyText(value, row), tooltipFormat: (value, row) => landCasualtyText(value, row, true) },
  { label: 'Peak size', title: 'Peak divisions', hint: 'Largest simultaneous recorded division count, incl. reserves. ≥ marks a recorded minimum.',
    value: row => ({ label: 'Peak size', value: row.peak.value, atLeast: row.start?.bound === 'by' || row.end !== null && 'unknown' in row.end }),
    format: value => battleValueText(value), tooltipFormat: (value, row) => `${battleValueText(value)} at ${formatDateTime(row.peak.hour)}` },
  { label: 'Length', title: 'Length', value: row => row.length, format: value => `${battleValueText(value)} h` },
];
const NOTE = 'Side counts include reserves. Peak is the largest simultaneous recorded roster. ≥ marks a recorded minimum. Length uses recorded reads/rounds; gaps split battles.';
const battleNote = (row: LandBattleRow) => `Province ${row.provinceId}. ${NOTE} Side counts read at ${formatDateTime(row.readHour)}.${row.end !== null && row.lossRead && !row.lossRead.terminal ? ' Terminal losses not recorded; losses shown are so far.' : ''}`;

export default function LandBattlesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  const playbackHour = useTableHour(runtime.clock, active && live);
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'biggest' ? 'biggest' : 'log';
  const measure = typeof options.measure === 'string' ? options.measure : 'Casualties';
  const topN = typeof options.topN === 'number' ? options.topN : DEFAULT_TOP_N;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window ? Math.floor(window.lastHour) : null;
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const manifestRequest = track ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battles-manifest', params: { branchId: track.branch.id } } : null;
  const manifest = useProduct<LandBattlesManifestReply>(runtime.client, manifestRequest, active, validateLandBattlesManifest);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const playheadWindow = battleWindowIndex(manifest.data?.windows ?? [], hour ?? first);
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const shownHour = view === 'log' && browse.identity === identity && browse.hour !== null ? Math.min(browse.hour, hour ?? browse.hour) : hour;
  const browseIndex = battleWindowIndex(manifest.data?.windows ?? [], shownHour ?? first);
  const [visibleWindows, setVisibleWindows] = useState<{ identity: string | null; at: number; windows: number[] }>({ identity: null, at: -1, windows: [] });
  const visibleIndices = visibleWindows.identity === identity && visibleWindows.at === playheadWindow ? visibleWindows.windows : [];
  const wantedWindows = view === 'log' ? battleWindows(manifest.data, hour ?? first, live, [...visibleIndices, ...(browse.hour !== null && browse.identity === identity ? [browseIndex] : [])], first) : [];
  const history = useProductWindows<LandBattlesReply>(runtime.client, identity, index => ({ sourceId: track!.source.id,
    revision: track!.source.revision, product: 'land-battles', params: { branchId: track!.branch.id, window: index } }),
  wantedWindows, active && manifest.data?.available === true, validateLandBattlesReply);
  const countries = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const rankingRequest = track && hour !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battles-ranking', params: { branchId: track.branch.id, firstHour: first, lastHour: hour, topN, measure, countries } } : null;
  const ranking = useProduct<LandBattlesRankingReply>(runtime.client, rankingRequest,
    active && view === 'biggest' && match.ready && !match.reason && manifest.data?.available === true, validateLandBattlesRanking);
  const shown = useMemo(() => shownHour === null || !match.ready ? NO_ROWS : view === 'biggest' ? rankedLandBattleRows(ranking.data, shownHour)
    : landBattleRows(history.data, shownHour, match.matches, widget.countries.kind === 'all', widget.time.mode === 'range' ? first : null),
  [view, ranking.data, history.data, hour, shownHour, match, widget.countries.kind, widget.time.mode, first]);
  const [visible, setVisible] = useState<readonly LandBattleRow[]>(NO_ROWS);
  const currentRows = useMemo(() => new Map(shown.map(row => [row.id, row])), [shown]);
  const drawn = useMemo(() => visible.flatMap(row => { const current = currentRows.get(row.id); return current ? [current] : []; }), [visible, currentRows]);
  const wanted = useMemo(() => battleCountryRequests(drawn), [drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(country => country.hour)).size));
  const country = useCallback((tag: string, at: number) => labelAt(at)(tag), [labelAt]);
  const places = useMemo(() => landBattlePlaceRequests(drawn), [drawn]);
  const placeRequest = track && places.length ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'place-names', params: { branchId: track.branch.id, places, states: [] } } : null;
  const placeData = useProduct<PlaceNamesReply>(runtime.client, placeRequest, active, validatePlaceNamesReply);
  const named = useMemo(() => namedLandBattles(shown, placeData.data), [shown, placeData.data]);
  const onVisibleRows = useCallback((drawn: readonly LandBattleRow[]) => {
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
  useEffect(() => {
    onDiagnostics([...new Set([...(manifest.data?.contentWarnings ?? NO_WARNINGS), ...warnings,
      ...(placeData.data?.warnings ?? NO_WARNINGS), ...(placeData.error ? [`Place names: ${placeData.error}`] : [])])]);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, manifest.data, placeData.data, placeData.error, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its land battles." />;
  if (manifest.error) return <WidgetState kind="error" message={manifest.error} onRetry={manifest.retry} />;
  if (groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign branch." />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!manifest.data || !match.ready) return <WidgetState kind="loading" label="Loading land battles…" />;
  if (!manifest.data.available) return <WidgetState kind="unavailable" message={manifest.data.unavailable ?? 'Not in this recording'} />;
  const required = view === 'log' && browseIndex >= 0 ? [browseIndex] : [];
  const failed = view === 'biggest' ? ranking.error : required.map(index => history.errors.get(index)).find(Boolean);
  if (failed) return <WidgetState kind="error" message={failed} onRetry={view === 'biggest' ? ranking.retry : history.retry} />;
  const pending = view === 'biggest' ? !ranking.data : required.some(index => !history.data.has(index));
  if (pending) return <WidgetState kind="loading" label="Loading battles…" />;
  const censusHour = view === 'biggest' ? ranking.data?.censusHour ?? null : shownCensusHour(history.data, shownHour!);
  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? Math.min(...preceding) : 0;
  const earlier = view === 'log' && oldest > 0 && manifest.data.windows[oldest - 1].lastHour >= first;
  const earlierButton = earlier && <button type="button" onClick={() => { setBrowse({ identity, hour: manifest.data!.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier battles</button>;
  const navigation = view === 'log' && <div className="land-battle-navigation">{earlierButton}
    {shownHour !== hour && <button type="button" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest battles</button>}
  </div>;
  if (!shown.length) return <div className="land-battles">{navigation}<WidgetState kind={censusHour === null ? 'unavailable' : 'empty'}
    message={censusHour === null ? 'Not in this recording' : 'No recorded land battles match these settings.'} /></div>;
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const casualtyNote = view === 'biggest' && measure === 'Casualties' ? casualtyRankingNote(ranking.data)
    : landCasualtiesMissing(shown) ? 'Casualties: Not in this recording.' : null;
  return <div className="land-battles">
    <p className="land-battle-note">{casualtyNote && `${casualtyNote} `}Divisions include reserves.</p>
    {widget.time.mode === 'range' && <p className="land-battle-note">Peak size and length include recorded reads before the selected range.</p>}
    {navigation}
    {shownHour !== hour && <p className="land-battle-note">Battle reads through {formatDateTime(shownHour!)}.</p>}
    {censusHour !== null && censusHour < shownHour! && <p className="land-battle-note">Shown census reads through {formatDateTime(censusHour)}.</p>}
    <BattleLog rows={named} label="Land battles" labels={LABELS} country={country} live={live && shownHour === hour} view={view}
      log={logSettings(options)} measures={MEASURES} sort={sort.column === 'measure' ? { ...sort, column: ranking.data?.effectiveMeasure ?? 'Peak size' } : sort}
      setOption={setOption} cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true} hoursPerSecond={hoursPerSecond}
      menu={row => [jumpItem(runtime, track.id, battleJumpHour(row))]} empty="No recorded land battles match these settings."
      note={battleNote} formatLoss={landLossText} compactSides={SIDES} onVisibleRows={onVisibleRows} />
  </div>;
}
