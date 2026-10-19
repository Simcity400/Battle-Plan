import type { WidgetMeta } from '../../dashboard/definitions';
export const definition = {
  kind: 'war-goals', version: 1, title: 'War goals', icon: 'handshake',
  description: 'Owned war goals, target terms, expiry and justifications in progress.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 }, countries: { max: 1 }, options: [],
} as const satisfies WidgetMeta;
