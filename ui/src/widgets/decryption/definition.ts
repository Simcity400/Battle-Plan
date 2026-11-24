import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  {
    key: 'view',
    section: 'options',
    label: 'View',
    kind: 'choice',
    default: 'ciphers',
    choices: [
      { value: 'ciphers', label: 'Ciphers' },
      { value: 'matrix', label: 'Who reads whom' },
    ],
  },
  {
    key: 'countrySide',
    section: 'countries',
    label: 'Countries filter',
    kind: 'choice',
    default: 'decryptor',
    visibleWhen: (_, countries) => countries?.kind !== 'all',
    choices: [
      { value: 'decryptor', label: 'Decryptor' },
      { value: 'target', label: 'Target' },
    ],
  },
];

export const definition = {
  kind: 'decryption',
  version: 1,
  title: 'Decryption',
  icon: 'lock',
  description: 'Cipher-breaking progress, broken ciphers, and who reads whom.',
  timeModes: ['campaign', 'follow', 'pin', 'range'],
  defaultTime: 'follow',
  scope: 'history',
  data: true,
  history: true,
  minSize: { w: 4, h: 4 },
  defaultSize: { w: 8, h: 6 },
  countries: { max: Number.MAX_SAFE_INTEGER, all: true },
  options: OPTIONS,
  viewSwitch: { option: 'view' },
  variants: [
    { key: 'who-reads-whom', initial: { title: 'Who reads whom', options: { view: 'matrix' } } },
  ],
} as const satisfies WidgetMeta;
