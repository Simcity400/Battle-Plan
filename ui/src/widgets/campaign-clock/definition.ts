import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'campaign-clock', version: 1, title: 'Campaign clock', icon: 'clock', category: 'Overview',
  description: 'Recorded real time spent playing and paused, with recording gaps kept separate.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'moment',
  data: true, countrySelection: false, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 8 }, options: [],
} as const satisfies WidgetMeta;
