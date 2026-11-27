import { mkdir, readFile, writeFile } from 'node:fs/promises';
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
  if (!leases.has(tuple[0])) leases.set(tuple[0], leases.size + 1);
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
    const { id, source, steps, traversal, geometry, diagnostics, ...content } = list;
    return canonicalListOwnership({ ...content, label: list.label ?? null }, leases);
  });
}

function mapPictureTolerance(base, head) {
  const { width, height } = head;
  const differences = new Uint8Array(width * height);
  let changed = 0, toleratedPixels = 0;
  for (let pixel = 0; pixel < differences.length; pixel++) {
    const index = pixel * 4;
    for (let channel = index; channel < index + 4; channel++)
      differences[pixel] = Math.max(differences[pixel], Math.abs(base.data[channel] - head.data[channel]));
    if (differences[pixel]) changed++;
  }
  for (let pixel = 0; pixel < differences.length; pixel++) {
    if (differences[pixel] !== 1) continue;
    const x = pixel % width, y = Math.floor(pixel / width);
    let isolated = true;
    for (let row = Math.max(0, y - 1); row <= Math.min(height - 1, y + 1); row++) {
      for (let col = Math.max(0, x - 1); col <= Math.min(width - 1, x + 1); col++) {
        const neighbour = row * width + col;
        if (neighbour !== pixel && differences[neighbour]) isolated = false;
      }
    }
    if (!isolated) continue;
    differences[pixel] = 0;
    toleratedPixels++;
  }
  return { differences, changed: changed - toleratedPixels, toleratedPixels };
}

export function diffImage(base, head, differences = null) {
  const out = new PNG({ width: head.width, height: head.height });
  let changed = 0;
  for (let index = 0; index < head.data.length; index += 4) {
    const same = differences ? differences[index / 4] === 0 : base.data[index] === head.data[index] && base.data[index + 1] === head.data[index + 1]
      && base.data[index + 2] === head.data[index + 2] && base.data[index + 3] === head.data[index + 3];
    if (!same) changed++;
    out.data[index] = same ? head.data[index] >> 2 : 255;
    out.data[index + 1] = same ? head.data[index + 1] >> 2 : 0;
    out.data[index + 2] = same ? head.data[index + 2] >> 2 : 0;
    out.data[index + 3] = 255;
  }
  return { png: PNG.sync.write(out), changed };
}

export async function compareCaptures(baseDirectory, headDirectory, { diffDirectory = null, tolerateMapPictures = false } = {}) {
  const base = await readJSON(path.join(baseDirectory, 'manifest.json'));
  const head = await readJSON(path.join(headDirectory, 'manifest.json'));
  const identity = item => item.caseId ?? item.kind;
  for (const manifest of [base, head]) if (new Set(manifest.items.map(identity)).size !== manifest.items.length)
    throw Error('Duplicate capture identity');
  const kinds = new Set([...base.items.map(identity), ...head.items.map(identity)]);
  const results = [];
  for (const key of kinds) {
    let status = 'ERROR', reason, diff, toleratedPixels;
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
      const sameSize = pixels[0].width === pixels[1].width && pixels[0].height === pixels[1].height;
      const exact = sameSize && pixels[0].data.equals(pixels[1].data);
      const tolerance = tolerateMapPictures && sameSize && !exact && kind === 'map' ? mapPictureTolerance(...pixels) : null;
      if (tolerateMapPictures && sameSize && kind === 'map') toleratedPixels = tolerance?.toleratedPixels ?? 0;
      const samePixels = exact || tolerance?.changed === 0;
      const models = [a, b].map(capture => ({ ...capture.model, lists: canonicalLists(capture.model.lists) }));
      const sameModel = JSON.stringify(models[0]) === JSON.stringify(models[1]);
      status = samePixels && sameModel ? 'PASS' : 'DIFF';
      if (status === 'DIFF') reason = `view-model=${sameModel ? 'same' : 'different'}, pixels=${samePixels ? 'same' : 'different'}`;
      if (status === 'DIFF' && !samePixels && !sameSize) reason += `, size ${pixels[0].width}x${pixels[0].height} became ${pixels[1].width}x${pixels[1].height}`;
      if (diffDirectory && !samePixels && sameSize) {
        const { png, changed } = diffImage(...pixels, tolerance?.differences);
        diff = `${String(key).replace(/[^\w.-]+/g, '_')}.diff.png`;
        await mkdir(diffDirectory, { recursive: true });
        await writeFile(path.join(diffDirectory, diff), png);
        reason += `, ${changed} pixels differ`;
      }
      if (toleratedPixels) reason = [reason, `${toleratedPixels} isolated map pixel${toleratedPixels === 1 ? '' : 's'} tolerated`].filter(Boolean).join(', ');
    } catch (error) { reason = String(error); }
    results.push({ kind, ...(a?.caseId || b?.caseId ? { caseId: key } : {}), status, ...(reason ? { reason } : {}), ...(diff ? { diff } : {}), ...(toleratedPixels === undefined ? {} : { toleratedPixels }) });
  }
  const counts = Object.fromEntries(['PASS', 'DIFF', 'ERROR'].map(status => [status, results.filter(result => result.status === status).length]));
  return { pass: base.complete && head.complete && kinds.size > 0 && counts.PASS === kinds.size, counts, results };
}
