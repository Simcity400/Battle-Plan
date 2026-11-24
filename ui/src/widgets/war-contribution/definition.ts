import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import type { WarMeasureKey } from '../../generated/dashboard';

export const MEASURE_LIST: readonly { readonly key: WarMeasureKey; readonly label: string; readonly whole: string }[] = [
  { key: 'equipmentDamage', label: 'Equipment destroyed', whole: 'equipment destroyed' },
  { key: 'occupation', label: 'Territory occupied', whole: 'territory occupied' },
  { key: 'airDamage', label: 'Air damage', whole: 'air damage' },
  { key: 'strategicBombing', label: 'Strategic bombing', whole: 'strategic bombing' },
  { key: 'shipDamage', label: 'Naval damage', whole: 'naval damage' },
  { key: 'convoyDamage', label: 'Convoy raiding', whole: 'convoy raiding' },
  { key: 'casualties', label: 'Casualties inflicted', whole: 'casualties inflicted' },
  { key: 'lendLeaseSent', label: 'Lend-lease sent', whole: 'lend-lease sent' },
  { key: 'lendLeaseReceived', label: 'Lend-lease received', whole: 'lend-lease received' },
  { key: 'participation', label: 'War participation', whole: 'war participation' },
];

export const DEFAULT_MEASURE: WarMeasureKey = 'equipmentDamage';

export const VIEWS = [{ value: 'sides', label: 'Sides' }, { value: 'measure', label: 'By measure' }] as const;

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'sides', choices: VIEWS },
  { key: 'war', section: 'options', label: 'War', kind: 'content-choice', default: '', automatic: 'The viewpoint’s main war' },
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: DEFAULT_MEASURE,
    choices: MEASURE_LIST.map(item => ({ value: item.key, label: item.label })), visibleWhen: options => options.view === 'measure' },
];

export const definition = {
  kind: 'war-contribution', version: 1, title: 'War contribution', icon: 'swords', category: 'Diplomacy',
  description: 'Who carried each side of a war: every member’s share of damage, ground, losses and lend-lease.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 6, h: 6 }, defaultSize: { w: 12, h: 10 },
  countries: { max: 1 }, options: OPTIONS, viewSwitch: { option: 'view' },
} as const satisfies WidgetMeta;
