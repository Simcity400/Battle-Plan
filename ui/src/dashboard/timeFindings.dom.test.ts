import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

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
const TRANSFORMS: Record<string, (code: string) => string> = {
  '/charts/TimeChart.tsx': capture('TimeChart'),
  '/charts/timeline/Timeline.tsx': capture('Timeline'),
  '/charts/heat/HeatMatrix.tsx': capture('HeatMatrix'),
  '/charts/heat/HeatChart.tsx': capture('HeatChart'),
  '/charts/race/CountryRaceView.tsx': capture('CountryRaceView'),
  '/components/CountryPicker.tsx': capture('CountryPicker'),
  '/charts/flow/FlowDiagram.tsx': capture('FlowDiagram'),
  '/widgets/trade-flows/TradeMarketView.tsx': capture('TradeMarketView'),
  '/widgets/trade-flows/useTradeFlows.ts': source('useTradeFlows'),
  '/dashboard/raids.ts': source('useRaidLog'),
  '/dashboard/follow/useFollow.ts': source('useFollow'),
};
const stubs = { name: 'time-findings', enforce: 'pre' as const, transform(code: string, id: string) {
  const path = id.replaceAll('\\', '/').split('?')[0];
  return Object.entries(TRANSFORMS).find(([suffix]) => path.endsWith(suffix))?.[1](code);
} };

interface Ask { product: string; hour: number | null; hours: number[]; params: Record<string, unknown> | null }
interface Read { text: string; drillOpen: boolean; drill: string; day: string | null; asks: Ask[]; options: [string, unknown][]; pins: unknown[];
  follows: string[]; buttons: string[]; errors: string[] }
const KINDS = ['training-queue', 'occupation-trend', 'diplomatic-relations', 'general-career', 'navy-size', 'fuel', 'raid-tally', 'volunteers', 'equipment-produced',
  'stability', 'weather-by-region', 'trade-flows', 'factories', 'scoreboard', 'resource-balance', 'research', 'casualties', 'equipment-stockpile'];
