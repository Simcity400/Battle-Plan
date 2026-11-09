import { useLayoutEffect, useState } from 'react';
import type { RefObject } from 'react';
import { APPEARANCE_EVENT } from '../../shell/appearance/events.ts';
import { sameRevision } from '../memoRows.tsx';
import { fontFamilies, loadedFontFamily } from '../fonts';

function typography() {
  if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') return { tableSize: 12, headerSize: 11, tableFont: 'sans-serif', wrapping: true };
  const root = document.documentElement, css = getComputedStyle(root);
  return { tableSize: parseFloat(css.getPropertyValue('--type-sm')) || 12, headerSize: parseFloat(css.getPropertyValue('--type-xs')) || 11,
    tableFont: css.getPropertyValue('--font-tables'), wrapping: root.dataset.wordWrap !== 'false' };
}

export function useTableAppearance(element: RefObject<HTMLElement | null>) {
  const [appearance, setAppearance] = useState(() => ({ ...typography(), comfortable: 0, compact: 0, headerMinimum: 0, fonts: 0 }));
  useLayoutEffect(() => {
    if (!element.current) return;
    const heights = () => {
      const css = getComputedStyle(element.current!);
      return { comfortable: parseFloat(css.getPropertyValue('--table-comfortable-height')), compact: parseFloat(css.getPropertyValue('--table-compact-height')),
        headerMinimum: parseFloat(css.getPropertyValue('--table-header-min-height')) };
    };
    const initial = heights();
    setAppearance(before => ({ ...before, ...initial }));
    const update = (event: Event) => {
      const next = { ...typography(), ...heights() };
      const loaded = event.type === 'loadingdone' && loadedFontFamily(event as FontFaceSetLoadEvent, new Set(fontFamilies(next.tableFont)));
      setAppearance(before => {
        const measured = { ...next, fonts: before.fonts + (loaded ? 1 : 0) };
        return sameRevision(Object.values(before), Object.values(measured)) ? before : measured;
      });
    };
    window.addEventListener(APPEARANCE_EVENT, update);
    document.fonts?.addEventListener('loadingdone', update);
    return () => { window.removeEventListener(APPEARANCE_EVENT, update); document.fonts?.removeEventListener('loadingdone', update); };
  }, [element]);
  return appearance;
}
