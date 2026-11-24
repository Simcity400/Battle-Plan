import type { WidgetMeta } from '../../dashboard/definitions';
import { powerSpendingOptions } from '../political-power-spending/powerSpendingProjection.ts';

export const definition = {
  kind: 'command-power-spending', version: 1, title: 'Command power spending', icon: 'chart-line',
  description: 'Command power spent by category and item, with its share, the latest payment, what was gained and what was lost at the cap.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: 1 }, options: powerSpendingOptions('Command'),
} as const satisfies WidgetMeta;
