import { usePlaybackHour } from '../../dashboard/playback.ts';
import type { PlaybackController } from '../../dashboard/playback';

/** The clock's hour for a table: paced while playing, exact at once on a seek or pause. */
export function useTableHour(clock: PlaybackController, follow: boolean): number {
  return usePlaybackHour(clock, follow);
}
