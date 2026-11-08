import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LeaderRow, UnitLeadersData } from './generalsData';
import type { TableProps } from '../../charts/table/Table';

test('a leaderboard read at its hour jumps to the first seen hour and keeps its parity row', async () => {
  const data: UnitLeadersData = { branchId: '1', firstHour: 100,
    people: [{ name: '5', portraitHour: 100, portraitReference: '5' }],
    roles: [{ owner: 'current', category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] }],
    traits: [{ token: 'synthetic_trait', name: 'Synthetic trait', icon: null }],
    leaders: [{ reference: '5', runs: [{ firstHour: 101, lastHour: 102, value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [0] } }],
      traitGains: [{ hour: 90, token: 0, date: 'heldBefore' }, { hour: 101, token: 0, date: 'gained' }], commands: [], partialFrom: null,
      atHour: { hour: 102, firstSeenHour: 95, fought: 2, won: 1, lost: 0 } }],
    withheld: [], observed: [{ firstHour: 100, lastHour: 102, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
    resultsMissing: 'Result not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] };
  const replacements: Record<string, string> = {
    '/widgets/generals/useLeaders.ts': `export const useLeaders = () => ({ data: ${JSON.stringify(data)}, retry() {} });
      export const useLeaderCountries = () => ({ countries: null, requestSelection: { kind: 'all' }, group: {}, known: true });`,
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/widgets/general-career/CareerView.tsx': 'export default () => null;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 102;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/jump.ts': 'export const jumpItem = (_runtime, _track, hour) => ({ hour });',
    '/charts/table/Table.tsx': `export const CountryCell = () => null;
      export default props => { globalThis.__projectedLeaders = props; return null; };`,
  };
  const host = globalThis as typeof globalThis & { __projectedLeaders?: TableProps<LeaderRow> };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'projected-leaders-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/generals/GeneralsWidget.tsx') as typeof import('./GeneralsWidget.tsx');
    const props = { widget: { id: 'generals', kind: 'generals', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 102 } }],
        clock: { getHour: () => 102 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(Widget, props));
    const table = host.__projectedLeaders!;
    const [row] = table.rows;
    const identity = table.columns.find(column => column.key === 'leader')!.render!;
    assert.equal(renderToStaticMarkup(identity(row)), renderToStaticMarkup(identity({ ...row, hour: row.hour + 1 })), 'hourly metadata does not change the rendered identity');
    assert.notEqual(renderToStaticMarkup(identity(row)), renderToStaticMarkup(identity({ ...row, person: { ...row.person, name: 'Renamed leader' } })), 'a displayed identity change still invalidates its cell');
    assert.deepEqual([row.fought, row.won, row.lost], [2, 1, 0]);
    const [jump] = table.menu!(row) as unknown as { hour: number; label: string }[];
    assert.equal(jump.hour, 95, 'the first seen hour, not the projected run\'s');
    const item = table.parity!.item!(row) as { firstSeen: number; traitDates: unknown };
    assert.equal(item.firstSeen, 95);
    assert.deepEqual(item.traitDates, [[101, 'gained']], 'the latest gain through the hour');
  } finally {
    delete host.__projectedLeaders;
    await vite.close();
  }
});

