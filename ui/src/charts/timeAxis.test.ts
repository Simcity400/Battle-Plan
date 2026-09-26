import test from 'node:test';
import assert from 'node:assert/strict';
import { engineHour, gameDate } from '../dashboard/calendar.ts';
import { tickLevel, timeTicks } from './timeAxis.ts';

const at = (year: number, month = 1, day = 1) => engineHour({ year, month, day, hour: 1 })!;
const measure = (text: string) => text.length * 7;

test('ticks come from the recording itself, for any start year', () => {
  for (const [start, end] of [[1910, 1914], [1936, 1945], [2000, 2008]]) {
    const ticks = timeTicks(at(start, 3, 5), at(end, 8, 20), 900, measure);
    assert.equal(ticks.level, 'year');
    const texts = ticks.labels.map(label => label.text);
    assert.equal(texts[0], String(start), `first label for ${start}`);
    assert.equal(texts.at(-1), String(end));
    assert.ok(ticks.grid.every(hour => gameDate(hour)?.month === 1 && gameDate(hour)?.day === 1));
  }
});

test('a partial first year is labelled at the middle of its visible part, if it fits', () => {
  const first = at(1936, 7, 1);
  const last = at(1940, 1, 1);
  const ticks = timeTicks(first, last, 400, measure);
  const label = ticks.labels[0];
  assert.equal(label.text, '1936');
  assert.equal(label.hour, (first + at(1937)) / 2);
  // Three days of 1936 have no room for a label.
  const tight = timeTicks(at(1936, 12, 29), at(1946, 1, 1), 600, measure);
  assert.equal(tight.labels[0].text, '1937');
});

test('zooming in moves to quarters, months and days', () => {
  assert.equal(tickLevel(at(1936), at(1946), 800), 'year');
  assert.equal(tickLevel(at(1936), at(1976), 300), 'decade');
  assert.equal(tickLevel(at(1940), at(1942), 900), 'quarter');
  assert.equal(tickLevel(at(1940), at(1940, 7), 900), 'month');
  assert.equal(tickLevel(at(1940, 3), at(1940, 3, 10), 900), 'day');
  const months = timeTicks(at(1940, 11), at(1941, 5), 900, measure);
  assert.deepEqual(months.labels.map(label => label.text), ['Nov', 'Dec', 'Jan 1941', 'Feb', 'Mar', 'Apr']);
});

test('hours before the calendar epoch give no ticks rather than invented dates', () => {
  assert.deepEqual(timeTicks(0, 1000, 800, measure).labels, []);
});
