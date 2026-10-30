import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';

for (const opened of [false, true]) test(`doctrine panel ${opened ? 'shows exact selected state' : 'defers its request while closed'}`, async () => {
  const host = globalThis as typeof globalThis & { __doctrineCalls?: { active: boolean; request: { params: { tag: string; hour: number } } }[]; __doctrineReply?: unknown };
  host.__doctrineCalls = [];
  host.__doctrineReply = { branchId: '1', tag: 'MOD', hour: 7, warnings: [], inventory: [], folders: [{ ordinal: 0, name: 'Mod folder', grandDoctrine: 'Mod grand doctrine', completed: ['Earlier subdoctrine'], benefits: 'Grand benefits', authoredCost: '73 army', milestones: [], tracks: [{ ordinal: 0, name: 'Mod track', subDoctrine: 'Mod subdoctrine', rewards: 1, mastery: '9223372036854775807', masteryBank: '-1', dailyMastery: '34', unitDailyMastery: '12', rewardRequirements: ['123456'], benefits: 'Subdoctrine benefits', authoredCost: '42 army', commanders: [{ name: 'Mod commander', value: '0.1' }], rewardBenefits: [{ name: 'Mod reward', mastery: '123456', unlocked: true, description: 'Organisation bonus' }] }] }] };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = (client, request, active) => { globalThis.__doctrineCalls.push({request, active}); return {data: globalThis.__doctrineReply}; }; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
  };
  const vite = await createServer({ root: fileURLToPath(new URL('../../../../', import.meta.url)), configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'doctrine-panel-fixture', enforce: 'pre', transform(source, id) {
    const path = id.replaceAll('\\', '/');
    if (opened && path.endsWith('/research/DoctrinePanel.tsx')) return source.replace('useState(false)', 'useState(true)');
    return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
  } }] });
  try {
    const { ResearchWithDoctrines } = await vite.ssrLoadModule('/ui/src/widgets/research/DoctrinePanel.tsx');
    const props = { widget: { id: 'research', kind: 'research', trackIds: ['track'], countries: { kind: 'tags', tags: ['MOD'] }, time: { mode: 'follow' } }, runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: 10 }, source: { id: 'source', revision: '1' } }], clock: { getHour: () => 7 } }, active: true, onDiagnostics() {}, body: () => createElement('p', null, 'Research fixture') } as unknown as WidgetProps;
    const markup = renderToStaticMarkup(createElement(ResearchWithDoctrines, props));
    assert.match(markup, /Research fixture/);
    assert.equal(host.__doctrineCalls.length, 1);
    assert.equal(host.__doctrineCalls[0].active, opened);
    assert.equal(host.__doctrineCalls[0].request.params.tag, 'MOD');
    assert.equal(host.__doctrineCalls[0].request.params.hour, 7);
    if (opened) {
      for (const text of ['Mod grand doctrine', 'Mod subdoctrine', '92233720368547.75807', '0.00034', '0.00012', 'Mod commander', 'Mod reward', 'Organisation bonus', 'Earlier subdoctrine']) assert.ok(markup.includes(text), text);
    } else assert.doesNotMatch(markup, /Mod grand doctrine|Mastery bank|Mod reward/);
  } finally { delete host.__doctrineCalls; delete host.__doctrineReply; await vite.close(); }
});
