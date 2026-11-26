import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { datedIdentity } from '../../families/identity/dated.fixture.ts';
import { deriveMatrixAudit, deriveMatrixMirror, deriveMatrixTooltip, matrixCellAt } from '../../families/heat/matrixModel.ts';
import type { OperativeMatrixReply } from '../../generated/dashboard';
import { operativeMatrixProjection, operativePairsProjection, projectOperativeMatrix, projectOperativePairs } from './whoSpiesProjection.ts';
import { EVER_UNRECORDED } from '../../dashboard/groups.ts';
import { NO_MEMBERS } from '../../dashboard/noMembers.ts';

const HOUR = 60_810_000;
const agent = (id: number) => ({ id: String(id), name: `Agent ${id}`, nameMissing: null });

const reply: OperativeMatrixReply = {
  countries: ['ENG', 'GER', 'ITA', 'SOV'].map(tag => ({ tag, factionId: null, factionName: null, factionNameMissing: null })),
  rows: [
    { country: 0, capacity: { kind: 'known', active: 7, maximum: 7 }, missing: null },
    { country: 1, capacity: { kind: 'known', active: 8, maximum: 8 }, missing: null },
    { country: 2, capacity: { kind: 'missing', reason: 'Agencies were not read' }, missing: 'An operative\'s target country was not recorded at this hour' },
    { country: 3, capacity: { kind: 'noAgency' }, missing: null },
  ],
  columns: [0, 1, 2, 3],
  cells: [
    { row: 0, column: 0, home: true, operatives: [agent(1), agent(2)] },
    { row: 0, column: 1, home: false, operatives: [agent(3), agent(4), agent(5)] },
    { row: 1, column: 0, home: false, operatives: [agent(6)] },
  ],
  most: 3,
  largestMaximum: 8,
  pairs: [{ spy: 0, target: 1, operatives: [agent(3), agent(4), agent(5)] }, { spy: 1, target: 0, operatives: [agent(6)] }],
  readHour: HOUR - 12,
  missing: null,
  warnings: [],
};

const identity = datedIdentity(Object.fromEntries(['ENG', 'GER', 'ITA', 'SOV'].map(tag => [tag, [{ from: 0, name: tag }]])));
const view = (data: unknown) => createBoundedReadView({ hour: HOUR, identity, panels: new Map([['main', data]]) });

test('spy matrix: counter-intelligence on the diagonal, unknown pairs hatched never zero, agency capacity in the margin', () => {
  const model = projectOperativeMatrix(view(reply));
  assert.equal(model.shape, 'wide');
  assert.deepEqual(matrixCellAt(model, 0, 0), { kind: 'diagonal', outlined: true, text: '2' });
  assert.deepEqual(matrixCellAt(model, 1, 1), { kind: 'diagonal', outlined: true });
  const value = matrixCellAt(model, 0, 1);
  assert.equal(value.kind, 'value');
  assert.equal(value.kind === 'value' && value.text, '3');
  assert.equal(value.kind === 'value' && value.intensity, 1);
  assert.equal(matrixCellAt(model, 1, 2).kind, 'empty');
  assert.deepEqual(matrixCellAt(model, 2, 0), { kind: 'missing', reason: 'An operative\'s target country was not recorded at this hour' });
  assert.deepEqual(model.margin!.cell(0), { kind: 'value', fraction: 7 / 8, text: '7 / 7' });
  assert.deepEqual(model.margin!.cell(2), { kind: 'missing', reason: 'Agencies were not read' });
  assert.deepEqual(model.margin!.cell(3), { kind: 'none', label: 'No agency' });
  assert.ok(model.legend?.entries?.some(entry => entry.hatched), 'a not-recorded key is shown when a cell is hatched');
  const tip = deriveMatrixTooltip(model, 0, 1);
  assert.equal(tip?.title, 'ENG has 3 operatives in GER');
  assert.equal(tip?.hour, HOUR - 12, 'dated by the census read hour');
  assert.deepEqual(deriveMatrixTooltip(model, 0, model.columns.length)?.items, [{ label: 'Active / maximum', value: '7 / 7' }]);
  const audit = deriveMatrixAudit(model, { firstRow: 0, endRow: model.rows.length, firstCol: 0, endCol: model.columns.length });
  assert.ok(audit.some(row => row.key === 'ENG:ENG' && row.row === 'ENG' && row.value === '2'));
  assert.ok(audit.some(row => row.key.startsWith('ITA:') && String(row.value).startsWith('Not recorded')));
  assert.ok(audit.some(row => row.key === 'ENG:margin' && row.value === '7 / 7'), 'the capacity margin is audited');
  assert.equal(deriveMatrixMirror(model, 0, 1, 0, model.columns.length).rows[0]!.cells.at(-1)!.column, model.margin!.header, 'and spoken in the mirror');
});

