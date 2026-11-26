import { memo } from "react";

const logoUrl = `${import.meta.env.BASE_URL}favicon.png`;

const CX = 290;
const RX = 108;
const RY = 18;
const TIER_HEIGHT = 38;
const CAP_TOP = 60;
const CAP_BOTTOM = 120;

/** What the engine serves, read top to bottom. */
const TIERS = [
  { label: "SQL tables", top: 130, delay: "0.15s", icon: "table" },
  { label: "Live queries", top: 184, delay: "0.3s", icon: "query" },
  { label: "Durable topics", top: 238, delay: "0.45s", icon: "topic" },
] as const;

type TierIconKind = (typeof TIERS)[number]["icon"];

function TierIcon({ kind, x, y }: { kind: TierIconKind; x: number; y: number }) {
  return (
    <g transform={`translate(${x} ${y})`} stroke="white" strokeWidth={1.5} fill="none">
      {kind === "table" && (
        <g strokeLinecap="round">
          <rect x={1} y={1} width={16} height={16} rx={3} />
          <path d="M1,6.5 H17 M6.5,6.5 V17" />
        </g>
      )}
      {kind === "query" && (
        <g strokeLinecap="round" strokeLinejoin="round">
          <path d="M1,14 L6,7.5 L10,11 L17,2.5" />
          <circle cx={17} cy={2.5} r={1.8} fill="white" stroke="none" />
        </g>
      )}
      {kind === "topic" && (
        <g strokeLinecap="round">
          <circle cx={9} cy={9} r={2.2} fill="white" stroke="none" />
          <path d="M4.4,4.4 A6.5,6.5 0 0 0 4.4,13.6" />
          <path d="M13.6,4.4 A6.5,6.5 0 0 1 13.6,13.6" />
        </g>
      )}
    </g>
  );
}

