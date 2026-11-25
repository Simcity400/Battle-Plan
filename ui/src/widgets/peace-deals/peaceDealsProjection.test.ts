import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { PeaceDealsTailReply } from '../../generated/dashboard';
import { peaceDealsProjection } from './peaceDealsProjection.ts';

test('a treaty counts once when expanded, its transfers show the former owner at the ownership read, and a capped treaty offers the rest', () => {
  const key = 'group:conference:2';
  const head: PeaceDealsTailReply = {
    branchId: '1', total: '4', offset: '0', missing: [], warnings: [], nextChange: null, counts: [{ key: 'treaties', total: '1' }],
    rows: [
      { key, hour: 32, parent: null, detail: { kind: 'treaty', winner: 'BBB', loser: 'AAA', whitePeace: false, bounded: true, derived: true,
        members: 2, terms: [{ kind: 'state', count: 1 }], limitations: [] } },
      { key: 'recipient', hour: 32, parent: key, detail: { kind: 'recipient', tag: 'BBB', terms: [{ kind: 'state', count: 1 }] } },
      { key: 'states', hour: 32, parent: key, detail: { kind: 'terms', items: [{ kind: 'state', state: 7, tag: 'AAA', at: 28 }] } },
      { key: 'more', hour: 32, parent: key, detail: { kind: 'more', terms: 12, hidden: 11 } },
    ],
  };
  const source = buildRowSourceFromHead('branch', head, undefined, 40)!;
  const reads: string[] = [];
  const view = createBoundedReadView({ hour: 40, rows: new Map([['main', source]]), identity: {
    resolve: (kind, id, hour) => {
      reads.push(`${kind} ${id} at ${hour}`);
      return { kind, key: id, atHour: hour, name: { status: 'ready', value: `${kind} ${id}` },
        holder: { status: 'absent', reason: 'fixture' }, validInterval: [hour, hour], nextChangeAt: Infinity };
    },
  } });
  const model = peaceDealsProjection.project(view, { expanded: [key] });
  assert.equal(model.empty, undefined);
  assert.equal(model.titleSummary, '1 peace deal');
  assert.deepEqual(Object.keys(model.rows[0]?.cells ?? {}), ['date', 'kind', 'country', 'terms', 'action']);
  assert.equal(model.rows[0]?.cells.country?.flags?.[1]?.dimmed, true);
  assert.equal(model.rows[2]?.cells.terms?.flags?.[0]?.label, 'state 7');
  assert.equal(model.rows[3]?.key, `all:${key}`);
  assert.equal(model.rows[3]?.name, 'Show all 12 terms · 11 more');
  assert.equal(model.rows[3]?.cells.terms?.value.kind === 'known' && model.rows[3].cells.terms.value.value, model.rows[3]?.name);
  assert.ok(reads.includes('country AAA at 28'));
  assert.ok(reads.includes('state 7 at 32'));
});
