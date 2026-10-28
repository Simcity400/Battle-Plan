import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useEffect, useState } from 'react';
import { loadTransientProduct, useProduct } from '../../dashboard/products';
import * as requests from '../../dashboard/batch5Requests';
import { selectedTags } from '../../dashboard/countrySelection';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { doctrineSprite, folderName, rewardDescriptions, validateDoctrineMastery } from './doctrineMasteryData';
import type { DoctrineMasteryReply } from './doctrineMasteryData';
import { OPTIONS } from './definition';
import { doctrineMasteryRequest } from '../../dashboard/widgetRequests';
import { useContentRevision } from '../../dashboard/contentRevision';

/** The country's prepared doctrine history over the branch, looked up locally at every hour. */
export function useDoctrineMastery(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = doctrineMasteryRequest(widget, track);
  const product = useProduct<DoctrineMasteryReply>(runtime.client, request, active, validateDoctrineMastery, track?.source);
  return { track, tag, options, ...product };
}

/** Doctrine folders in content order, as the content stack names them. */
export function useDoctrineChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { track, data } = useDoctrineMastery(widget, runtime, true);
  const image = useSpriteArtwork(runtime, track, (data?.folders ?? []).map(folder => doctrineSprite(folder.icon)), true);
  return { folders: (data?.folders ?? []).map(folder => ({ value: setToken(folder.id), label: folderName(folder), icon: image(doctrineSprite(folder.icon)) })) };
}

/** Reward descriptions with the country's own values at `hour` (a recorded read), asked only
 * when `wanted`; undefined until they load or when the ask fails, which the next ask retries. */
export function useRewardDescriptions(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | null, hour: number | undefined,
  wanted: boolean): ReadonlyMap<string, string | null> | undefined {
  const content = useContentRevision(runtime.client);
  const request = track && tag && hour !== undefined ? { sourceId: track.source.id, revision: track.source.revision, product: 'doctrine-mastery',
    params: requests.doctrineMasteryAt(track.branch.id, tag, hour) } : null;
  const key = JSON.stringify([request, content]);
  const [state, setState] = useState<{ client: WidgetRuntime['client']; key: string; descriptions?: ReadonlyMap<string, string | null> }>();
  const current = state?.client === runtime.client && state.key === key ? state : undefined;
  useEffect(() => {
    if (!wanted || !request || current?.descriptions) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal, track?.source).then(reply => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, descriptions: rewardDescriptions(validateDoctrineMastery(reply)) });
    }).catch(() => { if (!controller.signal.aborted) setState(undefined); });
    return () => controller.abort();
    // The key holds the source, country, hour and content.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, wanted]);
  return current?.descriptions;
}
