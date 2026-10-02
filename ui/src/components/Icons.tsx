import type { ReactNode, SVGProps } from 'react';

// One outline family: 24px grid, 1.75 stroke, round caps and joins.
const paths = {
  map: <><path d="M9 4 3.5 6v14L9 18l6 2 5.5-2V4L15 6 9 4Z" /><path d="M9 4v14M15 6v14" /></>,
  globe: <><circle cx="12" cy="12" r="8.75" /><path d="M3.5 12h17" /><path d="M12 3.25c2.4 2.4 3.6 5.3 3.6 8.75s-1.2 6.35-3.6 8.75c-2.4-2.4-3.6-5.3-3.6-8.75S9.6 5.65 12 3.25Z" /></>,
  grid: <><rect x="3.5" y="3.5" width="7" height="7" rx="1.75" /><rect x="13.5" y="3.5" width="7" height="4.5" rx="1.5" /><rect x="13.5" y="11" width="7" height="9.5" rx="1.75" /><rect x="3.5" y="13.5" width="7" height="7" rx="1.75" /></>,
  layout: <rect x="3.75" y="3.75" width="16.5" height="16.5" rx="3" strokeDasharray="3.2 2.8" />,
  settings: <><path d="M12 2.9 13.6 5l2.6-.6.9 2.5 2.5.9-.6 2.6 2.1 1.6-2.1 1.6.6 2.6-2.5.9-.9 2.5-2.6-.6L12 21.1l-1.6-2.1-2.6.6-.9-2.5-2.5-.9.6-2.6L2.9 12 5 10.4l-.6-2.6 2.5-.9.9-2.5 2.6.6L12 2.9Z" /><circle cx="12" cy="12" r="3.1" /></>,
  folder: <path d="M3.5 7.5a2 2 0 0 1 2-2h3.6l2 2.2h7.4a2 2 0 0 1 2 2v8.3a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2Z" />,
  'folder-open': <><path d="M3.5 17.5v-10a2 2 0 0 1 2-2h3.6l2 2.2h6.4a2 2 0 0 1 2 2v1.3" /><path d="M3.9 18.3 6.2 12a2 2 0 0 1 1.9-1.3h11.6a1.2 1.2 0 0 1 1.1 1.6l-2 5.7a2 2 0 0 1-1.9 1.5H5.5a1.6 1.6 0 0 1-1.6-1.2Z" /></>,
  file: <><path d="M14 3.5H7a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5L14 3.5Z" /><path d="M14 3.5v5h5" /></>,
  arrow: <path d="M5 12h14m-6-6 6 6-6 6" />,
  'refresh-cw': <><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" /><path d="M21 3v5h-5" /><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" /><path d="M8 16H3v5" /></>,
  refresh: <><path d="M19.5 10.5A7.8 7.8 0 0 0 5.7 7.2L4.5 8.8" /><path d="M4.5 4.5v4.3h4.3" /><path d="M4.5 13.5a7.8 7.8 0 0 0 13.8 3.3l1.2-1.6" /><path d="M19.5 19.5v-4.3h-4.3" /></>,
  info: <><circle cx="12" cy="12" r="8.75" /><path d="M12 11v5.5" /><path d="M12 7.7v.1" /></>,
  alert: <><path d="M10.3 4.4 3 17.4a2 2 0 0 0 1.7 3h14.6a2 2 0 0 0 1.7-3L13.7 4.4a2 2 0 0 0-3.4 0Z" /><path d="M12 9.5V14M12 17.2v.1" /></>,
  close: <path d="m6.5 6.5 11 11m0-11-11 11" />,
  check: <path d="m5 12.5 4.5 4.5L19 7.5" />,
  trash: <><path d="M4 6.5h16" /><path d="M9.5 6.5V4.8a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1v1.7" /><path d="m6 6.5.8 12.2a2 2 0 0 0 2 1.8h6.4a2 2 0 0 0 2-1.8L18 6.5" /><path d="M10 10.5v6m4-6v6" /></>,
  calendar: <><rect x="3.5" y="5" width="17" height="15.5" rx="2.5" /><path d="M3.5 10h17M8 3v4m8-4v4" /></>,
  clock: <><circle cx="12" cy="12" r="8.75" /><path d="M12 7v5l3.2 2" /></>,
  link: <><path d="M10 14a4.5 4.5 0 0 0 6.4 0l2.8-2.8a4.5 4.5 0 0 0-6.4-6.4l-1.2 1.2" /><path d="M14 10a4.5 4.5 0 0 0-6.4 0l-2.8 2.8a4.5 4.5 0 0 0 6.4 6.4l1.2-1.2" /></>,
  edit: <><path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16v4Z" /><path d="m13.5 6.5 4 4" /></>,
  save: <><path d="M5 3.5h11l3.5 3.5v11.5a2 2 0 0 1-2 2h-12.5a2 2 0 0 1-2-2v-13a2 2 0 0 1 2-2Z" /><path d="M8 3.5v5h7v-5M7.5 20.5v-6h9v6" /></>,
  plus: <path d="M12 5v14m-7-7h14" />,
  minus: <path d="M5 12h14" />,
  'chevron-left': <path d="m14.5 6-6 6 6 6" />,
  'chevron-right': <path d="m9.5 6 6 6-6 6" />,
  'chevron-down': <path d="m6 9.5 6 6 6-6" />,
  'chevron-up': <path d="m6 14.5 6-6 6 6" />,
  'chevrons-left': <><path d="m11 17-5-5 5-5" /><path d="m18 17-5-5 5-5" /></>,
  'chevrons-right': <><path d="m6 17 5-5-5-5" /><path d="m13 17 5-5-5-5" /></>,
  sidebar: <><rect x="3.5" y="4.5" width="17" height="15" rx="2.5" /><path d="M9.5 4.5v15" /></>,
  search: <><circle cx="11" cy="11" r="6.5" /><path d="m20 20-4.2-4.2" /></>,
  star: <path d="m12 3.6 2.6 5.3 5.8.8-4.2 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.2-4.1 5.8-.8L12 3.6Z" />,
  'star-filled': <path d="m12 3.6 2.6 5.3 5.8.8-4.2 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.2-4.1 5.8-.8L12 3.6Z" fill="currentColor" />,
  copy: <><rect x="8.5" y="8.5" width="12" height="12" rx="2.5" /><path d="M15.5 8.5V6A2.5 2.5 0 0 0 13 3.5H6A2.5 2.5 0 0 0 3.5 6v7A2.5 2.5 0 0 0 6 15.5h2.5" /></>,
  download: <path d="M12 4v11m-4.5-4.5L12 15l4.5-4.5M4.5 19.5h15" />,
  upload: <path d="M12 15V4M7.5 8.5 12 4l4.5 4.5M4.5 19.5h15" />,
  image: <><rect x="3.5" y="4.5" width="17" height="15" rx="2.5" /><circle cx="9" cy="10" r="1.8" /><path d="m20.5 16-4.5-4.5-8.5 8" /></>,
  palette: <><path d="M12 3.5a8.5 8.5 0 1 0 0 17c1.2 0 1.8-.9 1.8-1.9 0-.6-.3-1-.6-1.4-.3-.4-.6-.8-.6-1.4 0-1 .8-1.8 1.8-1.8h2.1a4 4 0 0 0 4-4c0-3.6-3.8-6.5-8.5-6.5Z" /><circle cx="7.6" cy="11.2" r="1" fill="currentColor" /><circle cx="10.2" cy="7.6" r="1" fill="currentColor" /><circle cx="14.6" cy="7.8" r="1" fill="currentColor" /></>,
  type: <path d="M5 7.5V5h14v2.5M12 5v14m-3 0h6" />,
  sun: <><circle cx="12" cy="12" r="4" /><path d="M12 2.5v2m0 15v2M4.6 4.6 6 6m12 12 1.4 1.4M2.5 12h2m15 0h2M4.6 19.4 6 18M18 6l1.4-1.4" /></>,
  moon: <path d="M19.5 14.6A8 8 0 0 1 9.4 4.5a8 8 0 1 0 10.1 10.1Z" />,
  monitor: <><rect x="3" y="4" width="18" height="12.5" rx="2" /><path d="M8.5 20.5h7m-3.5-4v4" /></>,
  sliders: <><path d="M4 7h9m4 0h3M4 17h3m4 0h9" /><circle cx="15" cy="7" r="2" /><circle cx="9" cy="17" r="2" /></>,
  grip: <>{[6, 12, 18].map(y => [9, 15].map(x => <circle key={`${x}${y}`} cx={x} cy={y} r="1.35" fill="currentColor" stroke="none" />))}</>,
  more: <>{[6, 12, 18].map(x => <circle key={x} cx={x} cy="12" r="1.5" fill="currentColor" stroke="none" />)}</>,
  eject: <><path d="M12 5 5.5 12.5h13L12 5Z" /><path d="M5.5 17.5h13" /></>,
  pin: <><path d="M9 3.5h6l-.8 5.6 3.3 3.2v1.7H6.5v-1.7l3.3-3.2L9 3.5Z" /><path d="M12 14v6.5" /></>,
  target: <><circle cx="12" cy="12" r="8.75" /><circle cx="12" cy="12" r="4.5" /><circle cx="12" cy="12" r="1" fill="currentColor" /></>,
  flask: <><path d="M9.5 3.5h5m-4 0v5.2L5 18.3a1.5 1.5 0 0 0 1.3 2.2h11.4a1.5 1.5 0 0 0 1.3-2.2l-5.5-9.6V3.5" /><path d="M7.4 14.5h9.2" /></>,
  sparkle: <path d="m12 3.5 1.9 5.6 5.6 1.9-5.6 1.9-1.9 5.6-1.9-5.6-5.6-1.9 5.6-1.9L12 3.5Z" />,
  play: <path d="M8 5.6v12.8a1 1 0 0 0 1.5.9l10-6.4a1 1 0 0 0 0-1.8l-10-6.4A1 1 0 0 0 8 5.6Z" fill="currentColor" stroke="none" />,
  pause: <><rect x="6.5" y="5" width="4" height="14" rx="1.2" fill="currentColor" stroke="none" /><rect x="13.5" y="5" width="4" height="14" rx="1.2" fill="currentColor" stroke="none" /></>,
  layers: <><path d="m12 3.5 8.5 4.5-8.5 4.5L3.5 8 12 3.5Z" /><path d="m3.5 12 8.5 4.5 8.5-4.5M3.5 16l8.5 4.5 8.5-4.5" /></>,
  history: <><path d="M4 12a8 8 0 1 0 2.4-5.7L4 8.5" /><path d="M4 4v4.5h4.5M12 8v4.2l3 1.8" /></>,
  droplet: <path d="M12 3.5c2.6 3.4 6 7.1 6 10.8a6 6 0 0 1-12 0c0-3.7 3.4-7.4 6-10.8Z" />,
  hammer: <><path d="m13.5 10.5-8.4 8.4a1.6 1.6 0 0 0 2.2 2.2l8.4-8.4" /><path d="m10.5 7.5 4-4 6 6-4 4-6-6Z" /><path d="m17.5 3.5-2 2" /></>,
  sunburst: <><circle cx="12" cy="12" r="8.75" /><circle cx="12" cy="12" r="3.5" /><path d="M12 3.25V8.5M15.2 13.6l4.6 2.6M8.8 13.6l-4.6 2.6" /></>,
  factory: <path d="M3.5 20.5h17M5 20.5V10l5 3v-3l5 3V6l4 2v12.5" />,
  flows: <><path d="M4 5.5v5M20 5.5v4M20 13.5v5M4 14.5v4" /><path d="M4 8c8 0 8 8 16 8M4 16.5c7 0 9-5 16-5.5" /></>,
  shield: <><path d="M12 3.5 19 6v5.5c0 4.3-2.9 7.6-7 9-4.1-1.4-7-4.7-7-9V6l7-2.5Z" /><path d="M12 3.5v17" /></>,
  hierarchy: <><rect x="9" y="3.5" width="6" height="4.5" rx="1.25" /><rect x="3.5" y="16" width="6" height="4.5" rx="1.25" /><rect x="14.5" y="16" width="6" height="4.5" rx="1.25" /><path d="M12 8v4M6.5 16v-4h11v4" /></>,
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof paths;

export function Icon({ name, ...props }: SVGProps<SVGSVGElement> & { name: IconName }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...props}>
      {paths[name]}
    </svg>
  );
}
