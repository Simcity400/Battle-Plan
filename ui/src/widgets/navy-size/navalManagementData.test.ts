import assert from 'node:assert/strict';
import test from 'node:test';
import { validateNavalManagement } from './navalManagementData.ts';

test('naval management retains exact native display strings and empty coverage', () => {
  const reply = { branchId: '1', hour: -1, warnings: [], sections: [{ key: 'task:18446744073709551615', title: 'Synthetic fleet', lines: ['-92233720368547.75808 IC'] }] };
  assert.equal(validateNavalManagement(reply), reply);
  assert.equal(validateNavalManagement({ ...reply, sections: [] }).sections.length, 0);
  assert.throws(() => validateNavalManagement({ ...reply, sections: undefined }), /malformed/);
});
test('naval management refuses malformed entries and non-native dates without a row cap', () => {
  const reply = { branchId: '1', hour: 2, warnings: [], sections: [] };
  for (const value of [{ ...reply, hour: 2147483648 }, { ...reply, sections: [null] }, { ...reply, warnings: [null] },
    { ...reply, sections: [{ key: 'x', title: 'y', lines: [4] }] }]) assert.throws(() => validateNavalManagement(value), /malformed/);
  const sections = Array.from({ length: 5000 }, (_, index) => ({ key: `task:${index}`, title: `Synthetic task ${index}`, lines: [] }));
  assert.equal(validateNavalManagement({ ...reply, sections }).sections.length, 5000);
});
