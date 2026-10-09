import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { SUNBURST_OPTIONS, levelOption } from '../../charts/sunburst/sunburstOptions.ts';

export const DETAIL = { id: 'loss-details', label: 'Recorded losses' };

export const OPTIONS: readonly OptionField[] = [
  { key: 'landSource', section: 'options', label: 'Land losses from', kind: 'choice', default: 'destruction',
    choices: [{ value: 'destruction', label: 'Every destruction' }, { value: 'combatLog', label: "Game's Combat Log" }] },
  levelOption('causeLevel', 'Cause', 'The loss causes reported by the campaign’s content'),
  levelOption('typeLevel', 'Equipment type', 'Equipment archetypes from the recorded content'),
  levelOption('designLevel', 'Exact design', 'Design names as recorded at the moment shown'),
  ...SUNBURST_OPTIONS, DRILL_OPTION, breakdownOption([DETAIL], 'Sunburst'),
];

export const definition = {
  kind: 'equipment-losses', version: 1, title: 'Equipment losses', icon: 'sunburst',
  description: 'One country’s recorded equipment losses by cause, equipment type and exact design, with land coverage.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 11 },
  options: OPTIONS,
} as const satisfies WidgetMeta;
