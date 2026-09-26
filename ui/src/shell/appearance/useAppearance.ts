import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { fontStack } from './fonts';
import { loadAppearance, resolveAppearance, saveAppearance } from './settings';
import type { AppearanceSettings, ResolvedAppearance } from './settings';
import { cssVariable, themeRoles } from './themes';
import type { ThemeColors, ThemeMode } from './themes';
import { getBackdrop } from './backdrops';
import { APPEARANCE_EVENT } from './events';

const darkQuery = () => window.matchMedia('(prefers-color-scheme: dark)');
const motionQuery = () => window.matchMedia('(prefers-reduced-motion: reduce)');
function useMedia(query: () => MediaQueryList): boolean {
  return useSyncExternalStore(listener => { const list = query(); list.addEventListener('change', listener); return () => list.removeEventListener('change', listener); },
    () => query().matches, () => false);
}

/** Writes the resolved appearance onto the root as CSS variables and data attributes. */
export function applyAppearance(root: HTMLElement, settings: AppearanceSettings, resolved: ResolvedAppearance, colors: ThemeColors = resolved.colors, mode: ThemeMode = resolved.mode): void {
  for (const role of themeRoles) root.style.setProperty(cssVariable(role), colors[role]);
  root.style.setProperty('--glass', `${settings.glass}%`);
  root.style.setProperty('--glass-blur', settings.glass >= 100 ? '0px' : `${Math.round(12 + (100 - settings.glass) * 0.2)}px`);
  root.style.setProperty('--radius', `${settings.radius}px`);
  root.style.setProperty('--font-ui', fontStack(settings.fonts.interface));
  root.style.setProperty('--font-display', fontStack(settings.fonts.display || settings.fonts.interface));
  root.style.setProperty('--backdrop-dim', `${settings.backdrop.dim}%`);
  root.style.setProperty('--backdrop-blur', `${settings.backdrop.blur}px`);
  root.style.colorScheme = mode;
  root.dataset.mode = mode;
  root.dataset.theme = resolved.theme.id;
  root.dataset.motion = resolved.reducedMotion ? 'reduced' : 'full';
  root.dataset.backdrop = settings.backdrop.kind;
  root.dataset.chartMotion = settings.charts.motion;
  root.dataset.countryColours = settings.charts.countryColours;
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', colors.chrome);
  // Canvas charts cannot see CSS changes; tell them to re-read their tokens.
  if (root === document.documentElement) window.dispatchEvent(new Event(APPEARANCE_EVENT));
}

/** Apply the saved appearance before React renders, so the first frame is already themed. */
export function applySavedAppearance(): void {
  const { settings } = loadAppearance();
  applyAppearance(document.documentElement, settings, resolveAppearance(settings, darkQuery().matches, motionQuery().matches));
}

export interface ThemePreview { colors: ThemeColors; mode: ThemeMode }

export function useAppearance() {
  const [initial] = useState(loadAppearance);
  const [settings, setSettings] = useState(initial.settings);
  const latest = useRef(settings);
  const [notice, setNotice] = useState<string | null>(initial.warning);
  const [preview, setPreview] = useState<ThemePreview | null>(null);
  const [backdropImage, setBackdropImage] = useState<string | null>(null);
  const systemDark = useMedia(darkQuery);
  const systemReduced = useMedia(motionQuery);
  const resolved = useMemo(() => resolveAppearance(settings, systemDark, systemReduced), [settings, systemDark, systemReduced]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    // Colour changes should snap, not animate every control at once.
    root.classList.add('theme-switching');
    applyAppearance(root, settings, resolved, preview?.colors, preview?.mode);
    const frame = requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('theme-switching')));
    return () => cancelAnimationFrame(frame);
  }, [settings, resolved, preview]);

  const imageId = settings.backdrop.kind === 'image' ? settings.backdrop.imageId : null;
  useEffect(() => {
    if (!imageId) { setBackdropImage(null); return; }
    let current = true;
    void getBackdrop(imageId).then(entry => { if (current) setBackdropImage(entry?.image ?? null); }).catch(() => { if (current) setBackdropImage(null); });
    return () => { current = false; };
  }, [imageId]);

  const update = useCallback((change: (previous: AppearanceSettings) => AppearanceSettings) => {
    const next = change(latest.current);
    latest.current = next;
    setSettings(next);
    setNotice(saveAppearance(next));
  }, []);

  return { settings, resolved, update, notice, preview, setPreview, backdropImage };
}

export type Appearance = ReturnType<typeof useAppearance>;
