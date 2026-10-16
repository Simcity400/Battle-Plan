import { defaultCountryTag } from './countrySelection';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision';
import { preparedSeriesQuery } from './preparedSeries';
import type { IdeaHistoryReply } from '../generated/dashboard';
import { validateIdeaHistoryReply } from './ideaHistoryReply';

/** Which ideas a widget's country held, from prepared `idea-history`: shared by
 * the idea timelines and the national spirits widgets and their settings.
 * Acquisition/removal bounds, seats and renames require preceding observations,
 * so this history keeps its prepared coverage gate rather than sampling a tail. */
export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const { client } = runtime;
  const content = useContentRevision(client);
  const key = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, content]) : '';
  const [state, setState] = useState<{ client: typeof client; key: string; data?: IdeaHistoryReply; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track || !tag) return;
    const controller = new AbortController();
    void preparedSeriesQuery(client, track.source, 'idea-history', { branchId: track.branch.id, tag }, controller.signal, attempt > 0)
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
