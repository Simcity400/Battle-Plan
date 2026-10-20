import type { BoundTrack } from './model';
import type { PlaybackController } from './playback';

type ClockTarget = Pick<ReturnType<PlaybackController['getSnapshot']>, 'hour' | 'fraction' | 'hoursPerSecond' | 'playing' | 'seekSequence'>;
type MapClock = { getSnapshot: () => ClockTarget; subscribe: PlaybackController['subscribe'] };
export type MapPlayback = Omit<ClockTarget, 'hour'>;
type SeekMap = (path: string, revision: string, branch: string, hour: number, playback: MapPlayback) => Promise<void>;

/** Send only the latest clock anchor when the native command is busy. */
export function followMapPlayback(clock: MapClock, track: BoundTrack, seek: SeekMap, onStatus: (error: unknown | null) => void) {
  let active = true;
  let sending = false;
  let pending: ClockTarget | null = null;
  let last: ClockTarget | null = null;
  const flush = async () => {
    if (sending) return;
    sending = true;
    try {
      while (active && pending != null) {
        const target = pending; pending = null;
        try {
          await seek(track.source.path, track.source.revision, track.branch.id, target.hour, { playing: target.playing, seekSequence: target.seekSequence, fraction: target.fraction, hoursPerSecond: target.hoursPerSecond });
          if (active && target === last) onStatus(null);
        } catch (error) {
          if (active && target === last) onStatus(error);
        }
      }
    } finally { sending = false; }
  };
  const update = () => {
    if (!active) return;
    const snapshot = clock.getSnapshot();
    const target = { hour: snapshot.hour + track.axisOffset, playing: snapshot.playing, seekSequence: snapshot.seekSequence, fraction: snapshot.fraction, hoursPerSecond: snapshot.hoursPerSecond };
    if (target.hour === last?.hour && target.fraction === last.fraction && target.hoursPerSecond === last.hoursPerSecond
      && target.playing === last.playing && target.seekSequence === last.seekSequence) return;
    last = target;
    const { hour } = target;
    if (hour < track.branch.firstHour || hour > track.branch.lastHour) {
      pending = null;
      onStatus(new Error('This date is outside the selected campaign branch. Choose a date within its recording.'));
      return;
    }
    pending = target;
    void flush();
  };
  const unsubscribe = clock.subscribe(update);
  update();
  return () => { active = false; pending = null; unsubscribe(); };
}
