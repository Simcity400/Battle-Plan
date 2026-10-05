import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'progress',
    choices: [{ value: 'progress', label: 'Progress' }, { value: 'decryptor', label: 'Decryptor' }] },
  { key: 'countrySide', section: 'countries', label: 'Countries filter', kind: 'choice', default: 'decryptor',
    visibleWhen: (_, countries) => countries?.kind !== 'all',
    choices: [{ value: 'decryptor', label: 'Decryptor' }, { value: 'target', label: 'Target' }] },
];

export const definition = {
  kind: 'decryption', version: 1, title: 'Decryption', icon: 'target',
  description: 'Who is breaking whose ciphers: progress, observed breaks and active bonus days.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 9 },
  countries: { max: 256, all: true }, options: OPTIONS,
} as const satisfies WidgetMeta;
