import assert from 'node:assert/strict';
import test from 'node:test';
import { gameBlocks } from './gameLayout.ts';
import type { GameRun } from '../../generated/dashboard';

const text = (value: string, tone: string | null = null): GameRun => ({ kind: 'text', text: value, tone });
const words = (runs: readonly GameRun[]) => runs.map(run => run.kind === 'text' || run.kind === 'link' ? run.text : '').join('');

test('stat lines become label and value pairs under their heading, and icons and bullets drop away', () => {
  const blocks = gameBlocks([
    { kind: 'icon', sprite: 'GFX_unit|1' }, text(' Super-Heavy Artillery:', 'H'), text('\n   Supply Use: '), text('-0.10', 'G'),
    text('\n   Reliability: '), text('+7%', 'G'), text('\n• Mod Gun:\n   Soft attack:'), text('+12', 'G'), text('\n'),
  ]);
  assert.deepEqual(blocks.map(block => block.kind), ['heading', 'stat', 'stat', 'heading', 'stat']);
  assert.equal(words(blocks[0].kind === 'heading' ? blocks[0].runs : []), 'Super-Heavy Artillery');
  const supply = blocks[1].kind === 'stat' ? blocks[1] : null;
  assert.equal(words(supply!.label), 'Supply Use');
  assert.deepEqual(supply!.value, [text('-0.10', 'G')]);
  assert.equal(words(blocks[3].kind === 'heading' ? blocks[3].runs : []), 'Mod Gun');
});

test('sentences stay whole, linked ones in full colour and plain notes quiet, and a long or wordy colon line is not a stat', () => {
  const link: GameRun = { kind: 'link', entity: 'module', token: 'mod_belt', text: '350mm Belt', tone: 'H' };
  const blocks = gameBlocks([
    text('Enables '), link, text('\n\n'), text('Mounting a better '), text('Armor', 'Y'), text(' module improves things.\nA second note line.\n'),
    text('Note: this changes how the ship works in combat.\n'),
  ]);
  assert.deepEqual(blocks.map(block => [block.kind, block.kind === 'text' && block.quiet]), [['text', false], ['text', true]]);
  assert.equal(words(blocks[1].kind === 'text' ? blocks[1].runs : []),
    'Mounting a better Armor module improves things. A second note line. Note: this changes how the ship works in combat.');
  assert.deepEqual(gameBlocks([text('- Penalty: '), text('-5%', 'R')]).map(block => block.kind === 'stat' && block.value), [[text('-5%', 'R')]]);
  assert.equal(gameBlocks([text('   \n\n')]).length, 0);
});
