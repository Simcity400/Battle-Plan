import { useEffect, useMemo, useState } from 'react';
import type { Failure } from '../dashboard/queryError.ts';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../dashboard/model';
import { resolveWindow } from '../dashboard/model';
import type { PaintCell, PaintMoment, PaintRow } from '../charts/canvasTable/paint';
import { AheadReads } from './aheadReads';
import { LiveLabels } from './liveLabels';
import { perfLog } from '../shell/perf.ts';
import { countWork, measuredWidget } from '../dashboard/widgetPerf.ts';

/** A row as read: its country is `tag` at `hour`, named by the reply (`name`) or by the
 * recording's country timeline; no tag, no flag. */
const OUTSIDE = 'This time is outside the recording.';
const nameCollator = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true });

export interface ReadRow<Data> { key: string; rank?: number; tag?: string; hour: number; name?: string | null; muted?: boolean; cells: readonly PaintCell[]; data: Data }
/** `unavailable`: the hour's reply says why it has nothing to show (`message`). */
export type LivePhase = { state: 'loading' | 'ready' | 'empty' } | { state: 'unavailable'; message: string } | { state: 'error'; failure: Failure };

/** A table's moments on the live runtime: each hour is read ahead of the playhead (`read`, which
 * shapes it as it lands), and `moment` (asked every frame) gives the one the clock shows, its
 * countries named and flagged. React renders only when the widget's state (loading, empty,
 * ready, failed) changes; `lane` names everything a read depends on but the hour. */
export function useLiveMoments<Data>({ active, lane, runtime, widget, track, content, read, countries = true, byName }: {
  active: boolean; lane: string | null; runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack | undefined; content: number;
  /** `partial`: answered while preparation was unfinished, so read again until whole. */
  read: (hour: number, signal: AbortSignal, restart: boolean) => Promise<{ through: number; rows: readonly ReadRow<Data>[]; unavailable?: string; partial?: boolean }>;
  /** Whether rows name countries (a country column); a table of other things reads no names. */
  countries?: boolean;
  /** Rows shown in country-name order, as named at the hour they answer (keys break ties). */
  byName?: 'ascending' | 'descending';
}) {
  const [phase, setPhase] = useState<{ lane: string | null; phase: LivePhase }>({ lane: null, phase: { state: 'loading' } });
  const measured = measuredWidget();
  const timeKey = JSON.stringify([widget.id, widget.kind, widget.time, widget.trackIds, track?.source.id, track?.source.revision, track?.branch.id,
    track?.branch.firstHour, track?.branch.lastHour, track?.axisOffset, runtime.tracks.map(item => [item.id, item.branch.firstHour, item.axisOffset])]);
  const store = useMemo(() => {
    if (!active || !lane || !track) return null;
    const wakers = new Set<() => void>();
    const wake = () => { for (const waker of wakers) waker(); };
    let labelsChanged = 0, shownLabels = -1, shownAnswer: unknown = null, shown: PaintMoment<Data> | null = null, told = '';
    const tell = (next: LivePhase) => {
      const key = next.state === 'error' ? `error:${next.failure.message}` : next.state === 'unavailable' ? `unavailable:${next.message}` : next.state;
      if (key !== told) { told = key; setPhase({ lane, phase: next }); }
    };
    // Subscribes only once started (an effect): a render that is thrown away leaves nothing behind.
    const labels = new LiveLabels(runtime.client, track.source, track.branch.id, content, 'panel', () => { labelsChanged++; wake(); });
    const ahead = new AheadReads<{ rows: readonly ReadRow<Data>[]; unavailable?: string }>({ batch: 1, inFlight: 2, told: wake,
      read: (hours, signal, restart) => Promise.all(hours.map(async hour => {
        countWork(measured, 'read');
        const { through, rows, unavailable, partial } = await read(hour, signal, restart);
        countWork(measured, 'reply');
        return { hour, through, partial, value: { rows, unavailable } };
      })) });
    let shownHour: number | null | undefined, clockHour = NaN, hour = 0, shownPhase: LivePhase | null = null, outside = false;
    const showing = (hour: number | null) => { if (hour !== shownHour) { shownHour = hour; perfLog?.shown(widget.id, hour); } };
    return {
      moment(): PaintMoment<Data> | null {
        const clock = runtime.clock.getSnapshot();
        if (clock.hour !== clockHour) {
          clockHour = clock.hour;
          const window = resolveWindow(widget, track, runtime.tracks, clock.hour);
          // Before this recording starts (calendar-aligned tracks) there is nothing to read, as the DOM widgets say.
          outside = !window || Math.floor(window.lastHour) < track.branch.firstHour;
          hour = window ? Math.floor(window.lastHour) : track.branch.lastHour;
        }
        if (outside) { tell({ state: 'unavailable', message: OUTSIDE }); showing(null); return null; }
        ahead.plan(clock, hour, track.branch.firstHour, track.branch.lastHour);
        const { answer, exact, failure } = ahead.at(hour);
        // Another hour's answer never stands in for one that failed.
        if (failure && !exact) { tell({ state: 'error', failure }); return shown; }
        if (!answer) { tell({ state: 'loading' }); if (!shown) showing(null); return shown; }
        showing(answer.hour);
        if (answer !== shownAnswer || labelsChanged !== shownLabels) {
          shownAnswer = answer; shownLabels = labelsChanged;
          if (countries) labels.ask(answer.hour);
          const named = answer.value.rows.map((row): PaintRow<Data> => {
            // `cells` stays as lazy as the read made it: a long table formats only the rows it shows.
            if (!countries) return { key: row.key, rank: row.rank, muted: row.muted, get cells() { return row.cells; }, data: row.data };
            const label = row.tag === undefined ? undefined : labels.label(row.hour, row.tag);
            const name = row.name !== undefined ? row.name ?? 'Country name unavailable'
              : label?.name === undefined ? 'Loading…' : label.name ?? label.nameReason ?? 'Name unavailable';
            return { key: row.key, rank: row.rank, muted: row.muted, get cells() { return row.cells; }, data: row.data,
              country: { name, flag: label?.flag, flagPending: label?.flagPending, flagReason: label?.flagReason ?? (row.tag === undefined ? 'not a recorded country' : undefined) } };
          });
          if (byName) {
            const sign = byName === 'ascending' ? 1 : -1;
            named.sort((a, b) => sign * nameCollator.compare(a.country?.name ?? '', b.country?.name ?? '') || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
            // Numbered as shown, as the DOM table numbers its sorted rows; the measure only chose them.
            named.forEach((row, slot) => { if (row.rank !== undefined) row.rank = slot + 1; });
          }
          shown = { rows: named };
          if (countries) labels.draw();
          const { rows, unavailable } = answer.value;
          shownPhase = unavailable !== undefined ? { state: 'unavailable', message: unavailable } : { state: rows.length ? 'ready' : 'empty' };
        }
        // Told every time: an answer shown again after another hour failed clears the error.
        if (shownPhase) tell(shownPhase);
        return shown;
      },
      bind(waker: () => void) { wakers.add(waker); return () => { wakers.delete(waker); }; },
      retry() { ahead.retry(); },
      start() { if (countries) labels.start(); wake(); },
      stop() { labels.stop(); ahead.reset(); },
    };
  }, [active, lane, runtime.client, runtime.clock, timeKey, content, countries, byName]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (!store) return; store.start(); return () => store.stop(); }, [store]);
  return { store, phase: phase.lane === lane ? phase.phase : { state: 'loading' } as LivePhase };
}
