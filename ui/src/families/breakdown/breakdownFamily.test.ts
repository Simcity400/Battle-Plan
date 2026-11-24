import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import type { Breakdown, BreakdownPart as WirePart, ConstructionBreakdownReply, PowerSpendingBreakdownReply, TerritoryBreakdownReply } from '../../generated/dashboard';
import { constructionProjection } from '../../widgets/construction-spending/constructionProjection.ts';
import { ppSpendingBreakdown, ppSpendingProjection } from '../../widgets/political-power-spending-breakdown/ppSpendingProjection.ts';
import { territoryBreakdown, territoryBreakdownProjection } from '../../widgets/territory-breakdown/territoryBreakdownProjection.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { IdentityProvider } from '../plan.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import { resolvePlaybackProvenInterval } from '../broker.ts';
import type { FamilyTheme } from '../contract.ts';
import { assertFreshPaint, assertQuietFrames, driveLane, installRecordingCanvas } from '../laneRuntime.fixture.ts';
import { registerFamilyProjection } from '../registry.ts';
import { parseBreakdownHit } from './hostAdapter.ts';
import { familyBreakdownLaneView } from './laneView.ts';
import { hitTestBreakdown, layoutBreakdown, type PartsViewLayout, type RingViewLayout, type MosaicViewLayout } from './layout.ts';
import { buildBreakdownTooltip, partColour, breakdownLevel, PATH, type BreakdownModel } from './model.ts';
import { deriveBreakdownAudit, deriveBreakdownNavAndMirror } from './structure.ts';
import { breakdownParts, breakdownTotal, type BreakdownWords } from './product.ts';

const THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  dark: true,
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  shortage: '#ff6b6f',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8c95a6',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

const START = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const WORDS: BreakdownWords = { unit: 'PP', verb: 'Spent', format: raw => raw.toLocaleString('en-US') };
const measure = (text: string) => text.length * 7;

function wire(key: string, ordinal: number, raw: string | null, extra: Partial<WirePart> = {}): WirePart {
  return {
    key,
    name: key,
    nameMissing: null,
    ordinal,
    other: false,
    raw,
    state: raw === null ? 'notRecorded' : 'known',
    reason: raw === null ? `${key} was not recorded` : null,
    share: null,
    parentShare: null,
    count: null,
    last: null,
    art: null,
    children: [],
    ...extra,
  };
}

function partlyKnown(): Breakdown {
  return {
    decimals: 0,
    total: { raw: '211', state: 'atLeast', reason: 'New ships were not recorded' },
    parts: [wire('Repairs', 2, '211'), wire('New ships', 0, null), wire('Refits', 1, null)],
    unrecordedHours: 0,
  };
}

function spent(): Breakdown {
  return {
    decimals: 0,
    total: { raw: '5972', state: 'partial', reason: 'Spending was not recorded' },
    parts: [
      wire('Daily costs', 3, '2071', { share: 0.35 }),
      wire('Decisions', 0, '1415', {
        share: 0.24,
        children: [wire('Plan A', 0, '640', { share: 0.11 }), wire('Plan B', 1, '352', { share: 0.059 })],
      }),
      wire('Other', 1, '63', { share: 0.01, other: true }),
    ],
    unrecordedHours: 2,
  };
}

function model(breakdown: Breakdown): BreakdownModel {
  return { hour: 10, rootLabel: 'Spent', total: breakdownTotal(breakdown, WORDS), parts: breakdownParts(breakdown, WORDS) };
}

test('the product decides the whole, its order and its shares; the family only reads them', () => {
  const read = model(spent());
  assert.deepEqual(read.parts.map(part => part.key), ['Daily costs', 'Decisions', 'Other'], 'drawn in the order the product gave');
  assert.equal(read.total.text, '5,972');
  assert.equal(read.total.partial, true);
  assert.equal(read.total.known, false);
  assert.equal(read.total.shortReason, '2 hours not recorded');
  assert.equal(read.parts[0]!.share, 0.35, 'a partial whole keeps its shares');
  assert.equal(read.parts[1]!.children![0]!.share, 0.11);
  assert.equal(read.parts[2]!.neutral, true);
  assert.equal(partColour(read.parts[2]!, THEME), THEME.grey, 'other is the reserved grey');
});

