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
        <linearGradient
          id="logo-mark-front"
          gradientUnits="userSpaceOnUse"
          x1="0"
          y1="6.6"
          x2="0"
          y2="22.2"
        >
          <stop offset="0" style={{ stopColor: "var(--logo-front-top)" }} />
          <stop offset="1" style={{ stopColor: "var(--logo-front-bottom)" }} />
        </linearGradient>
      </defs>
      {/* Each cloud is one outline, so large sizes show no seams between its parts. */}
      <path
        style={{ fill: "var(--logo-back)" }}
        d="M16 15.1A2.8 2.8 0 0 1 13.72 10.675A4.4 4.4 0 1 1 21.99 8.307A3.3 3.3 0 0 1 25.5 11.6L25.5 12.3A2.8 2.8 0 0 1 22.7 15.1Z"
      />
      <path
        fill="url(#logo-mark-front)"
        d="M4 18.6L4 15.6A4.4 4.4 0 0 1 7.96 11.222A6 6 0 0 1 19.799 12.505A3.9 3.9 0 0 1 23.5 16.4L23.5 18.6A3.6 3.6 0 0 1 19.9 22.2L7.6 22.2A3.6 3.6 0 0 1 4 18.6Z"
      />
    </svg>
  );
}
