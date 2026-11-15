import assert from 'node:assert/strict';
import test from 'node:test';
import { FrameEngine } from './frame.ts';
import { ENTRY_MS, EXIT_MS, RowFades } from './entry.ts';
import { cssEase, rowEase } from './glide.ts';

/** A row's style as the engine writes it. */
function row() {
  const props = new Map<string, string>();
  return { props, style: { opacity: '', setProperty: (name: string, value: string) => { props.set(name, value); }, removeProperty: (name: string) => { props.delete(name); return ''; } } };
}

test('an entering row fades in while its flash fades out along the rows\' curve over the entry, then is left as it was', () => {
  let run: ((now: number) => void) | null = null;
  const frames = new FrameEngine({ request: next => { run = next; } });
  let now = 0;
  const entries = new RowFades(frames, () => now, () => false);
  const entering = row();
  entries.enter(entering);
  assert.deepEqual([entering.style.opacity, entering.props.get('--row-flash-opacity')], ['0', '1'], 'it starts unseen, its flash full');
  now = ENTRY_MS / 2;
  run!(now);
  assert.equal(Number(entering.style.opacity), rowEase(0.5));
  assert.equal(Number(entering.props.get('--row-flash-opacity')), 1 - rowEase(0.5));
  now = ENTRY_MS;
  run!(now);
  assert.equal(entering.style.opacity, '', 'done: its own opacity again');
  assert.equal(entering.props.has('--row-flash-opacity'), false);
  assert.equal(entries.has(entering), false);
  entries.leave(entering);
  assert.equal(entering.style.opacity, '1', 'leaving starts from where it is');
  now = ENTRY_MS + EXIT_MS / 2;
  run!(now);
  assert.equal(Number(entering.style.opacity), 1 - cssEase(0.5));
  now = ENTRY_MS + EXIT_MS;
  run!(now);
  assert.equal(entering.style.opacity, '', 'gone: its leaving class holds it unseen');
});

test('a row leaving while it enters fades out from where its entry had it', () => {
  let run: ((now: number) => void) | null = null;
  const frames = new FrameEngine({ request: next => { run = next; } });
  let now = 0;
  const fades = new RowFades(frames, () => now, () => false);
  const entering = row();
  fades.enter(entering);
  now = ENTRY_MS / 4;
  run!(now);
  const reached = Number(entering.style.opacity);
  fades.leave(entering);
  assert.equal(Number(entering.style.opacity), reached);
  assert.equal(entering.props.has('--row-flash-opacity'), false, 'its flash goes with it');
});

test('a leaving row that stays after all fades back in from where it is, with no flash; an entering row is not restarted', () => {
  let run: ((now: number) => void) | null = null;
  const frames = new FrameEngine({ request: next => { run = next; } });
  let now = 0;
  const fades = new RowFades(frames, () => now, () => false);
  const back = row();
  fades.leave(back);
  now = EXIT_MS / 2;
  run!(now);
  const reached = Number(back.style.opacity);
  assert.ok(reached > 0 && reached < 1, `half way out: ${reached}`);
  fades.stay(back);
  assert.equal(Number(back.style.opacity), reached, 'it turns back from where it is');
  now = EXIT_MS / 2 + EXIT_MS / 2;
  run!(now);
  assert.equal(Number(back.style.opacity), reached + (1 - reached) * cssEase(0.5), 'back toward full along the exit\'s curve');
  assert.equal(back.props.has('--row-flash-opacity'), false, 'no entry flash: it never left');
  now = EXIT_MS / 2 + EXIT_MS;
  run!(now);
  assert.equal(back.style.opacity, '', 'back: its own opacity, never cleared to unseen at the old exit deadline');
  assert.equal(fades.has(back), false);

  const entering = row();
  fades.enter(entering);
  now += ENTRY_MS / 4;
  run!(now);
  const entered = Number(entering.style.opacity);
  fades.stay(entering);
  assert.equal(Number(entering.style.opacity), entered, 'an entry going on is left as it is');
  assert.equal(entering.props.get('--row-flash-opacity'), String(1 - entered));
});

test('with reduced motion a row simply shows', () => {
  const entries = new RowFades(new FrameEngine({ request: () => { throw new Error('no frame for reduced motion'); } }), () => 0, () => true);
  const entering = row();
  entries.enter(entering);
  assert.equal(entering.style.opacity, '');
});
