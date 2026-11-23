import { includeCampaigns } from './campaigns.ts';
import type { Campaign } from './campaigns.ts';
import type { DashboardTemplate } from './model';
import type { PageId } from './pages';
import { unsavedLayoutPages } from './storage.ts';
import type { LayoutLibrary } from './storage';
import type { Latest } from '../shell/latest.ts';

export interface KeptPage { library: LayoutLibrary; template: DashboardTemplate; editing: boolean }

export interface LayoutDrafts {
  page: Latest<PageId>;
  library: Latest<LayoutLibrary>;
  template: Latest<DashboardTemplate>;
  kept: Latest<Map<PageId, KeptPage>>;
  pages: Latest<readonly PageId[] | undefined>;
  campaigns: Latest<readonly Campaign[]>;
}

export function unsavedDrafts({ page, library, template, kept, pages, campaigns }: LayoutDrafts): PageId[] {
  const existing = pages.current ?? [...kept.current.keys()];
  const aside = [...kept.current].filter(([id]) => id !== page.current && existing.includes(id));
  return unsavedLayoutPages([[page.current, { library: library.current, template: template.current }], ...aside],
    merged => includeCampaigns(merged, campaigns.current));
}
