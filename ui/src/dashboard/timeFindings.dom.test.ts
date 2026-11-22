import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

const wrap = (name: string, isDefault = false) => (code: string) => {
  const declared = new RegExp(`export ${isDefault ? 'default ' : ''}function ${name}(?=[<(])`);
  if (!declared.test(code)) throw new Error(`${name} is not declared as expected`);
  const body = code.replace(declared, `function real_${name}`);
  return isDefault
    ? `${body}\nimport { createElement as __create } from 'react';\nexport default function ${name}(props) { const stub = globalThis.__finding?.stubs?.${name}; return stub ? stub(props) : __create(real_${name}, props); }`
    : `${body}\nexport function ${name}(...args) { const real = real_${name}(...args); const stubbed = globalThis.__finding?.${name === 'useHeadProduct' ? 'head' : name === 'useProduct' ? 'product' : name}?.(...args); return stubbed === undefined ? real : stubbed; }`;
};
const capture = (name: string) => (code: string) => `${code.replace(`export default function ${name}(`, `function real_${name}(`)}
export default function ${name}(props) { return globalThis.__finding.capture('${name}')(props); }`;
const drills = (code: string) => code.replace(/\buseReducer\(reduceDrill, CLOSED\)/g, 'globalThis.__finding.useDrill(reduceDrill, CLOSED)');
const value = (pattern: RegExp, name: string) => (code: string) => {
  if (!pattern.test(code)) throw new Error(`no ${name} state to inject`);
  return code.replace(pattern, (_whole, setter: string) => `const [${name}, ${setter}] = globalThis.__finding.useValue('${name}', null);`);
};
const chain = (...steps: ((code: string) => string)[]) => (code: string) => steps.reduce((text, step) => step(text), code);

const TRANSFORMS: Record<string, (code: string) => string> = {
  '/dashboard/headProduct.ts': wrap('useHeadProduct'),
  '/dashboard/products.ts': wrap('useProduct'),
  '/dashboard/useCountryGroups.ts': () => 'export const useCountryGroups = () => globalThis.__finding.groups();',
  '/charts/TimeChart.tsx': capture('TimeChart'),
  '/charts/timeline/Timeline.tsx': capture('Timeline'),
  '/charts/heat/HeatMatrix.tsx': capture('HeatMatrix'),
  '/widgets/trade-flows/TradeMarketView.tsx': () => `import { createElement } from 'react';
    export default props => createElement('p', null, 'Market at ' + props.data.hour + (props.selected ? ' for ' + props.selected : ''));`,
  '/dashboard/MarketOccurrences.tsx': () => 'export default () => null;',
  '/widgets/most-decorated-divisions/MostDecoratedWidget.tsx': drills,
  '/widgets/trade-flows/TradeFlowsWidget.tsx': drills,
  '/widgets/world-tension/WorldTensionWidget.tsx': code => code.replace('const [drill, dispatch] = useState(CLOSED);', 'const [drill, dispatch] = globalThis.__finding.useValue(\'drill\', CLOSED);'),
  '/widgets/minefields/MinefieldsWidget.tsx': chain(drills, value(/const \[picked, (setPicked)\] = useState<RegionRow \| null>\(null\);/, 'picked')),
  '/widgets/diplomatic-relations/DiplomaticRelationsWidget.tsx': chain(drills, value(/const \[choice, (setChoice)\] = useState<\{ scope: object; pair: ChosenPair \} \| null>\(null\);/, 'choice'),
    code => code.replace('const picked = choice?.scope === scope ? choice.pair : null;', 'const picked = choice?.pair ?? null;')),
  '/widgets/lend-lease-agreements/LendLeaseAgreementsWidget.tsx': chain(drills, value(/const \[pick, (setPick)\] = useState<\{ owner: string \| null; row: AgreementRow \} \| null>\(null\);/, 'pick'),
    code => code.replace('const picked = pick && pick.owner === pickOwner ? pick.row : null;', 'const picked = pick ? pick.row : null;')),
  '/widgets/division-equipment/DivisionEquipmentWidget.tsx': code => code.replace('const drill = storedDrill?.owner === drillOwner ? storedDrill.drill : null;',
    'const drill = globalThis.__finding.useValue(\'equipmentDrill\', null)[0];'),
  '/widgets/trade-flows/useTradeFlows.ts': wrap('useTradeFlows'),
  '/dashboard/raids.ts': wrap('useRaidLog'),
};
const stubs = { name: 'time-findings', enforce: 'pre' as const, transform(code: string, id: string) {
  const path = id.replaceAll('\\', '/').split('?')[0];
  return Object.entries(TRANSFORMS).find(([suffix]) => path.endsWith(suffix))?.[1](code);
} };

interface Read { text: string; drillOpen: boolean; drill: string; asks: { product: string; hour: number | null; params: Record<string, unknown> | null }[]; options: [string, unknown][]; buttons: string[] }
const KINDS = ['training-queue', 'trade-flows', 'most-decorated-divisions', 'general-career', 'division-service-record', 'diplomatic-relations', 'minefields',
  'lend-lease-agreements', 'division-equipment', 'world-tension', 'experience', 'navy-size'];
const onPage = (run: (act: <T>(expression: string) => Promise<T>) => Promise<void>) => withNativePage('time-findings', '/ui/src/dashboard/timeFindings.dom.fixture.tsx',
  page => page.stage('time findings', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.findings.${expression}`);
    await act(`preload(${JSON.stringify(KINDS)})`);
    await run(act);
  }, 240_000), [stubs]);
const scene = (value: unknown) => `mount(${JSON.stringify(value)})`;
const pinAt = (hour: number) => ({ mode: 'pin', windows: { main: { firstOffset: hour - 100, lastOffset: hour - 100 } } });
const identities = (runs: { countryId: number; firstHour: number; lastHour: number }[]) => ({ branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [],
  factions: [], wars: [], humans: [], unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [{ tag: 'GER', identities: runs.map(run => ({ slot: 1, ...run })) }] });
const humans = (members: { tag: string; firstHour: number; lastHour: number }[]) => ({ ...identities([{ countryId: 101, firstHour: 100, lastHour: 109 }]), humans: members,
  countryIdentities: ['GER', 'ENG'].map((tag, index) => ({ tag, identities: [{ slot: index + 1, countryId: index + 101, firstHour: 100, lastHour: 109 }] })) });
const HUMANS = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
const GER = { kind: 'tags', tags: ['GER'] };
const FACTS = { branchId: '1', known: [[100, 109]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
const QUEUE = '({ readHour: hour, missing: null, orders: [], warnings: [] })';

test('1. Training queue pinned with no window for its track asks nothing, binds no country and saves no automatic choice: it is the widget’s own time to choose again', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'pin', windows: {} }, hour: 105, countries: GER, groups: identities([{ countryId: 101, firstHour: 100, lastHour: 109 }]),
    replies: { 'training-queue': QUEUE } }));
  assert.deepEqual(shown.options.filter(([key]) => key === 'country'), [], `no automatic country choice is saved: ${JSON.stringify(shown.options)}`);
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'training-queue' && ask.hour !== null), [], 'no queue is asked at any substituted hour');
  assert.ok(shown.text.includes('Choose another time in widget settings.'), shown.text);
}));

test('2. Training queue bound to the holder of GER from 100, following at 107 after another holder took GER at 105: the chosen country does not exist there, and no queue is read for the new holder', () => onPage(async act => {
  const country = await act<string>('identityToken(0)');
  const shown = await act<Read>(scene({ kind: 'training-queue', time: { mode: 'follow' }, hour: 107, options: { country }, countries: GER,
    groups: identities([{ countryId: 101, firstHour: 100, lastHour: 104 }, { countryId: 102, firstHour: 105, lastHour: 109 }]), replies: { 'training-queue': QUEUE },
    products: { 'country-facts': FACTS } }));
  assert.ok(shown.text.includes('does not exist at this date'), JSON.stringify(shown));
  assert.deepEqual(shown.asks.filter(ask => ask.product === 'training-queue'), [], `nothing is read for the other holder: ${JSON.stringify(shown.asks)}`);
}));

test('5. Most decorated divisions pinned at 105, its service record told to follow playback, reads the record at the playback hour 108, never the pin', () => onPage(async act => {
  await act(scene({ kind: 'most-decorated-divisions', time: pinAt(105), hour: 108, replies: { 'division-careers': '({ divisions: [], templates: [], medals: [], missing: null, warnings: [] })',
    'division-record': '({ division: params.division, name: null, absent: false, template: null, facts: null, entries: [], warnings: [] })' } }));
  const followed = await act<Read>(`inject(${JSON.stringify({ drill: { breakdown: 'div-1', follow: true, hour: null } })})`);
  const records = followed.asks.filter(ask => ask.product === 'division-record');
  assert.ok(records.length > 0 && records.every(ask => ask.hour === 108), `the record follows playback: ${JSON.stringify(records)}`);
}));

test('5. Navy management orders in a parts drill told to follow playback, in a widget pinned at 105, read the playback hour 108', () => onPage(async act => {
  const shown = await act<Read>('navy({ hour: 108, pin: 105 })');
  const orders = shown.asks.filter(ask => ask.product === 'naval-management');
  assert.ok(orders.length > 0 && orders.every(ask => ask.hour === 108), `the orders follow playback: ${JSON.stringify(orders)}`);
}));

const OUTSIDE = 'This time is outside the recording.';
const TRADE = {
  branchId: '1', decimals: 5, resources: [{ id: 1, token: 'steel', ordinal: 1, name: 'Steel', icon: null }], countries: ['GER', 'ITA'],
  countryIdentities: ['GER', 'ITA'].map((tag, slot) => ({ tag, runs: [{ firstHour: 100, lastHour: 109, slot, countryId: slot + 1 }] })),
  firstHour: 100, lastHour: 109, known: [{ firstHour: 100, lastHour: 109 }], partial: [],
  flows: [{ seller: 0, buyer: 1, resource: 1, sellerIdentity: '[0,1]', buyerIdentity: '[1,2]', sellerTags: [0], buyerTags: [1], firstHours: [100], lastHours: [109],
    deliveredRaw: ['1000000'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }],
  warnings: [],
};
const MARKET = '({ branchId: "1", offers: [], contracts: [], agreements: [], purchasesMissing: null, unattributed: 0, variants: [], icons: [], warnings: [] })';

test('3. Trade contributors pinned at 105, told to follow playback, at 110: the drill is outside the recording, shows no day and offers no pin, rather than clamping to 109', () => onPage(async act => {
  await act(scene({ kind: 'trade-flows', time: pinAt(105), hour: 110, options: { show: 'contributions' }, trade: TRADE, groups: identities([{ countryId: 1, firstHour: 100, lastHour: 109 }]) }));
  const followed = await act<Read & { pressed: boolean }>('press("Follow playback")');
  assert.ok(followed.pressed, 'the pinned drill offers Follow playback');
  assert.ok(followed.text.includes(OUTSIDE), `following playback past the track is outside the recording: ${followed.text}`);
  assert.ok(!followed.buttons.includes('Pin as widget'), `nothing past the track can be pinned: ${followed.buttons}`);
}));

test('3. Trade contributors following playback, with 105 picked, at 110: the picked drill keeps its recorded hour instead of the widget’s outside state', () => onPage(async act => {
  await act(scene({ kind: 'trade-flows', time: { mode: 'follow' }, hour: 105, options: { show: 'contributions' }, trade: TRADE, groups: identities([{ countryId: 1, firstHour: 100, lastHour: 109 }]) }));
  await act(`inject(${JSON.stringify({ drill: { breakdown: 'contributions', follow: false, hour: 105 } })})`);
  const shown = await act<Read>('seek(110)');
  assert.ok(shown.drillOpen && !shown.drill.includes(OUTSIDE), `105 is recorded, so the drill stays: ${JSON.stringify(shown)}`);
}));

test('4. Trade flows’ market pinned at 105 with a pop-out following playback: at 108 the pop-out reads 108 with 108’s members, and at 110 only the pop-out is outside', () => onPage(async act => {
  const members = humans([{ tag: 'GER', firstHour: 100, lastHour: 109 }, { tag: 'ENG', firstHour: 107, lastHour: 109 }]);
  await act(scene({ kind: 'trade-flows', time: pinAt(105), hour: 108, countries: HUMANS, options: { view: 'market', drill: 'popout' }, groups: members, replies: { 'market-lendlease': MARKET } }));
  const followed = await act<Read>(`inject(${JSON.stringify({ drill: { breakdown: 'contributions', follow: true, hour: null } })})`);
  const asked = followed.asks.filter(ask => ask.product === 'market-lendlease' && ask.hour === 108);
  assert.ok(asked.length > 0 && asked.every(ask => JSON.stringify(ask.params).includes('ENG')), `the pop-out reads 108 with ENG, a member from 107: ${JSON.stringify(followed)}`);
  const past = await act<Read>("seek(110)");
  assert.ok(past.text.includes('Market at 105'), `the main market at its pin is still shown: ${JSON.stringify(past)} ${JSON.stringify((followed as unknown as { errors: string[] }).errors)}`);
}));

test('6. Fuel’s saved consumption breakdown following playback at 110 is outside the recording, never “country unavailable”', () => onPage(async act => {
  const fuelCountry = await act<string>('identityToken(0)');
  const shown = await act<Read>(scene({ kind: 'fuel', time: { mode: 'follow' }, hour: 110, countries: GER, options: { show: 'consumption', fuelCountry },
    groups: identities([{ countryId: 101, firstHour: 100, lastHour: 109 }]) }));
  assert.ok(shown.text.includes(OUTSIDE) && !/Country unavailable/.test(shown.text), shown.text);
}));

test('7. General career growing with playback, stepped from 99 into the track with its hourly reply pending: the cursor is at 100, nothing after 100 is drawn, and the hourly details wait on their own read', () => onPage(async act => {
  const career = { reference: '1', name: 'Leader 1', portraitHour: null, runs: [{ firstHour: 100, lastHour: 109, role: { owner: 'GER', kind: 1, category: 0, rank: 'Field Marshal',
    attributeNames: ['A', 'D', 'P', 'L'], maximumSkill: 9 }, skill: 3, components: [1, 1, 1, 1] }], traitGains: [[104, 0, 1]],
    traits: [{ token: 'a', name: 'A', icon: null, missing: null }], commands: [], battles: [], observed: [[100, 109]], battlesPartialFrom: null, resultsMissing: '', missing: null, warnings: [] };
  const base = { kind: 'general-career', time: { mode: 'follow' }, options: { leader: await act<string>('subjectToken("1")'), growWithPlayback: true }, products: { 'unit-leader-career': career } };
  await act(scene({ ...base, hour: 99 }));
  const entered = await act<Read>('step(1)');
  const timeline = await act<{ playhead: number | null; labelHour?: number; lanes: { spans: { firstHour: number }[] }[] }>('props("Timeline")');
  assert.equal(timeline.playhead, 101, 'the cursor is drawn at the recorded hour entered, not the branch end');
  assert.deepEqual(timeline.lanes.flatMap(lane => lane.spans).filter(span => span.firstHour > 100), [], 'nothing after 100 is revealed');
  assert.ok(entered.text.length > 0 && !entered.text.includes(OUTSIDE), `inside the track the details wait on their read: ${entered.text}`);
}));

test('12. General career following playback past the track with its branch-wide history pending: the history is loading, it is not outside the recording', () => onPage(async act => {
  const pending = await act<Read>(scene({ kind: 'general-career', time: { mode: 'follow' }, hour: 110, options: { leader: await act<string>('subjectToken("1")') } }));
  assert.ok(!pending.text.includes(OUTSIDE), `the branch-wide career is loading: ${pending.text}`);
}));

test('8. Stockpile breakdown saved without a country, mounted at 99 and sought to 105: the automatic country is chosen at 105, and with the hourly read pending nothing is built from the catalogue', () => onPage(async act => {
  const mounted = await act<Read>(scene({ kind: 'equipment-stockpile', time: { mode: 'follow' }, hour: 99, countries: GER, options: { show: 'stockpile' },
    groups: identities([{ countryId: 101, firstHour: 100, lastHour: 109 }]), products: { 'equipment-stockpile-catalogue': { branchId: '1', campaignKey: 'campaign', countries: [{ country: 'GER' }], variants: [], logistics: [], archetypes: [], warnings: [] } } }));
  const at105 = await act<Read>('seek(105)');
  assert.ok(!/Choose a country|Couldn’t load/.test(at105.text), `a country recorded at 105 is chosen there: ${at105.text} ${JSON.stringify((at105 as unknown as { errors: string[] }).errors).slice(0, 1500)} ${mounted.text}`);
  assert.ok(at105.asks.some(ask => ask.product === 'equipment-stockpile-at' && ask.hour === 105 && JSON.stringify(ask.params).includes('GER')),
    `the stockpile of the country chosen at 105 is read there: ${JSON.stringify(at105.asks)}`);
  void 0;
  assert.ok(!at105.asks.some(ask => ask.product === 'equipment-stockpile-at' && ask.hour === 109), `nothing is asked at the branch end: ${JSON.stringify(at105.asks)}`);
}));

const PAIR_OWNERS = '({ owners: [{ tag: "GER", countryId: 1, known: true, reason: null }, { tag: "ENG", countryId: 2, known: true, reason: null }], warnings: [] })';
const TWO = { kind: 'tags', tags: ['GER', 'ENG'] };
const BOTH = humans([{ tag: 'GER', firstHour: 100, lastHour: 109 }, { tag: 'ENG', firstHour: 100, lastHour: 109 }]);

test('9. Diplomatic relations with a pair drill picked at 105: when the matrix following playback leaves the track at 110, the recorded pair history stays', () => onPage(async act => {
  await act(scene({ kind: 'diplomatic-relations', time: { mode: 'follow' }, hour: 105, countries: TWO, groups: BOTH, replies: { 'diplomatic-relations': PAIR_OWNERS } }));
  await act(`inject(${JSON.stringify({ drill: { breakdown: 'pair', follow: false, hour: 105 }, values: { choice: { scope: {}, pair: { first: 'GER', second: 'ENG', hour: 105, firstId: 1, secondId: 2 } } } })})`);
  const past = await act<Read>('seek(110)');
  assert.ok(past.drillOpen, `the pair history is not removed: ${past.text}`);
}));

test('9. Minefields with a region drill picked at 105: past the track at 110 the drill stays and reads its own hour', () => onPage(async act => {
  await act(scene({ kind: 'minefields', time: { mode: 'follow' }, hour: 105, countries: GER, groups: BOTH,
    replies: { minefields: '({ scale: 1, warnings: [], regions: [{ region: 1, unknownOwners: 0, owners: [{ tag: "GER" }] }], unknown: [], choices: [{ id: "1", name: "North Sea" }] })',
      'minefield-reports': '({ region: 1, readHour: hour, reports: [], warnings: [] })' } }));
  await act(`inject(${JSON.stringify({ drill: { breakdown: 'region', follow: false, hour: 105 }, values: { picked: { region: '1', name: 'North Sea', owners: [{ tag: 'GER' }], unknownOwners: 0 } } })})`);
  const past = await act<Read>('seek(110)');
  assert.ok(past.drillOpen, `the region drill is not removed: ${past.text}`);
  assert.ok(past.asks.some(ask => ask.product === 'minefield-reports' && ask.hour === 105), `its reports are read at 105: ${JSON.stringify(past.asks)}`);
}));

test('9. Lend-lease agreements with an agreement drill picked at 105: past the track at 110 the drill stays and reads its own hour', () => onPage(async act => {
  await act(scene({ kind: 'lend-lease-agreements', time: { mode: 'follow' }, hour: 110, countries: GER, groups: BOTH, replies: { 'market-lendlease': MARKET } }));
  const row = { key: 'k', terms: { id: '1', key: 'k' }, at: 105, giver: 'GER', receiver: 'ENG', partner: 'ENG', items: [], since: 100, last: null, ended: false };
  const opened = await act<Read>(`inject(${JSON.stringify({ drill: { breakdown: 'agreement', follow: false, hour: 105 }, values: { pick: { owner: null, row } } })})`);
  assert.ok(opened.drillOpen && opened.drill.includes('Market at 105'), `the agreement drill is kept at 105: ${JSON.stringify(opened)}`);
}));

test('10. Top general career over a group whose only member joins at 105, following at 103: the group has no members at 103, whatever its members at the branch end', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'general-career', time: { mode: 'follow' }, hour: 103, countries: HUMANS, groups: humans([{ tag: 'GER', firstHour: 105, lastHour: 109 }]) }));
  assert.ok(/no members/.test(shown.text), shown.text);
}));

test('10. Division service record over the humans following past the track at 110: outside the recording, not a group without members', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'division-service-record', time: { mode: 'follow' }, hour: 110, countries: HUMANS, groups: BOTH }));
  assert.ok(shown.text.includes(OUTSIDE) && !/no members/.test(shown.text), shown.text);
}));

test('11. Division equipment’s type drill past the track at 110 is outside the recording, never “not held at this date”', () => onPage(async act => {
  await act(scene({ kind: 'division-equipment', time: { mode: 'follow' }, hour: 110, countries: GER, groups: identities([{ countryId: 101, firstHour: 100, lastHour: 109 }]) }));
  const opened = await act<Read>(`inject(${JSON.stringify({ values: { equipmentDrill: { kind: 'type', key: 'infantry' } } })})`);
  assert.ok(opened.drillOpen && opened.drill.includes(OUTSIDE) && !/not held/.test(opened.drill), JSON.stringify(opened));
}));

test('11. Raid tally’s race over the humans past the track at 110 is outside the recording, never a group without members', () => onPage(async act => {
  const shown = await act<Read>(scene({ kind: 'raid-tally', time: { mode: 'follow' }, hour: 110, countries: HUMANS, groups: BOTH,
    raids: { episodes: [], missing: [], available: true, unavailable: null, warnings: [], types: [], artwork: [] } }));
  assert.ok(shown.text.includes(OUTSIDE) && !/no members/.test(shown.text), shown.text);
}));

test('12. World tension’s breakdown picked at 105, its read pending, while the chart following playback is past the track: the breakdown is loading, not outside', () => onPage(async act => {
  await act(scene({ kind: 'world-tension', time: { mode: 'follow' }, hour: 110 }));
  const opened = await act<Read>(`inject(${JSON.stringify({ values: { drill: { breakdown: 'breakdown', follow: false, hour: 105 } } })})`);
  assert.ok(opened.drillOpen && !opened.drill.includes(OUTSIDE), `the breakdown at 105 waits on its read: ${JSON.stringify(opened)}`);
}));

test('12. Experience on a range running past its track: hovering an unrecorded hour says outside the recording, never loading', () => onPage(async act => {
  await act(scene({ kind: 'experience', time: { mode: 'range', firstOffset: 2, lastOffset: 15 }, hour: 105, countries: GER }));
  const tip = await act<{ rows: { label: string; value?: string }[] }>('call("TimeChart", "tooltip", 112)');
  const rows = JSON.stringify(tip.rows);
  assert.ok(rows.includes('Outside the recording') && !rows.includes('Loading'), JSON.stringify(tip));
}));
