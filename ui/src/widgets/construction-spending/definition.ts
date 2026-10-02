import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  levelOption('splitLevel', 'New or repair', 'The innermost ring splits new buildings from repairs'),
  levelOption('typeLevel', 'Building type', 'A ring of building types, from the campaign’s content'),
  levelOption('stateLevel', 'State', 'A ring of the states built in'),
  ...SUNBURST_OPTIONS,
];

export const definition = {
  kind: 'construction-spending', version: 1, title: 'Construction spending', icon: 'sunburst',
  description: 'Where a country’s civilian-factory output went: new buildings and repairs, each building type and each state.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
