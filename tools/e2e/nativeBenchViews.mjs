import { existsSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

registerHooks({
  resolve(specifier, context, next) {
    if (specifier.startsWith('.') && context.parentURL?.startsWith('file:') && !/\.[^/]+$/.test(specifier)) {
      for (const extension of ['.ts', '.tsx']) {
        const url = new URL(specifier + extension, context.parentURL);
        if (existsSync(fileURLToPath(url))) return next(url.href, context);
      }
    }
    return next(specifier, context);
  },
});

const { enumerateWidgetPlacements } = await import('./allWidgets.mjs');
const { manifestFamilyView, widgetViewOptions } = await import('../../ui/src/widgets/manifest.ts');
const { createWidget, GRID_COLUMNS, resolveWindow } = await import('../../ui/src/dashboard/model.ts');
const { PAGES } = await import('../../ui/src/dashboard/pages.ts');
const { selectedCountryKeys } = await import('../../ui/src/dashboard/countrySelection.ts');
const { countryFromKey } = await import('../../ui/src/dashboard/countryKey.ts');
const { countryIdentityIndex } = await import('../../ui/src/dashboard/countryIdentity.ts');
const { SlotTags } = await import('../../ui/src/dashboard/countrySlots.ts');
const { loadCountryGroups } = await import('../../ui/src/dashboard/products.ts');
const { rankedCandidates, resolveCountries } = await import('../../ui/src/dashboard/groups.ts');
const { followedProduct, lastRecordedHour, refusedHours } = await import('../../ui/src/dashboard/regionTime.ts');
const { buildPlaybackBoundedRequest, buildStaticRequest, compileFollowPlan, resolvePlaybackProvenInterval } = await import('../../ui/src/families/broker.ts');
const { gatedByCountry, planFamilySources } = await import('../../ui/src/families/host/planSources.ts');
const { unitEntries } = await import('../../ui/src/families/readUnits.ts');
const { effectiveCountryUnobserved, extractFactionData, trackGate } = await import('../../ui/src/families/sources.ts');
const { resolveSelectedCountryRefs } = await import('../../ui/src/families/table/logHelpers.ts');

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
const lines = input[Symbol.asyncIterator]();
const proofs = [];
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
async function receive() {
  const { done, value } = await lines.next();
  if (done) throw new Error('The native benchmark closed its request stream.');
  return JSON.parse(value);
}

async function answer(product, params) {
  send({ product, params });
  const reply = await receive();
  if ('reason' in reply) throw new Error(product + ': ' + reply.reason);
  return reply.data;
}

function countryState(widget, policy, track, slots, groups) {
  const { branch } = track;
  const selection = widget.countries;
  const keys = selectedCountryKeys(selection, track);
  const multi = policy.mode === 'multi';
  const all = selection.kind === 'all';
  const factionsOnly = selection.kind === 'factions' && multi && policy.factions;
  let resolved = multi && !all && !factionsOnly
    ? resolveCountries(selection, keys, groups, branch.lastHour, Number.MAX_SAFE_INTEGER)
    : null;
  const slot = keys.length === 1 ? countryFromKey(keys[0]) : null;
  if (resolved && policy.allowSingleSlot && selection.kind === 'selected' && typeof slot === 'number') {
    const runs = slots.tagRuns(slot);
    resolved = { ...resolved, tags: [...new Set(runs.map(run => run.tag))], membership: new Map([['', runs]]) };
  }
  const tags = resolved?.tags ?? [];
  const faction = extractFactionData(groups, multi && policy.factions);
  const unobserved = effectiveCountryUnobserved(selection, resolved?.unobserved);
  return {
    all, tags, every: (groups?.countryIdentities ?? []).map(country => country.tag).sort(),
    selectedSlots: multi || policy.mode === 'filter'
      ? resolveSelectedCountryRefs(selection, keys, groups ? countryIdentityIndex(groups.countryIdentities) : null)
      : undefined,
    holders: resolved && resolved.top == null ? Object.fromEntries(tags.map(tag => [tag, slots.runsForKey(tag)])) : null,
    top: resolved?.top ?? null,
    membership: resolved?.membership ? Object.fromEntries([...resolved.membership].map(([tag, runs]) =>
      [tag, runs.map(run => [run.firstHour, run.lastHour])])) : null,
    membershipRuns: resolved?.membership ? [...resolved.membership.values()].flat() : null,
    unobserved,
    filter: { passing: null, unrecorded: [], bounds: [] },
    factions: faction.runs,
    factionUnobserved: faction.unobserved,
    factionIncomplete: faction.incomplete,
    rankedAt: hour => resolved ? rankedCandidates(resolved, hour, tags) : [],
    known: resolved?.known ?? (() => true),
    unobservedIn: (first, last) => unobserved.some(([lo, hi]) => lo <= last && first <= hi),
    reason: resolved?.reason ?? null,
  };
}

async function viewRequests(placement, track, slots, getGroups, first, last, quantum, explicitCountry) {
  const { kind, view, meta } = placement;
  const page = PAGES.filter(page => page.panels.some(([widget]) => widget === kind)).map(page => page.label).join(', ') || meta.category || 'Catalogue';
  const defaultGridWidth = 1920 - 232 - 8;
  const width = Math.round(((defaultGridWidth + 6) / GRID_COLUMNS.lg) * meta.defaultSize.w - 6);
  const height = 32 * meta.defaultSize.h - 6 - 38;
  const result = { kind, view, page, viewport: { width, height }, country: null, requests: [], reasons: [] };
  const entry = manifestFamilyView(kind, view);
  if (!entry) {
    result.reasons.push('Native renderer has no dashboard product request.');
    return result;
  }
  try {
    const { declaration, spec } = await entry.loadSpec();
    const build = declaration?.sources ?? spec?.sources;
    if (!build) throw new Error('The view declares no source-plan builder.');
    const widget = { ...createWidget(kind, track.id, kind), options: placement.options };
    const options = widgetViewOptions(kind, widget.options, view);
    const policy = declaration?.countries ?? { mode: 'none' };
    const single = gatedByCountry(policy, options);
    if (single && explicitCountry) widget.countries = { kind: 'selected' };
    const following = widget.time.mode === 'follow';
    const branch = track.branch;
    const gate = trackGate(track);
    const window = following ? null : resolveWindow(widget, track, [track], first);
    const pinHour = gate.recorded(window?.lastHour ?? first);
    const planHour = following ? first : window?.lastHour ?? first;
    const countrySubject = single ? selectedCountryKeys(widget.countries, track)[0] ?? null : null;
    const country = countrySubject === null ? null : countryFromKey(countrySubject);
    const countryTag = typeof country === 'number' ? slots.tagAt(country, planHour) : country;
    if (single && !countryTag) throw new Error(countrySubject === null
      ? policy.choose ?? 'This view requires a selected country.'
      : 'The chosen country has no recorded holder at hour ' + planHour + '.');
    if (single) result.country = { tag: countryTag, ref: country };
    const selection = widget.countries;
    const groups = policy.mode === 'filter' || policy.mode === 'multi' && (policy.factions || policy.everyOnTop
      || (policy.requireGroupsWhen ? policy.requireGroupsWhen(widget, options) : selection.kind === 'group'))
      ? await getGroups() : undefined;
    const countries = countryState(widget, policy, track, slots, groups);
    if (countries.reason) throw new Error(countries.reason);
    const auxSpec = declaration?.aux;
    const params = auxSpec && (!auxSpec.when || auxSpec.when(options)) ? auxSpec.params(track, options, countryTag, countrySubject) : null;
    const aux = params ? auxSpec.validate(await answer(auxSpec.product, params)) : undefined;
    const recordedEnd = lastRecordedHour(track) ?? branch.lastHour;
    const { plan } = planFamilySources({
      track, branch, lastRecordedHour: recordedEnd, widget, options, following, aux,
      countryTag, countrySubject, countryName: track.source.metadata.countries.find(country => country.tag === countryTag)?.name ?? countryTag ?? '',
      countries, gateSpan: gate.span, planHour,
    }, window, true, build, { following, width, height });
    if (plan.refused || plan.unavailable) throw new Error(plan.refused ?? plan.unavailable);
    if (following && plan.valid && (plan.valid[0] > first || plan.valid[1] < last)) {
      throw new Error('The source plan covers hours ' + plan.valid.join('–') + ', outside part of this sample window.');
    }
    const checked = following ? compileFollowPlan(plan, query => {
      const asked = followedProduct(query);
      if (!asked) return { query: null, refused: 'The reading names no product to follow.' };
      const refused = refusedHours(asked.product, asked.params, branch, true);
      return { query: refused ? null : query, refused };
    }) : { subscriptions: [], refused: null };
    if (checked.refused) result.reasons.push(checked.refused);
    for (const { name, query } of checked.subscriptions) result.requests.push({ mode: 'follow', name, query });

    const requested = new Set(checked.subscriptions.map(subscription => subscription.name));
    for (const source of unitEntries(plan, following, checked.subscriptions)) {
      const hours = [];
      const tracksProof = following && typeof source.read.provenInterval === 'function';
      const specs = new Map();
      for (let hour = first; hour <= last; hour += quantum) {
        const read = following
          ? buildPlaybackBoundedRequest(source, { branch, effectiveEndHour: recordedEnd, currentHour: hour, gate, countriesKnown: countries.known })
          : buildStaticRequest(source, { branch, timeMode: widget.time.mode, window, pinHour, gate, countriesKnown: countries.known });
        if (read) {
          hours.push([hour, { product: read.request.product, params: read.request.params }]);
          if (tracksProof) specs.set(hour, read);
        }
      }
      if (hours.length) {
        const proof = tracksProof ? proofs.push({ specs, read: source.read, recordedEnd }) - 1 : null;
        result.requests.push({ mode: 'read', name: source.name, hours, proof });
        requested.add(source.name);
      }
    }
    for (const source of plan.entries) {
      if (source.read.shape === 'pixel-binned') {
        result.reasons.push(source.name + ': chart tile requests need a viewport and are unmeasured.');
      } else if (source.read.shape !== 'exact-hover' && !requested.has(source.name)) {
        result.reasons.push(source.name + ': no headless playback request for the ' + source.read.shape + ' source.');
      }
    }
    if (!result.requests.length && !result.reasons.length) {
      const shapes = [...new Set(plan.entries.map(source => source.read.shape))];
      result.reasons.push(shapes.length
        ? 'No headless playback request for the default ' + shapes.join(', ') + ' source plan.'
        : 'The default source plan is empty for this recording and selection.');
    }
  } catch (error) {
    result.reasons.push(error instanceof Error ? error.message : String(error));
  }
  return result;
}

try {
  const { metadata, branch, country, first, last, quantum, views: selectedViews = [] } = await receive();
  const source = { id: 'native-bench', revision: '1', path: metadata.path, metadata };
  const chosen = metadata.branches.find(candidate => candidate.id === branch);
  const slots = new SlotTags(await answer('country-identities', { branchId: branch }));
  const player = country === 'auto' || country === 'player';
  const selectedSlot = player
    ? metadata.playerCountryId === null ? null : Number(metadata.playerCountryId)
    : slots.slotOf(country, first);
  if (country !== 'auto' && (!Number.isSafeInteger(selectedSlot) || slots.tagAt(selectedSlot, first) === null)) {
    const subject = player ? 'The recorded player country' : 'The selected country ' + country;
    throw new Error(subject + ' has no recorded holder at hour ' + first + '.');
  }
  const track = {
    id: 'main', roleId: 'primary', branch: chosen, source, axisOffset: 0, selectedSlot,
    selectedTag: selectedSlot === null ? null : slots.tagAt(selectedSlot, first),
  };
  const client = { product: async request => ({ ...request, data: await answer(request.product, request.params) }) };
  let groups;
  const getGroups = () => groups ??= loadCountryGroups(client, source, branch, new AbortController().signal);
  const placements = await enumerateWidgetPlacements({ mode: 'all-views' });
  const matches = (placement, name) => placement.kind === name || `${placement.kind}/${placement.view}` === name;
  for (const name of selectedViews) {
    if (!placements.some(placement => matches(placement, name))) throw new Error('Unknown widget view ' + name);
  }
  const views = [];
  for (const placement of placements) {
    if (!selectedViews.length || selectedViews.some(name => matches(placement, name))) {
      views.push(await viewRequests(placement, track, slots, getGroups, first, last, quantum, country !== 'auto'));
    }
  }
  send({ views });
  for await (const line of lines) {
    const { proof, hour, data } = JSON.parse(line);
    const { specs, read, recordedEnd } = proofs[proof];
    const spec = specs.get(hour);
    try {
      const validated = spec.validate ? spec.validate(data) : data;
      const through = resolvePlaybackProvenInterval(read, validated, spec, recordedEnd)[1];
      send(Math.min(through, 2147483647));
    } catch (error) {
      send({ reason: error instanceof Error ? error.message : String(error) });
    }
  }
} catch (error) {
  send({ reason: error instanceof Error ? error.message : String(error) });
} finally {
  input.close();
}
