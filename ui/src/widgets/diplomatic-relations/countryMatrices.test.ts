import assert from 'node:assert/strict';
import test from 'node:test';
import { diplomaticRelationsDeclaration } from './diplomaticRelationsProjection.ts';
import { intelCoverageDeclaration } from '../intel-network-coverage/intelCoverageProjection.ts';

const groupCtx = (membership: 'hour' | 'ever') => ({
  track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
  branch: { id: '1' },
  options: {},
  countryTag: null,
  countries: { all: false, top: null, tags: ['GER', 'ITA'] },
  widget: { countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership, aggregate: false } },
}) as never;

const readOf = (plan: { entries: readonly { read: unknown }[] }) => plan.entries[0]!.read as { params: Record<string, unknown>; coverage: unknown };

test('both country matrices send a faction as the group, resolved in Rust at the hour, never the union of its members', () => {
  const view = { following: true, staticSpan: null } as never;
  for (const declaration of [diplomaticRelationsDeclaration, intelCoverageDeclaration]) {
    const read = readOf(declaration.sources(groupCtx('hour'), view));
    assert.deepEqual(read.params.group, { type: 'faction', id: '42', membership: 'hour' }, declaration.kind);
    assert.ok(!(read.params.countries as unknown[] | undefined)?.length, `${declaration.kind}: no member list leaves the client`);
    assert.equal(read.params.top, undefined, `${declaration.kind}: a group is not a top`);
    assert.deepEqual(read.coverage, { mode: 'population', form: 'all' });
    assert.deepEqual(readOf(declaration.sources(groupCtx('ever'), view)).params.group, { type: 'faction', id: '42', membership: 'ever' });
  }
});
