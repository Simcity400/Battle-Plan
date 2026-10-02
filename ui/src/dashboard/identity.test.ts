import test from 'node:test';
import assert from 'node:assert/strict';
import { Lineage, countryMembership, joinTagChanges } from './identity.ts';
import { assertLinear } from '../charts/assertLinear.ts';

const run = (tag: string, firstHour: number, lastHour: number) => ({ tag, firstHour, lastHour });

test('membership follows countries across renames, with related-country changes split too', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }, { hour: 150, from: 'OWNER', to: 'NEW OWNER' }]);
  const member = lineage.countryAt('AAA', 100);
  const membership = countryMembership([run('AAA', 100, 199), run('BBB', 200, 300), run('AAA', 250, 300)], lineage);
  assert.deepEqual(membership.get(member), [run('AAA', 100, 199), run('BBB', 200, 300)]);
  assert.deepEqual(membership.get(lineage.countryAt('AAA', 250)), [run('AAA', 250, 300)]);
  const related = joinTagChanges([{ ...run('AAA', 100, 199), owner: 'OWNER' }], lineage, item => [item.owner]);
  assert.deepEqual(related[0].map(item => [item.firstHour, item.lastHour]), [[100, 149], [150, 199]]);
  const many = countryMembership(Array.from({ length: 5000 }, (_, index) => run(`mod ${index}`, 100, 300)), lineage);
  assert.equal(many.size, 5000);
});

test('a tag names one country until it is renamed; a later holder of the tag is another country', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }]);
  const first = lineage.countryAt('AAA', 150);
  assert.equal(lineage.countryAt('BBB', 250), first);
  // Nothing continues AAA after 200: whoever holds it later is someone else, even with no
  // run of the old country reaching the rename (it may have left a faction at 150).
  assert.notEqual(lineage.countryAt('AAA', 400), first);
  // A tag with no recorded change is one country throughout.
  assert.equal(lineage.countryAt('GER', 0), lineage.countryAt('GER', 1e6));
  // Before BBB was AAA's, BBB named another country (or none).
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
  // The country that took AAA at 100 is CCC at 300 and D01 at 400.
  assert.equal(lineage.countryAt('D01', 450), bbb);
  assert.equal(lineage.countryAt('CCC', 350), bbb);
});

test('a country is named by the tag it had at the hour, through every rename', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }, { hour: 300, from: 'BBB', to: 'CCC' }]);
  const country = lineage.countryAt('AAA', 100);
  assert.deepEqual([100, 199, 200, 299, 300, 1e6].map(hour => lineage.tagOf(country, hour)), ['AAA', 'AAA', 'BBB', 'BBB', 'CCC', 'CCC']);
  // Whoever takes AAA later is named AAA, and a country never renamed keeps its tag.
  assert.equal(lineage.tagOf(lineage.countryAt('AAA', 400), 450), 'AAA');
  assert.equal(lineage.tagOf(lineage.countryAt('GER', 0), 1e6), 'GER');
});

test('runs group by the country their tag named, however the tags were reused', () => {
  const lineage = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }]);
  // AAA is a member 100-150 and leaves; it is renamed BBB at 200 (and rejoins as BBB at 250);
  // a new AAA joins at 400.
  const groups = joinTagChanges([run('AAA', 100, 150), run('BBB', 250, 300), run('AAA', 400, 450)], lineage);
  assert.deepEqual(groups.map(group => group.map(item => `${item.tag}@${item.firstHour}`)), [['AAA@100', 'BBB@250'], ['AAA@400']]);
  assert.equal(joinTagChanges([run('GER', 0, 10), run('GER', 20, 30)], new Lineage([])).length, 1);
});

test('the lineage resolves each tag and hour in logarithmic time', () => {
  assertLinear('changes', count => {
    const changes = Array.from({ length: count }, (_, index) => ({ hour: index * 2, from: `T${index % 50}`, to: `T${(index + 1) % 50}` }));
    return () => {
      const lineage = new Lineage(changes);
      for (let index = 0; index < count; index++) lineage.countryAt(`T${index % 50}`, index * 2 + 1);
    };
  }, 2000);
});

test('runs spanning a swap are split by the country each tag names then', () => {
  const lineage = new Lineage([{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 10, from: 'BBB', to: 'AAA' }]);
  assert.deepEqual(joinTagChanges([run('AAA', 0, 20), run('BBB', 0, 14)], lineage), [
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
