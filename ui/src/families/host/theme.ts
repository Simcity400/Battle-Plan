import { useEffect, useMemo, useRef, useState } from 'react';
import { subscribeTokens } from '../../charts/tokens.ts';
import type { FamilyTheme } from '../contract.ts';
import type { CountryLooksMap } from '../looks.ts';
import { readFamilyTheme } from '../theme.ts';

export function reducedMotionEnabled(): boolean {
  if (typeof document !== 'undefined' && document.documentElement.dataset.motion === 'reduced') return true;
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function sameRecord(a: Readonly<Record<string, string>> | undefined, b: Readonly<Record<string, string>> | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  const keysA = Object.keys(a);
  const keysB = Object.keys(b);
  if (keysA.length !== keysB.length) return false;
  for (let i = 0; i < keysA.length; i++) {
    const k = keysA[i]!;
    if (a[k] !== b[k]) return false;
  }
  return true;
}

export function sameFamilyTheme(a: FamilyTheme | undefined, b: FamilyTheme | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.surface === b.surface
    && a.text === b.text
    && a.muted === b.muted
    && a.line === b.line
    && a.lineSoft === b.lineSoft
    && a.accent === b.accent
    && a.blue === b.blue
    && a.red === b.red
    && a.amber === b.amber
    && a.green === b.green
    && a.grey === b.grey
    && a.hatch === b.hatch
    && a.hatchEdge === b.hatchEdge
    && a.font === b.font
    && a.fontSize.small === b.fontSize.small
    && a.fontSize.body === b.fontSize.body
    && a.lineWidth === b.lineWidth
    && a.hatchSpacing === b.hatchSpacing
    && a.countryLooks === b.countryLooks
    && sameRecord(a.country, b.country);
}

interface ThemeSnapshot {
  readonly baseTheme: FamilyTheme | undefined;
  readonly reducedMotion: boolean;
}

export function useFamilyTheme(
  countryColours?: ReadonlyMap<string, string>,
  countryLooks?: CountryLooksMap,
): { readonly theme: FamilyTheme | undefined; readonly reducedMotion: boolean } {
  const [snap, setSnap] = useState<ThemeSnapshot>(() => ({
    baseTheme: readFamilyTheme(),
    reducedMotion: reducedMotionEnabled(),
  }));

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const refresh = () => {
      const nextTheme = readFamilyTheme();
      const nextReduced = reducedMotionEnabled();
      setSnap(prev => (
        prev.reducedMotion === nextReduced && sameFamilyTheme(prev.baseTheme, nextTheme)
          ? prev
          : { baseTheme: nextTheme, reducedMotion: nextReduced }
      ));
    };
    const stopTokens = subscribeTokens(refresh);
    const cleanups: (() => void)[] = [stopTokens];
    if (typeof matchMedia === 'function') {
      for (const query of ['(prefers-color-scheme: dark)', '(prefers-reduced-motion: reduce)']) {
        const list = matchMedia(query);
        const onChange = () => refresh();
        if (typeof list.addEventListener === 'function') {
          list.addEventListener('change', onChange);
          cleanups.push(() => list.removeEventListener('change', onChange));
        } else if (typeof list.addListener === 'function') {
          list.addListener(onChange);
          cleanups.push(() => list.removeListener(onChange));
        }
      }
    }
    return () => {
      for (const cleanup of cleanups) cleanup();
    };
  }, []);

  const prevComposedRef = useRef<FamilyTheme | undefined>(undefined);
  const theme = useMemo(() => {
    if (!snap.baseTheme) {
      prevComposedRef.current = undefined;
      return undefined;
    }
    const next: FamilyTheme = countryColours === undefined && countryLooks === undefined
      ? snap.baseTheme
      : {
          ...snap.baseTheme,
          country: countryColours ? Object.fromEntries(countryColours) : undefined,
          countryLooks,
        };
    if (sameFamilyTheme(prevComposedRef.current, next)) return prevComposedRef.current;
    prevComposedRef.current = next;
    return next;
  }, [snap.baseTheme, countryColours, countryLooks]);

  return { theme, reducedMotion: snap.reducedMotion };
}
