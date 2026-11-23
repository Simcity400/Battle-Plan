import type { GameRun } from '../../generated/dashboard';

export type GameBlock =
  | { kind: 'heading'; runs: GameRun[] }
  | { kind: 'stat'; label: GameRun[]; value: GameRun[] }
  | { kind: 'text'; runs: GameRun[]; quiet: boolean };

const BULLET = /^\s*(?:[•·*–—]|-(?=\s))?\s*/u;
const STAT_VALUE = 24;
const STAT_LABEL = 64;

function lines(runs: readonly GameRun[]): GameRun[][] {
  const out: GameRun[][] = [[]];
  for (const run of runs) {
    if (run.kind !== 'text') { out[out.length - 1].push(run); continue; }
    run.text.split('\n').forEach((part, at) => {
      if (at > 0) out.push([]);
      if (part) out[out.length - 1].push({ ...run, text: part });
    });
  }
  return out;
}

const plain = (runs: readonly GameRun[]) => runs.map(run => run.kind === 'text' || run.kind === 'link' ? run.text : run.kind === 'flag' ? run.tag : '').join('');

function tidy(line: readonly GameRun[], lead = BULLET): GameRun[] {
  const out = [...line];
  while (out[0]?.kind === 'text') {
    const text = out[0].text.replace(lead, '');
    if (text) { out[0] = { ...out[0], text }; break; }
    out.shift();
  }
  while (out.length && out[out.length - 1].kind === 'text') {
    const last = out[out.length - 1] as Extract<GameRun, { kind: 'text' }>;
    const text = last.text.replace(/\s+$/u, '');
    if (text) { out[out.length - 1] = { ...last, text }; break; }
    out.pop();
  }
  return out;
}

function withoutColon(runs: GameRun[]): GameRun[] {
  const out = [...runs];
  const last = out[out.length - 1];
  if (last?.kind === 'text') {
    const text = last.text.replace(/[\s:：]+$/u, '');
    if (text) out[out.length - 1] = { ...last, text }; else out.pop();
  }
  return out;
}

function stat(line: GameRun[]): { label: GameRun[]; value: GameRun[] } | null {
  for (let at = line.length - 1; at >= 0; at--) {
    const run = line[at];
    if (run.kind !== 'text') continue;
    const colon = Math.max(run.text.lastIndexOf(':'), run.text.lastIndexOf('：'));
    if (colon < 0) continue;
    const before = run.text.slice(0, colon), after = run.text.slice(colon + 1);
    const label = withoutColon([...line.slice(0, at), ...(before ? [{ ...run, text: before }] : [])]);
    const value = tidy([...(after.trim() ? [{ ...run, text: after.trim() }] : []), ...line.slice(at + 1)], /^\s+/u);
    const valueText = plain(value), labelText = plain(label).trim();
    if (!labelText || !valueText || valueText.length > STAT_VALUE || labelText.length > STAT_LABEL || !/\d/u.test(valueText)) return null;
    return { label, value };
  }
  return null;
}

export function gameBlocks(runs: readonly GameRun[]): GameBlock[] {
  const out: GameBlock[] = [];
  for (const raw of lines(runs)) {
    const line = tidy(raw);
    const text = plain(line).trim();
    if (!text && !line.some(run => run.kind === 'icon')) continue;
    if (/[:：]$/u.test(text)) { out.push({ kind: 'heading', runs: withoutColon(line) }); continue; }
    const parts = stat(line);
    if (parts) { out.push({ kind: 'stat', ...parts }); continue; }
    const quiet = line.every(run => run.kind !== 'link');
    const previous = out[out.length - 1];
    if (previous?.kind === 'text' && previous.quiet === quiet && quiet) previous.runs.push({ kind: 'text', text: ' ', tone: null }, ...line);
    else out.push({ kind: 'text', runs: line, quiet });
  }
  return out;
}
