import { useMemo } from 'react';
import { selectedCountryKeys } from './countrySelection.ts';
import type { WidgetRuntime } from './definitions';
import { resolveCountries } from './groups.ts';
import { groupNumbers } from './groupsReply.ts';
import { NumericReply } from './numericReply';
import type { CountryGroupReply } from '../generated/dashboard';
import type { WidgetBase } from './model';
import { useCountryGroups } from './useCountryGroups.ts';

export function useComparisonCountries(widget: WidgetBase, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const grouped = widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const retained = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries]);
  const numbers = retained.update(key, groups.data, groupNumbers);
  const countries = useMemo(() => resolveCountries(widget.countries, grouped ? [] : selectedCountryKeys(widget.countries, track), numbers,
    track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER), [widget.countries, grouped ? undefined : track, numbers, track?.branch.lastHour]);
  return { track, countries, error: numbers ? undefined : groups.error, refreshError: numbers ? groups.error : undefined, retry: groups.retry,
    contentPending: grouped && !!numbers && !groups.data && !groups.error,
    pending: !!track && grouped && !numbers && !groups.error };
}
