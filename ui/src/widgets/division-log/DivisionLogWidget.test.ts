import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { TooltipContent } from '../../charts/tooltip';
import type { DivisionLogEntry } from '../../generated/dashboard';

const row: DivisionLogEntry = { id: '1', division: '1', readHour: 12, hour: 11, by: false, afterHour: null,
  kind: 'lost', status: 'division_destroyed', tag: 'owner', countryHour: 10, name: 'Guards', nameMissing: null,
  icon: null, iconMissing: 'Type art unavailable', presentationHour: 10, template: '99', templateName: 'Guards',
  templateMissing: null, templateHour: 10, province: null, placeHour: null, placeAtEvent: false,
  killerTag: 'enemy', killerHour: null, killerDivision: '2' };

test('division rows forward dated flag state while null actors and the unknown killer name stay explicit', async () => {
  const reply = { window: 0, interval: { firstHour: 10, lastHour: 20 }, rows: [row, { ...row, id: '2', tag: null, killerTag: null }],
    contentWarnings: [], warnings: [] };
  const manifest = { windows: [reply.interval], available: true, lossesAvailable: true, templates: [], contentWarnings: [] };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product === 'division-log-manifest' ? ${JSON.stringify(manifest)} : undefined });
      export const loadProduct = () => { throw new Error('Unexpected country request'); };`,
    '/dashboard/productWindows.ts': `export const useProductWindows = () => ({ data: new Map([[0, ${JSON.stringify(reply)}]]), errors: new Map(), retry() {} });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: hour => tag => {
      globalThis.__divisionFlagReads.push([hour, tag]); return globalThis.__divisionFlagLabels[hour + ':' + tag]; } });`,
    '/charts/log/LogList.tsx': `import { createElement } from 'react'; export default props => createElement('div', null,
      props.rows.map(row => { globalThis.__divisionFlagTips.push(...props.tooltip(row)); return createElement('div', { key: row.key }, props.renderRow(row)); }));`,
  };
  const host = globalThis as typeof globalThis & {
    __divisionFlagLabels?: Record<string, ReturnType<CountryLabel>>;
    __divisionFlagReads?: [number, string][]; __divisionFlagTips?: TooltipContent[];
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'division-flag-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/division-log/DivisionLogWidget.tsx') as typeof import('./DivisionLogWidget.tsx');
    host.__divisionFlagLabels = { '10:owner': { name: 'Owner at 10', flagPending: true },
      '12:enemy': { name: 'Read-hour enemy name must stay unused', flagPending: false, flagReason: 'Native flag unavailable at read 12.' } };
    host.__divisionFlagReads = []; host.__divisionFlagTips = [];
    const props = { widget: { id: 'divisions', kind: 'division-log', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'campaign' }, options: { chips: false } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'synthetic' } },
        branch: { id: '1', firstHour: 10, lastHour: 20 } }], clock: { getHour: () => 20 }, client: {} },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const html = renderToStaticMarkup(createElement(Widget, props));
    assert.equal((html.match(/role="img"/g) ?? []).length, 2);
    assert.match(html, /aria-label="Flag loading"/);
    assert.match(html, /aria-label="Flag not available: Native flag unavailable at read 12\."/);
    assert.match(html, /Killer not known/); assert.match(html, /Country not known/);
    assert.doesNotMatch(html, /Read-hour enemy name must stay unused/);
    assert.ok(host.__divisionFlagReads.some(([hour, tag]) => hour === 10 && tag === 'owner'));
    assert.ok(host.__divisionFlagReads.some(([hour, tag]) => hour === 12 && tag === 'enemy'));
    assert.ok(host.__divisionFlagReads.every(([hour, tag]) => hour === 10 && tag === 'owner' || hour === 12 && tag === 'enemy'));
    const killer = host.__divisionFlagTips[0].rows.find(row => row.label === 'Killer')!;
    assert.equal(killer.value, 'Killer not known');
    assert.equal(killer.countryFlag?.flagReason, 'Native flag unavailable at read 12.');
    assert.ok(host.__divisionFlagTips[1].rows.every(row => row.countryFlag === undefined));
  } finally {
    delete host.__divisionFlagLabels; delete host.__divisionFlagReads; delete host.__divisionFlagTips;
    await vite.close();
  }
});
