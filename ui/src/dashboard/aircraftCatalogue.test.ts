import assert from 'node:assert/strict';
import test from 'node:test';
import type { AircraftCatalogueReply, AircraftEntry } from '../generated/dashboard.ts';
import { AIRCRAFT_PAGE, AircraftPages, catalogueParams, countryTag, originFilter, originLabel, pagesFor, refusalLine, refusalText, sameOrigin } from './aircraftCatalogue.ts';
import { hourFields } from './regionTime.ts';

const entry = (name: string): AircraftEntry => ({ name, kind: 'entity', titles: [], references: [], refusal: null, origin: null });

function reply(first: number, names: string[], matching: number): AircraftCatalogueReply {
  return { matching, found: matching + 7, first, entries: names.map(entry),
    refusals: [{ kind: 'unregistered', label: 'not registered by any descriptor', count: 3 }],
    origins: [{ origin: { kind: 'game', dlc: null }, count: matching }], warnings: ['a note'] };
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
  assert.equal(pages.flyable(), null);
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
  assert.equal(pages.found, 500_007);
  assert.equal(pages.flyable(), 500_000 - 3, 'refused rows follow every flyable one');
  assert.deepEqual(pages.origins, [{ origin: { kind: 'game', dlc: null }, count: 500_000 }]);
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

test('counted refusals read as a plain tooltip by consequence, and the raw reasons only as detail', () => {
  assert.equal(refusalLine([]), null);
  assert.equal(refusalLine([{ kind: 'noGeometry', label: 'no geometry to draw', count: 0 }]), null);
  assert.deepEqual(refusalLine([{ kind: 'noGeometry', label: 'no geometry to draw', count: 1 }]),
    { tooltip: '1 with a missing 3D model', detail: '1: no geometry to draw' });
  assert.deepEqual(refusalLine([
    { kind: 'unregistered', label: 'not registered by any descriptor', count: 1200 },
    { kind: 'declaration', label: 'a declaration it uses cannot be played', count: 2 },
    { kind: 'noGeometry', label: 'no geometry to draw', count: 3 },
  ]), { tooltip: `${(1203).toLocaleString()} with a missing 3D model\n2 that BattlePlan can’t read`,
    detail: `${(1200).toLocaleString()}: not registered by any descriptor\n2: a declaration it uses cannot be played\n3: no geometry to draw` });
  assert.equal(refusalText('unregistered'), 'Can’t be flown: its 3D model is missing.');
  assert.equal(refusalText('noGeometry'), 'Can’t be flown: its 3D model is missing.');
  assert.equal(refusalText('declaration'), 'Can’t be flown: BattlePlan can’t read this model.', 'a declaration BattlePlan can’t read says nothing about the game');
});

test('origins filter as the base game or one mod, and a DLC counts as the base game', () => {
  const game = { kind: 'game', dlc: 'By Blood Alone' } as const;
  const modern = { kind: 'mod', id: 'ugc_1', name: 'Modern Wings' } as const;
  assert.equal(originFilter(game), 'game');
  assert.deepEqual(originFilter(modern), { mod: 'ugc_1' });
  assert.equal(originLabel(game), 'Vanilla');
  assert.equal(originLabel(modern), 'Modern Wings');
  assert.ok(sameOrigin('game', 'game') && sameOrigin(null, null) && sameOrigin({ mod: 'ugc_1' }, { mod: 'ugc_1' }));
  assert.ok(!sameOrigin('game', null) && !sameOrigin({ mod: 'ugc_1' }, 'game') && !sameOrigin({ mod: 'ugc_1' }, { mod: 'ugc_2' }));
});

test('a country tag is a three-character ladder prefix, else pool bucket, and never a culture or default pool', () => {
  assert.equal(countryTag({ references: [{ kind: 'pool', bucket: 'default', equipment: 'fighter', file: 'f' }, { kind: 'ladder', key: 'fighter_equipment_1', prefix: 'ENG' }] }), 'ENG');
  assert.equal(countryTag({ references: [{ kind: 'pool', bucket: 'GER', equipment: 'fighter', file: 'f' }] }), 'GER');
  assert.equal(countryTag({ references: [{ kind: 'ladder', key: 'light_plane', prefix: 'western_european' }, { kind: 'pool', bucket: 'default', equipment: 'fighter', file: 'f' }] }), null);
  assert.equal(countryTag({ references: [{ kind: 'design', equipment: 'fighter_equipment_1' }] }), null);
});

test('every request the catalogue sends is one the request contract accepts', () => {
  for (const origin of [null, 'game', { mod: 'ugc_1' }] as const) {
    const refused: string[] = [];
    const params = catalogueParams(' spit ', 100, 100, origin);
    assert.ok(hourFields('aircraft-catalogue', params, () => {}, (path, expected) => refused.push(`${path} is not ${expected}`)));
    assert.deepEqual(refused, [], JSON.stringify(origin));
    assert.equal(params.query, 'spit');
  }
});