test('spy matrix: a spy with an operative on an unrecorded target does not know its own home cell either', () => {
  const model = projectOperativeMatrix(view(reply));
  assert.deepEqual(matrixCellAt(model, 2, 2), { kind: 'missing', reason: "An operative's target country was not recorded at this hour" });
  assert.deepEqual(matrixCellAt(model, 1, 1), { kind: 'diagonal', outlined: true }, 'a known spy without counter-intelligence keeps the outlined home cell');
  assert.deepEqual(matrixCellAt(model, 0, 0), { kind: 'diagonal', outlined: true, text: '2' }, 'a witnessed home cell is kept');
});

test('spy matrix: a census not recorded is the not-recorded state, not an empty matrix', () => {
  const model = projectOperativeMatrix(view({ ...reply, rows: [], columns: [], cells: [], pairs: [], missing: 'Operative memberships were not read' }));
  assert.equal(model.emptyKind, 'unavailable');
  assert.equal(model.emptyDetail, 'Operative memberships were not read');
  const none = projectOperativeMatrix(view({ ...reply, rows: [], columns: [], cells: [], pairs: [] }));
  assert.equal(none.empty, 'No operatives in the field at this date.');
  assert.equal(none.emptyKind, undefined);
});

test('top pairs keep the product order and scale', () => {
  const model = projectOperativePairs(view(reply));
  assert.deepEqual(model.rows.map(row => row.key), ['ENG>GER', 'GER>ENG']);
  assert.equal(model.scaleMax, 3);
});

