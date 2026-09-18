import type { SVGProps } from 'react';

type IconName = 'map' | 'chart' | 'folder' | 'arrow' | 'refresh' | 'info' | 'close';

export function Icon({ name, ...props }: SVGProps<SVGSVGElement> & { name: IconName }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...props}>
      {name === 'map' && <><path d="m3 5 6-2 6 2 6-2v16l-6 2-6-2-6 2V5Z" /><path d="M9 3v16M15 5v16" /></>}
      {name === 'chart' && <><path d="M4 3v17h17" /><path d="m7 14 4-5 4 3 5-7" /></>}
      {name === 'folder' && <><path d="M3 8V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v2" /><path d="M3 9h18l-2 10H5L3 9Z" /></>}
      {name === 'arrow' && <><path d="M4 12h15m-6-6 6 6-6 6" /></>}
      {name === 'refresh' && <><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.1 6.2a8 8 0 0 1 12.5.8L20 9M4 15l1.4 2a8 8 0 0 0 12.5.8" /></>}
      {name === 'info' && <><circle cx="12" cy="12" r="9" /><path d="M12 11v6m0-10v.2" /></>}
      {name === 'close' && <path d="m6 6 12 12M6 18 18 6" />}
    </svg>
  );
}
