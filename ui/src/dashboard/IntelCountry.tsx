import { useMemo } from 'react';
import { CountryFlagUrl } from '../components/CountryFlag';
import type { CountryLabel } from './countryFlagsAt';
import { useCountryLabelsAt } from './countryFlagsAt.ts';
import type { WidgetRuntime } from './definitions';
import type { HeadProduct } from './headProduct';
import type { WidgetSpec } from './model';
import { resolveWindow } from './model.ts';
import { usePushedLooksHour } from './pushedLooks.ts';
import type { Country } from '../generated/dashboard';

/** The track hour a rows-at-an-hour widget names its countries at: the hour asked, else (followed) the clock's at the
 * render, which a followed widget makes only when its rows change. */
export function shownHour(runtime: WidgetRuntime, widget: WidgetSpec, head: Pick<HeadProduct<unknown>, 'hour'>): number | null {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  if (!track) return null;
  const hour = head.hour ?? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour;
  return hour === undefined ? null : Math.max(track.branch.firstHour, Math.min(Math.floor(hour), track.branch.lastHour));
}

/** Names and flags of `tags` as the recording showed them at `hour`; while `pushed` (rows pushed at the head, whose
 * hour moves with every push), at the last hour one of them changed, so a push asks for no name. */
export function useIntelLabels(runtime: WidgetRuntime, widget: WidgetSpec, tags: readonly string[], hour: number | null, active: boolean, pushed = false): CountryLabel {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const named = usePushedLooksHour(runtime, widget, track, tags, active && pushed && hour !== null, hour ?? 0);
  const at = hour === null ? null : named.hour;
  const wanted = useMemo(() => at === null ? [] : tags.map(tag => ({ tag, hour: at })), [tags, at]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  return useMemo(() => labelAt(at ?? 0), [labelAt, at]);
}

/** A country with its flag and name; one the hour names no tag for says so. */
export function IntelCountryName({ country, label }: { country: Country; label: CountryLabel }) {
  if (!country.tag) return <span className="intel-country is-unresolved" title="The recording names no tag for this country at this hour">Country {country.id}</span>;
  const shown = label(country.tag);
  return <span className="intel-country"><CountryFlagUrl url={shown.flag} reason={shown.flagReason} detail={shown.flagDetail} pending={shown.flagPending} />
    <span>{shown.known === false ? country.tag : shown.name}</span></span>;
}
