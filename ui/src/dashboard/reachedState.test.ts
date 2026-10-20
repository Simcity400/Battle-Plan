import assert from 'node:assert/strict';
import test from 'node:test';
import { reachedKeys, reachedView, settle } from './reachedState.ts';

test('a failed request never makes an earlier answer current', () => {
  const answered = settle<string>(undefined, 'GER', 'hour 10', { data: 'as of hour 10' });
  assert.deepEqual(reachedView(answered, 'GER', 'hour 10'), { data: 'as of hour 10', current: true, error: undefined });
  const failed = settle(answered, 'GER', 'hour 11', { error: 'read failed' });
  assert.deepEqual(reachedView(failed, 'GER', 'hour 11'), { data: 'as of hour 10', current: false, error: 'read failed' },
    'the earlier answer is kept, stale, beside the failure');
  assert.deepEqual(reachedView(failed, 'GER', 'hour 12'), { data: 'as of hour 10', current: false, error: undefined },
    'a request still loading shows neither its own error nor a current answer');
  const recovered = settle(failed, 'GER', 'hour 12', { data: 'as of hour 12' });
  assert.deepEqual(reachedView(recovered, 'GER', 'hour 12'), { data: 'as of hour 12', current: true, error: undefined });
});

test('another subject never inherits an answer or a failure', () => {
  const answered = settle<string>(undefined, 'GER', 'hour 10', { data: 'Germany' });
  assert.deepEqual(reachedView(answered, 'FRA', 'hour 10'), { data: undefined, current: false, error: undefined });
  const failed = settle(answered, 'FRA', 'hour 10', { error: 'read failed' });
  assert.equal(failed.data, undefined);
});

test('changing only the choice, while its country holds the same tag, asks again', () => {
  const request = { sourceId: 's', revision: '1', product: 'training-queue', params: { branchId: '1', hour: 200, tag: 'DEU' } };
  const before = reachedKeys(request, JSON.stringify(['1', 'GER']), 0), after = reachedKeys(request, JSON.stringify(['1', 'DEU']), 0);
  assert.notEqual(before.key, after.key, 'the same parameters under another choice are another request');
  const answered = settle<string>(undefined, before.owner, before.key!, { data: 'queue' });
  assert.deepEqual(reachedView(answered, after.owner, after.key), { data: undefined, current: false, error: undefined },
    'the earlier choice’s answer is not shown, and the new key loads');
  assert.equal(reachedKeys(null, 'x', 0).key, null);
});
