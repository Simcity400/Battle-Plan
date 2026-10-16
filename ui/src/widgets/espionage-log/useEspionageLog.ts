import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useContentRevision } from '../../dashboard/contentRevision';

import type { EspionageLogReply } from '../../generated/dashboard';
import { operationChoices, operationWindowIdentity, validateEspionageLogReply } from './espionageData';

export function useEspionageLog(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'espionage-log', params: { branchId: track.branch.id, window: null } } : null;
  return { track, ...useProduct<EspionageLogReply>(runtime.client, request, active, validateEspionageLogReply, track?.source) };
}
export function useEspionageChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useEspionageLog(widget, runtime, true);
  return { types: operationChoices(data) };
}

export function useEspionageWindows(track: WidgetRuntime['tracks'][number] | undefined, runtime: WidgetRuntime, wanted: readonly number[], active: boolean) {
  const content = useContentRevision(runtime.client);
  const identity = track ? operationWindowIdentity(track.source.id, track.source.revision, track.branch.id, content) : null;
  return useProductWindows<EspionageLogReply>(runtime.client, identity, window => ({ sourceId: track!.source.id,
    revision: track!.source.revision, product: 'espionage-log', params: { branchId: track!.branch.id, window } }),
  wanted, active, validateEspionageLogReply, track?.source);
}