test('group selections filter spies and pairs at the displayed hour and distinguish unrecorded membership', () => {
  const tags = ['AAA', 'BBB', 'CCC', 'DDD'];
  const groupReply: OperativeMatrixReply = {
    countries: tags.map(tag => ({ tag, factionId: null, factionName: null, factionNameMissing: null })),
    rows: [
      { country: 0, capacity: { kind: 'known', active: 2, maximum: 4 }, missing: null },
      { country: 1, capacity: { kind: 'known', active: 6, maximum: 8 }, missing: null },
    ],
    columns: [0, 1, 2, 3],
    cells: [
      { row: 0, column: 3, home: false, operatives: [agent(1)] },
      { row: 1, column: 2, home: false, operatives: [agent(2), agent(3), agent(4)] },
    ],
    most: 3,
    largestMaximum: 8,
    pairs: [
      { spy: 1, target: 2, operatives: [agent(2), agent(3), agent(4)] },
      { spy: 0, target: 3, operatives: [agent(1)] },
    ],
    readHour: 200,
    missing: null,
    warnings: [],
  };
  const names = datedIdentity(Object.fromEntries(tags.map(tag => [tag, [{ from: 0, name: tag }]])));
  const at = (hour: number) => createBoundedReadView({ hour, identity: names, panels: new Map([['main', groupReply]]) });
  const membership = { AAA: [[100, 299]] as const, BBB: [[600, 900]] as const };
  const unobserved = [[500, 550]] as const;
  const opts = { membership, unobserved, membershipMode: 'hour' as const };

  const matrix200 = operativeMatrixProjection.project(at(200), opts);
  assert.deepEqual(matrix200.rows.map(row => row.id), ['AAA']);
  assert.deepEqual(matrix200.columns.map(col => col.id), ['AAA', 'DDD']);
  assert.deepEqual(matrixCellAt(matrix200, 0, 1), { kind: 'value', intensity: 1, text: '1', value: 1, selectable: true });
  assert.deepEqual(matrix200.margin!.cell(0), { kind: 'value', fraction: 2 / 4, text: '2 / 4' });
  assert.equal(matrix200.legend?.gradient?.highLabel, '1 operative');
  assert.ok(!matrix200.legend?.entries?.some(entry => entry.hatched));
  assert.equal(operativeMatrixProjection.nextChange?.(at(200), opts), 300);

  const pairs200 = operativePairsProjection.project(at(200), opts);
  assert.deepEqual(pairs200.rows.map(row => row.key), ['AAA>DDD']);
  assert.equal(pairs200.scaleMax, 1);
  assert.equal(operativePairsProjection.nextChange?.(at(200), opts), 300);

  const matrix400 = operativeMatrixProjection.project(at(400), opts);
  assert.equal(matrix400.empty, NO_MEMBERS.hour);
  assert.equal(matrix400.emptyKind, undefined);
  assert.equal(operativeMatrixProjection.nextChange?.(at(400), opts), 500);

  const matrix525 = operativeMatrixProjection.project(at(525), opts);
  assert.equal(matrix525.emptyKind, 'unavailable');
  assert.equal(matrix525.rows.length, 0);
  assert.equal(operativeMatrixProjection.nextChange?.(at(525), opts), 551);

  const pairs525 = operativePairsProjection.project(at(525), opts);
  assert.equal(pairs525.emptyKind, 'unavailable');
  assert.equal(pairs525.rows.length, 0);

  const everOpts = { membership: { AAA: [[100, 499]] as const }, unobserved: [[500, Number.MAX_SAFE_INTEGER]] as const, membershipMode: 'ever' as const };
  assert.equal(operativeMatrixProjection.project(at(50), everOpts).empty, NO_MEMBERS.yet);
  const everUnseen = operativeMatrixProjection.project(at(525), everOpts);
  assert.equal(everUnseen.emptyKind, 'unavailable');
  assert.equal(everUnseen.empty, EVER_UNRECORDED);

  const pastMemberOpts = { membership: { AAA: [[0, 10]] as const }, unobserved: [] as const, membershipMode: 'ever' as const };
  const pastMember = operativeMatrixProjection.project(at(15), pastMemberOpts);
  assert.equal(pastMember.empty, 'No operatives in the field at this date.');
  assert.notEqual(pastMember.empty, NO_MEMBERS.yet);

  const crossReply: OperativeMatrixReply = {
    countries: ['AAA', 'BBB', 'CCC'].map(tag => ({ tag, factionId: null, factionName: null, factionNameMissing: null })),
    rows: [
      { country: 0, capacity: { kind: 'known', active: 2, maximum: 4 }, missing: null },
      { country: 1, capacity: { kind: 'known', active: 2, maximum: 4 }, missing: null },
    ],
    columns: [0, 1, 2],
    cells: [
      { row: 0, column: 1, home: false, operatives: [agent(1)] },
      { row: 0, column: 2, home: false, operatives: [agent(2)] },
      { row: 1, column: 0, home: false, operatives: [agent(3)] },
      { row: 1, column: 1, home: true, operatives: [agent(4)] },
      { row: 1, column: 2, home: false, operatives: [agent(5)] },
    ],
    most: 1,
    largestMaximum: 4,
    pairs: [],
    readHour: 200,
    missing: null,
    warnings: [],
  };
  const crossModel = operativeMatrixProjection.project(
    createBoundedReadView({ hour: 200, identity: names, panels: new Map([['main', crossReply]]) }),
    { membership: { AAA: [[100, 299]] as const }, unobserved: [], membershipMode: 'hour' },
  );
  assert.deepEqual(crossModel.rows.map(row => row.id), ['AAA', 'BBB']);
  assert.deepEqual(crossModel.columns.map(col => col.id), ['AAA', 'BBB', 'CCC']);
  assert.equal(matrixCellAt(crossModel, 1, 2).kind, 'empty');
  assert.equal(deriveMatrixTooltip(crossModel, 1, 2)?.items?.[0]?.label, 'Outside selected group');
  assert.equal(deriveMatrixTooltip(crossModel, 1, 1)?.items?.[0]?.label, 'Outside selected group');
  const crossAudit = deriveMatrixAudit(crossModel, { firstRow: 0, endRow: 2, firstCol: 0, endCol: 3 });
  assert.equal(crossAudit.find(entry => entry.key === 'BBB:CCC')?.value, 'Outside selection');
  assert.equal(crossAudit.find(entry => entry.key === 'BBB:BBB')?.value, 'Outside selection');
  const crossMirror = deriveMatrixMirror(crossModel, 1, 2, 0, 3);
  assert.equal(crossMirror.rows[0]!.cells.find(cell => cell.column === 'CCC')?.spoken, 'CCC: Outside selection');

  const unreadTargetReply: OperativeMatrixReply = {
    countries: ['AAA', 'BBB'].map(tag => ({ tag, factionId: null, factionName: null, factionNameMissing: null })),
    rows: [{ country: 1, capacity: { kind: 'known', active: 1, maximum: 2 }, missing: 'Target not recorded' }],
    columns: [0, 1],
    cells: [],
    most: 0,
    largestMaximum: 2,
    pairs: [],
    readHour: 200,
    missing: null,
    warnings: [],
  };
  const unreadTarget = operativeMatrixProjection.project(
    createBoundedReadView({ hour: 200, identity: names, panels: new Map([['main', unreadTargetReply]]) }),
    { membership: { AAA: [[100, 299]] as const }, unobserved: [], membershipMode: 'hour' },
  );
  assert.equal(unreadTarget.empty, undefined);
  assert.deepEqual(unreadTarget.rows.map(row => row.id), ['BBB']);
  assert.deepEqual(unreadTarget.columns.map(col => col.id), ['AAA']);
  assert.deepEqual(matrixCellAt(unreadTarget, 0, 0), { kind: 'missing', reason: 'Target not recorded' });
  assert.ok(unreadTarget.legend?.entries?.some(entry => entry.hatched));

  const pastEmpty = operativeMatrixProjection.project(
    createBoundedReadView({ hour: 15, identity: names, panels: new Map([['main', unreadTargetReply]]) }),
    { membership: { AAA: [[0, 10]] as const }, unobserved: [], membershipMode: 'ever' },
  );
  assert.equal(pastEmpty.rows.length, 0);
  assert.equal(pastEmpty.empty, 'No operatives in the field at this date.');

  const missingCensus: OperativeMatrixReply = { ...unreadTargetReply, rows: [], columns: [], cells: [], pairs: [], missing: 'Operative census not recorded' };
  const emptyBeforeJoin = { membership: { AAA: [[600, 700]] as const }, unobserved: [] as const, membershipMode: 'hour' as const };
  const missingMatrix = operativeMatrixProjection.project(
    createBoundedReadView({ hour: 200, identity: names, panels: new Map([['main', missingCensus]]) }),
    emptyBeforeJoin,
  );
  assert.equal(missingMatrix.empty, 'This group has no members at this moment.');
  assert.equal(missingMatrix.emptyKind, undefined);
  const missingPairs = operativePairsProjection.project(
    createBoundedReadView({ hour: 200, identity: names, panels: new Map([['main', missingCensus]]) }),
    emptyBeforeJoin,
  );
  assert.equal(missingPairs.empty, 'This group has no members at this moment.');
  assert.equal(missingPairs.emptyKind, undefined);
});
