import assert from 'node:assert/strict';
import test from 'node:test';
import { experienceCard, experienceLines, pointsSummary, experienceStructure, followExperienceHeads, formatExperience, validateExperienceReply } from './experienceData.ts';
import { parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { tileHead, valueAtHead } from '../../dashboard/tiles/head.ts';

const reply = { tag: 'GER', scale: 32768, dayHours: 24, readHour: 48, banked: { armyRaw: '327680', navyRaw: '0', airRaw: '16384' }, bankedMissing: null,
  change: null, changeMissing: 'No read exactly a day earlier in this unbroken stretch', gain: { armyRaw: '32768', navyRaw: '0', airRaw: '0' }, gainMissing: null, warnings: [] };


test('one country draws its branches in the branch colours; several, each in its own colour, a line per branch', () => {
  const categories = ['c0', 'c1', 'c2'];
  assert.deepEqual(experienceLines(['GER'], ['army', 'air'], true, new Map([['GER', 'grey']]), categories).map(line => line.color), ['c0', 'c2']);
  const lines = experienceLines(['GER', 'ITA'], ['army', 'navy'], false, new Map([['GER', 'grey'], ['ITA', 'green']]), categories);
  assert.deepEqual(lines.map(line => `${line.tag}:${line.branch}:${line.color}`), ['GER:army:grey', 'GER:navy:grey', 'ITA:army:green', 'ITA:navy:green']);
  const summary = { from: 0, to: 5, binHours: 6, buckets: [{ firstHour: 0, lastHour: 5, firstValue: 32768, lastValue: 65536, minValue: 32768, maxValue: 65536 }] };
  assert.equal(pointsSummary(summary as never).buckets[0]!.lastValue, 2, 'drawn in points');
  const following = { ...summary, tiles: { binHours: 6, valueAt: () => 500 * 32768 } };
  assert.equal(pointsSummary(following as never).tiles!.valueAt(3), 500, 'the head and its end label in points, never 32,768 times the legend');
});

test('the card shows each value in points and a value not recorded says why, never zero', () => {
  const card = experienceCard(validateExperienceReply(reply), 'army');
  assert.deepEqual(card.rows.map(row => [row.label, row.value]), [['Banked', '10'], ['Daily gain', '+1'], ['Change per day', 'Not recorded']]);
  assert.equal(card.rows[2].detail, reply.changeMissing);
  assert.equal(formatExperience('-49152', true), '-1.5');
  assert.throws(() => validateExperienceReply({ ...reply, banked: null }), /malformed/, 'a missing value names its reason');
  assert.equal(experienceStructure({ ...reply, warnings: ['x'] }), experienceStructure(reply));
});

test("while followed, each line's head is the followed value the legend shows, never the bin before's last value", () => {
  const before = Math.round(71.09 * 32768), atHead = Math.round(75.34 * 32768), army = Math.round(27.99 * 32768);
  const summary = { from: 0, buckets: [{ firstHour: 0, lastHour: 5, firstValue: 0, lastValue: before, minValue: 0, maxValue: before }, null] };
  const tiled = (head: string) => pointsSummary({ ...summary, to: 11, width: 2, tiles: { binHours: 6, valueAt: (hour: number) => valueAtHead(head, summary as never, 6, hour) } } as never);
  const navyHead = tileHead('navy-lane', 'GER'), armyHead = tileHead('army-lane', 'GER');
  assert.equal(tiled(navyHead).tiles!.valueAt(9)!.toFixed(2), '71.09', "unfollowed: the bin before's last value");
  const answer = (value: number) => parseFollowed(message(9, [{ id: 1, answer: { keys: [1n], columns: 1, values: [BigInt(value)] } }])).answers[0]!.answer;
  const stop = followExperienceHeads({ army: { answer: answer(army) }, navy: { answer: answer(atHead) }, air: null }, ['GER'],
    (branch, tag) => tileHead(`${branch}-lane`, tag));
  assert.equal(tiled(navyHead).tiles!.valueAt(9)!.toFixed(2), '75.34', 'the end label agrees with the legend');
  assert.equal(tiled(armyHead).tiles!.valueAt(9)!.toFixed(2), '27.99');
  stop();
  assert.equal(tiled(navyHead).tiles!.valueAt(9)!.toFixed(2), '71.09', 'stopped with the widget');
  const unrecorded = followExperienceHeads({ army: null, navy: { answer: parseFollowed(message(9, [{ id: 1, answer: { notRecorded: 'Navy not read' } }])).answers[0]!.answer }, air: null },
    ['GER'], (branch, tag) => tileHead(`${branch}-lane`, tag));
  assert.equal(tiled(navyHead).tiles!.valueAt(9), null, 'a head not recorded draws nothing, never zero');
  unrecorded();
});
