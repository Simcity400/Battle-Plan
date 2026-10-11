import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'category', section: 'options', label: 'Category', kind: 'content-choice',
    automatic: 'First category', default: '' },
  { key: 'laws', section: 'options', label: 'Laws', kind: 'choice', default: 'both',
    choices: [{ value: 'shade', label: 'Shading' }, { value: 'icons', label: 'Icons' }, { value: 'both', label: 'Both' }] },
  { key: 'holders', section: 'options', label: 'Advisors and designers', kind: 'choice', default: 'both',
    choices: [{ value: 'both', label: 'Portrait and name' }, { value: 'name', label: 'Name' }, { value: 'portrait', label: 'Portrait' }] },
  { key: 'grow', section: 'axes', label: 'Grow with playback', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'idea-timeline', version: 1, title: 'Idea timeline', icon: 'layers',
  description: 'Laws, advisors and designers over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
