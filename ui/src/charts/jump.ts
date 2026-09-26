import { formatDate, gameDate } from '../dashboard/calendar.ts';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { MenuItem } from '../components/ContextMenu';
import { seekPositionFor } from '../dashboard/model.ts';

/** "Jump to <date>": moves the dashboard's playback to a recorded hour of this
 * widget's track. Disabled when the hour is outside the track's branch. */
export function jumpItem(runtime: Pick<WidgetRuntime, 'requestSeek' | 'tracks'>, trackId: string, rawHour: number): MenuItem {
  const date = gameDate(Math.floor(rawHour));
  const track = runtime.tracks.find(item => item.id === trackId);
  const inside = !!track && seekPositionFor(track, rawHour) != null;
  return {
    label: `Jump to ${date ? formatDate(date) : `hour ${Math.floor(rawHour)}`}`, icon: 'play',
    disabled: !inside, hint: inside ? undefined : 'Outside this campaign branch',
    run: () => { runtime.requestSeek(trackId, rawHour); },
  };
}
