import { RefusedError } from '../../dashboard/queryError.ts';
import { useCallback, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { dayLabel } from '../../charts/draw';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatFixed } from '../../charts/valueAxis';
import { useTableHour } from '../../charts/table/useTableHour';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDetails } from '../../dashboard/factionChoice';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useHeldFailures } from '../../dashboard/heldFailures';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { ABSENT_FACTION, asksDetails, completedCount, filterGoals, goalRows, goalsAt, missingProgress, termOrder, unmeasured } from './factionGoalsData';
import type { GoalFilter, GoalRow } from './factionGoalsData';
import { partAt, useGoalHistory } from './goalHistory';
import './factionGoals.css';

const rowKey = (row: GoalRow) => row.key;
const NO_WARNINGS: string[] = [];

export default function FactionGoalsWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const { groups, pick } = useChosenFaction(widget, runtime, track, hour, active);
  const faction = pick?.faction ?? null;
  // The goal history answers each playback hour locally; any hour it cannot answer asks its own details.
  const history = useGoalHistory(runtime.client, track, faction?.id ?? null, active);
  const part = partAt(history.parts, hour);
  const reply = part?.data;
  const lookup = useMemo(() => reply ? goalsAt(reply, hour) : null, [reply, hour]);
  const at = track && faction ? JSON.stringify([track.source.id, track.branch.id, faction.id, hour]) : '';
  // Retrying a proven absence asks only that hour again.
  const [retried, setRetried] = useState<string | null>(null);
  const fallback = !!faction && asksDetails(lookup, part?.error !== undefined, retried === at);
  const details = useFactionDetails(runtime, track, fallback ? faction.id : null, hour, active);
  const data = fallback ? details.data : lookup?.kind === 'held' ? lookup.view : undefined;
  // Asked details still shown while the next hour loads keep the hour they answer.
  const dataHour = fallback && details.data && details.moment ? (JSON.parse(details.moment) as [number])[0] : hour;
  const absent = useMemo(() => new RefusedError(ABSENT_FACTION), []);
  const failure = fallback ? details.error : lookup?.kind === 'absent' ? absent : undefined;
  const retry = fallback ? details.retry : () => setRetried(at);
  const historyFailure = history.error === undefined ? null : `Goal history unavailable: ${history.error}`;
  useHeldFailures(history.error ? [history.error] : [], history.retry);
  const all = useMemo(() => data && !data.goalReason ? goalRows(data) : [], [data]);
  const rows = useMemo(() => filterGoals(all, options.goals as GoalFilter), [all, options.goals]);
  const order = useMemo(() => termOrder(all), [all]);
  const diagnostics = useMemo(() => {
    const held = historyFailure ? [historyFailure] : NO_WARNINGS;
    if (!data) return failure ? [failure.message, ...held] : held;
    const unnamed = all.filter(row => row.definition.reason).length;
    const unnamedTerms = all.filter(row => row.category && !row.categoryName).length;
    return [...held, ...data.warnings, ...data.refusals.map(refusal => `${refusal.fact} (${refusal.count}): ${refusal.reason}`),
      ...(data.goalReason ? [`Goals unavailable: ${data.goalReason}`] : []),
      ...(unnamed ? [`${unnamed} goal ${unnamed === 1 ? 'name is' : 'names are'} unavailable.`] : []),
      ...(unnamedTerms ? [`${unnamedTerms} goal ${unnamedTerms === 1 ? 'term has' : 'terms have'} no name in the recorded content.`] : [])];
  }, [data, failure, all, historyFailure]);
  useDiagnostics(onDiagnostics, diagnostics);
  const columns = useMemo<readonly TableColumn<GoalRow>[]>(() => [
    { key: 'goal', header: 'Goal', label: 'Goal', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name,
      render: row => <span className="faction-goal-name" title={row.name}>{row.definition.icon ? <img src={row.definition.icon} alt="" /> : <ArtFallback icon="target" />}
        <span className={row.definition.reason ? 'faction-goal-missing' : undefined}>{row.name}</span></span> },
    { key: 'progress', header: 'Progress', label: 'Progress', numeric: true, width: 'minmax(120px, 1fr)', priority: 2, renderMissing: true,
      value: row => row.progress, text: row => row.progress == null ? missingProgress(row) : `${formatFixed(row.progressRaw!, 3, 1)}%`,
      render: row => <span className="faction-goal-progress">{!unmeasured(row) && <ProgressMeter value={row.progress} label={`${row.name} progress`}
        state={row.state === 'completed' ? 'done' : row.state === 'cancelled' ? 'idle' : 'live'} />}
        {row.progress == null ? <span className="faction-goal-missing">{missingProgress(row)}</span> : <span>{formatFixed(row.progressRaw!, 3, 1)}%</span>}</span> },
    { key: 'status', header: 'Status', label: 'Status', width: 'max-content', priority: 1, value: row => row.statusText,
      render: row => <span className={`pill${row.state === 'inProgress' ? ' accent' : row.state === 'completed' ? ' positive' : ''}`}>{row.statusText}</span> },
  ], []);
  const tooltip = useCallback((row: GoalRow) => {
    const rows: TooltipRow[] = [
      { label: 'Status', value: row.history ? `${row.statusText} (held in the faction’s list)` : row.statusText },
      { label: 'Progress', value: row.progress == null ? missingProgress(row) : `${formatFixed(row.progressRaw!, 3, 3)}%` },
      ...(row.categoryName ? [{ label: 'Term', value: row.categoryName }] : []),
    ];
    return fitTooltip([{ title: row.name, subtitle: dayLabel(dataHour), rows, description: row.definition.description ?? undefined }], viewport);
  }, [dataHour, viewport]);
  const groupBy = useCallback((row: GoalRow) => ({ key: row.category ?? '', order: order.get(row.category ?? '') ?? 0,
    label: row.categoryName ?? (row.category ? 'Term name unavailable' : 'No term') }), [order]);
  const header = headerSlot && data && !data.goalReason && createPortal(<span className="pill" title={faction ? factionNameAt(faction, dataHour) : undefined}>
    {completedCount(all)} completed</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see faction goals." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction goals…" /></>;
  if (!faction) return <>{groupFailure}<WidgetState kind="empty" icon="target" message={noFactionMessage(pick!.reason as 'none' | 'unobserved' | 'unknown', widget.time.mode === 'follow')} /></>;
  if (failure) return <>{groupFailure}<WidgetState kind="error" failure={failure} onRetry={retry} /></>;
  if (!data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction goals…" /></>;
  if (data.goalReason) return <>{groupFailure}<WidgetState kind="unavailable" message="This faction’s goals were not recorded at this time." /></>;
  if (!all.length) return <>{groupFailure}{header}<WidgetState kind="empty" icon="target" message="This faction has no goals." /></>;
  if (!rows.length) return <>{groupFailure}{header}<WidgetState kind="empty" icon="target" message="No goals match this setting." /></>;
  return <div className="faction-goals">{groupFailure}
    {header}
    <Table rowRevision={row => [row, columns]} key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, faction.id])} label={`${factionNameAt(faction, dataHour)} goals`}
      rows={rows} columns={columns} rowKey={rowKey} groupBy={groupBy} sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption}
      cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
      muted={row => row.state === 'cancelled'} tooltip={tooltip} />
  </div>;
}
