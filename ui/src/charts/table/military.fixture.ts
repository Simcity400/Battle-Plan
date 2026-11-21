import { createElement as h } from 'react';
import type { TableColumn, TableProps } from './Table';

export interface Leader {
  key: string; name: string; portrait: string; flag: string; subtitle: string;
  skill: number | null; attributes: readonly number[]; traits: readonly string[]; killed: boolean;
}

const rows: Leader[] = [
  { key: 'leader-z', name: 'Recorded leader Z', portrait: 'recorded-portrait-z', flag: 'recorded-flag-z', subtitle: 'Recorded rank', skill: 4, attributes: [3, 4, 2, 5], traits: ['recorded-trait'], killed: false },
  { key: 'leader-a', name: 'Recorded leader A', portrait: 'recorded-portrait-a', flag: 'recorded-flag-a', subtitle: 'Recorded nickname · aircraft type', skill: 4, attributes: [], traits: [], killed: true },
];

export const militaryColumns: TableColumn<Leader>[] = [
  { key: 'portrait', header: null, label: 'Portrait', sortable: false, width: '62px', value: () => null, renderMissing: true,
    render: row => h('img', { src: row.portrait, alt: '', style: { width: 32, height: 'min(43px, var(--table-row-height))', objectFit: 'contain' } }) },
  { key: 'name', header: 'General', label: 'General', width: 'minmax(190px, 2fr)', value: row => row.name,
    render: row => h('span', null, h('b', null, row.name), h('small', { style: { display: 'block' } }, h('img', { src: row.flag, alt: '', width: 20, height: 13 }), row.subtitle)) },
  { key: 'skill', header: 'Skill', label: 'Skill', numeric: true, align: 'start', width: '92px', value: row => row.skill,
    render: row => h('span', { 'aria-label': `Skill ${row.skill}` }, '★'.repeat(row.skill ?? 0)) },
  { key: 'attributes', header: 'Attributes', label: 'Attributes', width: '150px', value: row => row.attributes[0] ?? null,
    render: row => h('span', { style: { display: 'flex', gap: 'var(--space-2)' } }, ...row.attributes.map((value, key) =>
      h('span', { key }, value, h('meter', { value, min: 0, max: 10, 'aria-label': `Attribute ${key + 1}`, style: { display: 'block', width: '100%' } })))) },
  { key: 'traits', header: 'Traits', label: 'Traits', width: 'minmax(0, 1.1fr)', value: row => row.traits.length,
    render: row => h('span', { style: { display: 'flex', flexWrap: 'nowrap', overflow: 'hidden' } }, ...row.traits.map(src => h('img', { key: src, src, alt: src, style: { flex: 'none' } }))) },
];

export const generalsFixture = {
  label: 'Generals leaderboard', rows, columns: militaryColumns, rowKey: (row: Leader) => row.key,
  sort: { column: 'skill', direction: 'descending' }, rowHeight: 48, rank: true,
  muted: (row: Leader) => row.killed,
  onRowClick: (_row: Leader) => {},
  menu: (row: Leader) => [{ label: `Career of ${row.name}`, run: () => {} }],
} satisfies TableProps<Leader>;

export const acesFixture = {
  ...generalsFixture, label: 'Aces', rowHeight: 50, rank: false,
  columns: [militaryColumns[0], { ...militaryColumns[1], header: 'Ace', label: 'Ace' }],
} satisfies TableProps<Leader>;
