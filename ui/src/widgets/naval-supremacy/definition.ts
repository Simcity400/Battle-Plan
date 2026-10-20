import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'regions', section: 'options', label: 'Regions', kind: 'content-set', automatic: 'All sea regions', default: '' },
];
export const definition = {
  kind: 'naval-supremacy', version: 1, title: 'Naval supremacy', icon: 'anchor', category: 'Navy',
  description: 'Who controls each sea region day by day, from one country’s point of view.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 10, h: 8 }, options: OPTIONS,
} as const satisfies WidgetMeta;