test('leaderboard keeps the dated country selection when an alias is reused', async () => {
  const data: UnitLeadersData = { branchId: '1', firstHour: 100,
    selectionTags: ['original', 'alias', 'current'],
    people: [5, 6].map(characterId => ({ name: String(characterId), portraitHour: 100, portraitReference: String(characterId) })),
    roles: ['current', 'alias'].map(owner => ({ owner, category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] })),
    traits: [{ token: 'synthetic_trait', name: 'Synthetic trait', icon: null }], leaders: [5, 6].map((id, index) => ({ reference: String(id),
      runs: [{ firstHour: 100, lastHour: 102, value: { person: index, role: index, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [0] } }],
      traitGains: [], commands: [], battleStarts: [], battleStartReadHours: [], unknownBattleStarts: [], partialFrom: null })),
    withheld: [], observed: [{ firstHour: 100, lastHour: 102, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
    resultsMissing: 'Result not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] };
  const replacements: Record<string, string> = {
    '/widgets/generals/useLeaders.ts': `export const useLeaders = () => ({ data: ${JSON.stringify(data)}, retry() {} });
      export const useLeaderCountries = () => ({ countries: new Set(['current']), requestSelection: { kind: 'countries', tags: ['original'] }, group: {} });`,
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/widgets/general-career/CareerView.tsx': 'export default () => null;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 102;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/table/Table.tsx': `import { createElement } from 'react';
      export const CountryCell = () => null;
      export default props => { globalThis.__selectedLeaders = props.rows; globalThis.__leaderParity = props.parity;
        return createElement('div', null, props.rows.map(row => createElement('div', { key: row.key }, props.columns.find(column => column.key === 'traits').render(row)))); };`,
  };
  const host = globalThis as typeof globalThis & { __selectedLeaders?: readonly LeaderRow[]; __leaderParity?: TableProps<LeaderRow>['parity'] };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'leader-selection-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/generals/GeneralsWidget.tsx') as typeof import('./GeneralsWidget.tsx');
    const props = { widget: { id: 'generals', kind: 'generals', trackIds: ['main'], countries: { kind: 'tags', tags: ['original'] },
      time: { mode: 'campaign' }, options: {} }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 102 } }],
        clock: { getHour: () => 102 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const html = renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(host.__selectedLeaders?.map(row => row.key), ['5']);
    const original = host.__selectedLeaders![0];
    assert.deepEqual(host.__leaderParity?.reads, { traitCatalogue: data.traits });
    const projected = host.__leaderParity!.item!(original) as LeaderRow;
    assert.equal(projected.traitCatalogue, undefined);
    assert.deepEqual(projected.value.traits, [0]);
    assert.equal(JSON.stringify({ catalogue: host.__leaderParity?.reads, rows: [projected, projected] }).split('synthetic_trait').length - 1, 1);
    assert.match(html, /class="leader-trait-chip">Synthetic trait/);
    assert.doesNotMatch(html, /icon unavailable|>\?</);
    const { LeaderPortrait, SkillStars } = await vite.ssrLoadModule('/ui/src/widgets/generals/LeaderIdentity.tsx');
    const portrait = renderToStaticMarkup(createElement(LeaderPortrait, { art: { image: null, missing: 'Portrait supplying read is not recorded' } }));
    assert.match(portrait, /art-fallback/);
    assert.match(portrait, /data-detail="Portrait supplying read is not recorded"/);
    assert.match(portrait, /data-tooltip="Portrait unavailable\."/);
    assert.match(portrait, /aria-label="Portrait unavailable\."/);
    assert.doesNotMatch(portrait.replace(/<[^>]*>/g, ''), /supplying read/);
    assert.doesNotMatch(portrait, /&gt;\?&lt;|>\?</);
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    for (const error of [new Error('Synthetic portrait decoder failure'), new RefusedError('Synthetic portrait is not supported')]) {
      const failed = renderToStaticMarkup(createElement(LeaderPortrait, { art: error }));
      const text = error instanceof RefusedError ? error.message : 'Couldn’t load artwork.';
      assert.ok(failed.includes(`data-detail="${error.message}"`));
      assert.ok(failed.includes(`data-tooltip="${text}"`));
      assert.ok(failed.includes(`aria-label="${text}"`));
      assert.doesNotMatch(failed.replace(/<[^>]*>/g, ''), /Synthetic portrait decoder failure|>\?</);
    }
    const skill = renderToStaticMarkup(createElement(SkillStars, { level: 5, maximum: null }));
    assert.match(skill, />5</);
    assert.doesNotMatch(skill, /maximum unknown/);
  } finally {
    delete host.__selectedLeaders;
    delete host.__leaderParity;
    await vite.close();
  }
});
