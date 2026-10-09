import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { LedgerGovernment, PartyHistoryReply, PoliticsLedgerReply } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client';
import type { WidgetProps } from '../../dashboard/definitions';
import type { EventBadge } from '../../charts/badges';
import type { PlotFrame } from '../../charts/frame';
import type { TooltipContent } from '../../charts/tooltip';
import { governmentBadges, governmentCause, governmentRows, validateGovernmentLedger } from './governmentData.ts';
import { partyBadges } from './partyData.ts';

const transition: LedgerGovernment = { country: 1, instance: 1, epoch: '1', sequence: '1', hour: 24, readHour: 26,
  beforeParty: 41, afterParty: 42, beforeIdeology: 91, afterIdeology: 92, beforeToken: 'mod_old', afterToken: 'mod_new',
  actorCountry: null, originKind: 'OnAction', originToken: null, originKey: [], cause: 'CoupCompletion', covered: false };
const parties: PartyHistoryReply = { branchId: '1', tag: 'mod country', decimals: 5, parties: [], leaders: [], people: [],
  ideologyTypes: [], ideologies: [], partyNames: [], partyNameRuns: [], unobserved: [], warnings: [], elections: [26],
  ruling: [{ token: 'mod_old', firstHour: 24, lastHour: 25, startsSegment: true },
    { token: 'mod_new', firstHour: 26, lastHour: 30, startsSegment: false }] };
const ledger: PoliticsLedgerReply = { branchId: parties.branchId, tag: parties.tag, decimals: 5, governments: [transition],
  warnings: [], spending: [], spendItems: [], spendCategories: [], spendingCoverage: [], ideaTooltips: null, commandHolds: [] };

test('a native cause appears after its supplying read, at its execution date with explicit coverage', () => {
  assert.deepEqual(governmentBadges([transition], token => token, 25), []);
  const badge = governmentBadges([transition], token => token, 26)[0];
  assert.equal(badge.hour, 24);
  assert.match(badge.label, /mod_new took power from mod_old.*Coup.*read.*interval incomplete/);
  assert.equal(governmentRows([transition], 25, 30, token => token).length, 0);
  assert.equal(governmentRows([transition], 26, 30, token => token)[0].value, 'Coup');
  assert.equal(governmentCause('not-a-native-enum'), 'Not identified (not-a-native-enum)');
});

test('only exact endpoint and supplying-read matches supersede census; elections remain observations', () => {
  assert.deepEqual(partyBadges(parties, token => token, false, [transition]).map(badge => badge.glyph), ['\u2713']);
  for (const unrelated of [{ ...transition, afterToken: 'other' }, { ...transition, beforeToken: null }, { ...transition, readHour: 27 }]) {
    const badges = partyBadges(parties, token => token, false, [unrelated]);
    assert.equal(badges.filter(badge => badge.glyph === '\u21c6').length, 1);
    assert.equal(badges.find(badge => badge.glyph === '\u21c6')?.label.includes('Election'), false);
  }
  assert.equal(governmentRows([{ ...transition, beforeToken: null, afterToken: null }], 26, 30, token => token)[1].value,
    'Not identified / Not identified');
});

test('government replies authenticate the requested actor and preserve exact native identity widths', () => {
  assert.equal(validateGovernmentLedger(ledger, parties), ledger);
  assert.equal(validateGovernmentLedger({ ...ledger, governments: undefined }, parties).governments, undefined);
  for (const bad of [{ ...ledger, tag: 'another actor' }, { ...ledger, branchId: '2' }, { ...ledger, warnings: [null] }]) {
    assert.throws(() => validateGovernmentLedger(bad, parties));
  }
  for (const bad of [{ ...transition, readHour: 23 }, { ...transition, country: -1 }, { ...transition, instance: 0x1_0000_0000 },
    { ...transition, hour: 0x8000_0000 }, { ...transition, epoch: '18446744073709551616' }, { ...transition, sequence: '-1' }]) {
    assert.throws(() => validateGovernmentLedger({ ...ledger, governments: [bad] }, parties));
  }
});

