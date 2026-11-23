import assert from 'node:assert/strict';
import test from 'node:test';
import { arsenalNotice, arsenalRows, nukeParts, partName, validateNuclearArsenal } from './nuclearArsenalData.ts';
import type { NuclearArsenalReply } from '../../generated/dashboard';
import { panelViews, recordedPanelRows } from '../../draw/panels/views.ts';
import { dayLabel } from '../../charts/draw.ts';
import { ArsenalPicture } from '../../draw/arsenal.ts';
import type { ArsenalModel } from '../../draw/arsenal';

const REPLY: NuclearArsenalReply = { missing: null, warnings: [],
  types: [{ ordinal: 0, token: 'nuclear_bomb', name: 'Nuclear Bomb' }, { ordinal: 1, token: 'thermonuclear_bomb', name: null }, { ordinal: 2, token: null, name: null }],
  countries: [{ tag: 'USA', countryId: 1, total: 7, counts: [{ ordinal: 0, value: 5, ready: 1 }, { ordinal: 2, value: 2, ready: 0 }] }] };

test('a weapon type is named by the content, else its token, else its ordinal: never dropped', () => {
  assert.deepEqual(['0', '1', '2', '9'].map(part => partName(REPLY, part)),
    ['Nuclear Bomb', 'thermonuclear_bomb', 'Unnamed nuclear type 2', 'Unnamed nuclear type 9']);
  assert.equal(partName(undefined, '0'), 'Unnamed nuclear type 0', 'before the names land a part is still shown');
  const parts = nukeParts(REPLY);
  assert.equal(parts.kind, 'keyed');
  assert.deepEqual(parts.kind === 'keyed' && [parts.family, parts.partBits], [{ kind: 'country-nukes', measure: 'amount' }, 32]);
  assert.throws(() => validateNuclearArsenal({ ...REPLY, countries: [{ tag: 'USA', total: 1.5, counts: [] }] }));
});

test('a race ranks totals and splits by the weapon counts in the same dated record', () => {
  assert.deepEqual(validateNuclearArsenal(REPLY), REPLY, 'the dated envelope owns the hour');
  const read = { ...REPLY, countries: [...REPLY.countries, { tag: 'MOD', countryId: 2, total: 0, counts: [] }] };
  const [row] = arsenalRows(read, 20, { top: 3, tags: [], split: true });
  assert.deepEqual([row.tag, row.label, row.parts?.map(p => [p.id, p.value])], ['USA', '7', [['0', 5], ['2', 2]]]);
  assert.equal(arsenalRows(read, 20, { top: null, tags: ['MOD'], split: true })[0].value, 0);
  assert.deepEqual(arsenalRows({ ...read, missing: 'Stockpiles withheld' }, 20, { top: 3, tags: [], split: true }), []);
});

test('a held empty stockpile is dated at the displayed hour, including a later day than the source read', () => {
  const read = { ...REPLY, hour: 60759360, countries: [] };
  const shownAt = 60791033;
  const expected = 'No nuclear stockpile recorded on 13 Aug 1939.';
  assert.notEqual(dayLabel(read.hour), dayLabel(shownAt));
  assert.equal(arsenalNotice(read, shownAt), expected);
  assert.equal(panelViews['nuclear-arsenal']!.values(read, null, shownAt, 'empty')?.get('empty'), expected);
  const ink: string[] = [], cleared: number[][] = [];
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 8 }),
    fillText: (text: string) => ink.push(text), clearRect: (...rect: number[]) => cleared.push(rect) }, {
    get: (target, name) => Reflect.get(target, name) ?? (() => {}),
  });
  const hits: (string | undefined)[] = [];
  const picture = new ArsenalPicture({ getContext: () => ctx } as unknown as OffscreenCanvas, () => {}, item => hits.push(item?.key));
  const model = { colours: new Map(), looks: new Map(), top: 10, tags: [], offset: 0, scroll: 0,
    width: 500, height: 180, slots: 10, dpr: 1, tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } } as unknown as ArsenalModel;
  picture.draw(model, read, read.hour);
  assert.ok(ink.includes('No nuclear stockpile recorded on 1 Jan 1936.'));
  cleared.length = ink.length = 0;
  picture.draw(model, read, shownAt);
  assert.ok(ink.includes(expected));
  assert.ok(cleared.every(([, , w, h]) => w < model.width || h < model.height), 'changing the notice keeps the rest of the plot');
  assert.deepEqual(picture.rows(), [{ row: 'empty', key: 'notice', value: expected }]);
  picture.draw(model, REPLY, shownAt);
  const ys = Array.from({ length: model.height }, (_, y) => y);
  const y = ys.find(y => picture.inspect({ x: 100, y })?.item)!;
  picture.hover({ x: 100, y });
  picture.draw(model, { ...REPLY, countries: [{ ...REPLY.countries[0], tag: 'MOD' }] }, shownAt);
  picture.draw(model, read, shownAt);
  assert.deepEqual(hits, ['USA', 'MOD', undefined]);

});

