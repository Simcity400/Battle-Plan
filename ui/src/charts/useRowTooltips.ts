import { useEffect, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';
import { isMenuKey, rowDescriptionId, rowPoint } from './rowTooltips.ts';
import { hideTooltip, showTooltip } from './tooltip';
import type { TooltipContent } from './tooltip';

/** A list of rows whose hover or keyboard focus shows the row's tooltip, live: the
 * hovered row is described again whenever `revision` changes (the hour shown, the
 * reply), so a still pointer stays current during playback. Right-click, the menu
 * key or Shift+F10 on a row opens `onMenu` there; Escape hides the tooltip. Out of
 * view (`active` false), nothing shows and the pointer is forgotten. Give each row
 * `rowProps(key)` and a hidden element with id `descriptionId(key)` holding its full
 * description, since the tooltip layer is not read out. */
export function useRowTooltips<K extends string | number>({ owner, active, content, revision, onMenu }: {
  owner: string; active: boolean; content: (key: K) => TooltipContent | null; revision: string;
  onMenu: (key: K, x: number, y: number) => void;
}) {
  const [hovered, setHovered] = useState<{ key: K; x: number; y: number } | null>(null);
  useEffect(() => { if (!active) { setHovered(null); hideTooltip(owner); } }, [active, owner]);
  useEffect(() => {
    const shown = hovered && active ? content(hovered.key) : null;
    if (hovered && shown) showTooltip(owner, shown, hovered.x, hovered.y, `${hovered.key}:${revision}`);
    else hideTooltip(owner);
  }, [hovered, active, owner, content, revision]);
  useEffect(() => () => hideTooltip(owner), [owner]);
  const descriptionId = (key: K) => rowDescriptionId(owner, key);
  const rowProps = (key: K) => ({
    tabIndex: 0,
    'aria-describedby': descriptionId(key),
    onPointerMove: (event: ReactPointerEvent) => setHovered({ key, x: event.clientX, y: event.clientY }),
    onPointerLeave: () => setHovered(null),
    onFocus: (event: { currentTarget: HTMLElement }) => setHovered({ key, ...rowPoint(event.currentTarget.getBoundingClientRect()) }),
    onBlur: () => setHovered(null),
    onContextMenu: (event: { preventDefault: () => void; clientX: number; clientY: number }) => { event.preventDefault(); onMenu(key, event.clientX, event.clientY); },
    onKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => {
      if (isMenuKey(event)) {
        event.preventDefault();
        const point = rowPoint(event.currentTarget.getBoundingClientRect());
        onMenu(key, point.x, point.y);
      } else if (event.key === 'Escape') setHovered(null);
    },
  });
  return { hovered: hovered?.key ?? null, rowProps, descriptionId };
}
