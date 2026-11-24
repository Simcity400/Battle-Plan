import test from 'node:test';
import assert from 'node:assert/strict';
import { Lineage, countryMembership } from './identity.ts';

const run = (tag: string, firstHour: number, lastHour: number) => ({ tag, firstHour, lastHour });

test('membership follows countries across renames', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }, { hour: 150, from: 'OWNER', to: 'NEW OWNER' }]);
  const member = lineage.countryAt('AAA', 100);
  const membership = countryMembership([run('AAA', 100, 199), run('BBB', 200, 300), run('AAA', 250, 300)], lineage);
  assert.deepEqual(membership.get(member), [run('AAA', 100, 199), run('BBB', 200, 300)]);
  assert.deepEqual(membership.get(lineage.countryAt('AAA', 250)), [run('AAA', 250, 300)]);
  const many = countryMembership(Array.from({ length: 5000 }, (_, index) => run(`mod ${index}`, 100, 300)), lineage);
  assert.equal(many.size, 5000);
});

test('a tag names one country until it is renamed; a later holder of the tag is another country', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }]);
  const first = lineage.countryAt('AAA', 150);
  assert.equal(lineage.countryAt('BBB', 250), first);
  assert.notEqual(lineage.countryAt('AAA', 400), first);
  assert.equal(lineage.countryAt('GER', 0), lineage.countryAt('GER', 1e6));
  assert.notEqual(lineage.countryAt('BBB', 150), first);
});

test('chains and swaps in one hour move whole countries', () => {
  const lineage = new Lineage([
    { hour: 100, from: 'AAA', to: 'BBB' }, { hour: 100, from: 'BBB', to: 'AAA' },
    { hour: 300, from: 'AAA', to: 'CCC' }, { hour: 400, from: 'CCC', to: 'D01' },
  ]);
  const [aaa, bbb] = [lineage.countryAt('AAA', 50), lineage.countryAt('BBB', 50)];
  assert.equal(lineage.countryAt('BBB', 150), aaa);
  assert.equal(lineage.countryAt('AAA', 150), bbb);
  assert.equal(lineage.countryAt('D01', 450), bbb);
  assert.equal(lineage.countryAt('CCC', 350), bbb);
});

test('a country is named by the tag it had at the hour, through every rename', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }, { hour: 300, from: 'BBB', to: 'CCC' }]);
  const country = lineage.countryAt('AAA', 100);
  assert.deepEqual([100, 199, 200, 299, 300, 1e6].map(hour => lineage.tagOf(country, hour)), ['AAA', 'AAA', 'BBB', 'BBB', 'CCC', 'CCC']);
  assert.equal(lineage.tagOf(lineage.countryAt('AAA', 400), 450), 'AAA');
  assert.equal(lineage.tagOf(lineage.countryAt('GER', 0), 1e6), 'GER');
});

test('runs group by the country their tag named, however the tags were reused', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }]);
  const groups = [...countryMembership([run('AAA', 100, 150), run('BBB', 250, 300), run('AAA', 400, 450)], lineage).values()];
  assert.deepEqual(groups.map(group => group.map(item => `${item.tag}@${item.firstHour}`)), [['AAA@100', 'BBB@250'], ['AAA@400']]);
  assert.equal(countryMembership([run('GER', 0, 10), run('GER', 20, 30)], new Lineage([])).size, 1);
});

test('runs spanning a swap are split by the country each tag names then', () => {
  const lineage = new Lineage([{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 10, from: 'BBB', to: 'AAA' }]);
  assert.deepEqual([...countryMembership([run('AAA', 0, 20), run('BBB', 0, 14)], lineage).values()], [
    [run('AAA', 0, 9), run('BBB', 10, 14)],
    [run('BBB', 0, 9), run('AAA', 10, 20)],
  ]);
});

test('tag change hours are bounded to the range, without walking its hours', () => {
  const lineage = new Lineage([{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 30, from: 'BBB', to: 'AAA' }]);
  assert.deepEqual(lineage.changeHours('AAA', 10, 30), [10, 30]);
  assert.deepEqual(lineage.changeHours('AAA', 11, 29), []);
  assert.deepEqual(lineage.changeHours('BBB', -2_000_000_000, 2_000_000_000), [10, 30]);
  assert.deepEqual(lineage.changeHours('OTHER', 0, 40), []);
});

test('holder starts use per-tag binary search and are bounds, not native presence', () => {
  const changes = Array.from({ length: 5000 }, (_, i) => ({ hour: i + 1, from: `country ${i}`, to: `alias ${i}` }));
  const countries = new Lineage(changes);
  for (const { hour, from, to } of changes) {
    assert.equal(countries.holderStart(to, hour), hour);
    assert.equal(countries.holderStart(to, hour - 1), -Infinity);
    assert.equal(countries.holderStart(from, hour), hour, 'a vacated tag bound does not certify a country exists');
  }
  assert.equal(countries.holderStart('not recorded', 1e9), -Infinity);
  const chain = new Lineage(Array.from({ length: 5000 }, (_, i) => ({ hour: i + 1,
    from: i % 2 ? 'B' : 'A', to: i % 2 ? 'A' : 'B' })));
  const held = Reflect.get(chain, 'held') as Map<string, { hour: number; country: string }[]>;
  const list = held.get('A')!;
  let reads = 0;
  held.set('A', new Proxy(list, { get(target, key, receiver) {
    if (typeof key === 'string' && /^\d+$/.test(key)) reads++;
    return Reflect.get(target, key, receiver);
  } }));
  for (const hour of [0, 2379, 5000]) {
    reads = 0;
    assert.equal(chain.holderStart('A', hour), hour ? hour : -Infinity);
    assert.ok(reads <= Math.ceil(Math.log2(list.length)) + 1, 'only logarithmically many held entries are read');
  }
});
