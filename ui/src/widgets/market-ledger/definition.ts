import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { LOG_OPTIONS } from '../../charts/log/logOptions.ts';
import { DRILL_OPTION } from '../../charts/drill/drillState.ts';
import { KINDS } from '../../dashboard/marketLedger.ts';

export const DETAILS = { id: 'contract', label: 'Contract' };

export const OPTIONS: readonly OptionField[] = [
  ...KINDS.map((kind): OptionField => ({ key: kind.key, section: 'options', label: kind.label, kind: 'toggle', default: true })),
  ...LOG_OPTIONS, DRILL_OPTION,
];

export const definition = {
  kind: 'market-ledger', version: 1, title: 'Market ledger', icon: 'history', category: 'Economy',
  description: 'Every market contract event for a country: deals, deliveries and payments.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  defaultCountries: { kind: 'selected' }, options: OPTIONS, playsBackDated: true,
} as const satisfies WidgetMeta;