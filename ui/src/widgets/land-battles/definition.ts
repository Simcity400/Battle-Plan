import type { WidgetMeta } from '../../dashboard/definitions';
import { battleOptions } from '../battles/battleOptions.ts';
export { DEFAULT_SORT, DEFAULT_TOP_N } from '../battles/battleOptions.ts';

export const OPTIONS = battleOptions([
  { value: 'Casualties', label: 'Casualties' }, { value: 'Peak size', label: 'Peak size' }, { value: 'Length', label: 'Length' },
]);
export const definition = {
  kind: 'land-battles', version: 1, title: 'Land battles', icon: 'shield',
  description: 'Observed land battles, their recorded sides and division counts, with a biggest-battles leaderboard.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'land-battles-manifest', params: {} },
  options: OPTIONS,
} as const satisfies WidgetMeta;
