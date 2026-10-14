/** The Cumulas mark: a small cloud in front of a larger, paler one. */
export function LogoMark({ size = 28 }: { size?: number }) {
  return (
    <svg
      className="logo-mark"
      viewBox="3.3 3.4 22.8 19.4"
      width={size}
      height={Math.round((size * 19.4) / 22.8)}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <linearGradient id="logo-mark-front" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" style={{ stopColor: "var(--logo-front-top)" }} />
          <stop offset="1" style={{ stopColor: "var(--logo-front-bottom)" }} />
        </linearGradient>
      </defs>
      <g style={{ fill: "var(--logo-back)" }}>
        <circle cx="17.6" cy="8.6" r="4.4" />
        <circle cx="22.2" cy="11.6" r="3.3" />
        <rect x="13.2" y="9.5" width="12.3" height="5.6" rx="2.8" />
      </g>
      <g fill="url(#logo-mark-front)">
        <circle cx="8.4" cy="15.6" r="4.4" />
        <circle cx="13.8" cy="12.6" r="6" />
        <circle cx="19.6" cy="16.4" r="3.9" />
        <rect x="4" y="15" width="19.5" height="7.2" rx="3.6" />
      </g>
    </svg>
  );
}
