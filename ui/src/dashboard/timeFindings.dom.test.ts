import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';
import type { WidgetTime } from './model';

const capture = (name: string) => (code: string) => {
  if (!code.includes(`export default function ${name}(`)) throw new Error(`${name} is not declared as expected`);
  return `${code.replace(`export default function ${name}(`, `function real_${name}(`)}
export default function ${name}(props) { return globalThis.__finding.capture('${name}')(props); }`;
};
const source = (name: string) => (code: string) => {
  const declared = new RegExp(`export function ${name}\\(`);
  if (!declared.test(code)) throw new Error(`${name} is not declared as expected`);
  return `${code.replace(declared, `function real_${name}(`)}\nexport function ${name}(...args) { return globalThis.__finding.${name}(...args) ?? real_${name}(...args); }`;
};
const tracked = (name: string) => (code: string) => {
  const declared = new RegExp(`export function ${name}\\(`);
  if (!declared.test(code)) throw new Error(`${name} is not declared as expected`);
  return `${code.replace(declared, `function real_${name}(`)}\nexport function ${name}(...args) { const held = real_${name}(...args); globalThis.__finding.importing(held); return held; }`;
};
const TRANSFORMS: Record<string, (code: string) => string> = {
  '/charts/TimeChart.tsx': capture('TimeChart'),
  '/charts/timeline/Timeline.tsx': capture('Timeline'),
  '/charts/race/CountryRaceView.tsx': capture('CountryRaceView'),
  '/components/CountryPicker.tsx': capture('CountryPicker'),
  '/charts/flow/FlowDiagram.tsx': capture('FlowDiagram'),
  '/widgets/trade-flows/useTradeFlows.ts': source('useTradeFlows'),
  '/dashboard/raids.ts': source('useRaidLog'),
  '/dashboard/widgetPerf.ts': source('shownSink'),
  '/widgets/rendererLoaders.ts': tracked('loadWidgetRenderer'),
};
const stubs = { name: 'time-findings', enforce: 'pre' as const, transform(code: string, id: string) {
  const path = id.replaceAll('\\', '/').split('?')[0];
  return Object.entries(TRANSFORMS).find(([suffix]) => path.endsWith(suffix))?.[1](code);
} };

interface Ask { product: string; hour: number | null; hours: number[]; params: Record<string, unknown> | null }
interface Read { text: string; drillOpen: boolean; drill: string; day: string | null; asks: Ask[]; options: [string, unknown][]; pins: unknown[];
  follows: string[]; followLog: string[]; buttons: string[]; errors: string[]; shown: (number | null)[] }
const onPage = (run: (act: <T>(expression: string) => Promise<T>) => Promise<void>) => withNativePage('time-findings', '/ui/src/dashboard/timeFindings.dom.fixture.tsx',
  page => page.stage('time findings', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.findings.${expression}`);
    await run(act);
  }, 240_000), [stubs], true);
const scene = (value: unknown) => `mount(${JSON.stringify(value)})`;
const followedQuery = (ask: Ask, product: string): Record<string, unknown> | null => {
  const query = ask.product === 'follow' ? ask.params?.query as Record<string, unknown> | undefined : undefined;
  if (!query) return null;
  if (query.panel === product) { const { panel: _panel, ...params } = query; return params; }
  return query.product === product ? query.params as Record<string, unknown> ?? {} : null;
};
const follows = (asks: Ask[], product: string) => asks.flatMap(ask => { const params = followedQuery(ask, product); return params && !ask.hours.length ? [params] : []; });
const reads = (asks: Ask[], product: string) => [...asks.filter(ask => ask.product === product).map(ask => ask.params ?? {}), ...follows(asks, product)];
const pinAt = (hour: number) => ({ mode: 'pin', windows: { main: { firstOffset: hour - 100, lastOffset: hour - 100 } } });
const NO_PIN = { mode: 'pin', windows: {} };
const groupsOf = (tags: string[], members: { tag: string; firstHour: number; lastHour: number }[] = []) => ({ branchId: '1', warnings: [], warStarts: [], capitulations: [],
  identityChanges: [], factions: [], wars: [], humans: members, unobserved: { humans: [], factions: [], wars: [] },
  countryIdentities: tags.map((tag, index) => ({ tag, identities: [{ slot: index + 1, countryId: index + 101, firstHour: 100, lastHour: 109 }] })) });
const GER = { kind: 'tags', tags: ['GER'] }, BOTH = { kind: 'tags', tags: ['GER', 'ENG'] };
const OUTSIDE = 'This time is outside the recording.', UNRECORDED = 'Not recorded at this time.', SETTINGS = 'Change time';
const QUEUE = '({ readHour: hour, missing: null, orders: [], warnings: [] })';
const REASON = 'Branch 1 block 105 – 106 is unreadable: checksum';

test('1. Training queue pinned inside a declared gap 105–106 saves no country, asks nothing at 105, and says the time was not recorded with the recording’s reason', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'training-queue', time: pinAt(105), hour: 107, countries: GER, groups: groupsOf(['GER']), replies: { 'training-queue': QUEUE },
    gaps: [{ firstHour: 105, lastHour: 106, reason: REASON }] }));
  assert.deepEqual(shown.options.filter(([key]) => key === 'country'), [], `no country is chosen or saved inside the gap: ${JSON.stringify(shown.options)}`);
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'training-queue'), [], `no queue is asked in the gap: ${JSON.stringify(shown.asks)}`);
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'country-groups'), [], 'no country groups are read for an unrecorded time');
  assert.ok(shown.text.includes(UNRECORDED) && shown.text.includes(REASON) && shown.text.includes(SETTINGS), shown.text);
  const outside = await act<Read>(scene({ kind: 'training-queue', time: pinAt(115), hour: 107, countries: GER, groups: groupsOf(['GER']), replies: { 'training-queue': QUEUE } }));
  assert.deepEqual(outside.asks.filter(ask => ask.product === 'country-groups' || ask.product === 'training-queue'), [], `outside asks nothing: ${JSON.stringify(outside.asks)}`);
  const inside = await act<Read>(scene({ kind: 'training-queue', time: pinAt(104), hour: 107, countries: GER, groups: groupsOf(['GER']), replies: { 'training-queue': QUEUE },
    gaps: [{ firstHour: 105, lastHour: 106, reason: REASON }] }));
  assert.ok(inside.asks.some(ask => ask.product === 'training-queue' && ask.hour === 104), `the hour before the gap is read: ${JSON.stringify(inside.asks)}`);
}));

test('2. A widget pinned with no window for its track asks no dated read at any hour and shows only “choose another time”: Occupation trend’s mix and General career', () => onPage(async act => {
  const mix = await act<Read>(scene({ kind: 'occupation-trend', time: NO_PIN, hour: 105, countries: GER, options: { view: 'mix' }, groups: groupsOf(['GER']) }));
  assert.deepEqual(mix.asks.filter(ask => ask.product === 'occupation-at'), [], `no occupiers are read: ${JSON.stringify(mix.asks)}`);
  assert.ok(mix.text.includes(SETTINGS), mix.text);
  const career = await act<Read>(scene({ kind: 'general-career', time: NO_PIN, hour: 105, options: { leader: await act<string>('subjectToken("1")') } }));
  assert.deepEqual(career.asks.filter(ask => ask.hour !== null), [], `no dated career read: ${JSON.stringify(career.asks)}`);
}));

test('3. Navy size pinned at 105: picking 102 with GER and ENG under the pointer and choosing GER opens GER’s breakdown at 102', () => onPage(async act => {
  await act(scene({ kind: 'navy-size', time: pinAt(105), hour: 105, countries: BOTH, groups: groupsOf(['GER', 'ENG']) }));
  const chooser = await act<Read & { called: boolean }>('call("CountryRaceView", "onPick", 102, undefined, ["GER", "ENG"])');
  assert.ok(chooser.called && chooser.drill.includes('Choose a country for'), `the chooser opens: ${chooser.drill}`);
  const chosen = await act<Read & { called: boolean }>('call("CountryPicker", "onChange", "GER")');
  assert.ok(chosen.called && !chosen.drill.includes('Choose a country for'), `choosing commits: ${chosen.drill}`);
  assert.equal(chosen.day, await act<string>('day(102)'), 'the breakdown is at the picked hour, not the pin');
}));

test('4. Fuel’s saved consumption breakdown pinned at 105, the page at 110: Follow playback follows, and seeking back to 105 reads 105 instead of staying outside', () => onPage(async act => {
  const fuelCountry = await act<string>('identityToken(0)');
  const outside = await act<Read>(scene({ kind: 'fuel', time: pinAt(105), hour: 110, countries: GER, options: { show: 'consumption', fuelCountry }, groups: groupsOf(['GER']) }));
  assert.ok(!outside.text.includes(OUTSIDE), `the pin itself is recorded: ${outside.text}`);
  const followed = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(followed.pressed && followed.text.includes(OUTSIDE), `following the page at 110 is outside: ${followed.text}`);
  const back = await act<Read>('seek(105)');
  assert.ok(!back.text.includes(OUTSIDE) && !back.buttons.includes('Follow playback'), `back inside, still following: ${back.text} ${JSON.stringify(back.buttons)}`);
  assert.ok(follows(back.asks, 'country-fuel').some(params => params.country === 'GER'), `it follows playback for its country, naming no hour: ${JSON.stringify(back.asks)}`);
}));

test('5. Stability’s saved breakdown following playback, mounted at 95 before its 100–109 track: it asks nothing there, binds GER when the clock enters the track, and says loading while its parts are pending', () => onPage(async act => {
  const before = await act<Read>(scene({ kind: 'stability', time: { mode: 'follow' }, hour: 95, countries: GER, options: { show: 'stability' }, groups: groupsOf(['GER']) }));
  assert.ok(before.text.includes(OUTSIDE), `at 95 the breakdown is outside the recording: ${before.text}`);
  assert.deepEqual(before.asks.filter(ask => ask.product === 'series-parts' || ask.hour !== null && (ask.hour < 100 || ask.hour > 109)), [],
    `nothing is asked for an hour before the track: ${JSON.stringify(before.asks)}`);
  const inside = await act<Read>('seek(105)');
  assert.ok(!inside.text.includes('Selected country unavailable') && !inside.text.includes(OUTSIDE), `GER is bound at 105: ${inside.text}`);
  assert.ok(inside.asks.some(ask => ask.product === 'series-parts'), `the breakdown asks GER’s parts: ${JSON.stringify(inside.asks)}`);
  assert.ok(inside.text.includes('Loading the breakdown') && !inside.text.includes('No breakdown was recorded'), `a pending reply is loading: ${inside.text}`);
  const failed = await act<Read>(scene({ kind: 'stability', time: { mode: 'follow' }, hour: 105, countries: GER, options: { show: 'stability' }, groups: groupsOf(['GER']),
    failures: ['series-parts'] }));
  assert.ok(failed.text.includes('couldn’t be read') && !failed.text.includes('No breakdown was recorded'), `a failed reply is a failure: ${failed.text}`);
}));

test('7. Before a 100–109 track at 95, coverage wins over loading, failures and false zeroes, and nothing unrecorded is asked', () => onPage(async act => {
  const raids = { episodes: [], missing: [], available: true, unavailable: null, warnings: [], types: [], artwork: [] };
  for (const [label, value] of [['pending', undefined], ['failed', 'failed'], ['answered', raids]] as const) {
    const shown = await act<Read>(scene({ kind: 'raid-tally', time: { mode: 'follow' }, hour: 95, options: { view: 'race' }, groups: groupsOf(['GER']), raids: value }));
    assert.ok(shown.text.includes(OUTSIDE) && !/Couldn’t load|Loading/.test(shown.text), `${label}: ${shown.text}`);
  }
  const volunteers = await act<Read>(scene({ kind: 'volunteers', time: { mode: 'follow' }, hour: 95, groups: groupsOf(['GER']) }));
  assert.ok(volunteers.text.includes(OUTSIDE) && !/0 links/.test(volunteers.text), volunteers.text);
  assert.deepEqual(volunteers.asks.filter(ask => ask.product === 'volunteers-expeditionaries'), []);
  const queue = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 95, countries: GER, groups: groupsOf(['GER']) }));
  assert.deepEqual(queue.asks.filter(ask => ask.product === 'country-groups' || ask.product === 'training-queue'), [], `outside asks nothing: ${JSON.stringify(queue.asks)}`);
  await act(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 105, countries: GER, groups: groupsOf(['GER']) }));
  const hovered = await act<Read>('call("TimeChart", "onHover", 112)');
  assert.deepEqual(hovered.asks.filter(ask => ask.product === 'country-fuel' && ask.hour === 112), [], `no hover read past the track: ${JSON.stringify(hovered.asks)}`);
}));

test('8. Equipment produced: a country breakdown opened at 105 shows its own read, never the parent’s pending state, and stays mounted as the page moves', () => onPage(async act => {
  await act(scene({ kind: 'equipment-produced', time: { mode: 'follow' }, hour: 105, countries: GER, groups: groupsOf(['GER']) }));
  const opened = await act<Read & { called: boolean }>('call("CountryRaceView", "onPick", 105, "GER")');
  assert.ok(opened.called && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  assert.ok(opened.asks.some(ask => ask.product === 'equipment-produced-country'), `the breakdown asks its own read: ${JSON.stringify(opened.asks)}`);
  assert.ok(!/Loading equipment/.test(opened.drill), `the parent’s pending read is not the breakdown’s state: ${opened.drill}`);
  const moved = await act<Read>('seek(110)');
  assert.ok(moved.drillOpen, 'the breakdown stays mounted when the page leaves the track');
}));

const COLUMNS = `({ branchId: '1', warnings: [], defaultColumns: ['power'], defaultSort: 'power', columns: [{ key: 'power', label: 'Power', unit: '', decimals: 0,
  available: true, reason: null, family: { kind: 'country-measure', measure: 'politicalPower' }, partial: null, noneAtZero: false }] })`;
const RANKED = `({ branchId: '1', hour, missing: null, warnings: [], rows: [{ tag: 'GER', country: 'GER', cells: [{ valueRaw: '5', missing: null, partial: false }] },
  { tag: 'ENG', country: 'ENG', cells: [{ valueRaw: '3', missing: null, partial: false }] }] })`;
const GAP = [{ firstHour: 105, lastHour: 106, reason: REASON }];
const TOP = { kind: 'group', group: { type: 'top', count: 3 }, membership: 'hour', aggregate: false };
const dated = (asks: Ask[], hour: number) => asks.filter(ask => ask.hours.includes(hour));

test('11. Trade flows restored on its contributors breakdown opens that breakdown, not the main view', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'trade-flows', time: { mode: 'follow' }, hour: 105, options: { show: 'contributions' }, groups: groupsOf(['GER']) }));
  assert.ok(shown.drillOpen, `the saved breakdown is the view: ${shown.text}`);
}));

test('12. A missing pin reads no branch-end facts: Factories’ saved breakdown and the Scoreboard ask nothing at 109 and say to choose another time', () => onPage(async act => {
  const day = await act<string>('day(109)');
  const parts = await act<Read>(scene({ kind: 'factories', time: NO_PIN, hour: 105, countries: GER, options: { show: 'breakdown' }, groups: groupsOf(['GER']) }));
  assert.deepEqual(parts.asks.filter(ask => ask.hours.length), [], `no dated read: ${JSON.stringify(parts.asks)}`);
  assert.ok(parts.text.includes(SETTINGS) && !parts.text.includes(day), parts.text);
  const board = await act<Read>(scene({ kind: 'scoreboard', time: NO_PIN, hour: 105, countries: BOTH, groups: groupsOf(['GER', 'ENG']),
    replies: { 'scoreboard-columns': COLUMNS, scoreboard: RANKED } }));
  assert.deepEqual(board.asks.filter(ask => ask.product === 'scoreboard'), [], `the Scoreboard asks no hour: ${JSON.stringify(board.asks)}`);
  const gapped = await act<Read>(scene({ kind: 'scoreboard', time: pinAt(105), hour: 107, countries: BOTH, groups: groupsOf(['GER', 'ENG']), gaps: GAP,
    replies: { 'scoreboard-columns': COLUMNS, scoreboard: RANKED } }));
  assert.deepEqual(dated(gapped.asks, 105), [], `a pin in a gap sends its literal hour nowhere: ${JSON.stringify(gapped.asks)}`);
}));

const RESOURCES = `({ tag: 'GER', decimals: 3, resources: [{ id: 1, token: 'oil', ordinal: 1, name: 'Oil', icon: null }], pools: [], shown: [], poolsMissing: null,
  dealsMissing: null, deals: [], warnings: [] })`;

test('13. Hover, expansion, ranking and follow obey coverage: nothing is sent into a 105–106 gap and a paused seek past the track releases a follow', () => onPage(async act => {
  await act(scene({ kind: 'resource-balance', time: { mode: 'follow' }, hour: 107, countries: GER, options: { view: 'time', resources: 'oil' }, gaps: GAP,
    replies: { 'resource-balance': RESOURCES } }));
  const extent = (await act<{ extent: [number, number] } | null>('props("TimeChart")'))?.extent;
  assert.ok(extent && extent[0] <= 105 && 105 <= extent[1], `105 lies on the chart: ${JSON.stringify(extent)}`);
  const hovered = await act<Read & { called: boolean }>('call("TimeChart", "onHover", 105)');
  assert.ok(hovered.called, 'the chart takes the hover');
  assert.deepEqual(dated(hovered.asks, 105), [], `no hover read inside the gap: ${JSON.stringify(hovered.asks)}`);
  await act(scene({ kind: 'research', time: pinAt(105), hour: 107, countries: GER, gaps: GAP, groups: groupsOf(['GER']) }));
  const expanded = await act<Read & { expanded: boolean }>('expand(".doctrine-panel")');
  assert.ok(expanded.expanded, 'the doctrines expand');
  assert.deepEqual(expanded.asks.filter(ask => ask.product === 'doctrines'), [], `no doctrines read inside the gap: ${JSON.stringify(expanded.asks)}`);
  const ranked = await act<Read>(scene({ kind: 'equipment-produced', time: pinAt(105), hour: 107, countries: TOP, options: { count: 'ic' }, gaps: GAP, groups: groupsOf(['GER']) }));
  assert.deepEqual(dated(ranked.asks, 105), [], `no ranking at 105: ${JSON.stringify(ranked.asks)}`);
  const followed = await act<Read>(scene({ kind: 'casualties', time: { mode: 'follow' }, hour: 105, countries: TOP, groups: groupsOf(['GER']), accept: true }));
  const registered = followed.followLog.filter(entry => entry.startsWith('follow '));
  assert.ok(followed.follows.length > 0 && registered.length > 0, `inside the track the ranking registers a follow at the transport: ${JSON.stringify(followed.followLog)}`);
  assert.deepEqual(followed.asks.filter(ask => ask.product === 'follow' && ask.hours.length), [], `a follow registration names no hour: ${JSON.stringify(followed.asks)}`);
  const left = await act<Read>('seek(115)');
  assert.deepEqual(left.follows, [], `paused past the track, no follow stays registered: ${JSON.stringify(left.follows)}`);
  assert.deepEqual(left.followLog.filter(entry => entry.startsWith('unfollow ')).sort(), registered.map(entry => `un${entry}`).sort(), 'each registration is released');
  const back = await act<Read>('seek(105)');
  assert.ok(back.follows.length > 0 && back.followLog.some(entry => entry.startsWith('follow ') && !registered.includes(entry)), `back inside, a new follow registers: ${JSON.stringify(back.followLog)}`);
}));

const AID = `({ links: [{ key: 'l1', sender: 'GER', senderSlot: 1, senderCountryId: 101, host: 'ENG', hostSlot: 2, hostCountryId: 102,
  runs: [{ firstHour: 100, lastHour: 109, divisions: 3, startsSegment: true }] }], tallies: [], unobserved: [], warnings: [] })`;

test('14. Volunteers: a country’s partners opened at 105 stay mounted on their own date when the following parent leaves the track', () => onPage(async act => {
  await act(scene({ kind: 'volunteers', time: { mode: 'follow' }, hour: 105, groups: groupsOf(['GER', 'ENG']), replies: { 'volunteers-expeditionaries': AID } }));
  const nodes = (await act<{ nodes: { id: string }[] } | null>('props("FlowDiagram")'))?.nodes ?? [];
  assert.ok(nodes.length > 0, 'the diagram draws its countries');
  const opened = await act<Read & { called: boolean }>(`call("FlowDiagram", "onLabelPick", ${JSON.stringify(nodes[0])})`);
  assert.ok(opened.called && opened.drillOpen, `the partners open: ${opened.text}`);
  const day = await act<string>('day(105)');
  const left = await act<Read>('seek(115)');
  assert.ok(left.drillOpen && left.day === day, `the drill stays mounted at 105: ${left.day} ${left.text}`);
}));

test('15. Follow keeps the drill’s own time: a saved stockpile told to follow keeps following after a seek', () => onPage(async act => {
  const stockpileCountry = await act<string>('identityToken(5)');
  await act(scene({ kind: 'equipment-stockpile', time: pinAt(105), hour: 107, countries: GER, options: { show: 'stockpile', stockpileCountry }, groups: groupsOf(['GER']) }));
  const followed = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(followed.pressed, 'the saved breakdown offers Follow playback');
  const moved = await act<Read>('seek(108)');
  assert.ok(!moved.buttons.includes('Follow playback'), `at 108 it still follows: ${JSON.stringify(moved.buttons)}`);
}));

test('16. Equipment produced’s saved breakdown mounted at 95 waits outside, then reads its country inside the track, and the parent’s state is never its state', () => onPage(async act => {
  const before = await act<Read>(scene({ kind: 'equipment-produced', time: { mode: 'follow' }, hour: 95, countries: GER, options: { show: 'breakdown' }, groups: groupsOf(['GER']),
    failures: ['equipment-produced'] }));
  assert.ok(before.text.includes(OUTSIDE), `at 95 the breakdown is outside the recording: ${before.text}`);
  const inside = await act<Read>('seek(105)');
  assert.ok([...before.asks, ...inside.asks].some(ask => ask.product === 'equipment-produced-country'), `the breakdown reads its country: ${JSON.stringify(inside.asks)}`);
  const label = await act<string | null>('stateLabel("CountryRaceView")');
  assert.equal(label, 'Loading equipment types…', 'the breakdown shows its own pending read, not the parent’s state');
}));

const OFFER = { country: 5000, tag: 'GER', inventory: [], prices: [], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false };
const MARKET = `({ branchId: "1", offers: [${JSON.stringify(OFFER)}], contracts: [], agreements: [], purchasesMissing: null, unattributed: 0, variants: [], icons: [], warnings: [] })`;

test('17. Trade market: the moment follows pin, campaign and range; a clicked offer row pins at its own hour and restores on its own breakdown, which can follow playback', () => onPage(async act => {
  const replies = { 'market-lendlease': MARKET };
  const pinned = await act<Read>(scene({ kind: 'trade-flows', time: pinAt(105), hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  assert.ok(pinned.asks.some(ask => ask.product === 'market-lendlease' && ask.hours.includes(105)), `pinned reads 105: ${JSON.stringify(pinned.asks)}`);
  const campaign = await act<Read>(scene({ kind: 'trade-flows', time: { mode: 'campaign' }, hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  assert.ok(campaign.asks.some(ask => ask.product === 'market-lendlease' && ask.hours.includes(109)), `the whole campaign reads its end: ${JSON.stringify(campaign.asks)}`);
  const range = await act<Read>(scene({ kind: 'trade-flows', time: { mode: 'range', firstOffset: 0, lastOffset: 3 }, hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  assert.ok(range.asks.some(ask => ask.product === 'market-lendlease' && ask.hours.includes(103)), `a range reads its last hour: ${JSON.stringify(range.asks)}`);
  await act(scene({ kind: 'trade-flows', time: pinAt(105), hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  const opened = await act<Read & { clicked: boolean }>('click("details > summary")');
  assert.ok(opened.clicked && opened.drillOpen, `the offer row opens its contributions: ${opened.text}`);
  const pin = await act<Read & { pressed: boolean }>('press("Pin as widget")');
  const saved = pin.pins[0] as { time: unknown; options: Record<string, unknown> } | undefined;
  assert.ok(pin.pressed && saved, `the offer pins: ${JSON.stringify(pin.buttons)}`);
  assert.deepEqual(saved.time, pinAt(105), 'the offer pins at the row’s own hour');
  assert.equal(saved.options.view, 'market'); assert.equal(saved.options.show, 'contributions');
  const restored = await act<Read>(scene({ kind: 'trade-flows', time: saved.time, hour: 107, options: saved.options, countries: { kind: 'all' }, replies }));
  assert.ok(restored.drillOpen && await act<boolean>('has(".drill-saved details[open] > summary")'), `the offer restores open on its breakdown: ${restored.text}`);
  await act('press("Follow playback")');
  const moved = await act<Read>('seek(108)');
  assert.equal(moved.day, await act<string>('day(108)'), `the restored offer follows playback: ${moved.day} ${moved.drill}`);
}));

test('18. A country chosen by slot whose identities fail to load says so and offers a retry, instead of loading forever', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 105, countries: { kind: 'tags', tags: ['\u00001'] }, groups: groupsOf(['GER']),
    failures: ['country-identities'], replies: { 'training-queue': QUEUE } }));
  assert.ok(/Couldn’t load/.test(shown.text) && !/Loading/.test(shown.text) && shown.buttons.includes('Retry'), `${shown.text} ${JSON.stringify(shown.buttons)}`);
}));

const RENAMED = { ...groupsOf([]), countryIdentities: [
  { tag: 'GER', identities: [{ slot: 1, countryId: 101, firstHour: 100, lastHour: 104 }, { slot: 3, countryId: 103, firstHour: 105, lastHour: 109 }] },
  { tag: 'GDR', identities: [{ slot: 1, countryId: 101, firstHour: 105, lastHour: 109 }] }] };

test('19. Fuel: GER picked at 103, its drill followed to 108 where another country holds GER, pins the country it picked and restores reading its alias GDR', () => onPage(async act => {
  await act(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 103, countries: GER, groups: RENAMED }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 103)');
  assert.ok(opened.called && opened.drillOpen, `GER’s breakdown opens at 103: ${opened.text}`);
  await act('seek(108)');
  const pinned = await act<Read & { pressed: boolean }>('press("Pin as widget")');
  const saved = pinned.pins[0] as { time: WidgetTime; countries: unknown; options: Record<string, unknown> } | undefined;
  assert.ok(pinned.pressed && saved, `the breakdown pins: ${JSON.stringify(pinned.buttons)}`);
  const country = JSON.parse(await act<string>(`content(${JSON.stringify(saved.options.fuelCountry)})`)) as { tag: string; originOffset: number };
  assert.deepEqual([country.tag, country.originOffset, saved.countries], ['GER', 3, GER], `the pin holds the country picked at 103, not GER at 108: ${JSON.stringify(country)}`);
  const restored = await act<Read>(scene({ kind: 'fuel', time: saved.time, hour: 108, countries: saved.countries, options: saved.options, groups: RENAMED }));
  const read = [...restored.asks.filter(ask => ask.hours.includes(108)).map(ask => ask.params ?? {}), ...follows(restored.asks, 'country-fuel')]
    .flatMap(params => params.country === undefined ? [] : [params.country]);
  assert.ok(read.includes('GDR') && !read.includes('GER'), `the restored pin reads the picked country under its tag at 108: ${JSON.stringify(restored.asks)}`);
}));

const HUMANS = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };

test('20. Exact reads are checked for coverage and membership: Resource balance’s GER, a member from 107, sends no hover read at 103', () => onPage(async act => {
  await act(scene({ kind: 'resource-balance', time: { mode: 'follow' }, hour: 108, countries: HUMANS, options: { view: 'time', resources: 'oil' },
    groups: groupsOf(['GER'], [{ tag: 'GER', firstHour: 107, lastHour: 109 }]), replies: { 'resource-balance': RESOURCES } }));
  const hovered = await act<Read & { called: boolean }>('call("TimeChart", "onHover", 103)');
  assert.ok(hovered.called, 'the chart takes the hover');
  assert.deepEqual(hovered.asks.filter(ask => ask.product === 'resource-balance' && ask.hours.includes(103)), [], `no read before GER joins: ${JSON.stringify(hovered.asks)}`);
  const member = await act<Read>('call("TimeChart", "onHover", 107)');
  assert.ok(member.asks.some(ask => ask.product === 'resource-balance' && ask.hour === 107), `a member hour is read: ${JSON.stringify(member.asks)}`);
}));

const LINES = '({ branchId: "1", tag: "GER", decimals: 5, detailHours: 720, warnings: [], icons: [], organisations: [], lineHours: [], variants: [], lines: [] })';

test('21. Production lines on a 100–109 branch: a breakdown pinned at 105 asks its 720-hour bucket at the bucket’s first recorded hour', () => onPage(async act => {
  const line = await act<string>('lineToken("GER", "k1")');
  const shown = await act<Read>(scene({ kind: 'production-lines', time: pinAt(105), hour: 107, countries: GER, groups: groupsOf(['GER']),
    options: { show: 'efficiency', line, productionCountry: await act<string>('identityToken(0)') }, replies: { 'production-lines': LINES } }));
  const asked = shown.asks.filter(ask => ask.product === 'production-line-breakdown').map(ask => ask.hour);
  assert.deepEqual([...new Set(asked)], [100], `the breakdown is asked once for 105’s bucket: ${JSON.stringify(shown.asks)}`);
}));

const TENSION = '({ tensionScale: 100, contributionScale: 100000, ceilingRaw: "0", unnamed: false, warnings: [], missing: null, contributions: [], sources: [] })';

test('22. World tension following, paused at 107: a pick at 105 inside the 105–106 gap opens the breakdown at 105 and says why it was not recorded', () => onPage(async act => {
  await act(scene({ kind: 'world-tension', time: { mode: 'follow' }, hour: 107, gaps: GAP, replies: { 'world-tension': TENSION } }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 105)');
  assert.ok(opened.called && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  assert.ok(opened.drill.includes(UNRECORDED) && opened.drill.includes(REASON), `the pick keeps its gap and reason: ${opened.drill}`);
  assert.deepEqual(dated(opened.asks, 105), [], `nothing is asked at 105: ${JSON.stringify(opened.asks)}`);
}));

test('23. World tension’s breakdown is its own region: under a parent pinned at 105 it offers Follow playback, and a paused breakdown stays when the following parent’s next read fails', () => onPage(async act => {
  await act(scene({ kind: 'world-tension', time: pinAt(105), hour: 108, replies: { 'world-tension': TENSION } }));
  const pinned = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 105)');
  assert.ok(pinned.called && pinned.drillOpen && pinned.buttons.includes('Follow playback'), `the pinned parent’s breakdown can follow: ${JSON.stringify(pinned.buttons)}`);
  await act(scene({ kind: 'world-tension', time: { mode: 'follow' }, hour: 105,
    replies: { 'world-tension': `hour === 107 ? (() => { throw new Error('tension failed'); })() : ${TENSION}` } }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 105)');
  assert.ok(opened.called && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  const paused = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(paused.pressed && paused.drillOpen, 'the breakdown pauses at 105');
  const moved = await act<Read>('seek(107)');
  assert.ok(moved.drillOpen && moved.day === await act<string>('day(105)'), `the paused breakdown stays when the parent’s read at 107 fails: ${moved.text}`);
}));

test('24. Training queue saves a chosen country only once its holder is verified: nothing while country groups are pending, failed or split between holders', () => onPage(async act => {
  const base = { kind: 'training-queue', time: { mode: 'follow' }, hour: 105, countries: GER, replies: { 'training-queue': QUEUE } };
  const saves = (read: Read) => read.options.filter(([key]) => key === 'country');
  assert.deepEqual(saves(await act<Read>(scene(base))), [], 'nothing is saved while country groups are pending');
  assert.deepEqual(saves(await act<Read>(scene({ ...base, failures: ['country-groups'] }))), [], 'nothing is saved when country groups fail');
  const split = { ...groupsOf([]), countryIdentities: [{ tag: 'GER', identities: [{ slot: 1, countryId: 101, firstHour: 100, lastHour: 103 }, { slot: 3, countryId: 103, firstHour: 107, lastHour: 109 }] }] };
  const holders = await act<Read>(scene({ ...base, groups: split }));
  assert.deepEqual(saves(holders), [], `nothing is saved while the user must choose a holder: ${JSON.stringify(holders.options)}`);
  const held = saves(await act<Read>(scene({ ...base, groups: groupsOf(['GER']) })));
  assert.equal(held.length, 1, 'a verified holder is saved');
  assert.equal((JSON.parse(await act<string>(`content(${JSON.stringify(held[0][1])})`)) as { originOffset: number }).originOffset, 5);
}));

const FACTIONS = { ...groupsOf(['GER', 'ENG']), factions: [{ id: '1', name: 'Axis', nameReason: null, names: [], recordedName: 'Axis', firstHour: 100, lastHour: 109,
  members: [{ tag: 'GER', firstHour: 100, lastHour: 109 }, { tag: 'ENG', firstHour: 100, lastHour: 109 }], leaders: [], incomplete: [] }] };
const naming = (asks: Ask[], tag: string) => asks.filter(ask => JSON.stringify(ask.params ?? {}).includes(`"${tag}"`));

test('25. Faction totals following fixed rosters follow their members at playback: the follow names no hour, so it registers', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'territory', time: { mode: 'follow' }, hour: 105, countries: { kind: 'factions', count: 3, membership: 'end' },
    groups: FACTIONS, accept: true }));
  const members = shown.follows.filter(query => query.includes('"series-state"'));
  assert.ok(members.length > 0, `the members’ values are followed: ${JSON.stringify(shown.followLog)} ${shown.text}`);
  assert.ok(members.every(query => !/"hour"/.test(query)), `a followed read names no hour: ${JSON.stringify(members)}`);
}));

test('26. Time settings: pinning a week-long window 3 hours into the branch pins it clipped to the branch, and a pin that cannot be made says why', () => onPage(async act => {
  await act(`settings(${JSON.stringify({ kind: 'fuel', time: { mode: 'follow' }, hour: 103 })})`);
  const pinned = await act<Read & { pressed: boolean }>('press("Pinned")');
  assert.deepEqual(pinned.options, [['time', { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 3 } } }]], `the pin is made: ${pinned.text}`);
  await act(`settings(${JSON.stringify({ kind: 'fuel', time: { mode: 'follow' }, hour: 105, gaps: GAP })})`);
  const refused = await act<Read & { pressed: boolean }>('press("Pinned")');
  assert.ok(refused.pressed && refused.options.length === 0 && refused.text.includes('Can’t pin now'), `a refused pin is named, not silent: ${refused.text}`);
}));

test('27. Fuel on the default player selection (a slot key): a breakdown picked at 103 pins the country that slot held then, by its tag', () => onPage(async act => {
  await act(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 103, countries: { kind: 'selected' }, groups: groupsOf(['GER']) }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 103)');
  assert.ok(opened.called && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  const pinned = await act<Read & { pressed: boolean }>('press("Pin as widget")');
  const saved = pinned.pins[0] as { options: Record<string, unknown> } | undefined;
  assert.ok(pinned.pressed && saved, `a slot-key pick can be pinned: ${JSON.stringify(pinned.buttons)}`);
  const country = JSON.parse(await act<string>(`content(${JSON.stringify(saved.options.fuelCountry)})`)) as { tag: string; originOffset: number };
  assert.deepEqual([country.tag, country.originOffset], ['GER', 3]);
}));

test('28. Resource balance: GER’s breakdown picked at 103 keeps that country through continuous playback into GDR, reading its history by identity', () => onPage(async act => {
  await act(scene({ kind: 'resource-balance', time: { mode: 'follow' }, hour: 103, requestsWhilePlaying: true, countries: GER, groups: RENAMED,
    replies: { 'resource-balance': RESOURCES.replace('shown: []', 'shown: [1]') } }));
  const opened = await act<Read & { clicked: boolean }>('click(".resource-bars li")');
  assert.ok(opened.clicked && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  const viewed = await act<Read & { called: boolean }>('call("TimeChart", "onSummaryView", { from: 100, to: 109, end: 109, width: 400 })');
  const tiles = [...opened.asks, ...viewed.asks].filter(ask => ask.product === 'series-tiles');
  assert.ok(tiles.length > 0 && tiles.every(ask => JSON.stringify(ask.params?.lines).includes('"country":1') && JSON.stringify(ask.params?.within ?? null).includes('[1,')),
    `the history is read by the held identity’s slot, within its runs: ${JSON.stringify(tiles.map(ask => ask.params))}`);
  const played = await act<Read & { hour: number; playing: boolean }>('play(1, 5, 1000)');
  assert.ok(played.playing && played.hour >= 107, `playback runs to ${played.hour}`);
  assert.ok(naming(played.asks, 'GDR').length > 0, `past 105 the breakdown reads GDR without a seek: ${JSON.stringify(played.asks.map(ask => [ask.product, ask.params]))}`);
  const drilled = reads(played.asks, 'resource-balance').filter(params => !('order' in params));
  assert.ok(drilled.some(params => params.country === 'GDR'), `the breakdown’s own read names GDR: ${JSON.stringify(played.asks)}`);
  assert.deepEqual(played.asks.filter(ask => ask.product === 'resource-balance' && ask.params?.country === 'GER' && (ask.hour ?? 0) >= 105), [],
    'the breakdown never reads the other country now named GER');
  await act('pause()');
}));

test('29. Fuel: GER picked at 103 and followed by continuous playback into 108 reads GDR without a seek, and pins the country it picked', () => onPage(async act => {
  await act(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 103, requestsWhilePlaying: true, countries: GER, groups: RENAMED }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 103)');
  assert.ok(opened.called && opened.drillOpen, `GER’s breakdown opens at 103: ${opened.text}`);
  const played = await act<Read & { hour: number; playing: boolean }>('play(1, 5, 1000)');
  assert.ok(played.playing && played.hour >= 107, `playback runs to ${played.hour}`);
  assert.ok(naming(played.asks, 'GDR').length > 0, `the following drill reads GDR past 105: ${JSON.stringify(played.asks.map(ask => [ask.product, ask.params]))}`);
  const pinned = await act<Read & { pressed: boolean }>('press("Pin as widget")');
  const saved = pinned.pins[0] as { options: Record<string, unknown> } | undefined;
  assert.ok(pinned.pressed && saved, `the breakdown pins: ${JSON.stringify(pinned.buttons)}`);
  const country = JSON.parse(await act<string>(`content(${JSON.stringify(saved.options.fuelCountry)})`)) as { tag: string; originOffset: number };
  assert.deepEqual([country.tag, country.originOffset], ['GER', 3], 'the pin holds the country picked at 103');
  await act('pause()');
}));

test('30. World tension’s saved breakdown told to follow playback registers its own follow under its own root', () => onPage(async act => {
  await act(scene({ kind: 'world-tension', time: pinAt(105), hour: 105, options: { show: 'breakdown' }, replies: { 'world-tension': TENSION }, accept: true }));
  const followed = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(followed.pressed, 'the breakdown offers Follow playback');
  assert.ok(followed.followLog.some(entry => entry.startsWith('follow ')), `the drill’s read follows playback: ${JSON.stringify(followed.followLog)} ${followed.text}`);
}));

test('31. Production lines’ breakdown following playback across a 720-hour bucket asks each bucket once, at its first recorded hour', () => onPage(async act => {
  const line = await act<string>('lineToken("GER", "k1")');
  const long = { ...groupsOf([]), countryIdentities: [{ tag: 'GER', identities: [{ slot: 1, countryId: 101, firstHour: 100, lastHour: 1600 }] }] };
  const shown = await act<Read>(scene({ kind: 'production-lines', time: { mode: 'follow' }, hour: 715, last: 1600, requestsWhilePlaying: true, countries: GER, groups: long,
    options: { show: 'efficiency', line, productionCountry: await act<string>('identityToken(0)') }, replies: { 'production-lines': LINES } }));
  const played = await act<Read & { hour: number; playing: boolean }>('play(1, 10, 1000)');
  assert.ok(played.playing && played.hour >= 723, `playback crosses 720: ${played.hour}`);
  const asked = [...shown.asks, ...played.asks].filter(ask => ask.product === 'production-line-breakdown').map(ask => ask.hour);
  assert.deepEqual([...new Set(asked)].sort((a, b) => (a ?? 0) - (b ?? 0)), [100, 720], `one recorded hour per bucket, the next bucket asked on crossing: ${JSON.stringify(asked)}`);
  await act('pause()');
}));

test('32. A country chosen by slot whose groups fail but whose identities load re-resolves its tag at the identity change during continuous playback', () => onPage(async act => {
  await act(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 103, requestsWhilePlaying: true, countries: { kind: 'tags', tags: ['\u00001'] }, groups: RENAMED, failures: ['country-groups'],
    replies: { 'training-queue': QUEUE, 'country-identities': `({ branchId: '1', countries: ${JSON.stringify(RENAMED.countryIdentities)}, unknown: [], warnings: [] })` } }));
  const played = await act<Read & { hour: number; playing: boolean }>('play(1, 4, 1000)');
  assert.ok(played.playing && played.hour >= 106, `playback runs to ${played.hour}`);
  assert.ok(played.asks.some(ask => ask.product === 'training-queue' && ask.params?.country === 'GDR') || naming(played.asks, 'GDR').length > 0,
    `past 105 the slot is read as GDR: ${JSON.stringify(played.asks.map(ask => [ask.product, ask.params]))}`);
  await act('pause()');
}));

test('33. A saved range starting before its branch is clipped to the branch before it is asked', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'raid-tally', time: { mode: 'range', firstOffset: -20, lastOffset: 5 }, hour: 105, options: { view: 'race' }, groups: groupsOf(['GER']) }));
  const raids = shown.asks.filter(ask => ask.product === 'raid-log');
  assert.ok(raids.length > 0 && raids.every(ask => ask.params?.from === 100 && ask.params?.to === 105), `the range is clipped to 100–105: ${JSON.stringify(raids)}`);
}));

test('34. Fuel’s exact series reads name the country the native reader expects, and only at hours inside each country’s membership', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 108, countries: HUMANS, groups: groupsOf(['GER'], [{ tag: 'GER', firstHour: 107, lastHour: 109 }]) }));
  const named = reads(shown.asks, 'series-state').flatMap(params => (params.reads ?? []) as Record<string, unknown>[]);
  assert.ok(named.length > 0 && named.every(read => 'country' in read && !('tag' in read)), `series reads name their country: ${JSON.stringify(shown.asks)}`);
  const hovered = await act<Read & { called: boolean }>('call("TimeChart", "onHover", 103)');
  const early = hovered.asks.filter(ask => ask.product === 'series-state').flatMap(ask => (ask.params?.reads ?? []) as { hour?: number }[]).filter(read => read.hour === 103);
  assert.deepEqual(early, [], 'no exact read before GER joins the group');
  assert.deepEqual(hovered.asks.filter(ask => ask.product === 'country-fuel' && ask.hour === 103), [], `the hover reads no country before it joins: ${JSON.stringify(hovered.asks)}`);
}));

test('35. Fuel’s held country is read under its alias at each request’s hour: GER picked at 103, its breakdown hovered at 108 reads GDR, never the new GER', () => onPage(async act => {
  await act(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 103, countries: GER, groups: RENAMED }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 103)');
  assert.ok(opened.called && opened.drillOpen, `GER’s breakdown opens at 103: ${opened.text}`);
  const gained = await act<Read & { pressed: boolean }>('press("Fuel gained")');
  assert.ok(gained.pressed && gained.drillOpen, 'the gain breakdown is shown, held at 103');
  const hovered = await act<Read & { called: boolean }>('call("TimeChart", "onHover", 108)');
  const named = hovered.asks.filter(ask => ask.product === 'country-fuel' && ask.hour === 108).map(ask => ask.params?.country);
  assert.ok(hovered.called && named.length > 0 && named.every(country => country === 'GDR'), `the hover at 108 names the held country’s alias: ${JSON.stringify(hovered.asks)}`);
}));

test('36. Stability pinned at 103 holds GER through its rename: the history is read by the held identity within its runs, and a hover at 108 never reads the new GER', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'stability', time: pinAt(103), hour: 108, countries: GER, options: { show: 'stability' }, groups: RENAMED }));
  const parts = shown.asks.filter(ask => ask.product === 'series-parts');
  assert.ok(parts.length > 0 && parts.every(ask => JSON.stringify(ask.params?.countries) === '[1]'), `parts are listed for the held identity’s slot: ${JSON.stringify(parts)}`);
  const hovered = await act<Read & { called: boolean }>('call("TimeChart", "onHover", 108)');
  const reads = hovered.asks.filter(ask => ask.product === 'series-state').flatMap(ask => (ask.params?.reads ?? []) as { country?: unknown; hour?: number }[]);
  assert.deepEqual(reads.filter(read => read.country === 'GER' && read.hour === 108), [], `a hover at 108 never reads the other country now named GER: ${JSON.stringify(reads)}`);
}));

test('37. Production lines’ saved breakdown whose line list fails says so with a retry instead of loading forever', () => onPage(async act => {
  const line = await act<string>('lineToken("GER", "k1")');
  const shown = await act<Read>(scene({ kind: 'production-lines', time: pinAt(105), hour: 107, countries: GER, groups: groupsOf(['GER']), failures: ['production-lines'],
    options: { show: 'efficiency', line, productionCountry: await act<string>('identityToken(0)') } }));
  assert.ok(/Couldn’t load/.test(shown.text) && shown.buttons.includes('Retry') && !/Loading breakdown/.test(shown.text), `${shown.text} ${JSON.stringify(shown.buttons)}`);
}));

test('38. A breakdown’s Follow comes from its own region: Resource balance pinned at 105 with its breakdown open offers Follow playback at 108', () => onPage(async act => {
  await act(scene({ kind: 'resource-balance', time: pinAt(105), hour: 108, countries: GER, groups: groupsOf(['GER']),
    replies: { 'resource-balance': RESOURCES.replace('shown: []', 'shown: [1]') } }));
  const opened = await act<Read & { clicked: boolean }>('click(".resource-bars li")');
  assert.ok(opened.clicked && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  assert.ok(opened.buttons.includes('Follow playback'), `the pinned parent’s breakdown can follow: ${JSON.stringify(opened.buttons)}`);
}));

test('39. Resource balance’s saved breakdown on its “First resource” setting shows the first resource of the reply at its hour', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'resource-balance', time: pinAt(105), hour: 107, countries: GER, options: { show: 'resource' }, groups: groupsOf(['GER']),
    replies: { 'resource-balance': RESOURCES } }));
  assert.ok(!/Resource unavailable/.test(shown.text), `the setting chooses the first resource: ${shown.text}`);
}));

test('40. World tension’s breakdown open and paused at 105 is the region that reports its hour: when the following parent leaves the branch, nothing reports the parent’s hour or its missing read', () => onPage(async act => {
  const mounted = await act<Read>(scene({ kind: 'world-tension', time: { mode: 'follow' }, hour: 105, replies: { 'world-tension': TENSION } }));
  const opened = await act<Read & { called: boolean }>('call("TimeChart", "onPick", 105)');
  assert.ok(opened.called && opened.drillOpen, `the breakdown opens: ${opened.text}`);
  const paused = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(paused.pressed && paused.drillOpen, 'the breakdown pauses at 105');
  const left = await act<Read>('seek(112)');
  assert.ok(left.drillOpen, 'the paused breakdown stays open');
  const reported = [...mounted.shown, ...opened.shown, ...paused.shown, ...left.shown];
  assert.ok(reported.length > 0 && reported.at(-1) === 105, `the widget last reports the breakdown’s hour: ${JSON.stringify(reported)}`);
  assert.deepEqual([...opened.shown, ...paused.shown, ...left.shown].filter(hour => hour !== 105 && hour !== null), [], `once it opens, only the hour the breakdown’s reply shows is reported, or nothing while it reads: ${JSON.stringify(reported)}`);
}));

test('41. Factories: GER picked at 103 and pinned after GER passes to another country pins the country it picked, by its verified identity', () => onPage(async act => {
  await act(scene({ kind: 'factories', time: { mode: 'follow' }, hour: 103, countries: GER, groups: RENAMED }));
  const opened = await act<Read & { called: boolean }>('call("CountryRaceView", "onPick", 103, "GER")');
  assert.ok(opened.called && opened.drillOpen, `GER’s breakdown opens at 103: ${opened.text}`);
  await act('seek(108)');
  const pinned = await act<Read & { pressed: boolean }>('press("Pin as widget")');
  const saved = pinned.pins[0] as { time: WidgetTime; countries: unknown; options: Record<string, unknown> } | undefined;
  assert.ok(pinned.pressed && saved, `the breakdown pins: ${JSON.stringify(pinned.buttons)}`);
  const token = saved.options.holder;
  assert.equal(typeof token, 'string', `the pin saves the picked country’s identity: ${JSON.stringify(saved.options)}`);
  const country = JSON.parse(await act<string>(`content(${JSON.stringify(token)})`)) as { tag: string; originOffset: number };
  assert.deepEqual([country.tag, country.originOffset, saved.countries], ['GER', 3, GER], 'the pin holds the country picked at 103');
  const restored = await act<Read>(scene({ kind: 'factories', time: saved.time, hour: 108, countries: saved.countries, options: saved.options, groups: RENAMED }));
  const named = restored.asks.filter(ask => ['series-parts', 'series-state', 'series-rank', 'follow'].includes(ask.product)).map(ask => JSON.stringify(ask.params));
  assert.ok(named.length > 0 && named.every(params => !params.includes('"GER"') && !/"countries":\[3\]|"country":3\b/.test(params)) && named.some(params => params.includes('"GDR"') || /"countries":\[1\]|"country":1\b/.test(params)),
    `the restored pin reads the country it picked, never the new GER: ${JSON.stringify(restored.asks.map(ask => [ask.product, ask.params]))}`);
}));

const REUSED = { ...groupsOf([]), countryIdentities: [
  { tag: 'GER', identities: [{ slot: 1, countryId: 101, firstHour: 100, lastHour: 104 }] },
  { tag: 'ENG', identities: [{ slot: 1, countryId: 102, firstHour: 105, lastHour: 109 }] }] };

test('42. Stability pinned at 103 on GER, whose slot ENG holds from 105: its parts and their names are read only within GER’s own runs', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'stability', time: pinAt(103), hour: 103, countries: GER, options: { show: 'stability' }, groups: REUSED,
    replies: { 'series-parts': '({ branchId: "1", partBits: 32, owners: [{ country: 1, parts: [{ part: "1", firstHour: params.from, lastHour: params.to }], missing: [] }] })' } }));
  const parts = shown.asks.filter(ask => ask.product === 'series-parts');
  assert.ok(parts.length > 0 && parts.every(ask => (ask.params?.to as number) <= 104), `the part catalogue stays inside GER’s runs: ${JSON.stringify(parts.map(ask => ask.params))}`);
  const names = shown.asks.filter(ask => ask.product === 'stability-part-names');
  assert.ok(names.length > 0 && names.every(ask => ask.hour !== null && ask.hour >= 100 && ask.hour <= 104), `part names are read, and only while GER holds the slot: ${JSON.stringify(names.map(ask => [ask.hour, ask.params]))}`);
}));

const SUBJECTS = `({ branchId: '1', warnings: [], unobserved: [], levels: [{ token: 'puppet', name: 'Puppet', ordered: true }],
  runs: [{ firstHour: 100, lastHour: 109, overlord: 'ENG', subject: 'GER', startHour: 100, steps: [{ firstHour: 100, lastHour: 109, level: 'puppet' }] }] })`;

test('43. Autonomy of the human subjects at 108: hovering 103, before GER joins the humans, reads nothing for GER', () => onPage(async act => {
  await act(scene({ kind: 'autonomy', time: { mode: 'follow' }, hour: 108, countries: HUMANS, options: { countriesAre: 'subjects' },
    groups: groupsOf(['GER', 'ENG'], [{ tag: 'GER', firstHour: 107, lastHour: 109 }]), replies: { subjects: SUBJECTS } }));
  const hovered = await act<Read & { called: boolean }>('call("Timeline", "onHover", 103)');
  assert.ok(hovered.called, 'the timeline takes the hover');
  const early = hovered.asks.flatMap(ask => (ask.params?.reads ?? []) as { country?: unknown; hour?: number }[]).filter(read => read.hour === 103);
  assert.deepEqual(early, [], `no read names GER at 103: ${JSON.stringify(hovered.asks)}`);
  await act(scene({ kind: 'autonomy', time: { mode: 'follow' }, hour: 108, countries: HUMANS,
    groups: groupsOf(['GER', 'ENG'], [{ tag: 'GER', firstHour: 100, lastHour: 109 }, { tag: 'ENG', firstHour: 107, lastHour: 109 }]), replies: { subjects: SUBJECTS } }));
  const before = await act<Read & { called: boolean }>('call("Timeline", "onHover", 103)');
  const read = (asks: Ask[], hour: number) => asks.flatMap(ask => (ask.params?.reads ?? []) as { country?: unknown; hour?: number }[]).filter(item => item.hour === hour);
  assert.ok(before.called, 'the overlords timeline takes the hover');
  assert.deepEqual(read(before.asks, 103), [], `GER’s overlord ENG is no human at 103, so nothing is read there: ${JSON.stringify(before.asks)}`);
  const after = await act<Read & { called: boolean }>('call("Timeline", "onHover", 108)');
  assert.ok(read(after.asks, 108).length > 0, `once ENG is human at 108 its subject GER is read: ${JSON.stringify(after.asks)}`);
}));

test('44. Production lines’ saved breakdown pinned at 105 offers Follow playback on its own surface', () => onPage(async act => {
  const line = await act<string>('lineToken("GER", "k1")');
  const shown = await act<Read>(scene({ kind: 'production-lines', time: pinAt(105), hour: 108, countries: GER, groups: groupsOf(['GER']),
    options: { show: 'efficiency', line, productionCountry: await act<string>('identityToken(0)') }, replies: { 'production-lines': LINES } }));
  assert.ok(shown.buttons.includes('Follow playback'), `the saved breakdown can follow: ${JSON.stringify(shown.buttons)}`);
  const lines = shown.asks.filter(ask => ask.product === 'production-lines' || ask.product === 'production-line-breakdown');
  assert.ok(lines.length > 0 && lines.every(ask => ask.params?.country === 1 && Array.isArray(ask.params?.within)), `every line read names the held identity within its runs, never the whole tag: ${JSON.stringify(lines.map(ask => [ask.product, ask.params]))}`);
}));
