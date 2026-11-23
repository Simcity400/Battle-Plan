import { useEffect, useMemo } from 'react';
import { hideTooltip, refreshTooltip, showTooltip } from './tooltip.ts';
import type { TooltipContent } from './tooltip';

export function openTooltip(owner: string) {
  let open: (() => TooltipContent | null) | null = null;
  return {
    show(content: () => TooltipContent | null, key: string, x: number, y: number) {
      open = content;
      const shown = content();
      if (shown) showTooltip(owner, shown, x, y, key); else hideTooltip(owner);
    },
    hide() { open = null; hideTooltip(owner); },
    refresh() { const held = open; if (held) refreshTooltip(owner, held); },
  };
}

export function useOpenTooltip(owner: string, changes: ((changed: () => void) => () => void) | undefined) {
  const tips = useMemo(() => openTooltip(owner), [owner]);
  useEffect(() => changes?.(tips.refresh), [changes, tips]);
  useEffect(() => tips.hide, [tips]);
  return tips;
}
