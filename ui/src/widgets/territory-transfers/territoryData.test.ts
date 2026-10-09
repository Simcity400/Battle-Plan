import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import { Lineage } from '../../dashboard/identity.ts';
import { validatePeaceDealsReply } from '../../dashboard/peaceDealsData.ts';
import type { PeaceDeal, PeaceDealsReply, PeaceItem, PeaceTransferGraph } from '../../generated/dashboard';
import { buildTransferDiagram, chosenConference, conferenceChoiceId, conferenceToken, decodeTransfers, transferAmount, transferColourPlan, transferCutoff, transferDiagramDescription, transferProvenance,
  transfersThrough, transferNodeCountries, transferLookPlan, transferGapCount, transferGapWindow, victoryPointNote, visibleConferences } from './territoryData.ts';
import { transferSelected, transferSelection, transferSelectionToken } from './transferDrillData.ts';
import { definition, OPTIONS } from './definition.ts';

test('Territory transfers metadata keeps the whole recorded country roster', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  assert.equal(definition.defaultCountries.kind, 'all');
  assert.equal(definition.countrySelection, false);
});

const holders = new Map<string, number>();
const country = (tag: string, hour: number, slot = holders.get(tag) ?? holders.size + 1, countryId = slot + 1000) => {
  if (!holders.has(tag)) holders.set(tag, slot);
  return { tag, hour, slot, countryId };
};
const item = (state: number, after: number, vp: string | null, derived = true, from = 'Loser', to = 'Winner'): PeaceItem => ({
  derived, dates: { beforeHour: derived ? after - 1 : after, afterHour: after }, winner: to,
  action: { kind: 'stateTransfer', key: null, name: null }, change: { kind: 'stateTransfer', state, from, to, fromCountry: country(from, after), toCountry: country(to, after), victoryPoints: vp, victoryPointsReadHour: vp == null ? null : after + 1 },
});
function graph(items: readonly PeaceItem[]): PeaceTransferGraph {
  const nodes = new Map();
  const links = new Map();
  for (const item of items) {
    if (item.change.kind !== 'stateTransfer') continue;
    const change = item.change;
    const source = JSON.stringify(['source', countryIdentityKey(change.fromCountry)]);
    const target = JSON.stringify(['target', countryIdentityKey(change.toCountry)]);
    nodes.set(source, { id: source, country: change.fromCountry, side: 'source' });
    nodes.set(target, { id: target, country: change.toCountry, side: 'target' });
    const id = JSON.stringify([source, target]);
    const link = links.get(id) ?? { id, source, target, stateCount: 0, victoryPoints: '0', states: [] };
    link.stateCount++;
    link.victoryPoints = link.victoryPoints == null || change.victoryPoints == null ? null : String(BigInt(link.victoryPoints) + BigInt(change.victoryPoints));
    link.states.push({ state: change.state, fromCountry: change.fromCountry, toCountry: change.toCountry, dates: item.dates, victoryPoints: change.victoryPoints, victoryPointsReadHour: change.victoryPointsReadHour });
    links.set(id, link);
  }
  return { nodes: [...nodes.values()], links: [...links.values()] };
}
const deal = (id: string, hour: number, items: PeaceItem[]): PeaceDeal => ({ id, derived: true, observation: 'recordedConference', conferenceId: id,
  warName: null, firstObservedHour: hour, lastObservedHour: hour, currentTurn: null, finalized: null, firstFinalizedHour: null,
  endHour: hour, endIsBy: true, firstAbsentHour: hour, participantsDerived: true, winners: [], losers: [], items, captured: null,
  captureUnavailable: null, transfers: graph(items), limitations: [] });
const capturedDeal = (id: string, hour: number, publicationHour: number, items: PeaceItem[]): PeaceDeal => ({ ...deal(id, hour, items),
  derived: false, finalized: true, firstFinalizedHour: hour, endIsBy: false, firstAbsentHour: null,
  captured: { hour, publicationHour, winners: [], losers: [], actions: [], transfers: [], subjects: [] } });
const reply = (deals: PeaceDeal[]): PeaceDealsReply => ({ branchId: 'branch', derived: true, deals, transfers: graph(deals.flatMap(deal => deal.items)),
  unobserved: { settlements: [], conferences: [], ownership: [], wars: [], subjects: [], governments: [], existence: [], victoryPoints: [] }, warnings: [] });

