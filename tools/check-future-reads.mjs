import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { produceSpanParams, producePointParams } from '../ui/src/families/broker.ts';
import { normalizeView } from '../ui/src/families/manifest.ts';
import { FAMILY_MANIFESTS } from '../ui/src/widgets/familyManifest.ts';

const ratchetPath = fileURLToPath(new URL('./future-reads-ratchet.json', import.meta.url));
const write = process.argv.includes('--write');

const HOURS = Array.from({ length: 21 }, (_, at) => at * 1_000);
const span = hour => ({ firstHour: 0, lastHour: hour });
const ctx = {
  track: { source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign' } }, axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: 20_000 } },
  branch: { id: '1', firstHour: 0, lastHour: 20_000 },
  lastRecordedHour: 20_000,
  widget: { id: 'check', kind: 'check', countries: { kind: 'tags', tags: ['GER'] }, options: {}, time: { mode: 'follow' } },
  options: {},
  following: true,
  aux: undefined,
  countryTag: 'GER',
  countrySubject: 'GER',
  countryName: 'German Reich',
  countries: {
    all: false, tags: ['GER'], holders: null, every: ['GER'], top: null, membership: null, membershipRuns: null, unobserved: [],
    filter: { passing: null, unrecorded: [], bounds: [] }, labels: {},
  },
  gateSpan: (first, last) => ({ firstHour: first, lastHour: last }),
  staticSpan: null,
};
const viewport = { following: true, width: 800, height: 400, hoverHour: null, staticSpan: null };
const text = value => JSON.stringify(value, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));
const constant = request => new Set(HOURS.map(hour => text(request(hour)))).size === 1 && request(HOURS[0]) !== null;

function unbounded(read) {
  switch (read.shape) {
    case 'pixel-binned':
    case 'paged-rows':
    case 'exact-hover':
      return null;
    case 'point':
      if (read.followQuery !== null || !read.params) return null;
      return constant(hour => producePointParams(read, hour, 'static')) ? 'a point read whose request is the same at every hour' : null;
    case 'range':
    case 'branch-to-hour':
    case 'trailing-window':
      if (read.shape !== 'range' && read.followQuery !== null) return null;
      if (!read.params) return null;
      return constant(hour => produceSpanParams(read.params, span(hour), read.buildSpanParams)) ? `a ${read.shape} read that asks the same history at every hour` : null;
    default:
      return `an unknown read shape ${read.shape}`;
  }
}

const found = new Map();
const unevaluated = [];
for (const mod of FAMILY_MANIFESTS) {
  for (const entry of mod.views) {
    const view = normalizeView(entry.view);
    const spec = await entry.loadSpec();
    const declaration = spec.declaration;
    if (!declaration || typeof declaration.sources !== 'function') continue;
    let plan;
    try {
      plan = declaration.sources(ctx, viewport);
    } catch (error) {
      unevaluated.push(`${entry.kind}/${view}: its plan could not be built for a followed country (${error instanceof Error ? error.message : String(error)})`);
      continue;
    }
    for (const item of plan?.entries ?? []) {
      let reason;
      try {
        reason = unbounded(item.read);
      } catch (error) {
        reason = `its request could not be built (${error instanceof Error ? error.message : String(error)})`;
      }
      if (reason) found.set(`${entry.kind}/${view}/${item.name}`, `${mod.family}: ${reason}`);
    }
  }
}

const listed = JSON.parse(readFileSync(ratchetPath, 'utf8'));
const keys = [...found.keys()].sort();
if (write) {
  writeFileSync(ratchetPath, `${JSON.stringify(Object.fromEntries(keys.map(key => [key, found.get(key)])), null, 2)}\n`);
  console.log(`future-reads: wrote ${keys.length} entries`);
  process.exit(0);
}
const errors = [...unevaluated];
for (const key of keys) if (!Object.hasOwn(listed, key)) errors.push(`${key}: ${found.get(key)}: a new source must be asked to the displayed hour (a span or point request that names the hour, tiles, paged rows or a followed answer), so its reply holds nothing after it (ADR 1402 item 5)`);
for (const key of Object.keys(listed)) if (!found.has(key)) errors.push(`${key} is asked to the hour now: remove it from tools/future-reads-ratchet.json (the list only falls)`);
if (errors.length) {
  for (const error of errors) console.error(`future-reads: ${error}`);
  process.exit(1);
}
console.log(`future-reads: OK (${keys.length} source(s) still asked beyond the displayed hour)`);
