import { useEffect, useState } from 'react';
import type { DashboardClient } from './client';
import type { Campaign } from './campaigns';
import { sameCampaignPath } from './campaigns';

/** Manual jobs refresh only campaigns still open, after pending opens finish. */
export function useCompactedCampaignReload(client: DashboardClient, campaigns: readonly Campaign[], reload: (campaign: Campaign) => Promise<unknown>) {
  const [pending, setPending] = useState<string[]>([]);
  useEffect(() => {
    if (!client.onRecordingCompacted) return;
    let active = true;
    let detach: (() => void) | undefined;
    void client.onRecordingCompacted(path => {
      if (active) setPending(paths => paths.some(existing => sameCampaignPath(existing, path)) ? paths : [...paths, path]);
    }).then(stop => { if (active) detach = stop; else stop(); });
    return () => { active = false; detach?.(); };
  }, [client]);
  useEffect(() => {
    if (!pending.length) return;
    const waiting: string[] = [];
    for (const path of pending) {
      const campaign = campaigns.find(item => sameCampaignPath(item.path, path));
      if (campaign?.loading) waiting.push(path);
      else if (campaign) void reload(campaign);
    }
    if (waiting.length !== pending.length) setPending(waiting);
  }, [campaigns, pending, reload]);
}
