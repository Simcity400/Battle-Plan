import { defaultCountryTag } from './countrySelection';
import type { WidgetRuntime } from './definitions';
import { resolveWindow, type WidgetSpec } from './model';
import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision';
import { preparedSeriesQuery } from './preparedSeries';
import type { IdeaHistoryFirstView } from './ideaHistoryReply';
import { validateIdeaHistoryReply } from './ideaHistoryReply';

/** Prepared holdings with visibly approximate keyframe lanes beyond coverage;
 * the icon grid instead requests an exact held set at its displayed hour. */
export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const { client } = runtime;
  const content = useContentRevision(client);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const through = window && Math.floor(window.lastHour);
  const held = widget.kind === 'national-spirits';
  const key = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, through, held, content]) : '';
  const [state, setState] = useState<{ client: typeof client; key: string; data?: IdeaHistoryFirstView; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track || !tag) return;
    const controller = new AbortController();
    const publish = (value: unknown) => {
      const data = validateIdeaHistoryReply(value);
      if (!controller.signal.aborted) setState({ client, key, data });
    };
    void preparedSeriesQuery(client, track.source, 'idea-history', { branchId: track.branch.id, tag, through, preview: !held, hour: held ? through : undefined }, controller.signal, attempt > 0,
      undefined, undefined, publish)
      .then(validateIdeaHistoryReply).then(
        data => { if (!controller.signal.aborted) setState({ client, key, data }); },
        error => { if (!controller.signal.aborted) setState({ client, key, error: error instanceof Error ? error.message : String(error) }); },
      );
    return () => controller.abort();
    // The structured key owns source, branch, country and content revision.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, key, attempt]);
  const current = key && state?.client === client && state.key === key ? state : undefined;
  return { track, tag, data: current?.data, error: current?.error, retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
