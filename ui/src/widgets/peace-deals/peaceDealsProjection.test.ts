import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import type { PeaceDealsTailReply } from '../../generated/dashboard';
import { peaceDealsProjection } from './peaceDealsProjection.ts';

test('a treaty counts once when expanded, its transfers show the former owner at the ownership read, and a capped treaty offers the rest', () => {
  const key = 'group:conference:2';
  const head: PeaceDealsTailReply = {
    branchId: '1', total: '3', offset: '0', missing: [], warnings: [], nextChange: null, counts: [{ key: 'treaties', total: '1' }],
    rows: [
      { key, hour: 32, parent: null, detail: { kind: 'treaty', winner: 'BBB', loser: 'AAA', whitePeace: false, bounded: true, derived: true,
        members: 2, terms: [{ kind: 'state', count: 1 }], limitations: [] }, last: false },
      { key: 'recipient', hour: 32, parent: key, detail: { kind: 'recipient', tag: 'BBB', terms: [{ kind: 'state', count: 1 }], items: [{ kind: 'state', state: 7, tag: 'AAA', at: 28 }] }, last: false },
      { key: 'more', hour: 32, parent: key, detail: { kind: 'more', terms: 12, hidden: 11, recipients: 1, hiddenRecipients: 0 }, last: true },
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
  assert.equal(model.rows[1]?.cells.terms?.heading?.note, '1 state', 'the recipient heads its own terms');
  assert.equal(model.rows[1]?.cells.terms?.flags?.[0]?.label, 'state 7');
  assert.equal(model.rows[1]?.detailEnd, undefined, 'a block continues past a row the product does not mark last');
  assert.equal(model.rows[2]?.key, `all:${key}`);
  assert.equal(model.rows[2]?.detailEnd, true, 'the row the product marks last closes the block');
  assert.equal(model.rows[2]?.name, 'Show all 12 terms · 11 more');
  assert.equal(model.rows[2]?.cells.terms?.value.kind === 'known' && model.rows[2].cells.terms.value.value, model.rows[2]?.name);
  assert.ok(reads.includes('country AAA at 28'));
  assert.equal(model.rows[1]?.cells.terms?.value.kind, 'known');
  const unnamed = createBoundedReadView({ hour: 40, rows: new Map([['main', source]]), identity: {
    resolve: (kind, id, hour) => ({ kind, key: id, atHour: hour,
      name: id === 'BBB' ? { status: 'absent', reason: 'Name not in the recording' } : { status: 'ready', value: `${kind} ${id}` },
      holder: { status: 'absent', reason: 'fixture' }, validInterval: [hour, hour], nextChangeAt: Infinity }),
  } });
  const terms = peaceDealsProjection.project(unnamed, { expanded: [key] }).rows[1]?.cells.terms?.value;
  assert.deepEqual(terms?.kind === 'partial' && terms.reason, 'BBB: Name not in the recording', 'a recipient whose name was not read keeps that reason in the terms the tooltip reads');
  assert.ok(reads.includes('state 7 at 32'));
  const pendingHead: PeaceDealsTailReply = {
    ...head,
    total: '2',
    rows: [
      head.rows[1]!,
      { key: 'items-only', hour: 32, parent: key, detail: { kind: 'terms', items: [{ kind: 'state', state: 7, tag: 'AAA', at: 28 }] }, last: true },
    ],
  };
  const pendingSource = buildRowSourceFromHead('branch', pendingHead, undefined, 40)!;
  const pendingView = createBoundedReadView({ hour: 40, rows: new Map([['main', pendingSource]]), identity: {
    resolve: (kind, id, hour) => ({ kind, key: id, atHour: hour,
      name: id === '7' ? { status: 'pending' } : { status: 'ready', value: `${kind} ${id}` },
      holder: { status: 'absent', reason: 'fixture' }, validInterval: [hour, hour], nextChangeAt: Infinity }),
  } });
  const pendingRows = peaceDealsProjection.project(pendingView, { expanded: [key] }).rows;
  assert.equal(pendingRows[0]?.cells.terms?.value.kind, 'partial', 'a recipient row with a pending state term is partial, never known');
  assert.deepEqual(pendingRows[1]?.cells.terms?.value, { kind: 'pending', reason: 'Name is loading' }, 'a sole pending peace term stays pending');
  assert.equal(pendingRows[1]?.cells.terms?.flags?.[0]?.label, 'Loading…', 'expanded pending peace terms use the shared pending label');
});
