import type { BoundTrack } from './model';
import type { PlaybackController } from './playback';

type SeekMap = (path: string, revision: string, branch: string, hour: number) => Promise<void>;

/** Send only the latest requested hour when the native command is busy. */
export function followMapPlayback(clock: Pick<PlaybackController, 'getHour' | 'subscribeHour'>, track: BoundTrack, seek: SeekMap, onStatus: (error: unknown | null) => void) {
  let active = true;
  let sending = false;
  let pending: number | null = null;
  let lastHour: number | null = null;
  const flush = async () => {
    if (sending) return;
    sending = true;
    try {
      while (active && pending != null) {
        const hour = pending; pending = null;
        try {
          await seek(track.source.path, track.source.revision, track.branch.id, hour);
          if (active && hour === lastHour) onStatus(null);
        } catch (error) {
          if (active && hour === lastHour) onStatus(error);
        }
      }
    } finally { sending = false; }
  };
  const update = () => {
    if (!active) return;
    const hour = clock.getHour() + track.axisOffset;
    if (hour === lastHour) return;
    lastHour = hour;
    if (hour < track.branch.firstHour || hour > track.branch.lastHour) {
      pending = null;
      onStatus(new Error('This date is outside the selected campaign branch. Choose a date within its recording.'));
      return;
    }
    pending = hour;
    void flush();
  };
  const unsubscribe = clock.subscribeHour(update);
  update();
  return () => { active = false; pending = null; unsubscribe(); };
}
