import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { FACTION_OPTION } from '../../dashboard/factionOption.ts';

export type FactionPoolChoice = 'resources' | 'manpower' | 'research' | 'doctrine';

export const POOL_CHOICES: readonly { value: FactionPoolChoice; label: string }[] = [
  { value: 'resources', label: 'Resources' },
  { value: 'manpower', label: 'Manpower' },
  { value: 'research', label: 'Research' },
  { value: 'doctrine', label: 'Doctrine' },
];

export const OPTIONS: readonly OptionField[] = [
  FACTION_OPTION,
  { key: 'pool', section: 'options', label: 'Pool', kind: 'choice', default: 'resources', choices: POOL_CHOICES },
];

export const definition = {
  kind: 'faction-pooled-strength', version: 1, title: 'Faction pooled strength', icon: 'layers', category: 'Diplomacy',
  description: 'What a faction pools: its shared resources, each member’s manpower, research sharing and shared doctrines.',
  timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 }, countries: { max: 1 }, options: OPTIONS,
} as const satisfies WidgetMeta;