export const HeroGraphic = memo(function HeroGraphic() {
  return (
    <svg viewBox="0 0 600 330" className="absolute inset-0 size-full" role="img">
      <title>KalamDB serves SQL tables, live queries and durable topics from one engine</title>

      <defs>
        <linearGradient id="kdb-cap-face" x1="0" y1="0" x2="0.9" y2="1">
          <stop offset="0%" stopColor="var(--kdb-accent)" />
          <stop offset="45%" stopColor="var(--kdb-accent-strong)" />
          <stop offset="100%" stopColor="var(--kdb-accent-deep)" />
        </linearGradient>
        <linearGradient id="kdb-tier-wall" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="var(--kdb-accent-deep)" />
          <stop offset="52%" stopColor="var(--kdb-accent-strong)" />
          <stop offset="100%" stopColor="var(--kdb-accent)" />
        </linearGradient>
        <linearGradient id="kdb-tier-top" x1="0" y1="0" x2="0.8" y2="1">
          <stop offset="0%" stopColor="var(--kdb-accent)" />
          <stop offset="100%" stopColor="var(--kdb-accent-strong)" />
        </linearGradient>
        <linearGradient id="kdb-dome-sheen" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="white" stopOpacity={0.34} />
          <stop offset="100%" stopColor="white" stopOpacity={0} />
        </linearGradient>
        <radialGradient id="kdb-base-glow">
          <stop offset="0%" stopColor="white" stopOpacity={0.95} />
          <stop offset="34%" stopColor="var(--kdb-accent)" stopOpacity={0.8} />
          <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0} />
        </radialGradient>
        <filter id="kdb-mark-white" x="0%" y="0%" width="100%" height="100%">
          <feColorMatrix type="luminanceToAlpha" result="mask" />
          <feComponentTransfer in="mask" result="sharpened">
            <feFuncA type="gamma" exponent={4} amplitude={1} offset={0} />
          </feComponentTransfer>
          <feFlood floodColor="white" result="paint" />
          <feComposite in="paint" in2="sharpened" operator="in" />
        </filter>
        <radialGradient id="kdb-stack-glow">
          <stop offset="0%" stopColor="var(--kdb-accent)" stopOpacity={0.26} />
          <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0} />
        </radialGradient>
      </defs>

      <ellipse cx={CX} cy={190} rx={210} ry={155} fill="url(#kdb-stack-glow)" />

      {/* Reads fan out to the right; writes arrive from the left. */}
      <g fill="none" stroke="var(--kdb-accent)" strokeOpacity={0.45} strokeWidth={1.5} strokeLinecap="round">
        <path d="M392,82 C424,78 430,74 452,74" />
        <path d="M398,203 C424,203 432,203 452,203" />
        <path d="M392,257 C424,258 430,258 452,258" />
        <path d="M96,203 H178" />
      </g>
      <g fill="var(--kdb-accent)">
        <circle cx={454} cy={74} r={3} />
        <circle cx={454} cy={203} r={3} />
        <circle cx={454} cy={258} r={3} />
      </g>
      <path
        d="M171,198.5 L178,203 L171,207.5"
        fill="none"
        stroke="var(--kdb-accent)"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
      />

      {/* Light pooling under the stack. */}
      <g>
        <ellipse cx={CX} cy={294} rx={152} ry={36} fill="url(#kdb-base-glow)" />
        <ellipse
          cx={CX}
          cy={292}
          rx={122}
          ry={24}
          fill="none"
          stroke="white"
          strokeOpacity={0.45}
          strokeWidth={3}
        />
        <ellipse cx={CX} cy={290} rx={72} ry={14} fill="white" opacity={0.75} />
      </g>

      {[...TIERS].reverse().map((tier) => (
        <g
          key={tier.label}
          className="motion-safe:animate-[kdb-float_7s_ease-in-out_infinite]"
          style={{ animationDelay: tier.delay }}
        >
          <path
            d={`M${CX - RX},${tier.top} V${tier.top + TIER_HEIGHT} A${RX},${RY} 0 0 0 ${CX + RX},${tier.top + TIER_HEIGHT} V${tier.top} Z`}
            fill="url(#kdb-tier-wall)"
          />
          <ellipse cx={CX} cy={tier.top} rx={RX} ry={RY} fill="url(#kdb-tier-top)" />
          <ellipse
            cx={CX}
            cy={tier.top}
            rx={RX}
            ry={RY}
            fill="none"
            stroke="white"
            strokeOpacity={0.3}
          />
          <TierIcon kind={tier.icon} x={CX - 82} y={tier.top + 9} />
          <text x={CX - 52} y={tier.top + 24} fill="white" fontSize={15.5} fontWeight={500}>
            {tier.label}
          </text>
        </g>
      ))}

      {/* Domed cap carrying the brand mark. */}
      <g className="motion-safe:animate-[kdb-float_7s_ease-in-out_infinite]">
        <path
          d={`M${CX - RX},${CAP_BOTTOM} V${CAP_TOP} A${RX},42 0 0 1 ${CX + RX},${CAP_TOP} V${CAP_BOTTOM} A${RX},${RY} 0 0 1 ${CX - RX},${CAP_BOTTOM} Z`}
          fill="url(#kdb-cap-face)"
        />
        <path
          d={`M${CX - RX},${CAP_TOP} A${RX},42 0 0 1 ${CX + RX},${CAP_TOP} V${CAP_TOP + 18} A${RX},${RY} 0 0 0 ${CX - RX},${CAP_TOP + 18} Z`}
          fill="url(#kdb-dome-sheen)"
        />
        <image
          href={logoUrl}
          x={CX - 32}
          y={40}
          width={64}
          height={46}
          filter="url(#kdb-mark-white)"
          preserveAspectRatio="xMidYMid meet"
        />
        <text
          x={CX}
          y={110}
          textAnchor="middle"
          fill="white"
          fontSize={23}
          fontWeight={600}
          letterSpacing="-0.3"
        >
          KalamDB
        </text>
      </g>
    </svg>
  );
});
