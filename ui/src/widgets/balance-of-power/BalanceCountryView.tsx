import { useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, drawLine } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { pendingLayer } from '../../charts/prepared';
import { PreparedKey } from '../../charts/PreparedKey';
import type { Span } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import Timeline from '../../charts/timeline/Timeline';
import type { TimelineLane } from '../../charts/timeline/Timeline';
import { AXIS_HEIGHT, inSpans, ROW_HEIGHT } from '../../charts/timeline/layout';
import { describeRun, runState, startUnseen } from '../../charts/timeline/runState';
import type { TooltipRow } from '../../charts/tooltip';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { PlaybackMode } from '../../charts/view';
import { contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { BalanceHistory } from '../../generated/dashboard';
import { BALANCE_UNAVAILABLE, balanceAt, balanceAxis, balanceLanes, balanceName, balanceStatusAt, exactValue, plotRuns, RANGE_UNAVAILABLE, rangeName,
  SIDE_UNAVAILABLE, sideName, statusText } from './balanceOfPowerData';
import type { BalanceNames, BalanceSubject } from './balanceOfPowerData';

/** The balance's in-game name in strong text, then its state in a muted line, as
 * Ruling party heads its chart. The key shows on hover; members when it is shared. */
export function BalanceHeadline({ name, token, detail, members }: { name: string; token: string; detail: string; members?: string }) {
  return <div className="balance-headline">
    <strong data-tooltip={contentTitleHint(token)}>{name}</strong>
    {detail && <span data-tooltip={members}>{detail}</span>}
  </div>;
}

/** The members' names at the hour, as the header's tooltip lists a shared balance. */
export const membersText = (names: readonly string[] | null) => names && names.length > 1 ? `Members: ${names.join(', ')}` : undefined;

export interface BalanceViewProps {
  runtime: WidgetRuntime; track: BoundTrack; active: boolean;
  subject: BalanceSubject; names: BalanceNames;
  /** The balance shown, and the one the game shows the country at `at`. */
  balance: BalanceHistory; gameShown: string | null;
  countryName: string;
  /** The hour the header and lanes describe, and the last hour drawn (Live cuts later ones). */
  at: number; limit: number;
  extent: readonly [number, number]; playhead: number | null;
  playback: { mode: PlaybackMode; afterPlayhead: boolean; windowHours: number };
  /** Hours not recorded or still being prepared, and those still being prepared alone. */
  unknown: readonly Span[]; pending: readonly Span[];
  /** Members of each balance at `at`, where the hour's details have loaded. */
  membersOf: (token: string) => readonly string[] | null;
  pickOwner: unknown;
  /** Saves another balance to show; absent where the widget cannot save one. */
  onChoose?: (token: string) => void;
  onPick?: (hour: number) => void;
}

const WORDS = { since: (hour: number) => `Active since ${dayLabel(hour)}`, before: (hour: number) => `Held before ${dayLabel(hour)}`,
  ended: (hour: number) => `Ended ${dayLabel(hour)}`, endedBy: (hour: number) => `Ended by ${dayLabel(hour)}`, lastSeen: (hour: number) => `Last recorded ${dayLabel(hour)}` };

/** One country's balance: the header line, the value over its lifetime on the authored
 * ranges' gridlines, and one lane per balance the country had. */
export default function BalanceCountryView({ runtime, track, active, subject, names, balance, gameShown, countryName, at, limit, extent, playhead, playback,
  unknown, pending, membersOf, pickOwner, onChoose, onPick }: BalanceViewProps) {
  const viewport = useViewportHeight();
  const menu = useContextMenu('Chart actions');
  const name = balanceName(balance, names) ?? BALANCE_UNAVAILABLE;
  const isPending = (hour: number) => inSpans(pending, hour);
  const status = balanceStatusAt(balance, at, limit, unknown, isPending);
  const axis = useMemo(() => balanceAxis(balance), [balance]);
  const runs = useMemo(() => plotRuns(balance), [balance]);
  const keyHours = useMemo(() => balance.values.map(run => run.firstHour), [balance]);
  const layers = useMemo<ChartLayer[]>(() => [pendingLayer(pending), {
    draw: (ctx, frame, tokens, state) => { drawLine(ctx, frame, tokens, runs, tokens.text, { until: state.playhead ?? undefined }); },
  }], [pending, runs]);
  const lanes = useMemo(() => balanceLanes(subject, names, limit), [subject, names, limit]);
  const timelineLanes = useMemo<TimelineLane[]>(() => lanes.map(lane => {
    const label = lane.name ?? BALANCE_UNAVAILABLE;
    return { id: lane.token, label: describeRun(lane.spans, at, WORDS, unknown, track.branch.firstHour) ?? '', identity: { label }, icons: false, names: false,
      spans: lane.spans.map((span, index) => ({ id: `${lane.token}:${span.firstHour}`, firstHour: span.firstHour, lastHour: span.lastHour, label,
        emphasis: lane.token === balance.token, ...(lane.cut && index === lane.spans.length - 1 ? { openEnd: true } : {}) })) };
  }), [lanes, at, unknown, track.branch.firstHour, balance.token]);
  const byToken = useMemo(() => new Map(subject.balances.map(item => [item.token, item])), [subject]);

  const tooltip = (hour: number) => {
    const rows: TooltipRow[] = [];
    const state = balanceAt(balance, hour);
    if (state.value === null) {
      rows.push({ label: name, value: isPending(hour) ? 'Still being prepared' : inSpans(unknown, hour) ? 'Not recorded' : 'Not active', tone: 'muted', lookupKey: balance.token });
    } else {
      rows.push({ label: name, value: exactValue(state.value), strong: true, lookupKey: balance.token });
      if (state.range !== null) rows.push({ label: 'Range', value: rangeName(balance, state.range, names) ?? RANGE_UNAVAILABLE, lookupKey: state.range });
      if (state.trending !== null) rows.push({ label: 'Moving toward', value: sideName(balance, state.trending, names) ?? SIDE_UNAVAILABLE, lookupKey: state.trending });
    }
    return fitTooltip([{ title: dayLabel(hour), rows }], viewport);
  };
  const laneTooltip = (lane: TimelineLane, span: { firstHour: number; lastHour: number } | null) => {
    const item = byToken.get(lane.id);
    if (!item) return null;
    const title = balanceName(item, names) ?? BALANCE_UNAVAILABLE;
    const held = span ?? lanes.find(entry => entry.token === lane.id)?.spans.filter(entry => entry.firstHour <= at).at(-1) ?? null;
    const rows: TooltipRow[] = [];
    if (held) {
      // Bounds as the lane says them: a start or end next to unrecorded or pending hours is not a date.
      const gaps = [...unknown, ...pending];
      const start = startUnseen(held.firstHour, track.branch.firstHour, gaps) ? `held before ${dayLabel(held.firstHour)}` : `since ${dayLabel(held.firstHour)}`;
      const state = held.lastHour >= limit ? null : runState([held], Math.max(at, held.lastHour + 1), gaps);
      const end = !state || state.kind === 'held' || state.kind === 'none' ? null : state.kind === 'unknown' ? `last recorded ${dayLabel(state.lastSeen)}`
        : state.lastSeen == null ? `ended ${dayLabel(state.at)}` : `ended by ${dayLabel(state.at)}`;
      rows.push({ label: 'Active', value: end ? `${start}; ${end}` : start });
    }
    const members = membersOf(lane.id);
    if (members?.length) rows.push({ label: 'Members', value: members.join(', ') });
    if (lane.id === gameShown) rows.push({ label: 'Shown in the game at the playback date', tone: 'muted' });
    return { title, subtitle: lane.label, rows };
  };
  const members = membersOf(balance.token);
  return <div className="balance-view">
    <BalanceHeadline name={name} token={balance.token} detail={statusText(status, balance, names)} members={membersText(members)} />
    <div className="balance-chart">
      <TimeChart label={`${name} of ${countryName}`} extent={[extent[0], extent[1]]} playhead={playhead} playback={playback.mode}
        afterPlayhead={playback.afterPlayhead} windowHours={playback.windowHours} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
        branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: axis.min, yMax: axis.max, log: false }} valueTicks={axis.ticks}
        layers={layers} active={active} pickOwner={pickOwner} keyHours={keyHours}
        revision={`${balance.token}:${at}:${limit}:${runs.length}:${pending.length}`}
        tooltip={hour => tooltip(Math.floor(hour))}
        onPick={onPick ? hour => onPick(Math.floor(hour)) : undefined}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    <PreparedKey pending={pending} />
    {timelineLanes.length > 1 && <div className="balance-lanes" style={{ height: AXIS_HEIGHT + Math.min(timelineLanes.length, 3) * ROW_HEIGHT }}>
      <Timeline label={`Balances of ${countryName}`} extent={[extent[0], Math.max(extent[0] + 1, extent[1] + 1)]} playhead={playhead} labelHour={at} lanes={timelineLanes} showIcons={false} showNames={false}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} revision={`${balance.token}:${at}:${limit}`} pending={pending} unobserved={unknown}
        tooltip={(lane, span) => laneTooltip(lane, span)} labelMenu
        onContextMenu={(hour, x, y, lane) => menu.open(x, y, [
          ...lane && onChoose && lane.id !== balance.token ? [{ label: 'Show this balance', run: () => onChoose(lane.id) }] : [],
          jumpItem(runtime, track.id, hour)])} />
    </div>}
    {menu.menu}
  </div>;
}
