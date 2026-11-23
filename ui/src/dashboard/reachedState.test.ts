import { gatedHour } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { reachedKeys, reachedView } from './reachedState.ts';
import { productRequest } from './request.ts';

test('a failed request never makes an earlier answer current', () => {
  assert.deepEqual(reachedView({ data: 'as of hour 10', viewKey: 'hour 10' }, 'hour 10'), { data: 'as of hour 10', current: true, error: undefined });
  const failed = { data: 'as of hour 10', viewKey: 'hour 10', error: new Error('read failed'), errorKey: 'hour 11' };
  assert.deepEqual(reachedView(failed, 'hour 11'), { data: 'as of hour 10', current: false, error: new Error('read failed') },
    'the earlier answer is kept, stale, beside the failure');
  assert.deepEqual(reachedView(failed, 'hour 12'), { data: 'as of hour 10', current: false, error: undefined },
    'a request still loading shows neither its own error nor a current answer');
  assert.deepEqual(reachedView({ data: 'as of hour 12', viewKey: 'hour 12' }, 'hour 12'), { data: 'as of hour 12', current: true, error: undefined });
  assert.deepEqual(reachedView({ viewKey: 'hour 10', error: new Error('read failed'), errorKey: 'hour 10' }, 'hour 10'), { data: undefined, current: false, error: new Error('read failed') });
});

test('changing only the choice, while its country holds the same tag, asks again', () => {
  const request = productRequest({ sourceId: 's', revision: '1', product: 'training-queue', params: { branchId: '1', hour: gatedHour(200), tag: 'DEU' } });
  const before = reachedKeys(request, JSON.stringify(['1', 'GER']), 0), after = reachedKeys(request, JSON.stringify(['1', 'DEU']), 0);
  assert.notEqual(before.key, after.key, 'the same parameters under another choice are another request');
  assert.notEqual(before.owner, after.owner, 'another choice reads in another lane, which drops the earlier answer');
  assert.equal(reachedKeys(null, 'x', 0).key, null);
});