const onPage = (run: (act: <T>(expression: string) => Promise<T>) => Promise<void>) => withNativePage('time-findings', '/ui/src/dashboard/timeFindings.dom.fixture.tsx',
  page => page.stage('time findings', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.findings.${expression}`);
    await act(`preload(${JSON.stringify(KINDS)})`);
    await run(act);
  }, 240_000), [stubs]);
const scene = (value: unknown) => `mount(${JSON.stringify(value)})`;
const pinAt = (hour: number) => ({ mode: 'pin', windows: { main: { firstOffset: hour - 100, lastOffset: hour - 100 } } });
const NO_PIN = { mode: 'pin', windows: {} };
const groupsOf = (tags: string[], members: { tag: string; firstHour: number; lastHour: number }[] = []) => ({ branchId: '1', warnings: [], warStarts: [], capitulations: [],
  identityChanges: [], factions: [], wars: [], humans: members, unobserved: { humans: [], factions: [], wars: [] },
  countryIdentities: tags.map((tag, index) => ({ tag, identities: [{ slot: index + 1, countryId: index + 101, firstHour: 100, lastHour: 109 }] })) });
const GER = { kind: 'tags', tags: ['GER'] }, BOTH = { kind: 'tags', tags: ['GER', 'ENG'] };
const OUTSIDE = 'This time is outside the recording.', UNRECORDED = 'This time was not recorded.', SETTINGS = 'Choose another time in widget settings.';
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

test('2. A widget pinned with no window for its track asks no dated read at any hour and shows only “choose another time”: Occupation trend’s mix, Diplomatic relations’ top countries and General career', () => onPage(async act => {
  const mix = await act<Read>(scene({ kind: 'occupation-trend', time: NO_PIN, hour: 105, countries: GER, options: { view: 'mix' }, groups: groupsOf(['GER']) }));
  assert.deepEqual(mix.asks.filter(ask => ask.product === 'occupation-at'), [], `no occupiers are read: ${JSON.stringify(mix.asks)}`);
  assert.ok(mix.text.includes(SETTINGS), mix.text);
  const top = await act<Read>(scene({ kind: 'diplomatic-relations', time: NO_PIN, hour: 105, countries: { kind: 'group', group: { type: 'top', count: 3 }, membership: 'hour', aggregate: false },
    groups: groupsOf(['GER', 'ENG']), replies: { 'scoreboard-columns': '({ columns: [], defaultSort: null, warnings: [] })' } }));
  assert.deepEqual(top.asks.filter(ask => ask.product === 'scoreboard' || ask.product === 'diplomatic-relations'), [], `nothing is ranked or read: ${JSON.stringify(top.asks)}`);
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
  assert.ok(back.asks.some(ask => ask.product === 'country-fuel' && ask.hour === 105), `it reads the playback hour: ${JSON.stringify(back.asks)}`);
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

const PAIR_OWNERS = '({ owners: [{ tag: "GER", countryId: 101, known: true, reason: null }, { tag: "ENG", countryId: 102, known: true, reason: null }], warnings: [] })';

test('6. Diplomatic relations’ pair picked at 105 keeps its own date and stays mounted when the following matrix moves to 108 and leaves the track at 110', () => onPage(async act => {
  await act(scene({ kind: 'diplomatic-relations', time: { mode: 'follow' }, hour: 105, countries: BOTH, groups: groupsOf(['GER', 'ENG']), replies: { 'diplomatic-relations': PAIR_OWNERS } }));
  const opened = await act<Read & { called: boolean }>('call("HeatMatrix", "onSelect", 0, 1)');
  assert.ok(opened.called && opened.drillOpen, `the pair opens: ${opened.text}`);
  const day = await act<string>('day(105)');
  for (const hour of [108, 110]) {
    const moved = await act<Read>(`seek(${hour})`);
    assert.ok(moved.drillOpen && moved.day === day, `at ${hour} the pair is still at 105: ${moved.day} ${moved.drill}`);
  }
  const pinned = await act<Read & { pressed: boolean }>('press("Pin as widget")');
  assert.ok(pinned.pressed, `the pair offers Pin as widget: ${JSON.stringify(pinned.buttons)}`);
  assert.deepEqual((pinned.pins[0] as { time?: unknown } | undefined)?.time, pinAt(105), `Pin as widget saves the pair’s own hour, not the parent’s: ${JSON.stringify(pinned.pins)}`);
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

const WEATHER = `({ firstHour: 100, lastHour: 109, dayHours: 1, temperature: 'available', regions: [{ id: 'r1', name: 'Region one' }], flags: [], modifiers: [], warnings: [],
  columns: { days: 1, columns: [{ firstHour: 100, lastHour: 109, recorded: 1, sampledHour: 105, readHour: 105 }], regions: [{ id: 'r1', clear: [1], observed: [1], measures: [] }] } })`;
const PROVINCES = '({ readHour: hour, region: params.region, warnings: [], provinces: [{ id: "p1", name: "Province one", snow: null, mud: null, temperature: null }] })';

test('9. Weather over 100–108: a province breakdown picked at 103 keeps its own day as the page leaves the track, and Follow playback reads the playback hour through coverage', () => onPage(async act => {
  await act(scene({ kind: 'weather-by-region', time: { mode: 'range', firstOffset: 0, lastOffset: 8 }, hour: 105, replies: { 'regional-weather': WEATHER, 'regional-weather-provinces': PROVINCES } }));
  const extent = (await act<{ extent: [number, number] } | null>('props("HeatChart")'))?.extent;
  assert.ok(extent && extent[0] <= 103 && 103 <= extent[1], `103 lies on the chart: ${JSON.stringify(extent)}`);
  const opened = await act<Read & { called: boolean }>('call("HeatChart", "onPick", { id: "r1" }, null, 103)');
  const day = await act<string>('day(103)');
  assert.ok(opened.called && opened.drillOpen && opened.day === day, `the region opens at 103: ${opened.day} ${opened.drill}`);
  assert.ok(opened.asks.some(ask => ask.product === 'regional-weather-provinces' && ask.hour === 103), `its provinces are read at 103: ${JSON.stringify(opened.asks)}`);
  const left = await act<Read>('seek(110)');
  assert.ok(left.drillOpen && left.day === day && !left.drill.includes(OUTSIDE), `at 110 the region is still at 103: ${left.day} ${left.drill}`);
  const followed = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(followed.pressed && followed.drill.includes(OUTSIDE), `following the page at 110 is outside: ${followed.drill}`);
  assert.deepEqual(followed.asks.filter(ask => ask.product === 'regional-weather-provinces' && ask.hour !== null && ask.hour > 109), [], 'nothing is asked past the track');
  const back = await act<Read>('seek(104)');
  assert.ok(back.drillOpen && !back.drill.includes(OUTSIDE), `back inside, still following: ${back.drill}`);
  assert.ok(back.asks.some(ask => ask.product === 'regional-weather-provinces' && ask.hour === 104), `it reads the playback hour: ${JSON.stringify(back.asks)}`);
}));

const COLUMNS = `({ branchId: '1', warnings: [], defaultColumns: ['power'], defaultSort: 'power', columns: [{ key: 'power', label: 'Power', unit: '', decimals: 0,
  available: true, reason: null, family: { kind: 'country-measure', measure: 'politicalPower' }, partial: null, noneAtZero: false }] })`;
const RANKED = `({ branchId: '1', hour, missing: null, warnings: [], rows: [{ tag: 'GER', country: 'GER', cells: [{ valueRaw: '5', missing: null, partial: false }] },
  { tag: 'ENG', country: 'ENG', cells: [{ valueRaw: '3', missing: null, partial: false }] }] })`;
const GAP = [{ firstHour: 105, lastHour: 106, reason: REASON }];
const TOP = { kind: 'group', group: { type: 'top', count: 3 }, membership: 'hour', aggregate: false };
const dated = (asks: Ask[], hour: number) => asks.filter(ask => ask.hours.includes(hour));

test('10. Diplomatic relations: the default Top N ranks and reads opinions, and two countries chosen by slot are both read', () => onPage(async act => {
  const top = await act<Read>(scene({ kind: 'diplomatic-relations', time: { mode: 'follow' }, hour: 105, countries: TOP, groups: groupsOf(['GER', 'ENG']),
    replies: { 'scoreboard-columns': COLUMNS, scoreboard: RANKED, 'diplomatic-relations': PAIR_OWNERS } }));
  assert.ok(top.asks.some(ask => ask.product === 'diplomatic-relations'), `Top N reads opinions: ${top.text} ${JSON.stringify(top.asks.map(ask => ask.product))}`);
  assert.ok(!top.text.includes('Loading opinions'), top.text);
  const slots = await act<Read>(scene({ kind: 'diplomatic-relations', time: { mode: 'follow' }, hour: 105, countries: { kind: 'tags', tags: ['\u00001', '\u00002'] },
    groups: groupsOf(['GER', 'ENG']), replies: { 'diplomatic-relations': PAIR_OWNERS } }));
  const read = slots.asks.find(ask => ask.product === 'diplomatic-relations');
  assert.deepEqual(read?.params?.countries, [1, 2], `both slots are read: ${slots.text} ${JSON.stringify(slots.asks)}`);
}));

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
  const followed = await act<Read>(scene({ kind: 'casualties', time: { mode: 'follow' }, hour: 105, countries: TOP, groups: groupsOf(['GER']) }));
  assert.ok(followed.follows.length > 0, 'inside the track the ranking follows playback');
  const left = await act<Read>('seek(115)');
  assert.deepEqual(left.follows, [], `paused past the track, no follow stays open: ${JSON.stringify(left.follows)}`);
  const back = await act<Read>('seek(105)');
  assert.ok(back.follows.length > 0, 'back inside, the follow reopens');
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

const MARKET = '({ branchId: "1", offers: [], contracts: [], agreements: [], purchasesMissing: null, unattributed: 0, variants: [], icons: [], warnings: [] })';

test('17. Trade market: the moment follows pin, campaign and range; a picked row pins at its own hour and restores on its own breakdown, which can follow playback', () => onPage(async act => {
  const replies = { 'market-lendlease': MARKET };
  const pinned = await act<Read>(scene({ kind: 'trade-flows', time: pinAt(105), hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  assert.ok(pinned.asks.some(ask => ask.product === 'market-lendlease' && ask.hours.includes(105)), `pinned reads 105: ${JSON.stringify(pinned.asks)}`);
  const campaign = await act<Read>(scene({ kind: 'trade-flows', time: { mode: 'campaign' }, hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  assert.ok(campaign.asks.some(ask => ask.product === 'market-lendlease' && ask.hours.includes(109)), `the whole campaign reads its end: ${JSON.stringify(campaign.asks)}`);
  const range = await act<Read>(scene({ kind: 'trade-flows', time: { mode: 'range', firstOffset: 0, lastOffset: 3 }, hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
  assert.ok(range.asks.some(ask => ask.product === 'market-lendlease' && ask.hours.includes(103)), `a range reads its last hour: ${JSON.stringify(range.asks)}`);
  for (const key of ['offer:5000', 'contract:false:0011', 'agreement:0012']) {
    await act(scene({ kind: 'trade-flows', time: pinAt(105), hour: 107, options: { view: 'market' }, countries: { kind: 'all' }, replies }));
    const opened = await act<Read & { called: boolean }>(`call("TradeMarketView", "onPick", ${JSON.stringify(key)})`);
    assert.ok(opened.called && opened.drillOpen, `${key} opens its contributions: ${opened.text}`);
    const pin = await act<Read & { pressed: boolean }>('press("Pin as widget")');
    const saved = pin.pins[0] as { time: unknown; options: Record<string, unknown> } | undefined;
    assert.ok(pin.pressed && saved, `${key} pins: ${JSON.stringify(pin.buttons)}`);
    assert.deepEqual(saved.time, pinAt(105), `${key} pins at the row’s own hour`);
    assert.equal(saved.options.view, 'market'); assert.equal(saved.options.show, 'contributions');
    const restored = await act<Read>(scene({ kind: 'trade-flows', time: saved.time, hour: 107, options: saved.options, countries: { kind: 'all' }, replies }));
    assert.ok(restored.drillOpen, `${key} restores on its breakdown: ${restored.text}`);
    assert.equal(await act<string | null>('prop("TradeMarketView", "selected")'), key, `${key} is the restored row`);
    await act('press("Follow playback")');
    const moved = await act<Read>('seek(108)');
    assert.equal(moved.day, await act<string>('day(108)'), `${key} follows playback: ${moved.day} ${moved.drill}`);
  }
}));

test('18. A country chosen by slot whose identities fail to load says so and offers a retry, instead of loading forever', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 105, countries: { kind: 'tags', tags: ['\u00001'] }, groups: groupsOf(['GER']),
    failures: ['country-identities'], replies: { 'training-queue': QUEUE } }));
  assert.ok(/Couldn’t load/.test(shown.text) && !/Loading/.test(shown.text) && shown.buttons.includes('Retry'), `${shown.text} ${JSON.stringify(shown.buttons)}`);
}));
