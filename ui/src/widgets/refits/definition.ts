import type { WidgetMeta } from '../../dashboard/definitions';
import { YARD_OPTIONS } from '../refits/yardOptions.ts';

export const OPTIONS = YARD_OPTIONS;
export const definition = {
  kind: 'refits', version: 1, title: 'Refits', icon: 'anchor', category: 'Navy',
  description: 'Refit lines at work, their progress and the ships on them.',
  timeModes: ['campaign', 'follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, countries: { max: 1 }, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 8, h: 9 }, options: OPTIONS,
} as const satisfies WidgetMeta;
