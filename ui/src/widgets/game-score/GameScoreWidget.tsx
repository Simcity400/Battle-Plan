import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf, tableSortToken } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { useCountryColours } from '../../charts/race/countryColours';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAxisValue } from '../../charts/valueAxis';
import { flagReadSpec } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { countryAliasesAt } from '../../dashboard/countryIdentity';
import { rankedCandidates, resolveCountries } from '../../dashboard/groups';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { BAR_PARTS, SCORE_PARTS, formatMultiplier, formatScore, gameScoreCountryCell, gameScoreCountryKnown, gameScoreFaction, gameScoreLabel, gameScoreLooks, gameScoreModel, gameScoreRace, gameScoreRow, gameScoreRows, gameScoreTooltips, scoreValue, validateGameScoreReply } from './gameScoreData';
import type { ScoreRow } from './gameScoreData';
import type { GameScoreReply } from '../../generated/dashboard';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly ScoreRow[] = [];
const NO_TAGS: readonly string[] = [];
const rowKey = (row: ScoreRow) => row.key;
const tieKey = (row: ScoreRow) => row.tag;

export default function GameScoreWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const viewport = useViewportHeight();
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'race' ? 'race' : 'table';
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'game_score', params: { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<GameScoreReply>(runtime.client, request, active, validateGameScoreReply);
  const model = useMemo(() => data ? gameScoreModel(data) : null, [data]);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = widget.countries.kind === 'all' ? model?.tags ?? NO_TAGS : selectedTags(widget.countries, track);
  const fallbackKey = JSON.stringify(fallback);
  const resolved = useMemo(() => resolveCountries(widget.countries, [...fallback], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallbackKey, groups.data, track?.branch.lastHour]);
  const ranking = useMemo(() => ({ column: view === 'race' ? 'total' : SCORE_PARTS.includes(contentKey(String(options.topMeasure)) as typeof SCORE_PARTS[number])
    ? contentKey(String(options.topMeasure)) : 'land', direction: 'descending' as const }), [view, options.topMeasure]);
  const rows = useMemo(() => {
    if (!model || (grouped && !groups.data)) return NO_ROWS;
    const candidates = widget.countries.kind === 'all' ? model.tags : resolved.top == null && resolved.membership === null
      ? countryAliasesAt(model.identities, resolved.tags, hour) : rankedCandidates(resolved, hour, model.tags);
    return gameScoreRows(model, candidates, hour, ranking, resolved.top);
  }, [model, grouped, groups.data, widget.countries.kind, resolved, hour, ranking]);
  const raceRows = view === 'race' ? rows : NO_ROWS;
  const [visible, setVisible] = useState<readonly ScoreRow[]>(NO_ROWS);
  const onVisible = useCallback((rows: readonly ScoreRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => (view === 'race' ? raceRows : visible).filter(row => gameScoreCountryKnown(row.identity))
    .map(row => ({ hour: row.hour, tag: row.tag })), [view, raceRows, visible]);
  const flagReads = useCountryFactsRead(runtime.client, track, wanted, active, flagReadSpec('shield'));
  const columns = useMemo<readonly TableColumn<ScoreRow>[]>(() => data ? [
    { key: 'country', header: gameScoreLabel(data.labels, 'country'), label: gameScoreLabel(data.labels, 'country'), width: 'minmax(0, 2fr)',
      hint: data.hints.country,
      value: row => row.identity.name, renderMissing: true,
      render: row => {
        const cell = gameScoreCountryCell(row, (tag, at) => flagReads.get(nameKey(at, tag)));
        return cell ? <CountryCell {...cell} /> : row.identity.name ?? 'Country name unavailable';
      } },
    ...SCORE_PARTS.map(part => ({ key: part, header: gameScoreLabel(data.labels, part), label: gameScoreLabel(data.labels, part), numeric: true, width: 'max-content', bar: 'visible' as const,
      hint: data.hints[part],
      value: (row: ScoreRow) => scoreValue(row, part), render: (row: ScoreRow) => formatScore(row.cells[part].raw) })),
    { key: 'multiplier', header: gameScoreLabel(data.labels, 'multiplier'), label: gameScoreLabel(data.labels, 'multiplier'), numeric: true, width: 'max-content',
      value: row => scoreValue(row, 'multiplier'), render: row => formatMultiplier(row.multipliers.total.raw, String(options.multiplierPrecision)) },
  ] : [], [data, flagReads, options.multiplierPrecision]);
  const tooltip = useCallback((row: ScoreRow) => fitTooltip(gameScoreTooltips(data!, row), viewport, { keep: SCORE_PARTS.length }), [data, viewport]);
  const split = options.split === true;
  const entries = useMemo(() => gameScoreRace(raceRows, split), [raceRows, split]);
  const parts = useMemo(() => data ? BAR_PARTS.map(id => ({ id, label: gameScoreLabel(data.labels, id) })) : [], [data]);
  const looks = useMemo(() => gameScoreLooks(model, hour, (tag, at) => flagReads.get(nameKey(at, tag))), [model, hour, flagReads]);
  const raceTags = useMemo(() => entries.map(entry => entry.tag), [entries]);
  const countryColour = useCountryColours(raceTags, looks.color, tokens, looks);
  const entityLooks = useMemo(() => ({ name: looks.name, flag: looks.flag, countryFlag: looks.countryFlag, color: countryColour }), [looks, countryColour]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []),
      ...(data?.scoreWarnings.map(warning => `${warning.kind}: ${warning.reason} (${warning.observations} observations)`) ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to see its game scores." />;
    if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign branch." />;
    if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
    if (grouped && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
    if (!data || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading game scores…" />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (!rows.length) return <WidgetState kind="unavailable" message="No countries were recorded for this selection at this hour. Gaps stay gaps." />;
    return undefined;
  })();
  if (view === 'race') return <CountryRaceView runtime={runtime} track={track} active={active} state={state} options={options} view="race"
    extent={[track?.branch.firstHour ?? 0, track?.branch.lastHour ?? 1]} playhead={hour} moment={hour} slots={entries.length}
    ranked={entries} lines={[]} parts={parts} raceLegendParts={split ? parts : []} partsLabel="Score parts" entityLooks={entityLooks}
    presentation={{ exact: false, held: false, countryLooks: false, tooltipKeep: SCORE_PARTS.length }}
    label="Game score" empty="No known Total scores at this hour. Choose the table to see unavailable parts and their reasons."
    formatValue={formatAxisValue}
    tooltip={(tag, atHour) => { const row = model && gameScoreRow(model, tag, Math.floor(atHour)); return row && data ? gameScoreTooltips(data, row, split) : null; }} />;
  if (state) return state;
  return <Table key={JSON.stringify([track!.source.id, track!.source.revision, track!.branch.id])} label="Game score" rows={rows} columns={columns}
    rowKey={rowKey} tieKey={tieKey} sort={sort} setOption={setOption} sortOptionValue={tableSortToken} groupBy={options.factions === true ? gameScoreFaction : undefined}
    cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={hoursPerSecond}
    menu={row => [jumpItem(runtime, track!.id, row.hour)]} />;
}
