import type { ArtHandle } from '../../families/contract.ts';
import type { TimelineLane } from '../../families/timeline/model.ts';
import type { DiplomacyKind, DiplomacyPact, DiplomacyPactsReply } from '../../generated/dashboard';

type Pact = DiplomacyPact;

export type PactState = { state: 'present' } | { state: 'gone'; by: number } | { state: 'unknown'; since: number };

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const hour = (value: unknown) => Number.isSafeInteger(value);
const hourOrNull = (value: unknown) => value === null || Number.isSafeInteger(value);
const textOrNull = (value: unknown) => value === null || typeof value === 'string';

export function validatePacts(data: unknown): DiplomacyPactsReply {
  if (!object(data) || !hour(data.hour) || !(data.nextChange === null || (hour(data.nextChange) && (data.nextChange as number) > (data.hour as number)))
    || !Array.isArray(data.warnings) || !Array.isArray(data.unobserved) || !Array.isArray(data.kinds) || !Array.isArray(data.pacts)
    || !data.kinds.every(kind => object(kind) && typeof kind.kind === 'string' && hour(kind.count) && textOrNull(kind.name) && textOrNull(kind.reason))
    || !data.pacts.every(pact => object(pact) && typeof pact.kind === 'string' && textOrNull(pact.first) && textOrNull(pact.second)
      && hourOrNull(pact.startHour) && hourOrNull(pact.endHour) && hour(pact.firstSeen) && hour(pact.lastSeen) && hourOrNull(pact.goneBy)
      && (pact.lastSeen as number) <= (data.hour as number) && (pact.goneBy === null || (pact.goneBy as number) <= (data.hour as number))
      && typeof pact.held === 'boolean' && typeof pact.endedBy === 'boolean'))
    throw new Error('The diplomatic agreements reply is malformed.');
  return data as unknown as DiplomacyPactsReply;
}

export function pactState(pact: Pact): PactState {
  if (pact.held) return { state: 'present' };
  return pact.goneBy !== null && !pact.endedBy ? { state: 'gone', by: pact.goneBy } : { state: 'unknown', since: pact.lastSeen };
}

export const pactSince = (pact: Pact) => pact.startHour ?? pact.firstSeen;

const ELSEWHERE = new Set(['lend_lease', 'market_access_rights', 'equipment_purchase_contract_relation', 'improve_relation']);
const OFFERS: Readonly<Record<string, string>> = { offer_military_access: 'military_access', offer_docking_rights: 'docking_rights', offer_air_base_access: 'air_base_access' };

const KNOWN: readonly { kind: string; title: string; one: string; out?: (name: string) => string; in?: (name: string) => string; incomingFirst?: boolean }[] = [
  { kind: 'non_aggression_pact', title: 'Non-aggression pacts', one: 'Non-aggression pact' },
  { kind: 'guarantee', title: 'Guarantees', one: 'Guarantee', out: name => `${name} guarantees`, in: () => 'Guaranteed by' },
  { kind: 'military_access', title: 'Military access', one: 'Military access', out: name => `${name} has access in`, in: name => `Has access in ${name}` },
  { kind: 'docking_rights', title: 'Docking rights', one: 'Docking rights', out: name => `${name} docks in`, in: name => `Docks in ${name}` },
  { kind: 'air_base_access', title: 'Air base access', one: 'Air base access', out: name => `${name} flies from`, in: name => `Flies from ${name}` },
  { kind: 'embargo', title: 'Trade embargoes', one: 'Trade embargo', out: name => `${name} embargoes`, in: () => 'Embargoed by', incomingFirst: true },
  { kind: 'naval_blockade', title: 'Naval blockades', one: 'Naval blockade', out: name => `${name} blockades`, in: () => 'Blockaded by', incomingFirst: true },
];

export interface Agreement {
  key: string;
  kind: string;
  outgoing: boolean;
  partner: string | null;
  since: number;
  sinceBound: boolean;
  expires: number | null;
  lastSeen: number;
  state: PactState;
  pact: Pact;
}

export function agreementsOf(pacts: readonly Pact[], tag: string, kinds: ReadonlySet<string> | null): Agreement[] {
  const joined = new Map<string, { pacts: Pact[]; kind: string; outgoing: boolean; partner: string | null }>();
  for (const pact of pacts) {
    if (ELSEWHERE.has(pact.kind)) continue;
    const kind = OFFERS[pact.kind] ?? pact.kind;
    if (kinds && !kinds.has(kind)) continue;
    const [first, second] = OFFERS[pact.kind] ? [pact.second, pact.first] : [pact.first, pact.second];
    if (first !== tag && second !== tag) continue;
    const outgoing = first === tag;
    const partner = outgoing ? second : first;
    const key = JSON.stringify([kind, outgoing, partner, pact.startHour ?? `seen:${pact.firstSeen}`]);
    const entry = joined.get(key) ?? joined.set(key, { pacts: [], kind, outgoing, partner }).get(key)!;
    entry.pacts.push(pact);
  }
  return [...joined].map(([key, entry]) => {
    const latest = entry.pacts.reduce((a, b) => b.lastSeen > a.lastSeen ? b : a);
    const since = Math.min(...entry.pacts.map(pactSince));
    const expiry = latest.endHour != null && latest.endHour > since ? latest.endHour : null;
    return { key, kind: entry.kind, outgoing: entry.outgoing, partner: entry.partner, since, sinceBound: latest.startHour == null,
      expires: expiry, lastSeen: latest.lastSeen, state: pactState(latest), pact: latest };
  });
}

