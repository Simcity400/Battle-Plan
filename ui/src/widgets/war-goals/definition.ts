import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'targetNames', section: 'options', label: 'Show names', kind: 'toggle', default: false },
];

export const definition = {
  kind: 'war-goals', version: 1, title: 'War goals', icon: 'handshake',
  description: 'War goals being justified, ready, in use at war or expired, with their targets.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 }, countries: { max: 1 }, options: OPTIONS,
} as const satisfies WidgetMeta;
