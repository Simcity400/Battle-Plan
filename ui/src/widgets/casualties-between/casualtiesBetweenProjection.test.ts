import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { BarsDataRow } from '../../families/bars/model.ts';
import type { CasualtyMatrixReply } from '../../generated/dashboard';
import { casualtyMatrixDeclaration, casualtyMatrixProjection, projectCasualtyMatrix, projectCasualtyOpponents } from './casualtiesBetweenProjection.ts';
import { EVER_UNRECORDED } from '../../dashboard/groups.ts';
import { NO_MEMBERS } from '../../dashboard/noMembers.ts';
import { cumulativeEverMembership, datedIdentityMembership } from '../../families/groupFilter.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { deriveMatrixAudit, deriveMatrixMirror, deriveMatrixTooltip, matrixCellAt } from '../../families/heat/matrixModel.ts';

const HOUR = 60_810_000;
let nextIdentity = 1;
const country = (tag: string, dealt: string | null, taken: string | null, slot = nextIdentity++, countryId = slot * 10) => ({
  tag, slot, countryId, factionId: null, factionName: null, factionNameMissing: null, dealt, taken,
  missing: dealt === null || taken === null ? 'A war relation\'s casualties were not recorded at this hour' : null,
});

const reply: CasualtyMatrixReply = {
  countries: [country('GER', null, '100'), country('POL', '100', '900'), country('FRA', '0', '0')],
  order: [0, 1, 2],
  cells: [
    { row: 0, column: 1, dealt: '900', missing: null, incomplete: null },
    { row: 1, column: 0, dealt: '100', missing: null, incomplete: 'Part of this pair\'s history fell in a recording gap; the figure may be low' },
    { row: 0, column: 2, dealt: null, missing: 'A war relation\'s casualties were not recorded at this hour', incomplete: null },
    { row: 2, column: 0, dealt: '0', missing: null, incomplete: null },
  ],
  most: '900',
  largestDealt: '100',
  subject: null,
  opponents: [],
  largestSide: '0',
  missing: null,
  warnings: [],
};

const view = (data: unknown) => createBoundedReadView({ hour: HOUR, panels: new Map([['main', data]]) });

test('casualty matrix: not recorded pairs and totals are hatched, a recorded zero is empty, never a guess', () => {
  const model = projectCasualtyMatrix(view(reply));
  assert.deepEqual(matrixCellAt(model, 0, 0), { kind: 'diagonal', text: '—' });
  const big = matrixCellAt(model, 0, 1);
  assert.equal(big.kind === 'value' && big.text, '900');
  assert.equal(big.kind === 'value' && big.intensity, 1);
  assert.equal(matrixCellAt(model, 0, 2).kind, 'missing');
  assert.deepEqual(matrixCellAt(model, 2, 0), { kind: 'empty', label: '0 casualties', audit: '0' });
  assert.equal(matrixCellAt(model, 1, 2).kind, 'empty');
  assert.equal(model.margin!.cell(0).kind, 'missing');
  const total = model.margin!.cell(1);
  assert.equal(total.kind === 'value' && total.fraction, 1);
  assert.ok(model.legend?.entries?.some(entry => entry.hatched));
});

test('one country by opponent: taken on the left, dealt on the right, missing never zero', () => {
  const opponents: CasualtyMatrixReply = {
    ...reply,
    subject: 0,
    order: [],
    cells: [],
    opponents: [
      { country: 1, dealt: '900', taken: '100', missing: null, incomplete: null },
      { country: 2, dealt: null, taken: null, missing: 'A war relation\'s casualties were not recorded at this hour', incomplete: null },
    ],
    largestSide: '900',
  };
  const model = projectCasualtyOpponents(view(opponents));
  const [poland, france] = model.rows as BarsDataRow[];
  assert.equal(poland!.left!.numeric, 100);
  assert.equal(poland!.right!.numeric, 900);
  assert.equal(france!.left!.numeric, null);
  assert.equal(france!.left!.value.value.kind, 'notRecorded');
  assert.equal(model.scaleMax, 900);
  assert.equal(model.summaryText, undefined, 'a total that is not recorded is not shown');
});

