import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';
import { isMenuKey, rowDescriptionId, rowPoint } from './rowTooltips.ts';
import { hideTooltip, showTooltip } from './tooltip';
import type { TooltipContent } from './tooltip';

export function useRowTooltips<K extends string | number>({ owner, active, content, revision, onMenu, refresh }: {
  owner: string; active: boolean; content: (key: K) => TooltipContent | null; revision: string;
  onMenu: (key: K, x: number, y: number) => void;
  refresh?: (changed: () => void) => () => void;
}) {
  const [hovered, setHovered] = useState<{ key: K; x: number; y: number } | null>(null);
  const now = useRef({ hovered, active, content, revision });
  now.current = { hovered, active, content, revision };
  useEffect(() => refresh?.(() => {
    const { hovered: at, active: on, content: read, revision: shown } = now.current;
    const tip = at && on ? read(at.key) : null;
    if (at && tip) showTooltip(owner, tip, at.x, at.y, `${at.key}:${shown}`); else hideTooltip(owner);
  }), [refresh, owner]);
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
