import test from 'node:test';
import { firstHourOfMonth } from '../dashboard/calendar.ts';
import { thinLogTicks, logTicks } from './valueAxis.ts';
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
  const tight = timeTicks(at(1936, 12, 29), at(1946, 1, 1), 800, measure);
  assert.equal(tight.labels[0].text, '1937');
});

test('zooming in moves to quarters, months and days', () => {
  assert.equal(tickLevel(at(1936), at(1946), 800), 'year');
  assert.equal(tickLevel(at(1936), at(1976), 300), 'decade');
  assert.equal(tickLevel(at(1940), at(1942), 900), 'quarter');
  assert.equal(tickLevel(at(1940), at(1940, 7), 900), 'month');
  assert.equal(tickLevel(at(1940, 3), at(1940, 3, 10), 900), 'day');
  const months = timeTicks(at(1940, 11), at(1941, 5), 900, measure);
  assert.deepEqual(months.labels.map(label => label.text), ['Nov 1940', 'Dec', 'Jan 1941', 'Feb', 'Mar', 'Apr']);
});

test('hours before the calendar epoch give no ticks rather than invented dates', () => {
  assert.deepEqual(timeTicks(0, 1000, 800, measure).labels, []);
});

test('quarters are labelled with month names like months are', () => {
  const ticks = timeTicks(at(1940, 1), at(1942, 1), 900, measure);
  assert.equal(ticks.level, 'quarter');
  // Labels sit on their quarter's first tick; the one at the very edge has no room.
  assert.deepEqual(ticks.labels.map(label => label.text), ['Apr 1940', 'Jul', 'Oct', 'Jan 1941', 'Apr', 'Jul', 'Oct']);
  assert.ok(ticks.labels.every(label => ticks.grid.includes(label.hour)));
  // Without a visible January, the first label still names the year.
  assert.equal(timeTicks(at(1940, 2), at(1940, 12, 20), 700, measure).labels[0].text.endsWith('1940'), true);
});

test('fractional ranges from zoom and easing still get ticks', () => {
  const ticks = timeTicks(at(1937) + 0.37, at(1941) - 0.61, 800, measure);
  assert.deepEqual(ticks.labels.map(label => label.text), ['1937', '1938', '1939', '1940']);
});

test('a short year is named once on the first month label', () => {
  const ticks = timeTicks(firstHourOfMonth(42 * 12), firstHourOfMonth(42 * 12 + 5), 600);
  assert.equal(ticks.level, 'month');
  assert.equal(ticks.labels[0].text, 'Jan 42');
});

test('log ticks thin to powers of ten, then fewer of them, to fit the height', () => {
  const ticks = logTicks(1, 1e9);
  // 160 px over nine decades, as a log axis maps it.
  const toPixel = (value: number) => 160 - Math.log10(value) / 9 * 160;
  const kept = thinLogTicks(ticks, toPixel, 17);
  assert.ok(kept.length > 1 && kept.every(value => value.toExponential(0).startsWith('1e')));
  assert.ok(kept.every((value, index) => index === 0 || Math.abs(toPixel(value) - toPixel(kept[index - 1])) >= 17));
  assert.deepEqual(thinLogTicks([1, 2, 5, 10], toPixel, 1), [1, 2, 5, 10]);
});
