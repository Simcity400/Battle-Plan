import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { liveKey, valuesOf } from '../../charts/live.ts';
import type { SortValue } from '../../charts/table/logic';
import { followStore } from '../../dashboard/follow/store.ts';
import { liveFollowed } from '../../dashboard/follow/liveFollowed.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { followRefusal, followsHead, usePlaybackHour, useFollow } from '../../dashboard/follow/useFollow.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { useFollowedProduct } from '../../dashboard/products';
import type { CampaignClockReply } from '../../generated/dashboard';
import { clockRequest } from '../../dashboard/widgetRequests';
import { useHeadHour } from '../army-size/headHour.ts';
import { clockHead, clockHeadQuery } from '../player-apm/apmHead.ts';
import { UNCLASSIFIED, clockRows, clockStructure, clockText, clockWarnings, validateClock } from './clockData';
import type { ClockRow } from './clockData';

const NO_WARNINGS: string[] = [];
const rowKey = (row: ClockRow) => row.id;
const SORT = { column: 'clock', direction: 'ascending' } as const;
const COLUMNS: readonly TableColumn<ClockRow>[] = [
  { key: 'clock', header: 'Clock', label: 'Clock', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.name },
  { key: 'time', header: 'Real time (h:mm:ss)', label: 'Real time', width: 'max-content', align: 'end', numeric: true,
    value: row => row.seconds, renderMissing: true, render: row => row.text ?? clockText(row.seconds),
    paint: value => typeof value === 'number' ? clockText(value) : String(value) },
];
const FILL = { display: 'flex', flexDirection: 'column', flex: '1 1 auto', minHeight: 0, minWidth: 0, height: '100%' } as const;

export default function CampaignClockWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const headQuery = useMemo(() => following && active && track ? gate.follow(clockHeadQuery(track.branch)) : null,
    [following, active, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, gate.follow]);
  const woken = useMemo(() => new Set<() => void>(), []);
  const shown = useShownHour();
  const followed = useFollow(store, headQuery, root, useCallback((entry: FollowEntry) => {
    for (const wake of [...woken]) wake();
    shown(shownThrough(entry));
  }, [woken, shown]));
  const headEntry = followsHead(followed) ? followed.entry : null;
  const refused = followRefusal(followed);
  const structureOf = (entry: FollowEntry | null) => clockStructure(clockHead(entry?.answer));
  const entryRef = useRef(headEntry); entryRef.current = headEntry;
  useHeadHour(runtime.clock, headEntry !== null, () => structureOf(entryRef.current), woken);
  const head = headEntry ? clockHead(headEntry.answer) : null;
  const live = useMemo(() => store && headEntry ? liveFollowed(headEntry, entry => {
    const totals = clockHead(entry.answer), values = new Map<string, SortValue>();
    if (totals && !('reason' in totals)) {
      values.set(liveKey('total', 'time'), totals.recorded);
      values.set(liveKey('playing', 'time'), totals.playing);
      values.set(liveKey('paused', 'time'), totals.paused);
      values.set(liveKey('coverage', 'time'), totals.complete ? 'Complete' : 'Gaps excluded');
    }
    return valuesOf(values, totals && 'reason' in totals ? totals.reason : totals ? UNCLASSIFIED : 'Not read yet');
  }) : undefined, [store, headEntry]);

  const hour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, hour);
  const request = following ? null : clockRequest(gate, widget, track, window ?? null);
  const { data, error, retry, moment } = useFollowedProduct<CampaignClockReply>(runtime.client, request,
    ['firstHour', 'lastHour'], active, validateClock, track?.source);
  const showing = data && !error && moment ? (JSON.parse(moment) as [number, number])[1] : null;
  useEffect(() => { if (!headEntry) shown(showing); }, [shown, showing, headEntry]);
  const headStructure = structureOf(headEntry);
  const headWarnings = useMemo(() => head && !('reason' in head) ? clockWarnings(head) : NO_WARNINGS, [headStructure]);
  const rows = useMemo<ClockRow[]>(() => head && !('reason' in head) ? clockRows(head, headWarnings)
    : data ? clockRows({ recorded: data.recordedSeconds, playing: data.playingSeconds, paused: data.pausedSeconds, complete: data.complete }, data.warnings) : [],
  [headStructure, data]);
  useDiagnostics(onDiagnostics, headEntry ? headWarnings : data?.warnings ?? NO_WARNINGS);

  const body = (() => {
    if (!track) return <WidgetState kind="empty" icon="clock" message="Load a campaign to see its recorded clock times." />;
    if (refused) return <WidgetState kind="unavailable" message={refused} />;
    if (headEntry) {
      if (!head) return <WidgetState kind="loading" label="Loading campaign clock…" />;
      if ('reason' in head) return <WidgetState kind="unavailable" message={head.reason} />;
    } else {
      if (following && gate.outside === null) return <WidgetState kind="loading" label="Loading campaign clock…" />;
      if (!request) return <WidgetState kind="outside" />;
      if (error) return <WidgetState kind="error" what="campaign clock" failure={error} onRetry={retry} />;
      if (!data) return <WidgetState kind="loading" label="Loading campaign clock…" />;
      if (!data.available) return <WidgetState kind="unavailable" message="Playing and paused time were not recorded in this campaign." />;
      if (data.recordedSeconds === null) return <WidgetState kind="unavailable" message="Overlapping recordings prevent a unique campaign clock total." />;
    }
    return <Table rowRevision={row => [row, COLUMNS]} label="Recorded campaign clock times" rows={rows} columns={COLUMNS} rowKey={rowKey} unsorted live={live}
      sort={SORT} cellBars={false} tooltip={row => {
        const now = live?.read().get(liveKey(row.id, 'time'));
        return { title: row.name, rows: [{ label: 'Real time', value: now === undefined ? row.text ?? clockText(row.seconds) : typeof now === 'number' ? clockText(now) : String(now) }], note: row.note };
      }} />;
  })();
  return <div ref={setRoot} style={FILL}>{body}</div>;
}
