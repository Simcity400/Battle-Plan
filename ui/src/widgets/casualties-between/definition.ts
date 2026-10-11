import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import type { WidgetMeta } from '../../dashboard/definitions';

export const DETAIL = { id: 'pairs', label: 'Casualty pairs' };
export const definition = {
  kind: 'casualties-between', version: 1, title: 'Casualties between countries', icon: 'target',
  description: 'Recorded casualties dealt from each country to each enemy country.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'all' },
  availability: { product: 'casualty-pairs', params: { tags: [], between: false, window: null } },
  options: [{ key: 'mostCountries', section: 'options', label: 'Countries around the chord',
    hint: 'The remaining countries fold into Other; click to see every recorded pair',
    kind: 'number', min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, default: 12 },
    { key: 'between', section: 'options', label: 'With countries chosen, show', kind: 'choice', default: 'touching',
      choices: [{ value: 'touching', label: 'Their casualties with anyone' }, { value: 'between', label: 'Only casualties between them' }] }, DRILL_OPTION,
    breakdownOption([DETAIL], 'Chord'),
    { key: 'pairSelection', section: 'options', label: 'Pair selection', kind: 'content-choice', automatic: 'All recorded pairs',
      default: '', visibleWhen: () => false }],
} as const satisfies WidgetMeta;
