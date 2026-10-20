import type { WidgetMeta } from '../../dashboard/definitions';
import { YARD_OPTIONS } from '../refits/yardOptions.ts';

export const OPTIONS = YARD_OPTIONS;
export const definition = {
  kind: 'dockyards-and-repairs', version: 1, title: 'Dockyards & repairs', icon: 'anchor', category: 'Navy',
  description: 'Each port’s repair dockyards and the ships queued there, with their hull.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;
