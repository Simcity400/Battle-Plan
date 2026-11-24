import test from 'node:test';
import assert from 'node:assert/strict';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { openedIn, rowRequest, rowScope, rowSource, selectionTrack, type OpenedRow, type RowSource } from './rowSelection.ts';

const track = (id: string, branch: string): BoundTrack => ({ id, source: { id: `src-${id}`, revision: '3' }, branch: { id: branch } }) as unknown as BoundTrack;
const WIDGET = { countries: { kind: 'selected', tags: ['GER'] } } as unknown as WidgetSpec;

test('a detail reads the clicked row from the track it was clicked on, and closes when the widget moves to another track', () => {
  const first = track('a', '1');
  const second = track('b', '2');
  const selection: RowSource & { handle: string } = { ...rowSource(first, 1_200), handle: '7' };
  const opened: OpenedRow<typeof selection> = { scope: rowScope(first, WIDGET), key: '7', selection };

  assert.equal(openedIn(opened, rowScope(first, WIDGET)), opened);
  assert.equal(openedIn(opened, rowScope(second, WIDGET)), null, 'the widget now reads another track: the window closes');
  assert.equal(openedIn(opened, rowScope(first, { countries: { kind: 'all' } } as unknown as WidgetSpec)), null, 'a country change is a scope change');

  const request = rowRequest(selection, 'production-line-breakdown', { country: 'GER', handle: selection.handle, group: 'efficiency' });
  assert.deepEqual([request.sourceId, request.revision, request.params.branchId, request.params.hour], ['src-a', '3', '1', 1_200],
    'every request is built from the captured selection, never from the widget\'s current track');
  assert.equal(selectionTrack([second, first], selection), first);
  assert.equal(selectionTrack([second], selection), undefined, 'the track the row was read from is gone: nothing is read from another');
});
