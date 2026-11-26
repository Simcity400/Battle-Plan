import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { matrixCellAt } from '../../families/heat/matrixModel.ts';
import type { DiplomaticRelationsReply, IntelCoverageReply } from '../../generated/dashboard';
import { diplomaticRelationsDeclaration, projectDiplomaticRelations } from './diplomaticRelationsProjection.ts';
import { intelCoverageDeclaration, projectIntelCoverage } from '../intel-network-coverage/intelCoverageProjection.ts';

const HOUR = 60_810_000;
const view = (name: string, data: unknown) => createBoundedReadView({ hour: HOUR, panels: new Map([[name, data]]) });

test('a truncated intel read keeps its witnessed network and draws every other pair as not read, never "No network"', () => {
  const reply: IntelCoverageReply = {
    displayed: [{ id: 1, tag: 'GER' }, { id: 2, tag: 'POL' }, { id: 3, tag: 'FRA' }],
    pairs: [{ row: 0, column: 1, value: 5n, coveredControlled: 5, coveredCore: 2, controlledStrength: 0n, coreStrength: 0n, maxStrength: 0n }],
    unlisted: 'Intel network summaries read only in part at this hour (3 rows not read)',
    maxValue: 5n, scale: 1n, readHour: HOUR, missing: null, chosenMissing: null, warnings: [],
  } as unknown as IntelCoverageReply;
  const model = projectIntelCoverage(view('coverage', reply), { measure: 'covered', surface: '#ffffff' });
  assert.equal(model.cells.present.size, 1, 'one witnessed pair, no cell built per row and column');
  const witnessed = matrixCellAt(model, 0, 1);
  assert.equal(witnessed.kind === 'value' && witnessed.text, '5');
  assert.deepEqual(matrixCellAt(model, 1, 0), { kind: 'missing', reason: reply.unlisted });
  assert.equal(matrixCellAt(model, 2, 2).kind, 'diagonal');
  const whole = projectIntelCoverage(view('coverage', { ...reply, unlisted: null }), { measure: 'covered', surface: '#ffffff' });
  assert.equal(matrixCellAt(whole, 1, 0).kind, 'empty', 'a whole read proves no network');
  const strength = projectIntelCoverage(view('coverage', {
    ...reply, pairs: [{ ...reply.pairs[0], value: 29000n, maxStrength: 29000n }], scale: 100000n, maxValue: 29000n,
  }), { measure: 'strongest', surface: '#ffffff' });
  const cell = matrixCellAt(strength, 0, 1);
  assert.equal(cell.kind === 'value' && cell.text, '29%');
  assert.equal(strength.legend?.gradient?.highLabel, '29%');
  assert.equal(strength.tooltip?.(strength.rows[0]!, strength.columns[1]!, cell, 0, 1)?.items?.at(-1)?.value, '29%');
});

test('diplomatic relations: a known owner\'s unlisted opinion is zero, an unknown owner\'s row is not recorded with its reason', () => {
  const owner = (tag: string, known: boolean) => ({ tag, countryId: 1, known, reason: known ? null : 'Relation opinions read only in part at this hour' });
  const reply: DiplomaticRelationsReply = {
    chosenMissing: null,
    owners: [owner('GER', true), owner('POL', false)],
    pairs: [{ row: 1, column: 0, cell: { kind: 'value', opinion: -40 } }],
    domain: [-40, 0],
    warnings: [],
  };
  const model = projectDiplomaticRelations(view('relations', reply), { order: 'name', surface: '#ffffff' });
  assert.equal(model.cells.present.size, 1);
  const zero = matrixCellAt(model, 0, 1);
  assert.equal(zero.kind === 'value' && zero.value, 0);
  const witnessed = matrixCellAt(model, 1, 0);
  assert.equal(witnessed.kind === 'value' && witnessed.value, -40, 'a witnessed opinion shows even from a truncated read');
  const refused = projectDiplomaticRelations(view('relations', { ...reply, owners: [], pairs: [], chosenMissing: 'Membership of this group is not recorded at this hour' }), { order: 'name', surface: '#ffffff' });
  assert.equal(refused.emptyDetail, 'Membership of this group is not recorded at this hour');
});

const groupCtx = (membership: 'hour' | 'ever') => ({
  track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
  branch: { id: '1' },
  options: {},
  countryTag: null,
  countries: { all: false, top: null, tags: ['GER', 'ITA'] },
  widget: { countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership, aggregate: false } },
}) as never;

const readOf = (plan: { entries: readonly { read: unknown }[] }) => plan.entries[0]!.read as { params: Record<string, unknown>; coverage: unknown; dynamicKeys?: readonly string[] };

test('both country matrices send a faction as the group, resolved in Rust at the hour, never the union of its members', () => {
  const view = { following: true, staticSpan: null } as never;
  for (const declaration of [diplomaticRelationsDeclaration, intelCoverageDeclaration]) {
    const read = readOf(declaration.sources(groupCtx('hour'), view));
    assert.ok(!read.dynamicKeys?.includes('hour'), declaration.kind);
    assert.deepEqual(read.params.group, { type: 'faction', id: '42', membership: 'hour' }, declaration.kind);
    assert.ok(!(read.params.countries as unknown[] | undefined)?.length, `${declaration.kind}: no member list leaves the client`);
    assert.equal(read.params.top, undefined, `${declaration.kind}: a group is not a top`);
    assert.deepEqual(read.coverage, { mode: 'population', form: 'all' });
    assert.deepEqual(readOf(declaration.sources(groupCtx('ever'), view)).params.group, { type: 'faction', id: '42', membership: 'ever' });
  }
});