test('the nuclear panel audit names every field that changes its rendered structure', () => {
  const rows = recordedPanelRows('nuclear-arsenal', REPLY);
  assert.deepEqual(rows, [
    'missing:null',
    'type:[0,"nuclear_bomb","Nuclear Bomb"]',
    'type:[1,"thermonuclear_bomb",null]',
    'type:[2,null,null]',
    'country:[1,"USA",true,[0,2]]',
  ]);
  assert.deepEqual(recordedPanelRows('nuclear-arsenal', { ...REPLY,
    countries: [{ ...REPLY.countries[0], total: 12, counts: REPLY.countries[0].counts.map(count => ({ ...count, value: count.value + 1 })) }],
  }), rows, 'scalar stockpile values are painted and do not change the row structure');
  assert.notDeepEqual(recordedPanelRows('nuclear-arsenal', { ...REPLY,
    countries: [{ ...REPLY.countries[0], counts: [...REPLY.countries[0].counts, { ordinal: 1, value: 0, ready: 0 }] }],
  }), rows, 'a new weapon type changes the rendered structure');
});

test('review 4 finding 13: a paused race redraws when its scroll or size changes, though its rows, day and notice stand', () => {
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 8 }) }, { get: (target, name) => Reflect.get(target, name) ?? (() => {}) });
  const picture = new ArsenalPicture({ getContext: () => ctx, width: 1, height: 1 } as unknown as OffscreenCanvas, () => {});
  const model = { colours: new Map(), looks: new Map(), top: 10, tags: [], offset: 0, scroll: 0, overlay: false,
    width: 500, height: 180, slots: 10, dpr: 1, tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } } as unknown as ArsenalModel;
  picture.draw(model, REPLY, 100);
  picture.draw(model, REPLY, 100);
  assert.equal(picture.changed, false, 'nothing to redraw');
  picture.draw({ ...model, scroll: 40 }, REPLY, 100);
  assert.equal(picture.changed, true, 'the new scroll is drawn');
  picture.draw({ ...model, scroll: 40, width: 420 }, REPLY, 100);
  assert.equal(picture.changed, true, 'the new size is drawn');
});

test('review 4 finding 15: the race audits ranked rows as the auditor reads them: the row is the country, the key its part', () => {
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 8 }) }, { get: (target, name) => Reflect.get(target, name) ?? (() => {}) });
  const picture = new ArsenalPicture({ getContext: () => ctx, width: 1, height: 1 } as unknown as OffscreenCanvas, () => {});
  const model = { colours: new Map(), looks: new Map(), top: 10, tags: [], offset: 0, scroll: 0, overlay: false, split: true,
    width: 500, height: 180, slots: 10, dpr: 1, tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } } as unknown as ArsenalModel;
  picture.draw(model, REPLY, 100);
  const [first] = picture.rows()!;
  assert.deepEqual([first!.row, first!.key, `${first!.row}:${first!.key}`], ['USA', 'total', 'USA:total']);
});

test('review 5 finding 15: a paused race draws a new selection or weapon-type split of the same reply', () => {
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 8 }) }, { get: (target, name) => Reflect.get(target, name) ?? (() => {}) });
  const picture = new ArsenalPicture({ getContext: () => ctx, width: 1, height: 1 } as unknown as OffscreenCanvas, () => {});
  const read = { ...REPLY, countries: [...REPLY.countries, { tag: 'MOD', countryId: 2, total: 3, counts: [{ ordinal: 0, value: 3, ready: 0 }] }] };
  const model = { colours: new Map(), looks: new Map(), top: null, tags: ['USA'], offset: 0, scroll: 0, overlay: false, split: false,
    width: 500, height: 180, slots: 10, dpr: 1, tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } } as unknown as ArsenalModel;
  picture.draw(model, read, 100);
  picture.draw({ ...model, tags: ['MOD'] }, read, 100);
  assert.deepEqual(picture.inspect()!.items.map(item => item.key), ['MOD'], 'the new selection is drawn');
  picture.draw({ ...model, tags: ['MOD'], split: true }, read, 100);
  assert.deepEqual(picture.rows()!.map(row => row.key), ['total', '0'], 'the split by weapon type is drawn');
});

test('review 7 B: a paused race repaints when a country name or flag lands, with nothing else changed', () => {
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 8 }) }, { get: (target, name) => Reflect.get(target, name) ?? (() => {}) });
  const picture = new ArsenalPicture({ getContext: () => ctx, width: 1, height: 1 } as unknown as OffscreenCanvas, () => {});
  const model = { colours: new Map(), looks: new Map(), top: 10, tags: [], offset: 0, scroll: 0, overlay: false, split: false,
    width: 500, height: 180, slots: 10, dpr: 1, tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } } as unknown as ArsenalModel;
  picture.draw(model, REPLY, 100);
  picture.draw(model, REPLY, 100);
  assert.equal(picture.changed, false);
  picture.draw({ ...model, looks: new Map([['USA', { name: 'United States', color: '#00f' }]]) }, REPLY, 100);
  assert.equal(picture.changed, true, 'the looked-up name is drawn');
});