export interface AgreementLaneMeta { readonly agreement: Agreement; readonly title: string; readonly mutual: boolean }
export interface AgreementText {
  readonly name: string;
  readonly country: (tag: string, hour: number) => { readonly name: string; readonly flag: ArtHandle | null };
  readonly kindName: (kind: string) => string;
  readonly colour: (kind: string) => string;
}

const live = (item: Agreement) => item.state.state === 'present';
const order = (a: Agreement, b: Agreement) => Number(!live(a)) - Number(!live(b)) || a.since - b.since;

export function agreementLanes(agreements: readonly Agreement[], ended: boolean, hour: number, text: AgreementText): TimelineLane[] {
  const lanes: TimelineLane[] = [];
  const present = agreements.filter(item => ended || live(item));
  const known = new Map(KNOWN.map((entry, index) => [entry.kind, index]));
  const kinds = [...new Set(present.map(item => item.kind))].sort((a, b) => (known.get(a) ?? KNOWN.length) - (known.get(b) ?? KNOWN.length)
    || text.kindName(a).localeCompare(text.kindName(b)));
  for (const kind of kinds) {
    const spec = KNOWN.find(entry => entry.kind === kind);
    const title = spec?.title ?? text.kindName(kind);
    const items = present.filter(item => item.kind === kind);
    const fill = text.colour(kind);
    const active = items.filter(live).length;
    const kindKey = `kind:${kind}`;
    lanes.push({ key: kindKey, kind: 'group', label: title, subtitle: `${active} in force`, spans: [], auditValue: `${title}|${active} in force` });
    const sides: { label: string | null; items: Agreement[] }[] = spec && !spec.out ? [{ label: null, items }]
      : [{ label: spec?.out?.(text.name) ?? `${text.name} → Partner`, items: items.filter(item => item.outgoing) },
        { label: spec?.in?.(text.name) ?? `Partner → ${text.name}`, items: items.filter(item => !item.outgoing) }];
    if (spec?.incomingFirst) sides.reverse();
    for (const side of sides) {
      if (!side.items.length) continue;
      if (side.label) lanes.push({ key: `side:${kind}:${side.label}`, kind: 'heading', depth: 1, parentKey: kindKey, label: side.label, spans: [], auditValue: side.label });
      for (const item of [...side.items].sort(order)) {
        const look = item.partner === null ? { name: 'Country not recorded', flag: null } : text.country(item.partner, live(item) ? hour : Math.min(hour, item.lastSeen));
        const open = live(item);
        const meta: AgreementLaneMeta = { agreement: item, title: spec?.one ?? title, mutual: !!spec && !spec.out };
        const status = open ? 'In force' : item.state.state === 'gone' ? 'Ended' : 'Not recorded';
        lanes.push({
          key: `agreement:${item.key}`,
          kind: 'item',
          depth: 1,
          parentKey: kindKey,
          label: look.name,
          ...(item.partner !== null ? { flag: look.flag, flagTag: item.partner } : { flagTag: '?' }),
          spans: [{
            id: 'bar',
            firstHour: item.since,
            lastHour: open ? hour : Math.min(hour, item.lastSeen),
            color: fill,
            startBy: item.sinceBound,
            endUnseen: !open && !(item.state.state === 'gone' && item.state.by === item.lastSeen + 1),
            open,
          }],
          auditValue: `${title}|${look.name}|${status}`,
          meta,
        });
      }
    }
  }
  return lanes;
}

export function agreementKinds(kinds: readonly DiplomacyKind[]): { kind: string; name: string | null }[] {
  const named = new Map<string, string | null>();
  for (const kind of kinds) {
    if (ELSEWHERE.has(kind.kind)) continue;
    const base = OFFERS[kind.kind] ?? kind.kind;
    if (!named.get(base)) named.set(base, kind.name ?? named.get(base) ?? null);
  }
  return [...named].map(([kind, name]) => ({ kind, name: KNOWN.find(entry => entry.kind === kind)?.title ?? name }));
}

export const aboutElsewhere = (warning: string) => [...ELSEWHERE].some(kind => warning.includes(`Relation ${kind}:`));

export function pactsProof(data: unknown, _requested: readonly [number, number], at: number): readonly [number, number] | null {
  const reply = data as DiplomacyPactsReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= at && at <= last ? [reply.hour, last] : null;
}