test('colours follow the recorded ordinal, so a reordered breakdown keeps them', () => {
  const read = model(spent());
  const reordered = model({ ...spent(), parts: [...spent().parts].reverse() });
  for (const part of read.parts) {
    const same = reordered.parts.find(item => item.key === part.key)!;
    assert.equal(partColour(same, THEME), partColour(part, THEME));
  }
});

test('a partly known whole is the figure with its partial mark and no shares, with parts not recorded never as zero', () => {
  const read = model(partlyKnown());
  assert.equal(read.total.atLeast, true);
  assert.equal(read.total.text, '211');
  assert.equal(read.parts[1]!.notRecorded, true);
  assert.equal(read.parts[1]!.value, null);
  assert.equal(read.parts[1]!.missingReason, 'New ships was not recorded');

  const parts = layoutBreakdown({ view: 'parts', level: breakdownLevel(read), width: 1111, height: 346, theme: THEME, namesInRing: true, userExpanded: {}, measure }) as PartsViewLayout;
  assert.ok(parts.totalLine.partialTabRect, 'the at-least total carries the partial tab');
  assert.ok(parts.rows.every(row => row.shareText === ''), 'no shares beside a lower bound');

  const ring = layoutBreakdown({ view: 'ring', level: breakdownLevel(read), width: 1111, height: 346, theme: THEME, namesInRing: true, userExpanded: {}, measure }) as RingViewLayout;
  assert.equal(ring.center.verb, 'Spent', 'no lower-bound wording');
  assert.ok(ring.innerSlices.every(slice => slice.label === null), 'no names on a hatched ring');
  assert.deepEqual(deriveBreakdownAudit(read, parts).map(row => row.value), ['atLeast:211', '211', 'notRecorded:New ships was not recorded', 'notRecorded:Refits was not recorded']);
});

test('the three views lay out the same parts', () => {
  const level = breakdownLevel(model(spent()));
  const base = { level, width: 1111, height: 346, theme: THEME, namesInRing: true, userExpanded: {}, measure };
  const parts = layoutBreakdown({ ...base, view: 'parts' }) as PartsViewLayout;
  assert.deepEqual(parts.rows.filter(row => row.depth === 0).map(row => row.key), ['Daily costs', 'Decisions', 'Other']);
  const ring = layoutBreakdown({ ...base, view: 'ring' }) as RingViewLayout;
  assert.equal(ring.openKey, 'Decisions', 'the first part with sub-parts opens in the list');
  assert.ok(ring.outerSlices.some(slice => slice.key === `Decisions${PATH}Plan A`));
  const mosaic = layoutBreakdown({ ...base, view: 'mosaic' }) as MosaicViewLayout;
  assert.ok(mosaic.tiles.some(tile => tile.key === 'Decisions' && tile.isGroup));
  const narrow = layoutBreakdown({ ...base, view: 'ring', width: 320, height: 220 }) as RingViewLayout;
  assert.equal(narrow.sideList, null, 'at minimum size the ring has no list');
});

test('expanding a part is held by the host, shared by the parts list and the ring list', () => {
  const read = model(spent());
  const level = breakdownLevel(read);
  const base = { level, width: 1111, height: 346, theme: THEME, namesInRing: true, measure };
  const closed = layoutBreakdown({ ...base, view: 'parts', userExpanded: { Decisions: false } }) as PartsViewLayout;
  assert.equal(closed.rows.some(row => row.parentKey === 'Decisions'), false);
  const open = layoutBreakdown({ ...base, view: 'parts', userExpanded: { Decisions: true } }) as PartsViewLayout;
  assert.deepEqual(open.rows.filter(row => row.parentKey === 'Decisions').map(row => row.key), [`Decisions${PATH}Plan A`, `Decisions${PATH}Plan B`]);
  const { nav } = deriveBreakdownNavAndMirror(open, level.total);
  assert.equal(nav.find(entry => entry[0] === 'Decisions')![3], 1, 'navigation reports the open part so a toggle closes it');
  const ring = layoutBreakdown({ ...base, view: 'ring', userExpanded: { Decisions: false } }) as RingViewLayout;
  assert.equal(ring.openKey, null, 'closing the default open part closes it');
  assert.deepEqual(parseBreakdownHit('toggle:Decisions'), { key: 'Decisions', subject: 'Decisions', kind: 'toggle', toggle: true, inspectToken: 'row:Decisions' });
});

