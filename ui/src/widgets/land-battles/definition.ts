import type { WidgetMeta } from '../../dashboard/definitions';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { battleOptions, battleVariants } from '../battles/battleOptions.ts';

const TABLE_ONLY = new Set(['cellBars', 'rankMarkers', 'tableSort']);
export const OPTIONS = [...battleOptions([
  { value: 'Casualties', label: 'Casualties' }, { value: 'Peak size', label: 'Most divisions at once' }, { value: 'Length', label: 'Length' },
]).filter(field => !TABLE_ONLY.has(field.key)), DRILL_OPTION];
export const definition = {
  kind: 'land-battles', version: 1, title: 'Land battles', icon: 'shield',
  description: 'Land battles: where and when, who fought and who won, and what each side lost.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 10, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  availability: { product: 'land-battles-manifest', params: {} },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants: battleVariants('Land battles', OPTIONS),
} as const satisfies WidgetMeta;
