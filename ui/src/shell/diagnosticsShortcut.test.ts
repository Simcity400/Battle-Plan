import test from 'node:test';
import assert from 'node:assert/strict';
import { isDiagnosticsShortcut } from './diagnosticsShortcut.ts';

const press = { key: 'F3', ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, repeat: false, defaultPrevented: false };

test('only a single unmodified F3 press toggles map diagnostics', () => {
  assert.equal(isDiagnosticsShortcut(press), true);
  assert.equal(isDiagnosticsShortcut({ ...press, repeat: true }), false);
  assert.equal(isDiagnosticsShortcut({ ...press, ctrlKey: true }), false);
  assert.equal(isDiagnosticsShortcut({ ...press, shiftKey: true }), false);
  assert.equal(isDiagnosticsShortcut({ ...press, defaultPrevented: true }), false);
  assert.equal(isDiagnosticsShortcut({ ...press, key: 'F2' }), false);
});
