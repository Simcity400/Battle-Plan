import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { typographyVariables } from './typography';
import { loadAppearance, resolveAppearance, saveAppearance } from './settings';
import type { AppearanceSettings, Backdrop, ResolvedAppearance } from './settings';
import { contrastColors, cssVariable, themeRoles, themeVariables } from './themes';
import { paletteVariables } from '../../charts/palette.ts';
import type { ThemeColors, ThemeMode } from './themes';
import { getBackdrop } from './backdrops';
import { APPEARANCE_EVENT } from './events';
import { SIDEBAR_ICONS, sidebarIconIds } from './sidebarIcons';
import { prepareInterfaceFonts } from './bundledFonts';

function mediaStore(query: string) {
  let list: MediaQueryList | undefined;
  const media = () => list ??= window.matchMedia(query);
  return {
    matches: () => media().matches,
    subscribe: (listener: () => void) => { media().addEventListener('change', listener); return () => media().removeEventListener('change', listener); },
  };
}
const dark = mediaStore('(prefers-color-scheme: dark)');
const motion = mediaStore('(prefers-reduced-motion: reduce)');
function useMedia(store: ReturnType<typeof mediaStore>): boolean {
  return useSyncExternalStore(store.subscribe, store.matches, () => false);
}

export function applyAppearance(root: HTMLElement, settings: AppearanceSettings, resolved: ResolvedAppearance, themed: ThemeColors = resolved.colors, mode: ThemeMode = resolved.mode): void {
  const colors = contrastColors(themed, settings.contrast);
  for (const role of themeRoles) root.style.setProperty(cssVariable(role), colors[role]);
  for (const [name, value] of Object.entries({ ...themeVariables(colors), ...paletteVariables(colors.surface) })) root.style.setProperty(name, value);
  for (const id of sidebarIconIds) root.style.setProperty(`--sidebar-icon-${id}`, settings.sidebarIcons[id]?.colour ?? SIDEBAR_ICONS[id][mode]);
  root.style.setProperty('--glass', `${settings.glass}%`);
  root.style.setProperty('--glass-blur', settings.glass >= 100 ? '0px' : `${Math.round(12 + (100 - settings.glass) * 0.2)}px`);
  root.style.setProperty('--chrome-opacity', `${settings.chromeGlass}%`);
  root.style.setProperty('--chrome-blur', settings.chromeGlass >= 100 ? '0px' : `${Math.round(12 + (100 - settings.chromeGlass) * 0.25)}px`);
  root.style.setProperty('--radius', `${settings.radius}px`);
  for (const [name, value] of Object.entries(typographyVariables(settings.fonts, settings.typography))) root.style.setProperty(name, value);
  root.style.fontSize = `${settings.typography.sizes.interface}px`;
  if (settings.typography.smoothing) root.style.setProperty('-webkit-font-smoothing', 'antialiased');
  else root.style.removeProperty('-webkit-font-smoothing');
  root.dataset.wordWrap = String(settings.typography.wordWrap);
  root.style.colorScheme = mode;
  root.dataset.mode = mode;
  root.dataset.theme = resolved.theme.id;
  root.dataset.motion = resolved.reducedMotion ? 'reduced' : 'full';
  root.dataset.backdrop = settings.workspaceBackdrop.kind === 'same' ? settings.backdrop.kind : settings.workspaceBackdrop.kind;
  root.dataset.headerFlag = settings.headerFlag;
  root.dataset.chartMotion = settings.charts.motion;
  root.dataset.playbackRate = settings.charts.frameRate;
  root.dataset.countryColours = settings.charts.countryColours;
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', colors.chrome);
  if (root === document.documentElement) window.dispatchEvent(new Event(APPEARANCE_EVENT));
}

export function applySavedAppearance(): Promise<void> | undefined {
  const { settings } = loadAppearance();
  const apply = () => applyAppearance(document.documentElement, settings, resolveAppearance(settings, dark.matches(), motion.matches()));
  const fonts = prepareInterfaceFonts(settings);
  if (fonts) return fonts.catch(error => console.error('Could not load the saved fonts:', error)).then(apply);
  apply();
}

export interface ThemePreview { colors: ThemeColors; mode: ThemeMode }

function useBackdropImage(backdrop: Backdrop<string>): string | null {
  const [image, setImage] = useState<string | null>(null);
  const imageId = backdrop.kind === 'image' ? backdrop.imageId : null;
  useEffect(() => {
    if (!imageId) { setImage(null); return; }
    let current = true;
    void getBackdrop(imageId).then(entry => { if (current) setImage(entry?.image ?? null); }).catch(() => { if (current) setImage(null); });
    return () => { current = false; };
  }, [imageId]);
  return image;
}

export function useAppearance() {
  const [initial] = useState(loadAppearance);
  const [settings, setSettings] = useState(initial.settings);
  const latest = useRef(settings);
  const [notice, setNotice] = useState<string | null>(initial.warning);
  const [preview, setPreview] = useState<ThemePreview | null>(null);
  const systemDark = useMedia(dark);
  const systemReduced = useMedia(motion);
  const resolved = useMemo(() => resolveAppearance(settings, systemDark, systemReduced), [settings, systemDark, systemReduced]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    let live = true;
    let frame = 0;
    const apply = () => {
      if (!live) return;
      root.classList.add('theme-switching');
      applyAppearance(root, settings, resolved, preview?.colors, preview?.mode);
      frame = requestAnimationFrame(() => { frame = requestAnimationFrame(() => root.classList.remove('theme-switching')); });
    };
    const fonts = prepareInterfaceFonts(settings);
    if (fonts) void fonts.catch(error => console.error('Could not load the selected fonts:', error)).then(apply);
    else apply();
    return () => { live = false; cancelAnimationFrame(frame); root.classList.remove('theme-switching'); };
  }, [settings, resolved, preview]);

  const backdropImage = useBackdropImage(settings.backdrop);
  const workspaceImage = useBackdropImage(settings.workspaceBackdrop);

  const update = useCallback((change: (previous: AppearanceSettings) => AppearanceSettings) => {
    const next = change(latest.current);
    latest.current = next;
    setSettings(next);
    setNotice(saveAppearance(next));
  }, []);

  const commit = useCallback((change: (previous: AppearanceSettings) => AppearanceSettings): string | null => {
    const next = change(latest.current);
    const failed = saveAppearance(next);
    if (failed) return failed;
    latest.current = next;
    setSettings(next);
    setNotice(null);
    return null;
  }, []);

  return { settings, resolved, update, commit, notice, preview, setPreview, backdropImage, workspaceImage };
}

export type Appearance = ReturnType<typeof useAppearance>;
