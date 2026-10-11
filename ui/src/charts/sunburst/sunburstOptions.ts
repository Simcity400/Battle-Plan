/** The settings every sunburst offers: growth, slice
 * labels, the side legend, the breadcrumb and hover dimming, all on by default
 * except Fill. A sunburst widget lists `SUNBURST_OPTIONS` among its own, adds a
 * toggle per level of its hierarchy with `levelOption`, and reads them with
 * `sunburstSettings` and `levelsShown`. */
import type { OptionField, OptionValue } from '../../dashboard/options';
import type { Growth } from './layout';

export const SUNBURST_OPTIONS: readonly OptionField[] = [
  { key: 'growth', section: 'options', label: 'Growth', hint: 'Share shows proportions; Fill shows progress.',
    kind: 'choice', default: 'share', choices: [{ value: 'share', label: 'Share' }, { value: 'fill', label: 'Fill' }] },
  { key: 'sliceLabels', section: 'options', label: 'Slice labels', kind: 'toggle', default: true },
  { key: 'legend', section: 'options', label: 'Side legend', kind: 'toggle', default: true },
  { key: 'breadcrumb', section: 'options', label: 'Path', kind: 'toggle', default: true },
  { key: 'dimming', section: 'options', label: 'Hover dimming', kind: 'toggle', default: true },
];

/** A toggle for one level of a widget's hierarchy (every level can be switched off). */
export const levelOption = (key: string, label: string, hint?: string): OptionField =>
  ({ key, section: 'options', label, hint, kind: 'toggle', default: true });

export interface SunburstSettings { growth: Growth; labels: boolean; legend: boolean; breadcrumb: boolean; dimming: boolean }

/** The sunburst settings from resolved options (`resolveOptions`). */
export function sunburstSettings(options: Readonly<Record<string, OptionValue>>): SunburstSettings {
  return {
    growth: options.growth === 'fill' ? 'fill' : 'share',
    labels: options.sliceLabels !== false,
    legend: options.legend !== false,
    breadcrumb: options.breadcrumb !== false,
    dimming: options.dimming !== false,
  };
}

/** Which levels show, from the widget's level toggles in hierarchy order. */
export const levelsShown = (options: Readonly<Record<string, OptionValue>>, keys: readonly string[]): boolean[] =>
  keys.map(key => options[key] !== false);
