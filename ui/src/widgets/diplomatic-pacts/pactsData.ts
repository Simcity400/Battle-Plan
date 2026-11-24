import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { CountryFlagLook } from '../../components/countryFlagPresentation';
import type { DiplomacyKind, DiplomacyPact, DiplomacyPactsReply } from '../../generated/dashboard';

type Pact = DiplomacyPact;

export type PactState = { state: 'present' } | { state: 'gone'; by: number } | { state: 'unknown'; since: number };

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const hour = (value: unknown) => Number.isSafeInteger(value);
const hourOrNull = (value: unknown) => value === null || Number.isSafeInteger(value);
const textOrNull = (value: unknown) => value === null || typeof value === 'string';

export function validatePacts(data: unknown): DiplomacyPactsReply {
  if (!object(data) || !Array.isArray(data.warnings) || !Array.isArray(data.unobserved) || !Array.isArray(data.kinds) || !Array.isArray(data.pacts)
    || !data.kinds.every(kind => object(kind) && typeof kind.kind === 'string' && hour(kind.count) && textOrNull(kind.name) && textOrNull(kind.reason))
    || !data.pacts.every(pact => object(pact) && typeof pact.kind === 'string' && textOrNull(pact.first) && textOrNull(pact.second)
      && hourOrNull(pact.startHour) && hourOrNull(pact.endHour) && hour(pact.firstSeen) && hour(pact.lastSeen) && hourOrNull(pact.goneBy)
      && typeof pact.presentAtEnd === 'boolean' && typeof pact.endedBy === 'boolean'))
    throw new Error('The diplomatic agreements reply is malformed.');
  return data as unknown as DiplomacyPactsReply;
}

export function pactState(pact: Pact, at: number): PactState {
  if (at <= pact.lastSeen || pact.presentAtEnd) return { state: 'present' };
  if (pact.endedBy) return { state: 'unknown', since: pact.lastSeen };
  return pact.goneBy !== null && pact.goneBy <= at ? { state: 'gone', by: pact.goneBy } : { state: 'unknown', since: pact.lastSeen };
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

export function agreementsOf(pacts: readonly Pact[], tag: string, at: number, kinds: ReadonlySet<string> | null): Agreement[] {
  const joined = new Map<string, { pacts: Pact[]; kind: string; outgoing: boolean; partner: string | null }>();
  for (const pact of pacts) {
    if (pact.firstSeen > at || ELSEWHERE.has(pact.kind)) continue;
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
      expires: expiry, lastSeen: latest.lastSeen, state: pactState(latest, at), pact: latest };
  });
}

export interface AgreementLaneInfo { agreement: Agreement; title: string; mutual: boolean }
export interface AgreementText {
  name: string;
  country: (tag: string | null, hour: number) => CountryFlagLook & { name: string };
  kindName: (kind: string) => string;
  colors: readonly string[];
}

const faded = (color: string) => /^#[0-9a-f]{6}$/i.test(color) ? `${color}66` : color;
const live = (item: Agreement) => item.state.state === 'present';
const order = (a: Agreement, b: Agreement) => Number(!live(a)) - Number(!live(b)) || a.since - b.since;

export function agreementLanes(agreements: readonly Agreement[], ended: boolean, knownLastHour: number, text: AgreementText) {
  const lanes: TimelineLane[] = [];
  const info = new Map<string, AgreementLaneInfo>();
  const present = agreements.filter(item => ended || live(item));
  const known = new Map(KNOWN.map((entry, index) => [entry.kind, index]));
  const kinds = [...new Set(present.map(item => item.kind))].sort((a, b) => (known.get(a) ?? KNOWN.length) - (known.get(b) ?? KNOWN.length)
    || text.kindName(a).localeCompare(text.kindName(b)));
  kinds.forEach((kind, index) => {
    const spec = KNOWN.find(entry => entry.kind === kind);
    const title = spec?.title ?? text.kindName(kind);
    const items = present.filter(item => item.kind === kind);
    const color = text.colors[index % text.colors.length];
    const active = items.filter(item => item.state.state === 'present').length;
    lanes.push({ id: `kind:${kind}`, heading: true, rowHeight: 30, singleLine: true, separator: false, label: '',
      identity: { label: `${title} · ${active} in force` }, icons: false, names: false, spans: [] });
    const sides: { label: string | null; items: Agreement[] }[] = spec && !spec.out ? [{ label: null, items }]
      : [{ label: spec?.out?.(text.name) ?? `${text.name} → partner`, items: items.filter(item => item.outgoing) },
        { label: spec?.in?.(text.name) ?? `Partner → ${text.name}`, items: items.filter(item => !item.outgoing) }];
    if (spec?.incomingFirst) sides.reverse();
    for (const side of sides) {
      if (!side.items.length) continue;
      if (side.label) lanes.push({ id: `side:${kind}:${side.label}`, label: '', identity: { label: side.label }, heading: true, depth: 1, muted: true,
        labelStyle: 'side', separator: false, rowHeight: 18, singleLine: true, icons: false, names: false, spans: [] });
      for (const item of [...side.items].sort(order)) {
        const id = `agreement:${item.key}`;
        const look = text.country(item.partner, item.lastSeen);
        const listed = live(item);
        const span: TimelineSpan = { id: 'bar', firstHour: item.since, lastHour: item.pact.presentAtEnd ? knownLastHour : item.lastSeen,
          label: look.name, color: listed ? color : faded(color), barHeight: 8, openEnd: true };
        info.set(id, { agreement: item, title: spec?.one ?? title, mutual: !!spec && !spec.out });
        lanes.push({ id, label: '', identity: { label: look.name, countryFlag: look }, depth: 1, rowHeight: 22, singleLine: true,
          separator: false, icons: false, names: false, muted: !listed, spans: [span] });
      }
    }
  });
  return { lanes, info };
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

export function changeHours(reply: Pick<DiplomacyPactsReply, 'pacts' | 'unobserved'>): number[] {
  const hours = new Set<number>();
  for (const pact of reply.pacts) {
    hours.add(pact.firstSeen); hours.add(pact.lastSeen + 1);
    for (const hour of [pact.startHour, pact.endHour, pact.goneBy]) if (hour != null) hours.add(hour);
  }
  for (const span of reply.unobserved) { hours.add(span.firstHour); hours.add(span.lastHour + 1); }
  return [...hours].sort((a, b) => a - b);
}
