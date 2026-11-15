import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

test('a live calendar edits the day it shows: a time picked after playback moved to the next day is of that day', async () => {
  await withNativePage('date-picker', '/ui/src/components/DateTimePicker.dom.fixture.tsx', page => page.stage('date picker', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.datePicker.${expression}`);
    assert.equal(await act('ready()'), true);
    await act('open()');
    assert.equal(await act('selectedDay()'), '1');
    await act('play(24)');
    assert.equal(await act('selectedDay()'), '2', 'the calendar follows the live day');
    const { chosen, wanted } = await act<{ chosen: number | null; wanted: number }>('pickTime(12)');
    assert.equal(chosen, wanted, 'the time is set on the day highlighted, not the day it opened on');
  }));
});
