import { usePlaybackHour } from '../../dashboard/playback.ts';
import type { HourHold, PlaybackController } from '../../dashboard/playback';

/** The clock's hour for a table: paced while playing, exact at once on a seek or pause, held while
 * its answer still answers the clock (`usePlaybackHour`). */
export function useTableHour(clock: PlaybackController, follow: boolean, hold?: { readonly current: HourHold | null }): number {
  return usePlaybackHour(clock, follow, hold);
}
