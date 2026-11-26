import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'folders', section: 'options', label: 'Research folders', kind: 'content-set', automatic: 'All folders', default: '', catalogue: true },
];

export const definition = {
  kind: 'research', version: 2, title: 'Research history', icon: 'flask',
  description: 'The technologies each research slot worked through over time, when each finished and when slots opened.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 3, h: 5 }, defaultSize: { w: 7, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;
