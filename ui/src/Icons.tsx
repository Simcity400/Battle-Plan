import type { SVGProps } from 'react';

type IconName = 'map' | 'chart' | 'folder' | 'arrow' | 'refresh' | 'info' | 'close' | 'trash'
  | 'calendar' | 'clock' | 'link' | 'settings' | 'grid' | 'globe' | 'edit' | 'save' | 'plus' | 'chevron-left' | 'chevron-right' | 'chevron-down';

export function Icon({ name, ...props }: SVGProps<SVGSVGElement> & { name: IconName }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...props}>
      {name === 'map' && <><path d="m3 5 6-2 6 2 6-2v16l-6 2-6-2-6 2V5Z" /><path d="M9 3v16M15 5v16" /></>}
      {name === 'grid' && <><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></>}
      {name === 'globe' && <><circle cx="12" cy="12" r="9" /><ellipse cx="12" cy="12" rx="4" ry="9" /><path d="M3 12h18M5 6.5h14M5 17.5h14" /></>}
      {name === 'edit' && <><path d="m15 4 5 5M4 20l5-1L21 7a2 2 0 0 0-5-5L4 14l-1 7Z" /></>}
      {name === 'save' && <><path d="M4 3h13l4 4v14H3V3Z" /><path d="M7 3v6h9V3M7 21v-8h10v8" /></>}
      {name === 'plus' && <path d="M12 5v14M5 12h14" />}
      {name === 'link' && <><path d="m10 13 4-4M8 15l-2 2a3.5 3.5 0 0 1-5-5l5-5a3.5 3.5 0 0 1 5 0M16 9l2-2a3.5 3.5 0 0 1 5 5l-5 5a3.5 3.5 0 0 1-5 0" transform="translate(0 -1)" /></>}
      {name === 'settings' && <><path d="m9 3 1-1h4l1 3 3 1 2 2-1 3 1 3-2 3-3 1-1 3h-4l-1-3-3-1-2-3 1-3-1-3 2-2 3-1Z" /><circle cx="12" cy="11" r="3" /></>}
      {name === 'chart' && <><path d="M4 3v17h17" /><path d="m7 14 4-5 4 3 5-7" /></>}
      {name === 'folder' && <><path d="M3 8V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v2" /><path d="M3 9h18l-2 10H5L3 9Z" /></>}
      {name === 'arrow' && <><path d="M4 12h15m-6-6 6 6-6 6" /></>}
      {name === 'refresh' && <><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.1 6.2a8 8 0 0 1 12.5.8L20 9M4 15l1.4 2a8 8 0 0 0 12.5.8" /></>}
      {name === 'info' && <><circle cx="12" cy="12" r="9" /><path d="M12 11v6m0-10v.2" /></>}
      {name === 'close' && <path d="m6 6 12 12M6 18 18 6" />}
      {name === 'trash' && <><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7" /></>}
      {name === 'calendar' && <><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M16 3v4M8 3v4M3 11h18M8 15h2M14 15h2M8 18h2" /></>}
      {name === 'clock' && <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>}
      {name === 'chevron-left' && <path d="m14 6-6 6 6 6" />}
      {name === 'chevron-right' && <path d="m10 6 6 6-6 6" />}
      {name === 'chevron-down' && <path d="m6 9 6 6 6-6" />}
    </svg>
  );
}
