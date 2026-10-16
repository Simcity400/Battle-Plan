import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { PNG } from 'pngjs';
import { readJSON } from './cache.mjs';
import { modelError } from './capture.mjs';

function canonicalOwnerSource(source, leases) {
  if (typeof source !== 'string') return source;
  let tuple;
  try { tuple = JSON.parse(source); } catch { return `literal:${source}`; }
  if (!Array.isArray(tuple) || tuple.length !== 2 || typeof tuple[0] !== 'string'
    || !/^lease-[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}-\d+$/i.test(tuple[0])) return `literal:${source}`;
  // UUIDs and allocation counters belong to the bridge process. A capture-local
  // identity preserves which rows share an owner and which hold separate leases.
  if (!leases.has(tuple[0])) leases.set(tuple[0], leases.size + 1);
  // Distinct tags keep literal source strings from colliding with aliases.
  return `runtime:${JSON.stringify([leases.get(tuple[0]), tuple[1]])}`;
}

function canonicalListOwnership(value, leases, owner = false) {
  if (!value || typeof value !== 'object') return value;
  let canonical = value;
  for (const key of Object.keys(value)) {
    const next = owner && key === 'source' ? canonicalOwnerSource(value[key], leases)
      : canonicalListOwnership(value[key], leases, key === 'pickOwner');
    if (!Object.is(next, value[key])) {
      if (canonical === value) canonical = Array.isArray(value) ? value.slice() : { ...value };
      canonical[key] = next;
    }
  }
  return canonical;
}

function canonicalLists(lists) {
  const leases = new Map();
  return lists.map(list => {
    // Provider IDs and traversal timings describe this capture's runtime.
    // Preserve every semantic field, including titles/slots outside row arrays.
    const { id, source, steps, traversal, geometry, diagnostics, ...content } = list;
    return canonicalListOwnership({ ...content, label: list.label ?? null }, leases);
  });
}

export async function compareCaptures(baseDirectory, headDirectory) {
  const base = await readJSON(path.join(baseDirectory, 'manifest.json'));
  const head = await readJSON(path.join(headDirectory, 'manifest.json'));
  const identity = item => item.caseId ?? item.kind;
  for (const manifest of [base, head]) if (new Set(manifest.items.map(identity)).size !== manifest.items.length)
    throw Error('Duplicate capture identity');
  const kinds = new Set([...base.items.map(identity), ...head.items.map(identity)]);
  const results = [];
  for (const key of kinds) {
    let status = 'ERROR', reason;
    const a = base.items.find(item => identity(item) === key), b = head.items.find(item => identity(item) === key);
    const kind = a?.kind ?? b?.kind;
    try {
      if (a?.kind !== b?.kind || JSON.stringify(a?.scenario) !== JSON.stringify(b?.scenario)
        || JSON.stringify(a?.action) !== JSON.stringify(b?.action)) throw Error('Capture choices or reached action differ');
      for (const capture of [a, b]) {
        if (capture?.status !== 'captured') throw Error(capture?.error || 'Missing widget');
        if (!capture.model?.header || !capture.model?.body || !Array.isArray(capture.model.lists)) throw Error('Invalid widget view-model');
        for (const list of capture.model.lists) { const error = modelError(list); if (error) throw Error(error); }
      }
      const pixels = await Promise.all([baseDirectory, headDirectory].map(async (dir, index) =>
        PNG.sync.read(await readFile(path.join(dir, [a, b][index].screenshot)))));
      const samePixels = pixels[0].width === pixels[1].width && pixels[0].height === pixels[1].height && pixels[0].data.equals(pixels[1].data);
      const models = [a, b].map(capture => ({ ...capture.model, lists: canonicalLists(capture.model.lists) }));
      const sameModel = JSON.stringify(models[0]) === JSON.stringify(models[1]);
      status = samePixels && sameModel ? 'PASS' : 'DIFF';
      if (status === 'DIFF') reason = `view-model=${sameModel ? 'same' : 'different'}, pixels=${samePixels ? 'same' : 'different'}`;
    } catch (error) { reason = String(error); }
    results.push({ kind, ...(a?.caseId || b?.caseId ? { caseId: key } : {}), status, ...(reason ? { reason } : {}) });
  }
  const counts = Object.fromEntries(['PASS', 'DIFF', 'ERROR'].map(status => [status, results.filter(result => result.status === status).length]));
  return { pass: base.complete && head.complete && kinds.size > 0 && counts.PASS === kinds.size, counts, results };
}
