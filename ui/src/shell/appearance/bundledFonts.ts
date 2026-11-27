import { useEffect, useState } from 'react';
import { BUNDLED_FONT, familyNames } from './fonts.ts';
import type { AppearanceSettings } from './settings.ts';

const styles = new Map([
  [BUNDLED_FONT, () => import('@fontsource-variable/inter/index.css?inline')],
  ['Roboto Variable', () => import('@fontsource-variable/roboto/index.css?inline')],
  ['Roboto Condensed Variable', () => import('@fontsource-variable/roboto-condensed/index.css?inline')],
  ['Source Serif 4 Variable', () => import('@fontsource-variable/source-serif-4/index.css?inline')],
  ['Cascadia Code Variable', () => import('@fontsource-variable/cascadia-code/index.css?inline')],
]);
const pending = new Map<string, Promise<void>>();
const ready = new Set<string>();

export function prepareFonts(families: Iterable<string>): Promise<void> | undefined {
  const loads: Promise<void>[] = [];
  for (const name of new Set([...families].flatMap(familyNames))) {
    const load = styles.get(name);
    if (!load || ready.has(name)) continue;
    let wait = pending.get(name);
    if (!wait) {
      wait = load().then(async ({ default: css }) => {
        const style = document.createElement('style');
        style.textContent = css;
        document.head.append(style);
        try {
          const faces = [...document.fonts].filter(face => face.family === name);
          if (!faces.length) throw new Error(`No font faces were provided for ${name}`);
          await Promise.all(faces.map(face => face.load()));
          ready.add(name);
        } catch (error) {
          style.remove();
          throw error;
        }
      }).finally(() => pending.delete(name));
      pending.set(name, wait);
    }
    loads.push(wait);
  }
  return loads.length ? Promise.all(loads).then(() => {}) : undefined;
}

export function prepareInterfaceFonts(settings: AppearanceSettings): Promise<void> | undefined {
  return prepareFonts([settings.fonts.interface, settings.fonts.display]);
}

export function useBundledFonts(families: readonly string[], enabled = true): boolean {
  const key = JSON.stringify(families);
  const [settled, setSettled] = useState('');
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    const work = prepareFonts(JSON.parse(key));
    if (work) void work.catch(error => console.error('Could not load the fonts:', error)).then(() => { if (live) setSettled(key); });
    return () => { live = false; };
  }, [key, enabled]);
  return enabled && (settled === key || families.flatMap(familyNames).every(name => !styles.has(name) || ready.has(name)));
}

export function useDiagnosticsFont(enabled: boolean): boolean {
  const family = enabled && typeof document !== 'undefined' ? document.documentElement.style.getPropertyValue('--font-diagnostics') : '';
  return useBundledFonts([family], enabled);
}
