import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('../../../crates/desktop/src/main.rs', import.meta.url), 'utf8');

test('desktop setup does only the ordered work the initial window and its readers need', () => {
  const setup = source.slice(source.indexOf('fn main()'), source.indexOf('    app.run('));
  const startupCalls = /\b(?:shader_pipeline::artwork::warm|dashboard_text::artwork::(?:configure|prepare)_desktop|video::(?:VideoState::default|restore)|app_update::(?:init|prepare|start)|recorder::start_recorder|export_settings::sweep_pending_stages|native::start_gamepad)\(/g;
  assert.deepEqual([...setup.matchAll(startupCalls)].map(([call]) => call.slice(0, -1)), [
    'dashboard_text::artwork::prepare_desktop', 'video::VideoState::default',
    'video::restore', 'app_update::prepare', 'native::start_gamepad',
  ]);
});

test('desktop update checks start off-thread from native Ready, even without a page call', () => {
  const eventLoop = source.slice(source.indexOf('    app.run('));
  const ready = eventLoop.match(/tauri::RunEvent::Ready\s*=>\s*\{([^}]*)\}/)?.[1];
  assert.ok(ready, 'the desktop event loop handles native Ready');
  assert.match(ready, /tauri::async_runtime::spawn_blocking\(move \|\| app_update::start\(&app\)\)/);
  assert.equal([...source.matchAll(/\bapp_update::start\(/g)].length, 1);
});
