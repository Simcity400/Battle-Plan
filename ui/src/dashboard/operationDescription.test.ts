import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readDescriptionPages } from './operationDescription.ts';
const description = { hour: -3, id: '18446744073709551615', owner: 5000, sequence: '91' };
const signal = () => new AbortController().signal;
const page = (offset: string, bytes: number[], nextOffset: string | null, totalBytes = '4') => ({ description, offset, bytes, nextOffset,
  totalBytes, recordedLanguage: 'recorded language', unavailable: null });
test('description paging decodes UTF-8 across a byte boundary without replacing characters', async () => {
  const calls: string[] = [];
  const result = await readDescriptionPages(description, async offset => {
    calls.push(offset);
    return offset === '0' ? page(offset, [0xF0, 0x9F], '2') : page(offset, [0x8E, 0xAF], null);
  }, signal());
  assert.equal(result.text, '\u{1F3AF}');
  assert.deepEqual(calls, ['0', '2']);
  await assert.rejects(readDescriptionPages(description, async offset => page(offset, [0xFF], null, '1'), signal()), /non-UTF-8/);
});
test('a refused description and changed or stalled page remain named failures', async () => {
  await assert.rejects(readDescriptionPages(description, async offset => ({ ...page(offset, [], null, '1000000'), unavailable: 'Native bytes refused' }), signal()), /Native bytes refused/);
  await assert.rejects(readDescriptionPages(description, async offset => page(offset, [], '0'), signal()), /exact byte range/);
  await assert.rejects(readDescriptionPages(description, async offset => ({ ...page(offset, [0], null, '1'), description: { ...description, sequence: '92' } }), signal()), /malformed/);
  const empty = await readDescriptionPages(description, async offset => page(offset, [], null, '0'), signal());
  assert.equal(empty.text, '');
});
