import { useEffect, useMemo, useState } from 'react';
import { subscribeTokens } from '../../charts/tokens.ts';
import type { FamilyTheme } from '../contract.ts';
import type { CountryLooksMap } from '../looks.ts';
import { readFamilyTheme } from '../theme.ts';

export function reducedMotionEnabled(): boolean {
  if (typeof document !== 'undefined' && document.documentElement.dataset.motion === 'reduced') return true;
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

export function useFamilyTheme(
  countryColours?: ReadonlyMap<string, string>,
  countryLooks?: CountryLooksMap,
): { readonly theme: FamilyTheme | undefined; readonly reducedMotion: boolean } {
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const bump = () => setRevision(n => n + 1);
    const stopTokens = subscribeTokens(bump);
    const cleanups: (() => void)[] = [stopTokens];
    if (typeof matchMedia === 'function') {
      for (const query of ['(prefers-color-scheme: dark)', '(prefers-reduced-motion: reduce)']) {
        const list = matchMedia(query);
        const onChange = () => bump();
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

  const theme = useMemo(
    () => readFamilyTheme(countryColours, countryLooks),
    [countryColours, countryLooks, revision],
  );
  const reducedMotion = useMemo(() => reducedMotionEnabled(), [revision]);
  return { theme, reducedMotion };
}
