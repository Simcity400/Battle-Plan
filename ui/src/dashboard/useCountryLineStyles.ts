import { useEffect, useMemo, useState } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { lookHour } from './countryLooks.ts';
import { countryFromKey, useSlotTags } from './countrySlots.ts';
import { useHeldFailure } from './heldFailures';
import type { BoundTrack } from './model';
import { useProducts } from './products';
import { STYLES } from './useCountriesAt';
import type { Gate } from './regionGate';
import { productRequest } from './request.ts';

export function useCountryLineStyles(client: DashboardClient, gate: Pick<Gate, 'recorded'>, track: BoundTrack | undefined, keys: string[], at: number, active: boolean) {
  const moment = track ? Math.floor(Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, at))) : 0;
  const sample = track ? lookHour(track.branch, moment) : null;
  const hour = sample?.hour ?? null, stretch = sample?.stretch ?? null;
  const bySlot = keys.some(key => typeof countryFromKey(key) === 'number');
  const slots = useSlotTags(client, track, active && bySlot);
  const asked = useMemo(() => {
    const out = new Map<string, { tag: string; hour: number }>();
    if (hour === null) return out;
    for (const key of keys) {
      const country = countryFromKey(key);
      if (typeof country === 'string') { out.set(key, { tag: country, hour }); continue; }
      const run = slots?.runNear(country, moment);
      if (run) out.set(key, { tag: run.tag, hour: Math.min(run.lastHour, Math.max(run.firstHour, hour)) });
    }
    return out;
  }, [keys, slots, hour, moment]);
  const plan = useMemo(() => {
    const byHour = new Map<number, Set<string>>();
    for (const { tag, hour } of asked.values()) {
      const tags = byHour.get(hour);
      if (tags) tags.add(tag); else byHour.set(hour, new Set([tag]));
    }
    return [...byHour].sort(([a], [b]) => a - b).map(([hour, tags]) => ({ hour, tags: [...tags].sort() }));
  }, [asked]);
  const requests = track ? plan.flatMap(({ hour, tags }) => {
    const asked = gate.recorded(hour);
    return asked === null ? [] : [productRequest({ sourceId: track.source.id, revision: track.source.revision, product: STYLES.product, params: { branchId: track.branch.id, hour: asked, tags } })];
  }) : [];
  const styles = useProducts<CountryStyleReply>(client, requests, active);
  const data = useMemo<CountryStyleReply | undefined>(() => styles.data?.length ? {
    branchId: styles.data[0].branchId, hour: styles.data[0].hour,
    styles: styles.data.flatMap(reply => reply.styles),
    warnings: [...new Set(styles.data.flatMap(reply => reply.warnings))],
  } : undefined, [styles.data]);
  const fresh = useMemo(() => {
    if (!styles.data || styles.data.length !== plan.length) return undefined;
    const byHour = new Map(plan.map(({ hour }, index) => [hour, new Map(styles.data![index].styles.map(style => [style.tag, style] as const))] as const));
    const out: [string, CountryStyle][] = [];
    for (const [key, { tag, hour }] of asked) {
      const style = byHour.get(hour)?.get(tag);
      if (style) out.push([key, style]);
    }
    return out;
  }, [styles.data, plan, asked]);
  const [known, setKnown] = useState<ReadonlyMap<string, CountryStyle>>(new Map());
  useEffect(() => setKnown(new Map()), [track?.source.id, track?.source.revision, track?.branch.id]);
  useEffect(() => { if (stretch !== null) setKnown(new Map()); }, [stretch]);
  useEffect(() => {
    if (fresh) setKnown(previous => new Map([...previous, ...fresh]));
  }, [fresh]);
  const error = useHeldFailure(styles.error, known.size > 0, styles.retry);
  return { known, data, error, retry: styles.retry };
}
