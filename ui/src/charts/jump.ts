import { formatDate, gameDate } from '../dashboard/calendar.ts';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { MenuItem } from '../components/ContextMenu';
import { seekPositionFor } from '../dashboard/model.ts';

export function jumpItem(runtime: Pick<WidgetRuntime, 'requestSeek' | 'tracks'>, trackId: string, rawHour: number): MenuItem {
  const date = gameDate(Math.floor(rawHour));
  const track = runtime.tracks.find(item => item.id === trackId);
  const inside = !!track && seekPositionFor(track, rawHour) != null;
  return {
    label: `Jump to ${date ? formatDate(date) : `hour ${Math.floor(rawHour)}`}`, icon: 'play',
    disabled: !inside, hint: inside ? undefined : 'Outside the recording',
    run: () => { runtime.requestSeek(trackId, rawHour); },
  };
}