function reply(breakdown: Breakdown, extra: Partial<PowerSpendingBreakdownReply> = {}): PowerSpendingBreakdownReply {
  return { from: START, hour: START + 10, through: START + 30, breakdown, gainedRaw: '5306', lostAtCapRaw: '1', gainedReason: null, warnings: [], ...extra };
}

function project(data: PowerSpendingBreakdownReply | null) {
  const readView = createBoundedReadView({ hour: START + 10, panels: new Map(data ? [['main', data]] : []), validInterval: [START + 10, START + 30] });
  return ppSpendingProjection.project(readView, { digits: 0 });
}

test('political power spending reads the product at the displayed hour', () => {
  const read = project(reply(spent()));
  assert.equal(read.periodLabel, 'Spent since 1 Jan 1936');
  assert.deepEqual(read.comparison, { label: 'Gained', value: '5,306 PP', note: '1 PP lost at the cap.' });
  assert.equal(read.total.text, '5,972');
  assert.deepEqual(read.parts.map(part => part.name), ['Daily costs', 'Decisions', 'Other']);

  const partly = project(reply(spent(), { gainedReason: 'Gains in hours when power changes were not read are not counted.' }));
  assert.deepEqual(partly.comparison, { label: 'Gained', value: '5,306 PP', partial: true, note: 'Gains in hours when power changes were not read are not counted. 1 PP lost at the cap.' });

  const ungained = project(reply(spent(), { gainedRaw: null, gainedReason: 'Some gains were not recorded.' }));
  assert.deepEqual(ungained.comparison, { label: 'Gained', value: 'Not recorded', note: 'Some gains were not recorded.' });

  const none = project(reply({ decimals: 0, total: { raw: null, state: 'notRecorded', reason: 'Spending was not recorded' }, parts: [], unrecordedHours: 10 }));
  assert.equal(none.empty, 'Not recorded at this time.');
  assert.equal(none.emptyKind, 'unavailable');
  assert.equal(none.emptyDetail, 'Spending was not recorded');

  const zero = project(reply({ decimals: 0, total: { raw: '0', state: 'known', reason: null }, parts: [], unrecordedHours: 0 }));
  assert.equal(zero.empty, 'No spending at this date.');
  assert.equal(zero.total.text, '0', 'a known zero is 0');

  const unrecorded = project(reply({ decimals: 0, total: { raw: '0', state: 'partial', reason: 'Spending was not recorded' }, parts: [], unrecordedHours: 5 }));
  assert.equal(unrecorded.empty, 'No spending recorded at this date.', 'unrecorded hours never read as no spending');
  assert.equal(unrecorded.emptyDetail, 'Spending was not recorded');

  registerFamilyProjection(ppSpendingProjection);
  const parts = { view: 'parts' as const, namesInRing: true, digits: 0 };
  const lane = (data: PowerSpendingBreakdownReply, options: typeof parts | { view: 'ring'; namesInRing: boolean; digits: number } = parts, expanded: Record<string, boolean> = {}) => ({
    family: 'breakdown', kind: ppSpendingProjection.kind, options, sources: {}, staticPanels: { main: data }, scrollTop: 0, theme: THEME, width: 600, height: 300, expanded,
  });
  const held = lane(reply(spent()));
  const surface = { width: 600, height: 300, dpr: 1 };
  assertQuietFrames(() => familyBreakdownLaneView(() => {}), { label: 'spending', binding: held, hour: START + 10 }, 24, surface);
  assertFreshPaint(() => familyBreakdownLaneView(() => {}), [
    { label: 'spent', binding: held, hour: START + 10, check: true },
    { label: 'later', binding: lane(reply(partlyKnown(), { hour: START + 20 })), hour: START + 20, speed: 24 },
    { label: 'seek back', binding: held, hour: START + 10, check: true },
    { label: 'same-hour correction', binding: lane(reply(spent(), { gainedRaw: '6000' })), hour: START + 10, check: true },
    { label: 'another view', binding: lane(reply(spent()), { view: 'ring', namesInRing: true, digits: 0 }), hour: START + 10, check: true },
    { label: 'a part opened', binding: lane(reply(spent()), parts, { Decisions: true }), hour: START + 10, check: true },
  ], surface);
});

test('nothing after the displayed hour reaches the projection', () => {
  const readView = createBoundedReadView({ hour: 10, panels: new Map([['main', reply(spent())]]) });
  assert.throws(() => readView.hover('main', 11), FutureDataError);
});