test('group selections filter casualty matrix at the displayed hour for both touching and between modes', () => {
  const groupReply: CasualtyMatrixReply = {
    countries: [
      country('AAA', '300', '100'),
      country('BBB', '100', '200'),
      country('CCC', '900', '100'),
      country('DDD', null, '900'),
    ],
    order: [0, 1, 2, 3],
    cells: [
      { row: 0, column: 1, dealt: '200', missing: null, incomplete: null },
      { row: 0, column: 2, dealt: '100', missing: null, incomplete: null },
      { row: 2, column: 3, dealt: '900', missing: null, incomplete: null },
      { row: 3, column: 2, dealt: null, missing: 'Not read', incomplete: null },
    ],
    most: '900',
    largestDealt: '900',
    subject: null,
    opponents: [],
    largestSide: '0',
    missing: null,
    warnings: [],
  };
  const at = (hour: number) => createBoundedReadView({ hour, panels: new Map([['main', groupReply]]) });
  const membership = { AAA: [[100, 299]] as const, BBB: [[100, 299]] as const, CCC: [[600, 900]] as const };
  const unobserved = [[500, 550]] as const;

  const touching = casualtyMatrixProjection.project(at(200), { membership, unobserved, membershipMode: 'hour', between: false });
  assert.deepEqual(touching.rows.map(row => row.id), ['AAA', 'BBB', 'CCC']);
  const cell01 = matrixCellAt(touching, 0, 1);
  const cell02 = matrixCellAt(touching, 0, 2);
  assert.equal(cell01.kind === 'value' && cell01.intensity, 1);
  assert.equal(cell02.kind === 'value' && cell02.text, '100');
  assert.equal(touching.legend?.gradient?.highLabel, '200 casualties');
  assert.ok(!touching.legend?.entries?.some(entry => entry.hatched));
  assert.equal(casualtyMatrixProjection.nextChange?.(at(200), { membership, unobserved, membershipMode: 'hour', between: false }), 300);

  const between = casualtyMatrixProjection.project(at(200), { membership, unobserved, membershipMode: 'hour', between: true });
  assert.deepEqual(between.rows.map(row => row.id), ['AAA', 'BBB']);
  assert.deepEqual(between.margin!.cell(0), { kind: 'value', fraction: 1, text: '300', exact: '300' });

  const emptyBetween = casualtyMatrixProjection.project(at(400), { membership, unobserved, membershipMode: 'hour', between: false });
  assert.equal(emptyBetween.empty, NO_MEMBERS.hour);
  assert.equal(emptyBetween.emptyKind, undefined);

  const unrecorded = casualtyMatrixProjection.project(at(525), { membership, unobserved, membershipMode: 'hour', between: false });
  assert.equal(unrecorded.emptyKind, 'unavailable');
  assert.equal(unrecorded.rows.length, 0);
  assert.equal(casualtyMatrixProjection.nextChange?.(at(525), { membership, unobserved, membershipMode: 'hour', between: false }), 551);

  const everOpts = { membership: { AAA: [[100, 499]] as const }, unobserved: [[500, Number.MAX_SAFE_INTEGER]] as const, membershipMode: 'ever' as const, between: false };
  assert.equal(casualtyMatrixProjection.project(at(50), everOpts).empty, NO_MEMBERS.yet);
  const everUnseen = casualtyMatrixProjection.project(at(525), everOpts);
  assert.equal(everUnseen.emptyKind, 'unavailable');
  assert.equal(everUnseen.empty, EVER_UNRECORDED);

  const pastEverGroups: CountryGroupReply = {
    branchId: '1', warnings: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    humans: [{ tag: 'AAA', firstHour: 0, lastHour: 5 }],
    countryIdentities: [{ tag: 'AAA', identities: [{ slot: 1, countryId: 10, firstHour: 0, lastHour: 10 }] }],
    unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
  };
  const pastMembership = cumulativeEverMembership(
    { kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false },
    pastEverGroups,
    100,
  );
  const pastReply: CasualtyMatrixReply = {
    countries: [country('AAA', '125', '0'), country('BBB', '0', '125')],
    order: [0, 1],
    cells: [{ row: 0, column: 1, dealt: '125', missing: null, incomplete: null }],
    most: '125',
    largestDealt: '125',
    subject: null,
    opponents: [],
    largestSide: '0',
    missing: null,
    warnings: [],
  };
  const pastMember = casualtyMatrixProjection.project(
    createBoundedReadView({ hour: 15, panels: new Map([['main', pastReply]]) }),
    { membership: pastMembership, unobserved: [], membershipMode: 'ever', between: false },
  );
  assert.deepEqual(pastMember.rows.map(row => row.id), ['AAA', 'BBB']);
  const pastCell = matrixCellAt(pastMember, 0, 1);
  assert.equal(pastCell.kind === 'value' && pastCell.text, '125');

  const renamedEverGroups: CountryGroupReply = {
    branchId: '1', warnings: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    humans: [{ tag: 'AAA', firstHour: 0, lastHour: 5 }],
    countryIdentities: [
      { tag: 'AAA', identities: [{ slot: 1, countryId: 10, firstHour: 0, lastHour: 9 }, { slot: 2, countryId: 20, firstHour: 12, lastHour: 100 }] },
      { tag: 'BBB', identities: [{ slot: 1, countryId: 10, firstHour: 10, lastHour: 100 }] },
    ],
    unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
  };
  const renamedMembership = cumulativeEverMembership(
    { kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false },
    renamedEverGroups,
    100,
  );
  assert.deepEqual(renamedMembership, { AAA: [[0, 9]], BBB: [[10, 100]] });

  const crossReply: CasualtyMatrixReply = {
    countries: [country('AAA', '300', '0'), country('BBB', '50', '200'), country('CCC', '0', '150')],
    order: [0, 1, 2],
    cells: [
      { row: 0, column: 1, dealt: '200', missing: null, incomplete: null },
      { row: 0, column: 2, dealt: '100', missing: null, incomplete: null },
      { row: 1, column: 2, dealt: '50', missing: null, incomplete: null },
    ],
    most: '200',
    largestDealt: '300',
    subject: null,
    opponents: [],
    largestSide: '0',
    missing: null,
    warnings: [],
  };
  const crossModel = casualtyMatrixProjection.project(
    createBoundedReadView({ hour: 200, panels: new Map([['main', crossReply]]) }),
    { membership: { AAA: [[100, 299]] as const }, unobserved: [], membershipMode: 'hour', between: false },
  );
  assert.deepEqual(crossModel.rows.map(row => row.id), ['AAA', 'BBB', 'CCC']);
  assert.equal(deriveMatrixTooltip(crossModel, 1, 2)?.items?.[0]?.label, 'Outside selected group');
  const crossAudit = deriveMatrixAudit(crossModel, { firstRow: 0, endRow: 3, firstCol: 0, endCol: 3 });
  assert.equal(crossAudit.find(entry => entry.key === 'BBB:CCC')?.value, 'Outside selection');
  const crossMirror = deriveMatrixMirror(crossModel, 1, 2, 0, 3);
  assert.equal(crossMirror.rows[0]!.cells[3]?.spoken, `${crossModel.columns[2]!.label}: Outside selection`);

  const reusedGroups: CountryGroupReply = {
    branchId: '1', warnings: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    humans: [{ tag: 'AAA', firstHour: 200, lastHour: 299 }],
    countryIdentities: [
      { tag: 'AAA', identities: [{ slot: 1, countryId: 10, firstHour: 0, lastHour: 99 }, { slot: 2, countryId: 20, firstHour: 200, lastHour: 300 }] },
      { tag: 'BBB', identities: [{ slot: 3, countryId: 30, firstHour: 0, lastHour: 300 }] },
    ],
    unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
  };
  const reusedReply: CasualtyMatrixReply = {
    countries: [
      country('AAA', '100', '0', 1, 10),
      country('AAA', '25', '0', 2, 20),
      country('BBB', '0', '125', 3, 30),
    ],
    order: [0, 1, 2],
    cells: [
      { row: 0, column: 2, dealt: '100', missing: null, incomplete: null },
      { row: 1, column: 2, dealt: '25', missing: null, incomplete: null },
    ],
    most: '100',
    largestDealt: '100',
    subject: null,
    opponents: [],
    largestSide: '0',
    missing: null,
    warnings: [],
  };
  const hourIds = datedIdentityMembership({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, reusedGroups, 300);
  const everIds = datedIdentityMembership({ kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false }, reusedGroups, 300);
  assert.deepEqual(hourIds, { '[2,20]': [[200, 299]] });
  assert.deepEqual(everIds, { '[2,20]': [[200, 300]] });

  const reusedFiltered = casualtyMatrixProjection.project(
    createBoundedReadView({ hour: 250, panels: new Map([['main', reusedReply]]) }),
    { membership: { AAA: [[200, 299]] as const }, identityMembership: hourIds, unobserved: [], membershipMode: 'hour', between: false },
  );
  assert.deepEqual(reusedFiltered.rows.map(row => row.id), ['AAA', 'BBB']);
  const liveCell = matrixCellAt(reusedFiltered, 0, 1);
  assert.equal(liveCell.kind === 'value' && liveCell.text, '25', 'group filter selects only the live holder identity, never the dead earlier holder');

  const reusedAll = casualtyMatrixProjection.project(
    createBoundedReadView({ hour: 250, panels: new Map([['main', reusedReply]]) }),
    {},
  );
  assert.deepEqual(reusedAll.rows.map(row => row.id), ['AAA#1:10', 'AAA#2:20', 'BBB'], 'unfiltered matrix disambiguates duplicate tags by holder identity');
  assert.equal(matrixCellAt(reusedAll, 0, 2).kind, 'value');
  assert.equal(matrixCellAt(reusedAll, 1, 2).kind, 'value');
  assert.equal(casualtyMatrixDeclaration.countries.mode === 'multi' && casualtyMatrixDeclaration.countries.datedAtHour, true);

  const missingTable: CasualtyMatrixReply = { ...crossReply, order: [], cells: [], missing: 'Casualty table not recorded' };
  const emptyBeforeJoin = casualtyMatrixProjection.project(
    createBoundedReadView({ hour: 200, panels: new Map([['main', missingTable]]) }),
    { membership: { AAA: [[600, 700]] as const }, unobserved: [], membershipMode: 'hour', between: false },
  );
  assert.equal(emptyBeforeJoin.empty, 'This group has no members at this moment.');
  assert.equal(emptyBeforeJoin.emptyKind, undefined);
});
