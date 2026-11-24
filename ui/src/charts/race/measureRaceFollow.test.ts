import assert from 'node:assert/strict';
import { test } from 'node:test';
import { followedRace } from './measureRaceData.ts';
import { parseFollowed } from '../../test-support/followMessage.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';

const answerOf = (spec: Parameters<typeof message>[1][number]['answer']) => parseFollowed(message(10, [{ id: 1, answer: spec }])).answers[0]!.answer;
const format = (raw: string) => raw;

test('review 4 finding 10: a followed race names a recorded refusal, and applies membership at the hour it shows, never the clock\'s', () => {
  const refused = followedRace({ answer: answerOf({ notRecorded: 'Faction strength was not recorded at this hour' }), through: 10 }, null, null, 20, 5, 0, format);
  assert.deepEqual([refused.reason, refused.entries], ['Faction strength was not recorded at this hour', []]);
  const joinsAt = 15, member = (tag: string, hour: number) => tag !== 'ITA' || hour >= joinsAt;
  const shown = followedRace({ answer: answerOf({ keys: [1n, 2n], columns: 1, values: [5n, 7n], names: ['GER', 'ITA'] }), through: 10 }, null, member, 20, 5, 0, format);
  assert.deepEqual([shown.shown, shown.entries.map(entry => [entry.tag, entry.hour])], [10, [['GER', 10]]], 'confirmed through 10 at clock 20: ITA, a member from 15, is not shown');
});

