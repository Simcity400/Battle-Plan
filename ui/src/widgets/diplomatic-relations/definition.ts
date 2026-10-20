import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, breakdownVariants, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const PAIR = { id: 'pair', label: 'Pair history' };

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([PAIR], 'Matrix'), DRILL_OPTION,
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'faction',
    choices: [{ value: 'faction', label: 'Faction' }, { value: 'name', label: 'Name' }], visibleWhen: options => options.show !== PAIR.id },
  { key: 'pair', section: 'options', label: 'Pair', kind: 'content-choice', default: '', automatic: 'Pinned pair', visibleWhen: () => false },
];

export const definition = {
  kind: 'diplomatic-relations', version: 1, title: 'Diplomatic relations', icon: 'handshake', category: 'Diplomacy',
  description: 'Every chosen country’s opinion of every other, with a pair’s history.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 }, countries: { max: Number.MAX_SAFE_INTEGER },
  defaultCountries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false },
  options: OPTIONS, variants: breakdownVariants('Diplomatic relations', OPTIONS),
} as const satisfies WidgetMeta;
