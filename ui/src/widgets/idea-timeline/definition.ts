import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'category', section: 'options', label: 'Category', kind: 'content-choice',
    automatic: 'First category', default: '' },
  { key: 'iconsOnBars', section: 'options', label: 'Icons on bars', kind: 'toggle', default: true },
];

export const definition = {
  kind: 'idea-timeline', version: 1, title: 'Idea timeline', icon: 'layers',
  description: 'Laws, advisors and designers over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
