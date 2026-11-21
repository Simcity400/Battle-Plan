import assert from 'node:assert/strict';
import test from 'node:test';
import type { AircraftCatalogueReply, AircraftEntry } from '../generated/dashboard.ts';
import { AIRCRAFT_PAGE, AircraftPages, pagesFor, refusalLine, refusalText } from './aircraftCatalogue.ts';

const entry = (name: string): AircraftEntry => ({ name, kind: 'entity', titles: [], references: [], refusal: null });

function reply(first: number, names: string[], matching: number): AircraftCatalogueReply {
  return { matching, aircraft: matching, all: matching + 7, first, entries: names.map(entry),
    refusals: [{ kind: 'unregistered', label: 'not registered by any descriptor', count: 3 }], warnings: ['a note'] };
}

test('pages cover the rows asked for, the last partial page included', () => {
  assert.deepEqual(pagesFor(0, 0), [0]);
  assert.deepEqual(pagesFor(99, 100), [0, 1]);
  assert.deepEqual(pagesFor(250, 420), [2, 3, 4]);
  assert.deepEqual(pagesFor(5, 4), []);
  assert.deepEqual(pagesFor(-10, 3), [0]);
  assert.deepEqual(pagesFor(0, 9, 5), [0, 1]);
});

test('only the first page is asked for until the listing says how long it is', () => {
  const pages = new AircraftPages('listing');
  assert.equal(pages.matching, null);
  assert.ok(pages.wants(0));
  assert.ok(!pages.wants(1));
  pages.begin(0);
  assert.ok(!pages.wants(0), 'a page in flight is not asked again');
  pages.cancel(0);
  assert.ok(pages.wants(0) && !pages.loading(), 'a cancelled request is asked again');
  pages.begin(0);
  assert.ok(pages.loading());
  assert.ok(pages.accept(0, reply(0, ['a', 'b'], 500_000)));
  assert.equal(pages.matching, 500_000);
  assert.equal(pages.all, 500_007);
  assert.equal(pages.entry(1)?.name, 'b');
  assert.equal(pages.entry(2), undefined);
  const last = Math.ceil(500_000 / AIRCRAFT_PAGE) - 1;
  assert.ok(pages.wants(last));
  assert.ok(!pages.wants(last + 1));
  assert.deepEqual(pages.refusals, [{ kind: 'unregistered', label: 'not registered by any descriptor', count: 3 }]);
  assert.deepEqual(pages.warnings, ['a note']);
});

test('a page answered for other rows is refused, and a failed page waits for a retry', () => {
  const pages = new AircraftPages('listing');
  pages.begin(0);
  pages.accept(0, reply(0, ['a'], 300));
  pages.begin(2);
  assert.ok(!pages.accept(2, reply(100, ['wrong'], 300)));
  assert.equal(pages.entry(200), undefined);
  assert.match(pages.error(2)?.message ?? '', /rows from 100 for rows from 200/);
  assert.ok(!pages.wants(2), 'a failed page is not asked again by itself');
  pages.retry(2);
  assert.ok(pages.wants(2));
  pages.begin(1);
  pages.fail(1, new Error('The data service stopped'));
  assert.equal(pages.firstError()?.message, 'The data service stopped');
  assert.ok(!pages.loading());
});

test('counted refusals read as one plain total, a plain tooltip by consequence, and the raw reasons only as detail', () => {
  assert.equal(refusalLine([]), null);
  assert.equal(refusalLine([{ kind: 'noGeometry', label: 'no geometry to draw', count: 0 }]), null);
  assert.deepEqual(refusalLine([{ kind: 'noGeometry', label: 'no geometry to draw', count: 1 }]),
    { text: '1 model can’t be flown', tooltip: '1 with a missing 3D model', detail: '1: no geometry to draw' });
  assert.deepEqual(refusalLine([
    { kind: 'unregistered', label: 'not registered by any descriptor', count: 1200 },
    { kind: 'declaration', label: 'a declaration it uses cannot be played', count: 2 },
    { kind: 'noGeometry', label: 'no geometry to draw', count: 3 },
  ]), { text: `${(1205).toLocaleString()} models can’t be flown`, tooltip: `${(1203).toLocaleString()} with a missing 3D model\n2 the game can’t play`,
    detail: `${(1200).toLocaleString()}: not registered by any descriptor\n2: a declaration it uses cannot be played\n3: no geometry to draw` });
  assert.equal(refusalText('unregistered'), 'Can’t be flown: its 3D model is missing.');
  assert.equal(refusalText('noGeometry'), 'Can’t be flown: its 3D model is missing.');
  assert.equal(refusalText('declaration'), 'Can’t be flown: the game can’t play its model.');
});