test('full colour witnesses wait for publication independently of actual transfer dates', () => {
  const conference = capturedDeal('delayed', 112, 136, [item(1, 112, '3', false, 'A', 'B')]);
  const data = decodeTransfers(reply([conference]));
  const branch = { firstHour: 100, lastHour: 200, gaps: [] };
  const wanted = [{ tag: 'A', hour: 112 }, { tag: 'B', hour: 112 }];
  assert.deepEqual(transferColourPlan(data, branch, 135, true).wanted, []);
  assert.deepEqual(transferColourPlan(data, branch, 136, true).wanted, wanted);
  assert.deepEqual(transferColourPlan(data, { ...branch, gaps: [{ firstHour: 136, lastHour: 136 }] }, 140, true).wanted, wanted);
  assert.deepEqual(transferColourPlan(data, { ...branch, gaps: [{ firstHour: 112, lastHour: 112 }] }, 140, true).wanted, []);
  assert.deepEqual(transferColourPlan(data, branch, 200, false).wanted, []);
  const lineage = new Lineage([{ hour: 130, from: 'A', to: 'Renamed' }, { hour: 130, from: 'Other', to: 'A' }]);
  assert.notEqual(lineage.countryAt('A', 112), lineage.countryAt('A', 136), 'publication follows a real holder change');
  assert.equal(lineage.countryAt('A', transferColourPlan(data, branch, 140, true).wanted[0].hour), lineage.countryAt('A', 112));
});

test('late publication of an older event does not move a reached actor appearance backwards', () => {
  const data = decodeTransfers(reply([capturedDeal('old delayed', 112, 136, [item(1, 112, '3', false, 'A', 'B')]),
    capturedDeal('new reached', 120, 124, [item(2, 120, '3', false, 'B', 'C')])]));
  const branch = { firstHour: 100, lastHour: 200, gaps: [] };
  assert.deepEqual(transferColourPlan(data, branch, 135, true).wanted, [{ tag: 'B', hour: 120 }, { tag: 'C', hour: 120 }]);
  assert.deepEqual(transferColourPlan(data, branch, 136, true).wanted, [{ tag: 'A', hour: 112 }, { tag: 'B', hour: 120 }, { tag: 'B', hour: 112 }, { tag: 'C', hour: 120 }]);
});

