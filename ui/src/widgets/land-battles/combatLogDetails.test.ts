import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { BRANCH, humansBy, mountModule, PENDING_HOUR, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CombatLogReply, CountryGroupReply, CombatLogParams } from '../../generated/dashboard';
import type { CountrySelection, WidgetSpec } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';

const HOUR = 60_000_001;
type State = { groups?: CountryGroupReply; reply: CombatLogReply; request: { params: CombatLogParams } | null };
function reply(): CombatLogReply {
  return { hour: HOUR, groups: [{ tag: 'GER (11)', group: '30064771125', ordinal: 0 }],
    manpower: [{ tag: 'GER', group: '7', ordinal: 0, date: HOUR, reason: 0, losses: [-7, 0, 17] }],
    equipment: [{ tag: 'GER', group: '7', month: 0, ordinal: null, date: HOUR, kind: 5, reason: 0,
      allowZero: true, name: null, variant: null, amountRaw: null, missingSource: null }],
    templates: [], battles: [{ tag: 'GER', group: '7', ordinal: 0, id: 9, attacker: true, date: HOUR, province: 4,
      flags: [1, 2, 3, 4], manpower: [7, 9], references: 2, details: [
        { title: 'Sides', columns: ['Leader identity', 'Air factor'], rows: [['18446744073709551615', '-1.25']] },
        { title: 'Countries', columns: ['Country'], rows: [['SOV (12)']] },
      ] }], active: [{ combat: '71', attacker: true, terminal: false, progressRaw: '25000', damageRaw: '125000',
      activityGroups: 1, leaders: 1, lossSource: 'LandCombatProgress: loss gap', details: [
        { title: 'Activity templates', columns: ['Template / identity'], rows: [['mod template (17179869238)']] },
        { title: 'Activity division damage', columns: ['Division identity', 'Damage'], rows: [['38654705715', '0.125']] },
        { title: 'Damage attribution', columns: ['From', 'Receiver', 'To'], rows: [['GER', 'SOV', 'AAA']] },
        { title: 'Leader hours', columns: ['Leader identity', 'Hours'], rows: [['18446744073709551615', '-9']] },
        { title: 'Stored counters', columns: ['Index', 'Hours'], rows: [['29', '-2147483648']] },
      ] }], availability: { groups: null, manpower: null, months: null, ownEquipment: null, enemyEquipment: null,
      templates: null, battles: null, active: null, terminal: 'LandCombatResults: attachment interval' }, warnings: [] };
}
function catalogue(): CountryGroupReply {
  const members = ['GER', 'SOV'].map(tag => ({ tag, firstHour: HOUR - 10, lastHour: HOUR + 10 }));
  return { branchId: '1', humans: members.slice(0, 1), factions: [{ id: '4', name: 'Recorded faction', nameReason: null,
    names: [], recordedName: 'recorded', firstHour: HOUR - 10, lastHour: HOUR + 10, members, leaders: members.slice(0, 1), incomplete: [] }],
    wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] },
    countryIdentities: members.map((r, i) => ({ tag: r.tag, identities: [{ firstHour: r.firstHour, lastHour: r.lastHour, slot: i + 1, countryId: i + 11 }] })) };
}
async function fixture() {
  const state: State = { groups: catalogue(), reply: reply(), request: null };
  const globals = globalThis as typeof globalThis & { __combatLogDetails?: State };
  globals.__combatLogDetails = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'combat-log-consumers', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/dashboard/products.ts')) return `export const useProduct = (_client, request) => {
        const state = globalThis.__combatLogDetails; state.request = request; return { data: state.reply }; };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`;
      if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({data: globalThis.__combatLogDetails.groups});';
      if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = clock => clock.getHour();';
      if (path.endsWith('/land-battles/CombatLogDetails.tsx')) return source.replace(/\buseMemo(?:<[^;\n]*>)?\(/g, '((factory) => factory())(');
    } }] });
  const component = await vite.ssrLoadModule('/ui/src/widgets/land-battles/CombatLogDetails.tsx') as typeof import('./CombatLogDetails');
  const track = { id: 'main', axisOffset: 0, selectedTag: 'GER', branch: { id: '1', firstHour: HOUR - 10, lastHour: HOUR + 10 },
    source: { id: 'source', revision: '1', metadata: { countries: [] } } };
  const runtime = { tracks: [track], client: {}, clock: { getHour: () => HOUR } } as unknown as WidgetRuntime;
  const widget = (countries: CountrySelection): WidgetSpec => ({ id: 'test', kind: 'land-battles', title: '', configVersion: 1,
    trackIds: ['main'], countries, time: { mode: 'follow' } });
  return { state, component, select(countries: CountrySelection) {
    return component.CombatLogSnapshot({ widget: widget(countries), runtime, active: true, view: 'battles',
      mapOptions: { country: null, fog: true }, headerSlot: null, onDiagnostics() {} });
  }, async close() { await vite.close(); delete globals.__combatLogDetails; } };
}

test('retained Combat Log requests keep all manual, smart-group, faction and all-country selections', async () => {
  const f = await fixture();
  try {
    f.select({ kind: 'tags', tags: ['GER', 'SOV'] });
    assert.deepEqual(new Set(f.state.request!.params.countries.membership!.map(r => r.tag)), new Set(['GER', 'SOV']));
    f.select({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false });
    assert.deepEqual(f.state.request!.params.countries.membership!.map(r => r.tag), ['GER']);
    f.select({ kind: 'group', group: { type: 'faction', id: '4' }, membership: 'hour', aggregate: false });
    assert.deepEqual(new Set(f.state.request!.params.countries.membership!.map(r => r.tag)), new Set(['GER', 'SOV']));
    f.select({ kind: 'all' });
    assert.deepEqual(f.state.request!.params.countries, { tags: null, membership: null, unobserved: [] });
    f.state.groups = undefined;
    f.select({ kind: 'tags', tags: ['GER', 'SOV'] });
    assert.equal(f.state.request, null, 'pending identity resolution never becomes a dashboard-country request');
  } finally { await f.close(); }
});

test('complete retained sections and expandable detail tables survive independent event gaps', async () => {
  const f = await fixture();
  try {
    const render = (view: 'manpower' | 'equipment' | 'battles') => renderToStaticMarkup(f.component.CombatLogTables({ data: f.state.reply, view }) as ReactElement);
    assert.match(render('manpower'), /Manpower losses/);
    assert.match(render('equipment'), /Month entry; no amount rows/);
    const battles = render('battles');
    for (const text of ['LandCombatResults: attachment interval', 'LandCombatProgress: loss gap', 'Countries', 'SOV (12)',
      'Activity templates', 'mod template (17179869238)', 'Activity division damage', 'Damage attribution', 'Leader hours',
      '18446744073709551615', '-2147483648', '-1.25']) assert.ok(battles.includes(text), text);
    f.state.reply.availability.manpower = 'CombatLogManpower: history gap';
    assert.match(render('manpower'), /CombatLogManpower: history gap/);
    assert.doesNotMatch(render('manpower'), /No retained manpower months/);
    assert.match(render('equipment'), /Retained equipment months/);
    assert.equal(f.component.fixed5('-9223372036854775808'), '-92233720368547.75808');
  } finally { await f.close(); }
});

test('a retained Combat Log at an hour whose group membership is still being prepared says so', async () => {
  const state = { groups: pendingGroups(), reply: { ...reply(), hour: PENDING_HOUR } };
  const globals = globalThis as typeof globalThis & { __combatLogPending?: typeof state }; globals.__combatLogPending = state;
  const { module, close } = await mountModule<typeof import('./CombatLogDetails')>('/ui/src/widgets/land-battles/CombatLogDetails.tsx', {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__combatLogPending.reply }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__combatLogPending.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
  });
  try {
    const runtime = { tracks: [{ id: 'main', axisOffset: 0, selectedTag: 'GER', branch: BRANCH, source: { id: 'source', revision: '1', metadata: { countries: [] } } }],
      client: {}, clock: { getHour: () => PENDING_HOUR } } as unknown as WidgetRuntime;
    const widget: WidgetSpec = { id: 'test', kind: 'land-battles', title: '', configVersion: 1, trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' } };
    const render = () => renderToStaticMarkup(createElement(module.CombatLogSnapshot, { widget, runtime, active: true, view: 'battles',
      mapOptions: { country: null, fog: true }, headerSlot: null, onDiagnostics() {} } as never));
    const pending = render();
    assert.match(pending, /role="status">This range is still being prepared\.</);
    assert.match(pending, /Retained Combat Log/, 'entries already read stay shown');
    // The final reply has read the hour: the entries shown are all there are.
    state.groups = pendingGroups({ final: true });
    assert.doesNotMatch(render(), /This range is still being prepared/);
  } finally { await close(); delete globals.__combatLogPending; }
});
