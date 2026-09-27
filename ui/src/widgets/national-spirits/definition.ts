import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'names', section: 'options', label: 'Spirit names', kind: 'choice', default: 'hover',
    choices: [{ value: 'hover', label: 'On hover' }, { value: 'under', label: 'Under icons' }] },
];

export const definition = {
  kind: 'national-spirits', version: 1, title: 'National spirits', icon: 'star',
  description: 'The country’s national spirits at the playback moment, as the game shows them.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: true,
  minSize: { w: 3, h: 3 }, defaultSize: { w: 4, h: 5 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