test('actual Transfer caller plans hidden actors identically across campaign, conference and range scopes', async () => {
  const host = globalThis as typeof globalThis & { __transferFixture?: unknown; __transferStyleReads?: { tag: string; hour: number }[][] };
  const replacements: Record<string, string> = {
    '/dashboard/usePeaceDeals.ts': 'export const usePeaceDeals = () => globalThis.__transferFixture;',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => tag, color: () => null,
      flag: () => undefined, flagPending: () => false, flagReason: () => undefined, revision: 0 });`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryFactsRead = (_client, _track, wanted) => { globalThis.__transferStyleReads.push(wanted);
        return new Map(wanted.map(({ tag, hour }) => [nameKey(hour, tag), { tag, name: tag, color: [45, 67, 89] }])); };`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });
      export const flagReadSpec = () => ({}); export const flagReadMetadata = () => ({ flagPending: false });`,
    '/dashboard/useStateNamesAt.ts': 'export const useStateNamesAt = () => () => ({ name: null, missing: null });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ surface: '#111111', text: '#dddddd', countryColours: 'fixed' });`,
    '/charts/flow/FlowDiagram.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'transfer-colour-plan-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        // Preserve the real shared planner used by the widget; only its artwork hook is synthetic.
        if (path.endsWith('/dashboard/countryLooks.ts')) return source.slice(0, source.indexOf('export function useCountryLooks(')) + replacements['/dashboard/countryLooks.ts'];
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      } }] });
  try {
    const { default: TerritoryTransfersWidget } = await vite.ssrLoadModule('/ui/src/widgets/territory-transfers/TerritoryTransfersWidget.tsx');
    const track = { id: 'track', source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign' } },
      branch: { id: 'branch', firstHour: 100, lastHour: 200, gaps: [] as { firstHour: number; lastHour: number }[] }, axisOffset: 0 };
    const data = reply([capturedDeal('AB', 112, 136, [item(1, 112, '3', false, 'A', 'B')]), capturedDeal('BC', 120, 124, [item(2, 120, '3', false, 'B', 'C')])]);
    const wanted = [{ tag: 'A', hour: 112 }, { tag: 'B', hour: 120 }, { tag: 'B', hour: 112 }, { tag: 'C', hour: 120 }];
    for (const publicationGap of [false, true]) for (const conference of [null, 'AB', 'BC']) for (const range of [false, true]) {
      track.branch.gaps = publicationGap ? [{ firstHour: 136, lastHour: 136 }] : [];
      host.__transferFixture = { track, data };
      host.__transferStyleReads = [];
      renderToStaticMarkup(createElement(TerritoryTransfersWidget, { widget: { kind: 'territory-transfers', trackIds: ['track'],
        time: range ? { mode: 'range', firstOffset: 25, lastOffset: 100 } : { mode: 'campaign' },
        options: { scope: conference == null ? 'campaign' : 'conference', conference: conference == null ? '' : conferenceToken('campaign', 'branch', conference) } },
        runtime: { client: {}, tracks: [track], clock: { getHour: () => 200, getSnapshot: () => ({ hoursPerSecond: 1, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
      assert.deepEqual(host.__transferStyleReads[0], wanted, `${conference ?? 'campaign'}, range=${range}, publicationGap=${publicationGap}: hidden A retains its event-date input`);
    }
  } finally { await vite.close(); delete host.__transferFixture; delete host.__transferStyleReads; }
});

test('Live counts transfer occurrences at their after read and never shows a future state', () => {
  const data = decodeTransfers(reply([deal('new', 30, [item(7, 30, null)]), deal('old', 10, [item(7, 10, '8', false), item(9, 12, '4')])]));
  assert.equal(decodeTransfers(data.reply), data);
  assert.equal(transferCutoff(data.hours, 9), -Infinity);
  assert.equal(transferCutoff(data.hours, 29), 12);
  const beforeMissing = transfersThrough(data.campaign, 12);
  assert.equal(beforeMissing[0].stateCount, 2);
  assert.equal(beforeMissing[0].victoryPoints, 12n);
  assert.equal(transferAmount(beforeMissing, 'victoryPoints'), '12');
  const whole = transfersThrough(data.campaign, 30);
  assert.equal(whole[0].stateCount, 3, 'a state transferred twice counts twice');
  assert.equal(whole[0].victoryPoints, null);
  assert.equal(transferAmount(whole, 'victoryPoints'), 'Not recorded');
  assert.match(transferProvenance(whole[0].states), /1 exact transfer.*2 transfers derived/);
  assert.equal(whole[0].states[0].victoryPointsReadHour, 11);
});

test('VP mode withholds the entire unknown link before summed-other grouping', () => {
  const items = [item(1, 10, '12', false), item(2, 10, null, true, 'Loser', 'Winner'),
    item(3, 10, '-3', true, 'Other loser', 'Other winner'), item(4, 10, '0', true, 'Zero loser', 'Zero winner')];
  const data = decodeTransfers(reply([deal('conference', 10, items)]));
  const states = buildTransferDiagram(data.campaign, 'states', 2);
  assert.equal(states.total, 4);
  assert.equal(states.missingVictoryPoints, 0);
  const vps = buildTransferDiagram(data.campaign, 'victoryPoints', 2);
  assert.equal(vps.total, 0);
  assert.equal(vps.missingVictoryPoints, 1);
  assert.equal(vps.nonpositiveVictoryPoints, 2);
  assert.match(victoryPointNote(vps)!, /1 link lacks recorded victory points/);
  assert.match(victoryPointNote(vps)!, /2 links have zero or negative recorded victory points/);
  assert.equal(victoryPointNote(states), null);
});

test('Live withholds VPs until their supplying read and refreshes on that read without a new transfer', () => {
  const transfer = item(1, 10, '12', false);
  if (transfer.change.kind === 'stateTransfer') transfer.change.victoryPointsReadHour = 15;
  const record = reply([deal('conference', 10, [transfer])]);
  record.deals[0].transfers = graph(record.deals[0].items);
  record.transfers = graph(record.deals[0].items);
  const data = decodeTransfers(record);
  assert.deepEqual(data.victoryPointHours, [15]);
  const transferHour = transferCutoff(data.hours, 10);
  const beforeRead = transfersThrough(data.campaign, transferHour, transferCutoff(data.victoryPointHours, 14));
  assert.equal(beforeRead[0].stateCount, 1);
  assert.equal(beforeRead[0].victoryPoints, null);
  assert.equal(beforeRead[0].states[0].victoryPoints, null);
  assert.equal(beforeRead[0].states[0].victoryPointsReadHour, 15);
  assert.equal(beforeRead[0].states[0].victoryPointsPending, true);
  assert.equal(buildTransferDiagram(beforeRead, 'victoryPoints', 12).missingVictoryPoints, 1);
  assert.equal(transferCutoff(data.hours, 15), transferHour, 'no further state transferred');
  const atRead = transfersThrough(data.campaign, transferHour, transferCutoff(data.victoryPointHours, 15));
  assert.equal(atRead[0].victoryPoints, 12n);
  assert.equal(buildTransferDiagram(atRead, 'victoryPoints', 12).total, 12);
  assert.equal(data.campaign[0].states[0].victoryPoints, '12', 'Live never mutates the recorded graph');
  assert.equal(transfersThrough(data.campaign, transferHour)[0].victoryPoints, 12n, 'Live off uses the whole recorded read');
});

test('a VP value without a recorded supplying hour cannot become a dated Live width', () => {
  const transfer = item(1, 10, '12');
  if (transfer.change.kind === 'stateTransfer') transfer.change.victoryPointsReadHour = null;
  const data = decodeTransfers(reply([deal('conference', 10, [transfer])]));
  const undated = data.campaign[0];
  assert.equal(transfersThrough([undated], 10, 50)[0].victoryPoints, null);
  assert.equal(transfersThrough([undated], 10)[0].victoryPoints, 12n);
});

test('Live preserves native intermediate VP overflow and its graph order across chronological indexing', () => {
  const maximum = ((1n << 63n) - 1n).toString();
  const items = [item(1, 30, maximum), item(2, 20, '1'), item(3, 10, '-1')];
  const delayed = reply([deal('conference', 30, items)]);
  delayed.transfers.links[0].victoryPoints = null;
  delayed.deals[0].transfers.links[0].victoryPoints = null;
  assert.equal(validatePeaceDealsReply(delayed), delayed);
  assert.deepEqual(transfersThrough(decodeTransfers(delayed).campaign, 20, 100), [], 'the later supplying outcome cannot backfill hour 20');
  // Separate outcomes admit each occurrence at its own read while the campaign
  // graph retains the native fold order, independently of playback sorting.
  const record = reply(items.map(transfer => deal(`conference:${transfer.dates.afterHour}`, transfer.dates.afterHour, [transfer])));
  record.transfers.links[0].victoryPoints = null;
  assert.equal(validatePeaceDealsReply(record), record);
  const data = decodeTransfers(record);
  assert.deepEqual(data.campaign[0].states.map(state => state.state), [3, 2, 1], 'playback remains chronological');
  assert.deepEqual(data.campaign[0].nativeStates!.map(state => state.state), [1, 2, 3], 'native checked-fold order is retained');
  assert.equal(transfersThrough(data.campaign, 20, 100)[0].victoryPoints, 0n);
  const reached = transfersThrough(data.campaign, 30, 100);
  assert.equal(reached[0].victoryPoints, null, 'later negative terms cannot recover an overflowed graph sum');
  assert.equal(buildTransferDiagram(reached, 'victoryPoints', 100).missingVictoryPoints, 1);
  assert.equal(transfersThrough(data.campaign, 30)[0].victoryPoints, null);
});

test('Territory has no chosen country or screen-reader limit and fits solely to measured canvas capacity', () => {
  assert.equal(OPTIONS.some(option => option.key === 'perSide'), false);
  const data = decodeTransfers(world(80, 2));
  const whole = transfersThrough(data.campaign, Infinity);
  const unmeasured = buildTransferDiagram(whole, 'states', null);
  assert.equal(unmeasured.nodes.length, 0, 'wait for measured geometry without a provisional top-N or an unbounded first frame');
  const roomy = buildTransferDiagram(whole, 'states', 100);
  assert.equal(roomy.nodes.filter(node => node.side === 'source').length, 80, 'more than the former 40-country setting can fit');
  assert.equal(roomy.nodes.some(node => node.tag == null), false);
  const narrow = buildTransferDiagram(whole, 'states', 15);
  assert.equal(narrow.nodes.filter(node => node.side === 'source').length, 15);
  assert.equal(narrow.links.reduce((sum, link) => sum + link.parts.reduce((total, part) => total + part.stateCount, 0), 0), 80);
  assert.equal(narrow.total, 80, 'all overflow remains in other nodes');
  const description = transferDiagramDescription(roomy, 'states', node => transferNodeCountries(whole).get(node.tag!)!.tag);
  assert.ok(description.includes('Country 79 to Country 0'), 'every ribbon is described beyond the former 24-ribbon limit');
  assert.equal(description.split('; ').length, 80);
});

test('conference scope remains distinct and an unavailable stored choice is explicit', () => {
  const first = deal('new', 20, [item(1, 20, '10')]);
  const second = deal('old', 10, [item(2, 10, '5')]);
  const data = decodeTransfers(reply([first, second]));
  assert.equal(data.conferences.get('old')![0].victoryPoints, 5n);
  assert.deepEqual(visibleConferences(data.reply.deals, 10), [second]);
  assert.equal(chosenConference([first, second], ''), first);
  assert.equal(chosenConference([first, second], 'old'), second);
  assert.equal(chosenConference([first, second], 'missing'), null);
});

test('conference settings cannot select an unrelated reused handle in another campaign or branch', () => {
  const token = conferenceToken('campaign-key', 'branch with spaces', 'conference:with arbitrary text');
  assert.equal(conferenceChoiceId(token, 'campaign-key', 'branch with spaces'), 'conference:with arbitrary text');
  assert.equal(conferenceChoiceId(token, 'other-campaign', 'branch with spaces'), null);
  assert.equal(conferenceChoiceId(token, 'campaign-key', 'other branch'), null);
  assert.equal(conferenceChoiceId('', 'campaign-key', 'branch'), '');
  assert.equal(conferenceChoiceId('invalid', 'campaign-key', 'branch'), null);
});

test('stable country order follows the data service byte order for opaque Unicode tags', () => {
  const data = decodeTransfers(reply([deal('conference', 10, [item(1, 10, '1', true, '😀', '\ue000')])]));
  assert.deepEqual(transferColourPlan(data, { firstHour: 0, lastHour: 20, gaps: [] }, 20, true).wanted.map(sample => sample.tag), ['\ue000', '😀']);
});

test('an occurrence whose provenance was not recorded cannot become exact by default', () => {
  const record = reply([deal('conference', 10, [item(1, 10, '10')])]);
  record.deals[0].items = [];
  const data = decodeTransfers(record);
  assert.match(transferProvenance(data.campaign[0].states), /provenance not recorded/);
});

test('missing settlement, conference, ownership and war input spans remain gaps without duplicate counting', () => {
  const record = reply([]);
  record.unobserved.conferences = [{ firstHour: 10, lastHour: 20 }];
  record.unobserved.ownership = [{ firstHour: 15, lastHour: 25 }];
  record.unobserved.wars = [{ firstHour: 40, lastHour: 45 }];
  record.unobserved.settlements = [{ firstHour: 60, lastHour: 70 }];
  assert.equal(transferGapCount(record, 0, 9), 0);
  assert.equal(transferGapCount(record, 0, 35), 1);
  assert.equal(transferGapCount(record, 0, 45), 2);
  assert.equal(transferGapCount(record, 0, 70), 3);
});

test('one completed conference excludes later campaign gaps while active and campaign scopes retain them', () => {
  const completed = deal('conference', 20, [item(1, 20, '10')]);
  completed.firstObservedHour = 10;
  const record = reply([completed]);
  record.unobserved.conferences = [{ firstHour: 15, lastHour: 16 }, { firstHour: 40, lastHour: 45 }];
  const links = decodeTransfers(record).conferences.get(completed.id)!;
  const completedWindow = transferGapWindow(completed, links, 0, 50);
  assert.deepEqual(completedWindow, [10, 20]);
  assert.equal(transferGapCount(record, ...completedWindow), 1, 'the later gap is unrelated to this conference');
  const campaignWindow = transferGapWindow(null, links, 0, 50);
  assert.deepEqual(campaignWindow, [0, 50]);
  assert.equal(transferGapCount(record, ...campaignWindow), 2);
  const active = { ...completed, endHour: null, firstFinalizedHour: null, items: [], transfers: graph([]) };
  assert.deepEqual(transferGapWindow(active, [], 0, 50), [10, 50]);
  assert.equal(transferGapCount(record, ...transferGapWindow(active, [], 0, 50)), 2);
  assert.deepEqual(transferGapWindow(completed, links, 0, 16), [10, 16]);
});

function world(countries: number, conferences: number): PeaceDealsReply {
  const tags = Array.from({ length: countries }, (_, index) => `Country ${index}`);
  const items = tags.map((from, index) => item(index, (index % conferences) + 1, String(index % 11), index % 2 === 0, from, tags[(index + 1) % countries]));
  const buckets: PeaceItem[][] = Array.from({ length: conferences }, () => []);
  for (const item of items) buckets[item.dates.afterHour - 1].push(item);
  return reply(buckets.map((items, index) => deal(`Conference ${index}`, index + 1, items)).reverse());
}

test('5,000 countries and 2,000 conferences retain every transfer through summed nodes', () => {
  const data = decodeTransfers(world(5_000, 2_000));
  assert.equal(data.conferences.size, 2_000);
  assert.equal(data.campaign.length, 5_000);
  const diagram = buildTransferDiagram(transfersThrough(data.campaign, 2_000), 'states', 12);
  assert.equal(diagram.total, 5_000);
  assert.ok(diagram.nodes.length <= 26);
  assert.equal(diagram.links.reduce((sum, link) => sum + link.parts.reduce((parts, part) => parts + part.stateCount, 0), 0), 5_000);
});


test('transfer endpoints join a rename and keep tag and slot replacements separate in every scope', () => {
  const make = (state: number, hour: number, from: ReturnType<typeof country>) => {
    const transfer = item(state, hour, '1', false, from.tag, 'Recipient');
    if (transfer.change.kind === 'stateTransfer') transfer.change.fromCountry = from;
    return transfer;
  };
  const transfers = [make(1, 10, country('X', 10, 80, 900)), make(2, 20, country('Y', 20, 80, 900)),
    make(3, 30, country('X', 30, 81, 901)), make(4, 40, country('Y', 40, 80, 902))];
  const data = decodeTransfers(reply(transfers.map((transfer, index) => deal(`identity ${index}`, transfer.dates.afterHour, [transfer]))));
  assert.equal(data.campaign.length, 3);
  const original = data.campaign.find(link => link.source === countryIdentityKey(country('X', 10, 80, 900)))!;
  assert.equal(original.stateCount, 2);
  assert.deepEqual(original.states.map(state => state.fromCountry.tag), ['X', 'Y']);
  const pin = transfersThrough(data.campaign, 10);
  const range = transfersThrough(data.campaign, 20, Infinity, 20);
  assert.equal(pin[0].id, original.id);
  assert.equal(range[0].id, original.id);
  assert.deepEqual(transferLookPlan(transferNodeCountries(pin), { firstHour: 0, lastHour: 50, gaps: [] }).wanted,
    [{ tag: 'X', hour: 10 }, { tag: 'Recipient', hour: 10 }]);
  assert.equal(data.conferences.get('identity 1')![0].id, original.id);
  const replacements = data.campaign.filter(link => link !== original);
  assert.ok(replacements.every(link => link.source !== original.source && link.id !== original.id));
  const saved = transferSelectionToken('campaign', 'branch', [original.id], 10);
  const restored = transferSelection(saved, 'campaign', 'branch')!;
  assert.deepEqual(transferSelected(data.campaign, restored.ids), [original]);
  assert.equal(transferSelected(replacements, restored.ids).length, 0);
  assert.equal(validatePeaceDealsReply(data.reply), data.reply);
  const invalid = structuredClone(data.reply);
  invalid.transfers.links[0].states[0].fromCountry.countryId++;
  assert.throws(() => validatePeaceDealsReply(invalid), /malformed/);
});
