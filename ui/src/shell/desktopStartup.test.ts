import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('desktop setup does only the ordered work the initial window and its readers need', () => {
  const source = readFileSync(new URL('../../../crates/desktop/src/main.rs', import.meta.url), 'utf8');
  const setup = source.slice(source.indexOf('fn main()'), source.indexOf('    app.run('));
  const startupCalls = /\b(?:return_freed_memory_promptly|name_shared_pool|refused_during_setup|context|init|map_prefix|log_map_frames|warm|configure_desktop|start_exit_worker|restore|start_recorder|sweep_pending_stages|start_gamepad|install_follow|install_export_follow|watch_page|follow|ensure_started|prepare)\(/g;
  assert.deepEqual([...setup.matchAll(startupCalls)].map(([call]) => call.slice(0, -1)), [
    'return_freed_memory_promptly', 'name_shared_pool', 'refused_during_setup', 'context',
    'init', 'map_prefix', 'log_map_frames', 'configure_desktop', 'start_exit_worker',
    'restore', 'init', 'start_recorder', 'sweep_pending_stages', 'start_gamepad',
    'install_follow', 'install_export_follow', 'watch_page', 'follow', 'ensure_started',
  ]);
});
