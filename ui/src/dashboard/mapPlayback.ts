import { errorMessage } from '../shell/errors.ts';
import type { BoundTrack } from './model';
import type { PlaybackController } from './playback';

export function createPageClockId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  return `${performance.timeOrigin}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export const PAGE_CLOCK_ID = createPageClockId();

type ClockTarget = Pick<ReturnType<PlaybackController['getSnapshot']>, 'hour' | 'fraction' | 'hoursPerSecond' | 'playing' | 'seekSequence'>;
type MapClock = { getSnapshot: () => ClockTarget; composedAt: PlaybackController['composedAt']; clockId: string;
  subscribe: PlaybackController['subscribe']; subscribeFrame: PlaybackController['subscribeFrame'] };
export type MapPlayback = Omit<ClockTarget, 'hour'> & { composedAt: number; clockId: string };
type Anchor = ClockTarget & { composedAt: number; clockId: string };
type SeekMap = (path: string, revision: string, branch: string, hour: number, playback: MapPlayback) => Promise<void>;

export function followMapPlayback(clock: MapClock, track: BoundTrack, seek: SeekMap, onStatus: (error: unknown | null) => void) {
  let active = true;
  let sending = false;
  let pending: Anchor | null = null;
  let last: Anchor | null = null;
  const flush = async () => {
    if (sending) return;
    sending = true;
    try {
      while (active && pending != null) {
        const target = pending; pending = null;
        try {
          await seek(track.source.path, track.source.revision, track.branch.id, target.hour, { playing: target.playing, seekSequence: target.seekSequence, fraction: target.fraction, hoursPerSecond: target.hoursPerSecond, composedAt: target.composedAt, clockId: target.clockId });
          if (active && target === last) onStatus(null);
        } catch (error) {
          if (active && target === last) onStatus(error);
        }
      }
    } finally { sending = false; }
  };
  let outside = false;
  const update = (anchor: boolean) => {
    if (!active) return;
    const snapshot = clock.getSnapshot();
    const target = { hour: snapshot.hour + track.axisOffset, playing: snapshot.playing, seekSequence: snapshot.seekSequence, fraction: snapshot.fraction, hoursPerSecond: snapshot.hoursPerSecond, composedAt: clock.composedAt(), clockId: clock.clockId };
    if (target.hour === last?.hour && target.fraction === last.fraction && target.hoursPerSecond === last.hoursPerSecond
      && target.playing === last.playing && target.seekSequence === last.seekSequence && target.clockId === last.clockId) return;
    const { hour } = target;
    if (hour < track.branch.firstHour || hour > track.branch.lastHour) {
      last = target;
      outside = true;
      pending = null;
      onStatus(new Error('This date is outside the selected history. Choose a date within its recording.'));
      return;
    }
    if (!anchor && !outside) return;
    last = target;
    outside = false;
    pending = target;
    void flush();
  };
  const unsubscribe = clock.subscribe(() => update(true));
  const unsubscribeFrame = clock.subscribeFrame(() => update(false));
  update(true);
  const stop = () => { active = false; pending = null; unsubscribe(); unsubscribeFrame(); };
  return Object.assign(stop, { resend: () => { last = null; update(true); } });
}

export function viewsKey(clockId: string, views: { composedAt: number }[]): string {
  return JSON.stringify([clockId, views.map(({ composedAt: _composedAt, ...view }) => view)]);
}

export function mapCommands(report: (operation: string, error: string | null) => void, warn: (warning: string) => void = () => {}) {
  let sent = 0;
  const latest = new Map<string, number>();
  const failed = new Set<string>();
  const retries = new Map<string, () => void>();
  const forget = (operation: string) => {
    latest.delete(operation);
    retries.delete(operation);
    if (failed.delete(operation)) report(operation, null);
  };
  const send = (operation: string, command: () => Promise<string | null | void>, repeatable = true) => {
    const sequence = ++sent;
    latest.set(operation, sequence);
    retries.delete(operation);
    const current = () => latest.get(operation) === sequence && latest.delete(operation);
    void (async () => command())().then(warning => {
      if (!current()) return;
      if (failed.delete(operation)) report(operation, null);
      if (warning) warn(warning);
    }, (error: unknown) => {
      if (!current()) return;
      failed.add(operation);
      retries.set(operation, repeatable ? () => send(operation, command) : () => forget(operation));
      report(operation, errorMessage(error));
    });
  };
  return {
    send,
    forget: (...operations: string[]) => operations.forEach(operation => forget(operation)),
    retry: () => { for (const again of [...retries.values()]) again(); },
  };
}
