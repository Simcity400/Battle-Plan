import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'kinds', section: 'options', label: 'Kinds', kind: 'content-set', default: '', automatic: 'All kinds' },
  { key: 'ended', section: 'options', label: 'Ended agreements', kind: 'choice', default: 'shown',
    choices: [{ value: 'shown', label: 'Shown' }, { value: 'hidden', label: 'Hidden' }] },
];

export const definition = {
  kind: 'diplomatic-pacts', version: 1, title: 'Diplomatic agreements', icon: 'link', category: 'Diplomacy',
  description: 'A country’s non-aggression pacts, guarantees, access rights, embargoes and blockades over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  playsBackDated: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 12, h: 12 }, countries: { max: 1 }, options: OPTIONS,
} as const satisfies WidgetMeta;