test('construction spending reads IC used since the branch began and names what it leaves out', () => {
  const reply: ConstructionBreakdownReply = {
    from: START, hour: START + 10, through: START + 20, beforeFirstRead: 2, unattributed: 1, warnings: [],
    breakdown: { decimals: 5, total: { raw: '162101200000', state: 'known', reason: null }, unrecordedHours: 0,
      parts: [wire('New buildings', 0, '158376400000', { share: 0.98 }), wire('Repairs', 1, '3724800000', { share: 0.023 })] },
  };
  const readView = createBoundedReadView({ hour: START + 10, panels: new Map([['main', reply]]), validInterval: [START + 10, START + 20] });
  const read = constructionProjection.project(readView, { digits: 0 });
  assert.equal(read.total.text, '1,621,012');
  assert.equal(read.total.unit, 'IC');
  assert.equal(read.periodLabel, 'Used since 1 Jan 1936');
  assert.deepEqual(read.parts.map(part => part.text), ['1,583,764', '37,248']);
  assert.deepEqual(read.warnings, [
    'Output of 2 projects before the recording began is not counted.',
    'Output of 1 project not traced to a country is not counted.',
  ]);
});

function nested(): Breakdown {
  const items = Array.from({ length: 11 }, (_, at) => wire(`item-${at}`, at, String(10 + at), { share: 0.001, parentShare: 0.01 }));
  return {
    decimals: 0,
    total: { raw: '2000', state: 'known', reason: null },
    parts: [
      wire('New', 0, '1415', {
        share: 0.7075,
        parentShare: 0.7075,
        children: [wire('type:7', 1, '640', { share: 0.32, parentShare: 640 / 1415, children: items }), wire('type:8', 0, '352', { share: 0.176, parentShare: 352 / 1415 })],
      }),
      wire('Repairs', 1, '585', { share: 0.2925, parentShare: 0.2925, children: [wire('type:7', 0, '585', { share: 0.2925, parentShare: 1 })] }),
    ],
    unrecordedHours: 0,
  };
}

const at = (width: number, height: number) => ({ width, height, theme: THEME, namesInRing: true, measure });

test('known children are drawn against their part, never stretched over it, and say whose share they are', () => {
  const level = breakdownLevel(model(nested()));
  const parts = layoutBreakdown({ ...at(1111, 346), level, view: 'parts', userExpanded: { New: false } }) as PartsViewLayout;
  const row = parts.rows.find(item => item.key === 'New')!;
  const covered = row.segments.reduce((sum, segment) => sum + segment.width, 0) + row.segments.length - 1;
  assert.ok(Math.abs(covered - row.barFillWidth * (992 / 1415)) < 1, 'children worth 992 of 1,415 cover that much of the bar');
  const ring = layoutBreakdown({ ...at(1111, 346), level, view: 'ring', userExpanded: {} }) as RingViewLayout;
  const inner = ring.innerSlices.find(slice => slice.key === 'New')!;
  const outer = ring.outerSlices.filter(slice => slice.parentKey === 'New');
  assert.ok(outer[outer.length - 1]!.a1 < inner.a1 - 0.1, 'the outer ring leaves the part unattributed to a child unfilled');
  const tip = buildBreakdownTooltip(model(nested()), level, `New${PATH}type:7`, THEME)!;
  assert.ok(JSON.stringify(tip).includes('45%'), 'the share of the part is 640 of 1,415');
  const narrow = layoutBreakdown({ ...at(320, 220), level: breakdownLevel(model(spent())), view: 'ring', userExpanded: {} }) as RingViewLayout;
  assert.ok(narrow.center.partialTabRect, 'a partial total keeps its mark at minimum size');
  assert.equal(hitTestBreakdown(narrow, narrow.center.partialTabRect!.x + 3, narrow.center.partialTabRect!.y + 6, 0), 'row:__partial__');
});

test('a part is known by its whole path, and every painted tile is navigable', () => {
  const read = model(nested());
  const level = breakdownLevel(read);
  const keys = read.parts.flatMap(part => (part.children ?? []).map(child => child.key));
  assert.deepEqual(keys, [`New${PATH}type:7`, `New${PATH}type:8`, `Repairs${PATH}type:7`], 'one building type under two purposes is two parts');
  assert.equal(JSON.stringify(buildBreakdownTooltip(read, level, `Repairs${PATH}type:7`, THEME)).includes('585'), true, 'hovering repairs shows the repairs amount');
  const mosaic = layoutBreakdown({ ...at(1111, 346), level, view: 'mosaic', userExpanded: {} }) as MosaicViewLayout;
  const painted = mosaic.tiles.flatMap(tile => [tile.key, ...tile.children.map(child => child.key)]);
  const { nav } = deriveBreakdownNavAndMirror(mosaic, level.total);
  assert.deepEqual(nav.map(entry => entry[0]).sort(), painted.sort(), 'navigation is the installed tiles');
});

