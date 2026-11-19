import { errorMessage } from '../shell/errors.ts';
import type { BoundTrack } from './model';
import type { PlaybackController } from './playback';

type ClockTarget = Pick<ReturnType<PlaybackController['getSnapshot']>, 'hour' | 'fraction' | 'hoursPerSecond' | 'playing' | 'seekSequence'>;
type MapClock = { getSnapshot: () => ClockTarget; composedAt: PlaybackController['composedAt'];
  subscribe: PlaybackController['subscribe']; subscribeFrame: PlaybackController['subscribeFrame'] };
/** An anchor as the page composed it: where the clock stood, how it was moving, and when the page
 * read it (`composedAt`, on the page's own monotonic clock). The map places the anchor by that
 * time, so the time it spends in delivery moves no clock. */
export type MapPlayback = Omit<ClockTarget, 'hour'> & { composedAt: number };
/** The clock as one anchor carries it: its values and when the page read them. */
type Anchor = ClockTarget & { composedAt: number };
type SeekMap = (path: string, revision: string, branch: string, hour: number, playback: MapPlayback) => Promise<void>;

/** Sends the clock's anchors, only the latest when the native command is busy; the map runs its own clock between
 * them. Each frame only checks the branch's ends, sending the clock again when it comes back inside. Returns the
 * call that stops following, whose `resend` sends the clock's anchor again, unchanged or not. */
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
          await seek(track.source.path, track.source.revision, track.branch.id, target.hour, { playing: target.playing, seekSequence: target.seekSequence, fraction: target.fraction, hoursPerSecond: target.hoursPerSecond, composedAt: target.composedAt });
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
    const target = { hour: snapshot.hour + track.axisOffset, playing: snapshot.playing, seekSequence: snapshot.seekSequence, fraction: snapshot.fraction, hoursPerSecond: snapshot.hoursPerSecond, composedAt: clock.composedAt() };
    // Only the clock's own values decide whether this is a new anchor; when it was composed is not one.
    if (target.hour === last?.hour && target.fraction === last.fraction && target.hoursPerSecond === last.hoursPerSecond
      && target.playing === last.playing && target.seekSequence === last.seekSequence) return;
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

/** What decides whether a set of dashboard map targets is the one the maps already hold. When the
 * page composed a target is not part of it: a target the maps already hold is not sent again
 * because the page read its clock at a new time. */
export function viewsKey(views: { composedAt: number }[]): string {
  return JSON.stringify(views.map(({ composedAt: _composedAt, ...view }) => view));
}

/** Map commands, each failure reported by its operation until a later send of that operation succeeds. Only an
 * operation's latest send reports. */
export function mapCommands(report: (operation: string, error: string | null) => void) {
  let sent = 0;
  const latest = new Map<string, number>();
  const failed = new Set<string>();
  const retries = new Map<string, () => void>();
  const forget = (operation: string) => {
    latest.delete(operation);
    retries.delete(operation);
    if (failed.delete(operation)) report(operation, null);
  };
  /** Sends `command` as `operation`; Retry map sends it again if it fails, or only clears it when it cannot be repeated. */
  const send = (operation: string, command: () => Promise<void>, repeatable = true) => {
    const sequence = ++sent;
    latest.set(operation, sequence);
    retries.delete(operation);
    const current = () => latest.get(operation) === sequence && latest.delete(operation);
    void (async () => command())().then(() => {
      if (current() && failed.delete(operation)) report(operation, null);
    }, (error: unknown) => {
      if (!current()) return;
      failed.add(operation);
      retries.set(operation, repeatable ? () => send(operation, command) : () => forget(operation));
      report(operation, errorMessage(error));
    });
  };
  return {
    send,
    /** Drops operations no longer wanted: their failures clear, and no send already made reports. */
    forget: (...operations: string[]) => operations.forEach(operation => forget(operation)),
    /** What Retry map does for every failed command. */
    retry: () => { for (const again of [...retries.values()]) again(); },
  };
}
