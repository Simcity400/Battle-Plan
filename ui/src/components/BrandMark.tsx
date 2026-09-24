import { useId } from 'react';

/** The app mark: a globe inside a theme-tinted tile, following the accent and glow. */
export default function BrandMark({ size = 30 }: { size?: number }) {
  const id = useId();
  return <svg className="brand-mark" width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" focusable="false">
    <defs>
      <linearGradient id={`${id}-fill`} x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" style={{ stopColor: 'var(--accent)' }} />
        <stop offset="1" style={{ stopColor: 'var(--glow)' }} />
      </linearGradient>
      <radialGradient id={`${id}-shine`} cx=".3" cy=".2" r=".9">
        <stop offset="0" stopColor="#fff" stopOpacity=".35" />
        <stop offset=".6" stopColor="#fff" stopOpacity="0" />
      </radialGradient>
    </defs>
    <rect x="1" y="1" width="30" height="30" rx="9" fill={`url(#${id}-fill)`} />
    <rect x="1" y="1" width="30" height="30" rx="9" fill={`url(#${id}-shine)`} />
    <g fill="none" style={{ stroke: 'var(--accent-text)' }} strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="16" cy="16" r="7.5" />
      <path d="M8.5 16h15" />
      <path d="M16 8.5c2 2 3 4.5 3 7.5s-1 5.5-3 7.5c-2-2-3-4.5-3-7.5s1-5.5 3-7.5Z" />
    </g>
  </svg>;
}
