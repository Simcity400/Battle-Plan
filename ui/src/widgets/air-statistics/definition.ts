import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'channel', section: 'options', label: 'Statistic', kind: 'content-choice', automatic: 'Fighters lost', default: '' },
  { key: 'theatres', section: 'options', label: 'Air theatres', kind: 'content-set', automatic: 'All recorded theatres', default: '' },
  { key: 'regions', section: 'options', label: 'Regions', kind: 'content-set', automatic: 'Most active regions', default: '' },
  { key: 'regionCount', section: 'options', label: 'Number of regions', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 10,
    visibleWhen: o => !o.regions },
  { key: 'ratio', section: 'options', label: 'Ratio column', kind: 'toggle', default: true },
];
export const definition = {
  kind: 'air-statistics', version: 1, title: 'Air statistics', icon: 'plane', category: 'Air',
  description: 'Ours against the enemy’s per region for one air statistic in the current year window.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;
