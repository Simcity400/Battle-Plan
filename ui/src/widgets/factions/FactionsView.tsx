import { useCallback, useMemo, useRef, useState } from 'react';
import { formatDay } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { useFactionDetails } from '../../dashboard/factionChoice';
import { DetailWindowHost } from '../../families/host/DetailWindowHost';
import DeclaredTimelineWidget from '../../families/timeline/DeclaredTimelineWidget';
import { factionsDeclaration } from './factionsProjection';

export default function FactionsView(props: WidgetProps) {
  const [opened, setOpened] = useState<{ readonly factionId: string; readonly title: string; readonly hour: number } | null>(null);
  const anchorRef = useRef<HTMLDivElement>(null);

  const onActivate = useCallback((_key: string, meta?: unknown, hour?: number) => {
    const info = meta as { kind?: string; factionId?: string; title?: string } | undefined;
    if (info?.kind === 'faction' && info.factionId) {
      setOpened({ factionId: info.factionId, title: info.title ?? 'Faction', hour: hour ?? 0 });
    }
  }, []);

  const head = useFactionDetails(props.runtime, props.widget, opened?.factionId ?? null, props.active && opened !== null);
  const details = head.data;

  const detailBody = useMemo(() => {
    if (!opened) return null;
    if (head.error) return <p className="detail-window-note">Faction details could not be loaded: {head.error.message}</p>;
    if (!details) return <p className="detail-window-note">Loading faction details…</p>;
    return (
      <div className="detail-window-section">
        <div className="detail-window-row">
          <span>Active goals</span>
          <strong>{details.goals.length}</strong>
        </div>
        <div className="detail-window-row">
          <span>Completed goals</span>
          <strong>{details.completed.length}</strong>
        </div>
        {details.goals.length > 0 && (
          <ul className="detail-window-list">
            {details.goals.map(goal => {
              const def = details.definitions[goal.definition];
              return <li key={goal.ordinal}>{def?.name ?? def?.token ?? `Goal ${goal.ordinal}`}</li>;
            })}
          </ul>
        )}
      </div>
    );
  }, [opened, head.error, details]);

  return (
    <div ref={anchorRef} style={{ width: '100%', height: '100%', display: 'contents' }}>
      <DeclaredTimelineWidget
        declaration={factionsDeclaration}
        {...props}
        selectedKey={opened ? `faction:${opened.factionId}` : null}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={opened !== null}
        subject={opened?.factionId ?? null}
        hour={opened?.hour ?? null}
        title={opened?.title ?? 'Faction'}
        day={opened ? formatDay(opened.hour) : undefined}
        anchor={anchorRef}
        onClose={() => setOpened(null)}
      >
        {detailBody}
      </DetailWindowHost>
    </div>
  );
}
