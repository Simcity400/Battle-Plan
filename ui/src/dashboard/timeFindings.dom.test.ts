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
const tracked = (name: string) => (code: string) => {
  const declared = new RegExp(`export function ${name}\\(`);
  if (!declared.test(code)) throw new Error(`${name} is not declared as expected`);
  return `${code.replace(declared, `function real_${name}(`)}\nexport function ${name}(...args) { const held = real_${name}(...args); globalThis.__finding.importing(held); return held; }`;
};
const TRANSFORMS: Record<string, (code: string) => string> = {
  '/charts/TimeChart.tsx': capture('TimeChart'),
  '/dashboard/raids.ts': source('useRaidLog'),
  '/dashboard/widgetPerf.ts': source('shownSink'),
  '/widgets/rendererLoaders.ts': tracked('loadWidgetRenderer'),
};
const stubs = { name: 'time-findings', enforce: 'pre' as const, transform(code: string, id: string) {
  const path = id.replaceAll('\\', '/').split('?')[0];
  return Object.entries(TRANSFORMS).find(([suffix]) => path.endsWith(suffix))?.[1](code);
} };

interface Ask { product: string; hour: number | null; hours: number[]; params: Record<string, unknown> | null }
interface Read { text: string; detail: string | null; drillOpen: boolean; drill: string; day: string | null; asks: Ask[]; options: [string, unknown][]; pins: unknown[];
  follows: string[]; followLog: string[]; buttons: string[]; errors: string[]; shown: (number | null)[] }
const onPage = (run: (act: <T>(expression: string) => Promise<T>) => Promise<void>) => withNativePage('time-findings', '/ui/src/dashboard/timeFindings.dom.fixture.tsx',
  page => page.stage('time findings', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.findings.${expression}`);
    await run(act);
  }, 240_000), [stubs], true);
const scene = (value: unknown) => `mount(${JSON.stringify(value)})`;
const pinAt = (hour: number) => ({ mode: 'pin', windows: { main: { firstOffset: hour - 100, lastOffset: hour - 100 } } });
const NO_PIN = { mode: 'pin', windows: {} };
const groupsOf = (tags: string[], members: { tag: string; firstHour: number; lastHour: number }[] = []) => ({ branchId: '1', warnings: [], warStarts: [], capitulations: [],
  identityChanges: [], factions: [], wars: [], humans: members, unobserved: { humans: [], factions: [], wars: [] },
  countryIdentities: tags.map((tag, index) => ({ tag, identities: [{ slot: index + 1, countryId: index + 101, firstHour: 100, lastHour: 109 }] })) });
const GER = { kind: 'tags', tags: ['GER'] }, BOTH = { kind: 'tags', tags: ['GER', 'ENG'] };
const OUTSIDE = 'This time is outside the recording.', UNRECORDED = 'Not recorded at this time.', SETTINGS = 'Change time';
const QUEUE = '({ tag: "GER", readHour: hour, missing: null, orders: [], inTraining: 0, warnings: [] })';
const REASON = 'Branch 1 block 105 – 106 is unreadable: checksum';

test('1. Training queue pinned inside a declared gap 105–106 saves no country, asks nothing at 105, and says the time was not recorded with the recording’s reason', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'training-queue', time: pinAt(105), hour: 107, countries: GER, groups: groupsOf(['GER']), replies: { 'training-queue': QUEUE },
    gaps: [{ firstHour: 105, lastHour: 106, reason: REASON }] }));
  assert.deepEqual(shown.options.filter(([key]) => key === 'country'), [], `no country is chosen or saved inside the gap: ${JSON.stringify(shown.options)}`);
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'training-queue'), [], `no queue is asked in the gap: ${JSON.stringify(shown.asks)}`);
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'country-groups'), [], 'no country groups are read for an unrecorded time');
  assert.ok(shown.text.includes(UNRECORDED) && shown.detail === REASON && shown.text.includes(SETTINGS), `${shown.text} (detail=${shown.detail})`);
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

test('7. Before a 100–109 track at 95, coverage wins over loading, failures and false zeroes, and nothing unrecorded is asked', () => onPage(async act => {
  const raids = { episodes: [], missing: [], available: true, unavailable: null, warnings: [], types: [], artwork: [] };
  for (const [label, value] of [['pending', undefined], ['failed', 'failed'], ['answered', raids]] as const) {
    const shown = await act<Read>(scene({ kind: 'raid-tally', time: { mode: 'follow' }, hour: 95, options: { view: 'race' }, groups: groupsOf(['GER']), raids: value }));
    assert.ok(shown.text.includes(OUTSIDE) && !/Couldn’t load|Loading/.test(shown.text), `${label}: ${shown.text}`);
  }
  const queue = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 95, countries: GER, groups: groupsOf(['GER']) }));
  assert.deepEqual(queue.asks.filter(ask => ask.product === 'country-groups' || ask.product === 'training-queue'), [], `outside asks nothing: ${JSON.stringify(queue.asks)}`);
  await act(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 105, countries: GER, groups: groupsOf(['GER']) }));
  const hovered = await act<Read>('call("TimeChart", "onHover", 112)');
  assert.deepEqual(hovered.asks.filter(ask => ask.product === 'country-fuel' && ask.hour === 112), [], `no hover read past the track: ${JSON.stringify(hovered.asks)}`);
}));

const COLUMNS = `({ branchId: '1', warnings: [], defaultColumns: ['power'], defaultSort: 'power', columns: [{ key: 'power', label: 'Power', unit: '', decimals: 0,
  available: true, reason: null, family: { kind: 'country-measure', measure: 'politicalPower' }, partial: null, noneAtZero: false }] })`;
const RANKED = `({ branchId: '1', hour, missing: null, warnings: [], rows: [{ tag: 'GER', country: 'GER', cells: [{ valueRaw: '5', missing: null, partial: false }] },
  { tag: 'ENG', country: 'ENG', cells: [{ valueRaw: '3', missing: null, partial: false }] }] })`;
const GAP = [{ firstHour: 105, lastHour: 106, reason: REASON }];
const TOP = { kind: 'group', group: { type: 'top', count: 3 }, membership: 'hour', aggregate: false };
const dated = (asks: Ask[], hour: number) => asks.filter(ask => ask.hours.includes(hour));

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

test('13. Ranking and follow obey coverage: nothing is sent into a 105–106 gap and a paused seek past the track releases a follow', () => onPage(async act => {
  const ranked = await act<Read>(scene({ kind: 'equipment-produced', time: pinAt(105), hour: 107, countries: TOP, options: { count: 'ic' }, gaps: GAP, groups: groupsOf(['GER']) }));
  assert.deepEqual(dated(ranked.asks, 105), [], `no ranking at 105: ${JSON.stringify(ranked.asks)}`);
  const followed = await act<Read>(scene({ kind: 'navy-size', time: { mode: 'follow' }, hour: 105, countries: TOP, options: { view: 'race', 'measure@race': 'strength' }, groups: groupsOf(['GER']), accept: true }));
  const registered = followed.followLog.filter(entry => entry.startsWith('follow '));
  assert.ok(followed.follows.length > 0 && registered.length > 0, `inside the track the ranking registers a follow at the transport: ${JSON.stringify(followed.followLog)}`);
  assert.deepEqual(followed.asks.filter(ask => ask.product === 'follow' && ask.hours.length), [], `a follow registration names no hour: ${JSON.stringify(followed.asks)}`);
  const left = await act<Read>('seek(115)');
  assert.deepEqual(left.follows, [], `paused past the track, no follow stays registered: ${JSON.stringify(left.follows)}`);
  assert.deepEqual(left.followLog.filter(entry => entry.startsWith('unfollow ')).sort(), registered.map(entry => `un${entry}`).sort(), 'each registration is released');
  const back = await act<Read>('seek(105)');
  assert.ok(back.follows.length > 0 && back.followLog.some(entry => entry.startsWith('follow ') && !registered.includes(entry)), `back inside, a new follow registers: ${JSON.stringify(back.followLog)}`);
}));

test('18. A country chosen by slot whose identities fail to load says so and offers a retry, instead of loading forever', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 105, countries: { kind: 'tags', tags: ['\u00001'] }, groups: groupsOf(['GER']),
    failures: ['country-identities'], replies: { 'training-queue': QUEUE } }));
  assert.ok(/Couldn’t load/.test(shown.text) && !/Loading/.test(shown.text) && shown.buttons.includes('Retry'), `${shown.text} ${JSON.stringify(shown.buttons)}`);
}));

const RENAMED = { ...groupsOf([]), countryIdentities: [
  { tag: 'GER', identities: [{ slot: 1, countryId: 101, firstHour: 100, lastHour: 104 }, { slot: 3, countryId: 103, firstHour: 105, lastHour: 109 }] },
  { tag: 'GDR', identities: [{ slot: 1, countryId: 101, firstHour: 105, lastHour: 109 }] }] };

const HUMANS = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };

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
  const shown = await act<Read>(scene({ kind: 'territory', time: { mode: 'follow' }, hour: 105, countries: { kind: 'factions', count: 3, membership: 'ever' },
    groups: FACTIONS, accept: true }));
  const members = shown.follows.filter(query => query.includes('"faction-totals-at"'));
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

test('32. A country chosen by slot whose groups fail but whose identities load re-resolves its tag at the identity change during continuous playback', () => onPage(async act => {
  await act(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 103, countries: { kind: 'tags', tags: ['\u00001'] }, groups: RENAMED, failures: ['country-groups'],
    replies: { 'training-queue': QUEUE, 'country-identities': `({ branchId: '1', countries: ${JSON.stringify(RENAMED.countryIdentities)}, unknown: [], warnings: [] })` } }));
  const played = await act<Read & { hour: number; playing: boolean }>('play(1, 4, 1000)');
  assert.ok(played.playing && played.hour >= 106, `playback runs to ${played.hour}`);
  assert.ok(played.asks.some(ask => ask.product === 'training-queue' && ask.params?.country === 'GDR') || naming(played.asks, 'GDR').length > 0,
    `past 105 the slot is read as GDR: ${JSON.stringify(played.asks.map(ask => [ask.product, ask.params]))}`);
  await act('pause()');
}));

test('33. A saved range starting before its branch is clipped to the branch before it is asked', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'decisions', time: { mode: 'range', firstOffset: -20, lastOffset: 5 }, hour: 105, countries: GER, groups: groupsOf(['GER']) }));
  const decisions = shown.asks.filter(ask => ask.product === 'decisions-tail');
  assert.ok(decisions.length > 0 && decisions.every(ask => ask.params?.from === 100 && ask.params?.hour === 105), `the range is clipped to 100–105: ${JSON.stringify(decisions)}`);
}));

const AUTONOMY_LEVELS = `({ levels: [{ token: 'puppet', name: 'Puppet', ordered: true }], missing: null, warnings: [],
  subjects: [{ overlordId: 2, overlordSlot: 2, overlord: 'ENG', subjectId: 1, subjectSlot: 1, subject: 'GER', level: 'puppet', levelBelow: null, levelAbove: null, levelsUnknown: null, progressMilli: 50000, progressUnknown: null }] })`;

test('43. Autonomy lines follows autonomy-levels without naming an hour and never asks the removed whole-branch subjects product', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'autonomy', time: { mode: 'follow' }, hour: 108, countries: HUMANS, options: { view: 'lines', countriesAre: 'subjects' },
    groups: groupsOf(['GER', 'ENG'], [{ tag: 'GER', firstHour: 107, lastHour: 109 }]), replies: { 'autonomy-levels': AUTONOMY_LEVELS } }));
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'subjects'), [], `removed subjects product is never asked: ${JSON.stringify(shown.asks)}`);
  const levels = shown.follows.filter(query => query.includes('"autonomy-levels"'));
  assert.ok(levels.length > 0, `autonomy-levels is followed: ${JSON.stringify(shown.followLog)}`);
  assert.ok(levels.every(query => !/"hour"/.test(query)), `followed autonomy-levels names no hour: ${JSON.stringify(levels)}`);
}));

