import type { WidgetMeta } from '../../dashboard/definitions';
import { AIR_OPTIONS } from '../../dashboard/airOptions.ts';
import type { OptionField } from '../../dashboard/options';
export const OPTIONS: readonly OptionField[] = [...AIR_OPTIONS,
  { key: 'look', section: 'options', label: 'Cells', kind: 'choice', default: 'margin', choices: [
    { value: 'margin', label: 'Lead strength' }, { value: 'split', label: 'Split per side' }] },
  { key: 'regions', section: 'options', label: 'Regions', kind: 'content-set', automatic: 'Most active regions', default: '' },
  { key: 'regionCount', section: 'options', label: 'Number of regions', kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 10,
    visibleWhen: o => !o.regions },
];
export const definition = {
  kind: 'air-superiority-timeline', version: 1, title: 'Air superiority timeline', icon: 'plane',
  description: 'Daily air scores per region, in each leading side’s colour or split into shares.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;
