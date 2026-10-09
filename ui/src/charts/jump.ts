import { formatDate, gameDate } from '../dashboard/calendar.ts';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { MenuItem } from '../components/ContextMenu';
import type { HTMLAttributes } from 'react';
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

/** Keep native row activation; only pointer/menu keys open a dated Jump action. */
export function rowJumpProps(runtime: Pick<WidgetRuntime, 'requestSeek' | 'tracks'>, trackId: string, hour: number | null | (() => number | null),
  open: (x: number, y: number, items: readonly MenuItem[]) => void): Pick<HTMLAttributes<HTMLElement>, 'onContextMenu' | 'onKeyDown'> {
  if (hour === null) return {};
  const show = (x: number, y: number) => {
    const at = typeof hour === 'function' ? hour() : hour;
    if (at === null) return false;
    open(x, y, [jumpItem(runtime, trackId, at)]);
    return true;
  };
  return {
    onContextMenu: event => {
      if (show(event.clientX, event.clientY)) { event.preventDefault(); event.stopPropagation(); }
    },
    onKeyDown: event => {
      if (event.key !== 'ContextMenu' && !(event.key === 'F10' && event.shiftKey)) return;
      const box = event.currentTarget.getBoundingClientRect();
      if (show(box.left, box.bottom)) { event.preventDefault(); event.stopPropagation(); }
    },
  };
}
