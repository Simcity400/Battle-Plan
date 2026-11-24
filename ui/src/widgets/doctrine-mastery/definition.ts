import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'folders', section: 'options', label: 'Doctrine folders', kind: 'content-set', automatic: 'All folders', default: '' },
];

export const definition = {
  kind: 'doctrine-mastery', family: 'meters', version: 1, title: 'Doctrine mastery', icon: 'flask', category: 'Research',
  description: 'Each doctrine track’s mastery and rewards, with step thresholds.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 4 }, defaultSize: { w: 10, h: 6 }, options: OPTIONS,
} as const satisfies WidgetMeta;
