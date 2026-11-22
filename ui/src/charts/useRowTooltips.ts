import { useEffect, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';
import { isMenuKey, rowDescriptionId, rowPoint } from './rowTooltips.ts';
import { hideTooltip, showTooltip } from './tooltip';
import type { TooltipContent } from './tooltip';

export function useRowTooltips<K extends string | number>({ owner, active, held = false, content, revision, onMenu }: {
  owner: string; active: boolean; held?: boolean; content: (key: K) => TooltipContent | null; revision: string;
  onMenu: (key: K, x: number, y: number) => void;
}) {
  const [hovered, setHovered] = useState<{ key: K; x: number; y: number } | null>(null);
  useEffect(() => { if (!active) { setHovered(null); hideTooltip(owner); } }, [active, owner]);
  useEffect(() => {
    const shown = hovered && active && !held ? content(hovered.key) : null;
    if (hovered && shown) showTooltip(owner, shown, hovered.x, hovered.y, `${hovered.key}:${revision}`);
    else hideTooltip(owner);
  }, [hovered, active, held, owner, content, revision]);
  useEffect(() => () => hideTooltip(owner), [owner]);
  const descriptionId = (key: K) => rowDescriptionId(owner, key);
  const rowProps = (key: K) => ({
    tabIndex: 0,
    'aria-describedby': held ? undefined : descriptionId(key),
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
  return { hovered: held ? null : hovered?.key ?? null, rowProps, descriptionId };
}
