import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('desktop setup does only the ordered work the initial window and its readers need', () => {
  const source = readFileSync(new URL('../../../crates/desktop/src/main.rs', import.meta.url), 'utf8');
  const setup = source.slice(source.indexOf('fn main()'), source.indexOf('    app.run('));
  const startupCalls = /\b(?:shader_pipeline::artwork::warm|dashboard_text::artwork::(?:configure|prepare)_desktop|video::(?:VideoState::default|restore)|app_update::(?:init|prepare|start)|recorder::start_recorder|export_settings::sweep_pending_stages|native::start_gamepad)\(/g;
  assert.deepEqual([...setup.matchAll(startupCalls)].map(([call]) => call.slice(0, -1)), [
    'dashboard_text::artwork::prepare_desktop', 'video::VideoState::default',
    'video::restore', 'app_update::prepare', 'native::start_gamepad',
  ]);
});
