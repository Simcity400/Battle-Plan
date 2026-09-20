import test from 'node:test';
import assert from 'node:assert/strict';
import { countryData } from './countryData.ts';
import type { DashboardClient, CountryFlagRequest } from './client';

test('flag cache separates full, miniature and recorded appearance; retry does not rescan owners', async () => {
  let flags = 0;
  let owners = 0;
  const client = {
    async countryFlags(query: CountryFlagRequest) { flags++; return { ...query, flags: [], warnings: [] }; },
    async landOwners(sourceId: string, revision: string) { owners++; return { sourceId, revision, countries: [], warnings: [] }; },
  } as unknown as DashboardClient;
  const data = countryData(client);
  const signal = new AbortController().signal;
  const query: CountryFlagRequest = { sourceId: 'source', revision: 'r1', branchId: '1', hour: 100, size: 'full', tags: ['GER'] };
  await data.flags(query, signal);
  await data.flags(query, signal);
  assert.equal(flags, 1);
  await data.flags({ ...query, size: 'small' }, signal);
  await data.flags({ ...query, hour: 101 }, signal);
  await data.flags({ ...query, revision: 'r2' }, signal);
  assert.equal(flags, 4);
  await data.owners('source', 'r1', signal);
  data.retryFlags('source');
  await data.flags(query, signal);
  await data.owners('source', 'r1', signal);
  assert.equal(flags, 5);
  assert.equal(owners, 1);
  data.release('source');
  await data.owners('source', 'r1', signal);
  assert.equal(owners, 2);
});
