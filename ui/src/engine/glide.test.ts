import assert from 'node:assert/strict';
import test from 'node:test';
import { FrameEngine } from './frame.ts';
import { StyleGlides, cssEase, rowEase } from './glide.ts';

test('the glide curve is CSS ease: its ends, its known midpoint, and always rising', () => {
  assert.equal(cssEase(0), 0);
  assert.equal(cssEase(1), 1);
  assert.ok(Math.abs(cssEase(0.5) - 0.8024) < 0.002, `ease(0.5) = ${cssEase(0.5)}`);
  for (let t = 0.05; t < 1; t += 0.05) assert.ok(cssEase(t) > cssEase(t - 0.05));
  assert.ok(rowEase(0.25) > 0.7 && rowEase(0.25) < 0.85, `the rows' curve is fast early: ${rowEase(0.25)}`);
});

test('a bar glides to its target over the glide time in the paint phase, retargets from where it is, and snaps with none', () => {
  let run: ((now: number) => void) | null = null;
  const frames = new FrameEngine({ request: next => { run = next; } });
  let now = 0;
  const glides = new StyleGlides(fraction => `scaleX(${fraction})`, cssEase, frames, () => now);
  const frame = (at: number) => { now = at; const next = run!; run = null; next(at); };
  const bar: { transform?: string } = {};
  glides.to(bar, 0.2, 300);
  assert.equal(bar.transform, 'scaleX(0.2)', 'first shown at once: nothing to glide from');
  glides.to(bar, 0.8, 300);
  frame(150);
  const mid = glides.at(bar)!;
  assert.ok(mid > 0.2 && mid < 0.8 && Math.abs(mid - (0.2 + 0.6 * cssEase(0.5))) < 1e-9, `half way by the curve: ${mid}`);
  now = 150;
  glides.to(bar, 0.4, 300);
  frame(300);
  const retargeted = glides.at(bar)!;
  assert.ok(Math.abs(retargeted - (mid + (0.4 - mid) * cssEase(0.5))) < 1e-9, 'from where it was, with a whole glide again');
  frame(450);
  assert.equal(bar.transform, 'scaleX(0.4)');
  assert.equal(run, null, 'at rest: no frame asked for');
  glides.to(bar, 0.9, 0);
  assert.equal(bar.transform, 'scaleX(0.9)', 'no glide time (paused, reduced motion): at once');
});

test('a glide passes through its way over several frames, only rising, and ends at its target, whatever the frame times', () => {
  const play = (times: number[]) => {
    let run: ((now: number) => void) | null = null;
    const frames = new FrameEngine({ request: next => { run = next; } });
    let now = 0;
    const glides = new StyleGlides(fraction => `scaleX(${fraction})`, cssEase, frames, () => now);
    const waiting = () => run !== null;
    const frame = (at: number) => { now = at; const next = run!; run = null; next(at); };
    const bar: { transform?: string } = {};
    glides.to(bar, 0.2, 300);
    glides.to(bar, 0.8, 300);
    const seen: number[] = [];
    for (const at of times) {
      if (!waiting()) break;
      frame(at);
      seen.push(Number(/scaleX\(([^)]+)\)/.exec(bar.transform!)![1]));
    }
    return { seen, resting: !waiting() };
  };
  const steady = play(Array.from({ length: 40 }, (_, at) => (at + 1) * 1000 / 95));
  assert.ok(steady.seen.filter(value => value > 0.2 + 1e-6 && value < 0.8 - 1e-6).length >= 3, `through its way: ${steady.seen.join(' ')}`);
  for (const { seen, resting } of [steady, play([16, 33, 450, 466])]) {
    assert.ok(seen.every((value, at) => at === 0 || value >= seen[at - 1] - 1e-9), `only rising: ${seen.join(' ')}`);
    assert.equal(seen.at(-1), 0.8, 'ends at its target, a stalled frame included');
    assert.ok(resting, 'at rest afterwards: no frame asked for');
  }
});
