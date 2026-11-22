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
  '/charts/race/CountryRaceView.tsx': capture('CountryRaceView'),
  '/components/CountryPicker.tsx': capture('CountryPicker'),
  '/charts/flow/FlowDiagram.tsx': capture('FlowDiagram'),
  '/widgets/trade-flows/useTradeFlows.ts': source('useTradeFlows'),
  '/dashboard/raids.ts': source('useRaidLog'),
};
const stubs = { name: 'time-findings', enforce: 'pre' as const, transform(code: string, id: string) {
  const path = id.replaceAll('\\', '/').split('?')[0];
  return Object.entries(TRANSFORMS).find(([suffix]) => path.endsWith(suffix))?.[1](code);
} };

interface Ask { product: string; hour: number | null; params: Record<string, unknown> | null }
interface Read { text: string; drillOpen: boolean; drill: string; day: string | null; asks: Ask[]; options: [string, unknown][]; buttons: string[]; errors: string[] }
const KINDS = ['training-queue', 'occupation-trend', 'diplomatic-relations', 'general-career', 'navy-size', 'fuel', 'raid-tally', 'volunteers', 'equipment-produced'];
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