test('a child keeps its tint when the amounts reorder it', () => {
  const swapped = nested();
  const children = swapped.parts[0]!.children;
  swapped.parts[0] = { ...swapped.parts[0]!, children: [children[1]!, children[0]!] };
  const tint = (breakdown: Breakdown) => {
    const parts = layoutBreakdown({ ...at(1111, 346), level: breakdownLevel(model(breakdown)), view: 'parts', userExpanded: { New: false } }) as PartsViewLayout;
    return parts.rows.find(row => row.key === 'New')!.segments.find(segment => segment.key === `New${PATH}type:8`)!.color;
  };
  assert.equal(tint(swapped), tint(nested()));
});

test('the list scrolls to its last row and shows every item under an open part', () => {
  const level = breakdownLevel(model(nested()));
  const parts = layoutBreakdown({ ...at(1111, 200), level, view: 'parts', userExpanded: { New: true } }) as PartsViewLayout;
  const last = parts.rows[parts.rows.length - 1]!;
  assert.ok(parts.contentHeight >= parts.bodyTop + last.y + last.height, 'the scroll extent counts the total line above the list');
  assert.equal(parts.rows.filter(row => row.depth === 2).length, 11, 'every item of an open category is a row');
  const tip = JSON.stringify(buildBreakdownTooltip(model(nested()), level, `New${PATH}type:7`, THEME));
  assert.ok(tip.includes('3 more in the list'), 'the tooltip names what it leaves to the list');
  const row = parts.rows[3]!;
  assert.equal(hitTestBreakdown(parts, 40, parts.bodyTop + row.y + 2, 0), `row:${row.key}`);
});

test('the audit lists what the installed view draws: shares, open children and the comparison note', () => {
  const read: BreakdownModel = { ...model(nested()), comparison: { label: 'Gained', value: 'Not recorded', note: 'Some gains were not recorded.' } };
  const level = breakdownLevel(read);
  const closed = layoutBreakdown({ ...at(1111, 346), level, view: 'parts', userExpanded: { New: false } });
  const open = layoutBreakdown({ ...at(1111, 346), level, view: 'parts', userExpanded: { New: true } });
  const rows = deriveBreakdownAudit(read, open);
  assert.ok(rows.some(row => row.key === '__comparison__' && String(row.value).includes('Some gains were not recorded.')));
  assert.ok(rows.some(row => row.key === 'New' && String(row.value).endsWith('|71%')), 'a drawn share is audited');
  assert.ok(deriveBreakdownAudit(read, open).length > deriveBreakdownAudit(read, closed).length, 'opening a part audits its drawn children');
  const moved = nested();
  moved.parts[0] = { ...moved.parts[0]!, children: [{ ...moved.parts[0]!.children[0]!, raw: '700' }, { ...moved.parts[0]!.children[1]!, raw: '292' }] };
  assert.notDeepEqual(deriveBreakdownAudit(model(moved), layoutBreakdown({ ...at(1111, 346), level: breakdownLevel(model(moved)), view: 'parts', userExpanded: { New: true } })), deriveBreakdownAudit(model(nested()), layoutBreakdown({ ...at(1111, 346), level: breakdownLevel(model(nested())), view: 'parts', userExpanded: { New: true } })), 'moving spending between children under a fixed part changes the audit');
});

test('a lone sub-part that repeats its part is not drawn as a split, but its name stays in the tooltip', () => {
  const lone: Breakdown = { decimals: 0, total: { raw: '20', state: 'known', reason: null }, unrecordedHours: 0,
    parts: [wire('Daily', 0, '20', { share: 1, parentShare: 1, children: [wire('One daily cost', 0, '20', { share: 1, parentShare: 1 })] })] };
  const read = model(lone);
  const parts = layoutBreakdown({ ...at(1111, 346), level: breakdownLevel(read), view: 'parts', userExpanded: {} }) as PartsViewLayout;
  assert.deepEqual(parts.rows.map(row => row.key), ['Daily']);
  assert.equal(parts.rows[0]!.segments.length, 0);
  assert.ok(JSON.stringify(buildBreakdownTooltip(read, breakdownLevel(read), 'Daily', THEME)).includes('One daily cost'));
});

