import type { WidgetMeta } from '../../dashboard/definitions';
import { powerSpendingOptions } from '../political-power-spending/powerSpendingProjection.ts';

export const definition = {
  kind: 'political-power-spending', version: 1, title: 'Political power spending', icon: 'chart-line',
  description: 'Political power spent by category and item, with its share, the latest payment and what was gained.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: 1 }, options: powerSpendingOptions('Political'),
} as const satisfies WidgetMeta;
