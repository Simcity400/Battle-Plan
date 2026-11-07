import { useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../dashboard/model';
import { resolveWindow } from '../dashboard/model';
import { perfLog } from '../shell/perf.ts';
import { countWork, measuredWidget } from '../dashboard/widgetPerf.ts';
import { AheadReads, type Answer } from './aheadReads';
import type { LivePhase } from './useLiveMoments';

/** A widget's answers on the live runtime, for a painter that asks every frame: each hour is read
 * ahead of the playhead (`read`, which shapes it as it lands), and `answer` gives the one the clock
 * shows (the followed window's last hour). React renders only when `phaseOf` the answer changes;
 * `lane` names everything a read depends on but the hour. */
export interface LiveAnswers<T> {
  answer(): Answer<T> | undefined;
  bind(wake: () => void): () => void;
  /** Asks again at once for the hours that failed. */
  retry(): void;
}

export function useLiveAnswers<T>({ active, lane, runtime, widget, track, read, phaseOf }: {
  active: boolean; lane: string | null; runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack | undefined;
  /** `partial`: answered while preparation was unfinished, so read again until whole. */
  read: (hour: number, signal: AbortSignal, restart: boolean) => Promise<{ through: number; value: T; partial?: boolean }>;
  phaseOf: (value: T) => LivePhase;
}): { store: LiveAnswers<T> | null; phase: LivePhase } {
  const [phase, setPhase] = useState<{ lane: string | null; phase: LivePhase }>({ lane: null, phase: { state: 'loading' } });
  const measured = measuredWidget();
  const store = useMemo(() => {
    if (!active || !lane || !track) return null;
    const wakers = new Set<() => void>();
    let shownPhase: LivePhase | null = null, outside = false;
    let told = '', shown: Answer<T> | undefined, shownHour: number | null | undefined, clockHour = NaN, hour = 0;
    const tell = (next: LivePhase) => {
      const key = next.state === 'error' ? `error:${next.failure.message}` : next.state === 'unavailable' ? `unavailable:${next.message}` : next.state;
      if (key !== told) { told = key; setPhase({ lane, phase: next }); }
    };
    const showing = (at: number | null) => { if (at !== shownHour) { shownHour = at; perfLog?.shown(widget.id, at); } };
    const wake = () => { for (const waker of wakers) waker(); };
    const ahead = new AheadReads<T>({ batch: 1, inFlight: 2, told: wake,
      read: (hours, signal, restart) => Promise.all(hours.map(async at => {
        countWork(measured, 'read');
        const { through, value, partial } = await read(at, signal, restart);
        countWork(measured, 'reply');
        return { hour: at, through, value, partial };
      })) });
    return {
      store: {
        answer(): Answer<T> | undefined {
          const clock = runtime.clock.getSnapshot();
          if (clock.hour !== clockHour) {
            clockHour = clock.hour;
            const window = resolveWindow(widget, track, runtime.tracks, clock.hour);
            // Before this recording starts (calendar-aligned tracks) there is nothing to read, as the DOM widgets say.
            outside = !window || Math.floor(window.lastHour) < track.branch.firstHour;
            hour = window ? Math.floor(window.lastHour) : track.branch.lastHour;
          }
          if (outside) { tell({ state: 'unavailable', message: 'This time is outside the recording.' }); showing(null); return undefined; }
          ahead.plan(clock, hour, track.branch.firstHour, track.branch.lastHour);
          const { answer, exact, failure } = ahead.at(hour);
          // Another hour's answer never stands in for one that failed.
          if (failure && !exact) { tell({ state: 'error', failure }); return shown; }
          if (!answer) { tell({ state: 'loading' }); if (!shown) showing(null); return shown; }
          showing(answer.hour);
          if (answer !== shown) { shown = answer; shownPhase = phaseOf(answer.value); }
          // Told every time: an answer shown again after another hour failed clears the error.
          if (shownPhase) tell(shownPhase);
          return answer;
        },
        bind(waker: () => void) { wakers.add(waker); return () => { wakers.delete(waker); }; },
        retry() { ahead.retry(); },
      },
      start() { wake(); },
      stop() { ahead.reset(); },
    };
  }, [active, lane, runtime, widget, track]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (!store) return; store.start(); return () => store.stop(); }, [store]);
  return { store: store?.store ?? null, phase: phase.lane === lane ? phase.phase : { state: 'loading' } };
}