test('a lone item under a category is still a row', () => {
  const single: Breakdown = { decimals: 0, total: { raw: '20', state: 'known', reason: null }, unrecordedHours: 0,
    parts: [wire('Decisions', 0, '20', { share: 1, parentShare: 1, children: [
      wire('Group', 0, '12', { share: 0.6, parentShare: 0.6, children: [wire('Only decision', 0, '12', { share: 0.6, parentShare: 1 })] }),
      wire('Other group', 1, '8', { share: 0.4, parentShare: 0.4 }),
    ] })] };
  const parts = layoutBreakdown({ ...at(1111, 346), level: breakdownLevel(model(single)), view: 'parts', userExpanded: { Decisions: true } }) as PartsViewLayout;
  assert.ok(parts.rows.some(row => row.depth === 2 && row.part.name === 'Only decision'));
});

test('territory names each recorded slot by its identity at the hour, with its flag and colour, and leaves unnamed land as the product gave it', () => {
  const asked: string[] = [];
  const identity: IdentityProvider = {
    resolve(kind, key, atHour) {
      asked.push(key);
      const tag = key === countryKey(1) ? 'BBB' : key;
      return {
        kind, key, atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: tag },
        name: { status: 'ready', value: `${tag} land` },
        colour: { status: 'ready', value: '#4f86f7' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
  const territory: TerritoryBreakdownReply = { hour: 10, warnings: [], breakdown: {
    decimals: 2, unrecordedHours: 0, total: { raw: '583', state: 'known', reason: null },
    parts: [wire('slot:1', 1, '433', { share: 0.74, name: 'AAA' }), wire('slot:2', 2, '100', { other: true, name: 'Country not named at this hour' }), wire('~unheld', 9, '50', { other: true, name: 'Held by no country' })],
  } };
  const project = (reply: TerritoryBreakdownReply, chosen = false) => territoryBreakdownProjection.project(
    createBoundedReadView({ hour: 10, panels: new Map([['main', reply]]), identity }), { measure: 'states', rule: 'controlled', chosen });
  const read = project(territory);
  assert.deepEqual(read.parts.map(part => part.name), ['BBB land', 'Country not named at this hour', 'Held by no country'], 'the name is the one the slot holds at the hour, not the tag the product read');
  assert.deepEqual([...new Set(asked)], [countryKey(1)], 'only a named slot is asked, by its recorded slot');
  assert.equal(read.parts[0]!.color, '#4f86f7');
  assert.equal(read.parts[0]!.flag, 'BBB');
  assert.equal(read.parts[1]!.flag, undefined, 'a slot no tag names has no flag');
  assert.equal(read.total.text, '5.83');
  assert.equal(read.total.unit, 'states');
  assert.equal(read.periodLabel, 'Controlled by every country');
  assert.equal(project(territory, true).periodLabel, 'Controlled by the chosen countries');
  const unread = project({ ...territory, breakdown: { decimals: 0, unrecordedHours: 0, total: { raw: null, state: 'notRecorded', reason: 'territory by strategic region is not known at this hour' }, parts: [] } });
  assert.equal(unread.empty, 'Not recorded at this time.');
  assert.equal(unread.emptyDetail, 'Territory by strategic region is not known at this hour');
});

test('territory asks the product for the chosen countries or the group itself, never an empty list', () => {
  const ctx = (countries: CountrySelection) => ({
    widget: { countries },
    options: { measure: 'states', rule: 'controlled' },
    countries: { all: countries.kind === 'all', tags: [], top: null },
    track: { source: { id: 'source', revision: '1', metadata: {} }, axisOffset: 0, selectedSlot: 7 },
    branch: { id: '1' },
  }) as unknown as Parameters<typeof territoryBreakdown.sources>[0];
  const params = (countries: CountrySelection) => (territoryBreakdown.sources(ctx(countries), { width: 400, height: 300 } as Parameters<typeof territoryBreakdown.sources>[1]).entries[0]!.read as { params: unknown }).params;
  const base = { branchId: '1', measure: 'states', rule: 'controlled' };
  assert.deepEqual(params({ kind: 'all' }), base);
  assert.deepEqual(params({ kind: 'selected' }), { ...base, countries: [7] }, 'the selected country is its recorded slot');
  assert.deepEqual(params({ kind: 'tags', tags: ['AAA', countryKey(9)] }), { ...base, countries: ['AAA', 9] });
  assert.deepEqual(params({ kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false }),
    { ...base, group: { type: 'faction', id: '42', membership: 'hour' } }, 'a group is sent as the group: Rust resolves its members at the hour');
  assert.match(territoryBreakdown.unavailable?.(ctx({ kind: 'group', group: { type: 'humans' }, membership: 'end', aggregate: false })) ?? '', /end of the recording/,
    'members at the end of the recording would read past the displayed hour: refused by name');
  assert.equal(territoryBreakdown.unavailable?.(ctx({ kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false })), null);
});

test('a breakdown reply proves the stretch it stands through, so a quiet hundred played hours asks once', () => {
  const ctx = {
    countryTag: 'AAA',
    track: { source: { id: 'source', revision: '1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: START },
  } as unknown as Parameters<typeof ppSpendingBreakdown.sources>[0];
  const entry = ppSpendingBreakdown.sources(ctx, { width: 800, height: 300 } as Parameters<typeof ppSpendingBreakdown.sources>[1]).entries[0]!;
  const quiet = reply(spent(), { hour: START + 10, through: START + 110 });
  const proof = resolvePlaybackProvenInterval(entry.read, quiet, { anchorHour: START + 10, span: null }, START + 10_000);
  assert.deepEqual(proof, [START + 10, START + 110], 'one reply stands for every hour up to its next change');
  let asks = 1;
  for (let hour = START + 11; hour <= START + 110; hour++) if (hour > proof[1]) asks++;
  assert.equal(asks, 1);
});

test('the art asked for is the flags the laid-out rows draw, found once per layout, never a collapsed child and never again for a hover', async () => {
  const ready = (url: string) => ({ status: 'ready', assetId: url, url, origin: 'game', file: url, frame: { index: 0, count: 1, width: 18, height: 12 }, provenance: 'current' }) as never;
  const flagged = model(spent());
  const parts = flagged.parts.map(part => ({ ...part, flagArt: ready(`asset://${part.key}.png`), children: part.children?.map(child => ({ ...child, flagArt: ready(`asset://${child.key}.png`) })) }));
  registerFamilyProjection({ kind: 'test-breakdown-art', family: 'breakdown', project: () => ({ ...flagged, parts }) });
  const fetched: string[] = [];
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(familyBreakdownLaneView(() => {}, { fetchBitmap: async url => { fetched.push(url); return { width: 18, height: 12, close() {} } as never; } }), recording.canvases, { width: 600, height: 300, dpr: 1 });
    const binding = { family: 'breakdown', kind: 'test-breakdown-art', options: { view: 'parts', namesInRing: true, digits: 0 }, sources: {}, scrollTop: 0, theme: THEME, width: 600, height: 300, expanded: { Decisions: false } };
    lane.paint({ label: 'closed', binding, hour: START + 10 });
    for (let i = 0; i < 8; i++) await null;
    assert.ok(fetched.length > 0 && fetched.every(url => !url.includes('Plan')), `a collapsed part's children are not asked for: ${fetched.join(', ')}`);
    const asked = fetched.length;
    lane.paint({ label: 'hovered', binding, hour: START + 10, inspect: 'row:Decisions' });
    for (let i = 0; i < 8; i++) await null;
    assert.equal(fetched.length, asked, 'a hover asks for nothing more');
  } finally {
    recording.restore();
  }
});

test('a name the recording does not hold is the key with its reason, never passed off as a name', () => {
  const unnamed: Breakdown = { decimals: 0, unrecordedHours: 0, total: { raw: '5', state: 'known', reason: null },
    parts: [wire('group_a_cat', 0, '5', { share: 1, parentShare: 1, nameMissing: 'the recorded localisation holds no name for group_a_cat' })] };
  const read = model(unnamed);
  assert.equal(read.parts[0]!.name, 'group_a_cat');
  assert.equal(read.parts[0]!.nameMissing, 'The recorded localisation holds no name for group_a_cat');
  assert.ok(JSON.stringify(buildBreakdownTooltip(read, breakdownLevel(read), 'group_a_cat', THEME)).includes('The recorded localisation holds no name'));
});