test('registered ruling-party renderer requests causes conditionally, respects Pin and keeps parties on refusal', async () => {
  const host = globalThis as typeof globalThis & { __governmentParty?: PartyHistoryReply; __governmentLedger?: PoliticsLedgerReply;
    __governmentError?: string; __governmentRequests?: ProductRequest<Record<string, unknown>>[];
    __governmentChart?: { badges?: EventBadge[]; tooltip: (hour: number, frame: PlotFrame, badges: readonly EventBadge[]) => TooltipContent } };
  host.__governmentParty = { ...parties, parties: ['mod_old', 'mod_new'].map(token => ({ token,
    runs: [{ firstHour: 24, lastHour: 30, valueRaw: '5000000', startsSegment: true }] })),
    ideologies: [...['mod_old', 'mod_new'].map<PartyHistoryReply['ideologies'][number]>(token => ({ token, name: token, color: [30, 60, 90] })),
      { token: 'mod_between', name: 'Named transient ideology', color: null }],
    partyNames: ['Native party name'], partyNameRuns: [{ firstHour: 24, lastHour: 30, name: 0 }] };
  const requests: ProductRequest<Record<string, unknown>>[] = [];
  host.__governmentLedger = ledger; host.__governmentRequests = requests;
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/products.ts': `export const useProduct = (_client, request, _active, validate) => {
      if (!request) return { retry() {} }; globalThis.__governmentRequests.push(request);
      if (request.product === 'politics-ledger' && globalThis.__governmentError) return { error: globalThis.__governmentError, retry() {} };
      return { data: validate(request.product === 'party-history' ? globalThis.__governmentParty : globalThis.__governmentLedger), retry() {} };
    };`,
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__governmentChart = props; return null; };',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/tokens.ts': 'export const readTokens = () => ({ surface: "#101820" }); export const subscribeTokens = () => () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'government-renderer-inputs', enforce: 'pre',
      transform(_code, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; },
    }] });
  try {
    const { default: renderer } = await vite.ssrLoadModule('/ui/src/widgets/ruling-party/index.ts');
    let hour = 25;
    const props = { widget: { kind: 'ruling-party', trackIds: ['track'], countries: { kind: 'tags', tags: [parties.tag] }, time: { mode: 'follow' }, options: {} },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 24, lastHour: 30 }, source: { id: 'source', revision: '1' } }],
        clock: { getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 1 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(renderer.render, props));
    render();
    assert.equal(requests.find(request => request.product === 'politics-ledger')?.params.view, 'governments');
    assert.ok(host.__governmentChart?.badges?.every(badge => !badge.label.includes('Coup')));
    hour = 26; assert.match(render(), /Native party name/);
    assert.deepEqual(host.__governmentChart?.badges?.map(badge => badge.hour), [24, 26]);
    const tooltip = host.__governmentChart?.tooltip(26, {} as PlotFrame, []);
    assert.ok(tooltip?.rows.some(row => row.value === 'Coup'));
    assert.ok(tooltip?.rows.some(row => row.label === 'Read'));
    props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 1, lastOffset: 1 } } }; hour = 30; render();
    assert.ok(host.__governmentChart?.badges?.every(badge => !badge.label.includes('Coup')));
    props.widget.time = { mode: 'follow' }; host.__governmentError = 'Synthetic government refusal';
    const refused = render(); assert.match(refused, /Retry government causes/); assert.match(refused, /Native party name/);
    props.widget.options = { changes: false }; requests.length = 0;
    assert.doesNotMatch(render(), /Retry government causes/);
    assert.ok(requests.every(request => request.product !== 'politics-ledger'));
    props.widget.options = { changes: true }; delete host.__governmentError;
    host.__governmentLedger = { ...ledger, governments: [{ ...transition, afterToken: 'mod_between' }] };
    render();
    assert.ok(host.__governmentChart?.badges?.some(badge => badge.label.includes('Named transient ideology took power from mod_old')));
    assert.equal(host.__governmentChart?.tooltip(26, {} as PlotFrame, []).rows.find(row => row.label === 'Before / after')?.value,
      'mod_old / Named transient ideology');
  } finally {
    delete host.__governmentParty; delete host.__governmentLedger; delete host.__governmentError;
    delete host.__governmentRequests; delete host.__governmentChart; await vite.close();
  }
});
