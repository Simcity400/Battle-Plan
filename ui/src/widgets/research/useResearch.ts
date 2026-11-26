import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetBase, WidgetSpec } from '../../dashboard/model';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { useProduct } from '../../dashboard/products.ts';
import { setToken } from '../../dashboard/options';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import type { ResearchHistoryReply } from '../../generated/dashboard';
import type { ResearchData } from './types';
import { validateResearchHistory, validateResearchReply } from './researchData';
import { productRequest } from '../../dashboard/request.ts';
import { widgetViewOptions } from '../manifest.ts';

function researchTarget(widget: WidgetBase, runtime: WidgetRuntime) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  return { track, tag };
}

export function useResearchReply(widget: WidgetBase, runtime: WidgetRuntime, active: boolean, covered = true, countryOverride?: string | null) {
  const { track, tag } = researchTarget(widget, runtime);
  const country = countryOverride ?? tag;
  const request = track && country && covered ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'research', params: { branchId: track.branch.id, country: countryFromKey(country) } }) : null;
  return { track, tag, ...useProduct<ResearchData>(runtime.client, request, active, validateResearchReply, track?.source) };
}

export function useResearchChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const gate = useGate(runtime, widget, SPEC, true, { country: { stored: widgetViewOptions(widget.kind, widget.options).country } });
  const subject = gate.open ? gate.country?.history?.key ?? null : null;
  const track = gate.track;
  const hour = gate.hour;
  const request = track && subject && hour !== null
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'research-history',
      params: { branchId: track.branch.id, country: countryFromKey(subject), hour, folders: [] } })
    : null;
  const { data } = useProduct<ResearchHistoryReply>(runtime.client, request, true, validateResearchHistory, track?.source);
  return useMemo(() => ({
    folders: (data?.folders ?? []).map(folder => ({ value: setToken(folder.key), label: contentTitle(folder.name, folder.key, 'Folder name unavailable') })),
  }), [data]);
}
