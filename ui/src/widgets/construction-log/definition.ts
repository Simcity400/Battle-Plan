import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';

export const PROJECT_CHOICES = [{ value: 'all', label: 'New and repairs' }, { value: 'new', label: 'New buildings' }, { value: 'repair', label: 'Repairs' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'types', section: 'options', label: 'Building types', kind: 'content-set', automatic: 'Every type', default: '', catalogue: true },
  { key: 'projects', section: 'options', label: 'Projects', kind: 'choice', default: 'all', choices: PROJECT_CHOICES },
  { key: 'queue', section: 'options', label: 'In the queue', hint: 'Only projects still in the queue at the moment shown.', kind: 'toggle', default: false },
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'change',
    choices: [{ value: 'change', label: 'Latest change' }, { value: 'start', label: 'Start date' }] },
  ...LOG_OPTIONS,
];

export const definition = {
  kind: 'construction-log', version: 1, title: 'Construction log', icon: 'hammer',
  description: 'Every construction project: what was built or repaired where, when it started and finished, and the factories on it.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
} as const satisfies WidgetMeta;
