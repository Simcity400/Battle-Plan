import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TimelineProps } from '../../charts/timeline/Timeline';
import type { UnitLeadersData } from '../generals/generalsData';

const data: UnitLeadersData = { branchId: '1', firstHour: 100,
  people: [{ characterId: 9, token: 'mod_character', name: 'Commander', portraitHour: 100, portraitReference: '21474841193' }],
  roles: [{ owner: 'mod country', category: 1, rank: 'Rank', kind: 1, maximumSkill: 12, attributeNames: ['Attack', 'Defence', 'Planning', 'Logistics'] }],
  traits: [], leaders: [{ reference: '21474841193', runs: [{ firstHour: 100, lastHour: 500,
    value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [] } }],
    traitGains: [], commands: [], battleStarts: [100], battleStartReadHours: [100], unknownBattleStarts: [0], battleWon: [true], battleResultHours: [110], battleResultReadHours: [125], partialFrom: null }],
  observed: [{ firstHour: 100, lastHour: 500, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
  resultsMissing: 'Result not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] };

test('actual career renderer preserves growing zoom and supplies the shown label hour', async () => {
  // Keep the widget and shared career renderer; replace transport, artwork and canvas leaves.
  const replacements: Record<string, string> = {
    '/widgets/generals/useLeaders.ts': `export const useLeaders = () => ({ data: globalThis.__careerData ?? ${JSON.stringify(data)}, retry() {} });`,
    '/dashboard/parityRows.ts': 'export const useParityRows = (_root, read) => { globalThis.__careerParity = read; };',
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => ({});',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__careerTimelines.push(props); return null; };',
  };
  const host = globalThis as typeof globalThis & { __careerTimelines?: TimelineProps[]; __careerData?: UnitLeadersData; __careerParity?: () => { label: string; hour: number; rows: unknown[]; current: { won: number } } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'career-renderer-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/general-career/GeneralCareerWidget.tsx') as typeof import('./GeneralCareerWidget.tsx');
    const { default: Career } = await vite.ssrLoadModule('/ui/src/widgets/general-career/CareerView.tsx') as typeof import('./CareerView.tsx');
    let hour = 300;
    const props = { widget: { id: 'career', kind: 'general-career', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'follow' }, options: { leader: '21474841193' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 500 } }],
        clock: { getHour: () => hour, getSpeed: () => 24, subscribe: () => () => {} }, client: {} },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    host.__careerTimelines = [];
    renderToStaticMarkup(createElement(Widget, props));
    const full = host.__careerTimelines.at(-1)!;
    assert.deepEqual(full.extent, [100, 501]);
    assert.equal(full.preserveZoomOnExtentChange, false);
    assert.equal(full.labelHour, 300); assert.equal(full.playhead, 301);
    props.widget.options = { leader: '21474841193', growWithPlayback: true };
    for (hour of [300, 324]) {
      renderToStaticMarkup(createElement(Widget, props));
      const growing: TimelineProps = host.__careerTimelines.at(-1)!;
      assert.deepEqual(growing.extent, [100, hour + 1]);
      assert.equal(growing.preserveZoomOnExtentChange, true);
      assert.equal(growing.labelHour, hour); assert.equal(growing.playhead, hour + 1);
      assert.equal(growing.lanes[0].spans[0].firstHour, 100);
    }
    renderToStaticMarkup(createElement(Career, { ...props, widget: { ...props.widget, kind: 'generals' },
      reference: '21474841193', hour: 120, onPick() {} }));
    const picked = host.__careerTimelines.at(-1)!;
    assert.deepEqual(picked.extent, [100, 121]);
    assert.equal(picked.preserveZoomOnExtentChange, true);
    assert.equal(picked.labelHour, 120); assert.equal(picked.playhead, 121);
    assert.equal(picked.lanes.at(-1)!.spans[0].positiveCount, 0);
    renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 130,
      widget: { ...props.widget, time: { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 20 } } } }, onPick() {} }));
    assert.equal(host.__careerParity?.().label, 'General career values');
    assert.equal(host.__careerParity?.().hour, 130);
    assert.equal(host.__careerParity?.().current.won, 1);
    assert.deepEqual(host.__careerParity?.().rows, host.__careerTimelines.at(-1)!.lanes);
    assert.equal(host.__careerTimelines.at(-1)!.lanes.at(-1)!.spans[0].positiveCount, 1,
      'A window selects native result dates but eligibility uses the displayed read');
    host.__careerData = structuredClone(data);
    host.__careerData.leaders[0].runs[0].value.progress = {
      experienceRaw: '12345678', nextRequiredRaw: '20000000', factors: [{token: 'mod_trait', name: 'Custom factor', factorRaw: '25000'}], training: [
        { token: 'mod_trait', name: 'Custom training', experienceRaw: '9876543', requiredRaw: '12000000' },
        { token: 'mod_trait', name: 'Custom training', experienceRaw: '-100', requiredRaw: '12000000' },
      ],
    };
    const experience = renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 130 }));
    assert.match(experience, /123\.45678 \/ 200\.0/);
    assert.match(experience, /Custom factor: .*\+25\.0%/);
    assert.match(experience, /Custom training: .*98\.76543 \/ 120\.0/);
    assert.match(experience, /Custom training: .*-0\.001 \/ 120\.0/);
    host.__careerData.leaders[0].runs[0].value.progress!.training = [];
    assert.match(renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 130 })), /No traits in training/);
    host.__careerData = structuredClone(data);
    host.__careerData.roles[0].kind = 2;
    host.__careerData.leaders[0].partialFrom = 101;
    host.__careerData.leaders[0].commands = [{ firstHour: 100, lastHour: 500, formation: '999999', kind: 'fleet',
      name: null, nameReadHour: null, nameUnknown: [] }];
    const html = renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 130 }));
    assert.match(html, /Fleet command history is incomplete/);
    assert.match(html, /Battle counts are incomplete/);
    assert.doesNotMatch(html, /win rate among decided|Navy commands not recorded/);
    const fleetView = host.__careerTimelines.at(-1)!;
    const commands = fleetView.lanes.find(lane => lane.id === 'commands')!;
    const tip = fleetView.tooltip!(commands, commands.spans[0], 130)!;
    assert.equal(tip.rows[0].value, 'Unnamed fleet');
    assert.equal(tip.note, 'Fleet command history is incomplete.');
    assert.doesNotMatch(JSON.stringify(tip), /999999/);
    const prefix = { coverage: { firstHour: 100, lastHour: 149, complete: false }, pending: [{ firstHour: 150, lastHour: 500 }] };
    host.__careerData = Object.assign(structuredClone(data), prefix);
    host.__careerData.leaders[0].runs[0].lastHour = 149;
    host.__careerData.observed[0].lastHour = 149;
    const pending = renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 300 }));
    assert.match(pending, /Leader details are still being prepared at this time/);
    assert.match(pending, /Still being prepared/);
    assert.doesNotMatch(pending, /Leader details unavailable|Result not recorded/);
    const tail = host.__careerTimelines.at(-1)!;
    assert.deepEqual(tail.pending, prefix.pending);
    assert.deepEqual(tail.unobserved, prefix.pending);
    assert.equal(tail.tooltip!(tail.lanes[0], null, 300)!.rows[0].value, 'Still being prepared');
    assert.notEqual(tail.tooltip!(tail.lanes[0], tail.lanes[0].spans[0], 140)!.rows[0].value, 'Still being prepared');
    assert.ok(tail.lanes[0].spans.every(span => span.lastHour <= 149));
    host.__careerData.leaders = [];
    const cold = renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 300 }));
    assert.match(cold, /This range is still being prepared/);
    assert.doesNotMatch(cold, /Career unavailable/);
    host.__careerData = structuredClone(data);
    const exact = renderToStaticMarkup(createElement(Career, { ...props, reference: '21474841193', hour: 300 }));
    assert.doesNotMatch(exact, /still being prepared|Still being prepared/);
    assert.equal(host.__careerTimelines.at(-1)!.pending, undefined);

  } finally {
    delete host.__careerTimelines; delete host.__careerData; delete host.__careerParity;
    await vite.close();
  }
});
