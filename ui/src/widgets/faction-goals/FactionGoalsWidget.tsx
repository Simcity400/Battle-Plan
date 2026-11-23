import { useCallback, useEffect, useMemo, useRef } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { dayLabel } from '../../charts/draw';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { mapLive } from '../../charts/live';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDisplayNames, useFactionDetails } from '../../dashboard/factionChoice';
import type { NoFaction } from '../../dashboard/factionChoice';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { completedCount, filterGoals, goalDiagnostics, goalRows, goalTableValues, progressText, rowProgress, termOrder } from './factionGoalsData';
import type { GoalFilter, GoalRow } from './factionGoalsData';
import './factionGoals.css';

const rowKey = (row: GoalRow) => row.key;
const NO_WARNINGS: string[] = [];
const NO_ROWS: GoalRow[] = [];

export default function FactionGoalsWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  useChartTokens();
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { groups, pick, hour } = useChosenFaction(widget, runtime, track, active);
  const displayNames = useFactionDisplayNames(track?.source);
  const faction = pick?.faction ?? null;
  const head = useFactionDetails(runtime, widget, faction?.id ?? null, active);
  const data = head.data && head.data.faction === faction?.id ? head.data : undefined;
  const dataHour = head.at ?? hour ?? track?.branch.lastHour ?? 0;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const live = head.live;
  const tableLive = useMemo(() => mapLive(reporting(live, () => shown(shownAt.current())), goalTableValues), [live, shown]);
  const all = useMemo(() => data && !data.goalReason ? goalRows(data) : NO_ROWS, [data]);
  const rows = useMemo(() => filterGoals(all, options.goals as GoalFilter), [all, options.goals]);
  const order = useMemo(() => termOrder(all), [all]);
  const diagnostics = useMemo(() => data ? goalDiagnostics(data, all) : head.error ? [head.error.message] : NO_WARNINGS, [data, all, head.error]);
  useEffect(() => {
    onDiagnostics(diagnostics);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnostics, onDiagnostics]);
  const columns = useMemo<readonly TableColumn<GoalRow>[]>(() => [
    { key: 'goal', header: 'Goal', label: 'Goal', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name,
      render: row => <span className="faction-goal-name" title={row.name}>{row.definition.icon ? <img src={row.definition.icon} alt="" /> : <ArtFallback icon="target" />}
        <span className={row.definition.reason ? 'faction-goal-missing' : undefined}>{row.name}</span></span> },
    { key: 'progress', header: 'Progress', label: 'Progress', numeric: true, width: 'minmax(120px, 1fr)', priority: 2, bar: 1,
      value: () => null, paint: value => `${((value as number) * 100).toFixed(1)}%` },
    { key: 'status', header: 'Status', label: 'Status', width: 'max-content', priority: 1, value: row => row.statusText,
      render: row => <span className={`pill${row.state === 'inProgress' ? ' accent' : row.state === 'completed' ? ' positive' : ''}`}>{row.statusText}</span> },
  ], []);
  const tooltip = useCallback((row: GoalRow) => {
    const rows: TooltipRow[] = [
      { label: 'Status', value: row.history ? `${row.statusText} (held in the faction’s list)` : row.statusText },
      { label: 'Progress', value: progressText(rowProgress(live.read(), row), 3) },
      ...(row.categoryName ? [{ label: 'Term', value: row.categoryName }] : []),
    ];
    return fitTooltip([{ title: row.name, subtitle: dayLabel(dataHour), rows, description: row.definition.description ?? undefined }], viewport);
  }, [dataHour, viewport, live]);
  const groupBy = useCallback((row: GoalRow) => ({ key: row.category ?? '', order: order.get(row.category ?? '') ?? 0,
    label: row.categoryName ?? (row.category ? 'Term name unavailable' : 'No term') }), [order]);
  const header = headerSlot && data && !data.goalReason && createPortal(<span className="pill" data-tooltip={faction ? factionNameAt(faction, dataHour, displayNames[faction.id]) : undefined}>
    {completedCount(all)} completed</span>, headerSlot);
  const body = (() => {
    if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see faction goals." />;
    if (hour === null) return <WidgetState kind="outside" />;
    const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
    if (groupFailure && !groups.data) return groupFailure;
    if (!groups.data) return <WidgetState kind="loading" label="Loading faction goals…" />;
    if (!faction) return <>{groupFailure}<WidgetState kind="empty" icon="target" message={noFactionMessage(pick!.reason as NoFaction, widget.time.mode === 'follow')} /></>;
    if (head.error && !data) return <>{groupFailure}<WidgetState kind="error" failure={head.error} onRetry={head.retry} /></>;
    if (!data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction goals…" /></>;
    if (data.goalReason) return <>{groupFailure}<WidgetState kind="unavailable" message="This faction’s goals were not recorded at this time." /></>;
    if (!all.length) return <>{groupFailure}{header}<WidgetState kind="empty" icon="target" message="This faction has no goals." /></>;
    if (!rows.length) return <>{groupFailure}{header}<WidgetState kind="empty" icon="target" message="No goals match this setting." /></>;
    return <>{groupFailure}{header}
      <Table rowRevision={row => [row, columns]} key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, faction.id])} label={`${factionNameAt(faction, dataHour, displayNames[faction.id])} goals`}
        rows={rows} columns={columns} rowKey={rowKey} groupBy={groupBy} sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption}
        cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        muted={row => row.state === 'cancelled'} tooltip={tooltip} hoursPerSecond={hoursPerSecond} live={tableLive} /></>;
  })();
  return <div className="faction-goals" ref={head.root}>{body}</div>;
}
