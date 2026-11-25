import { memo } from "react";

const logoUrl = `${import.meta.env.BASE_URL}favicon.png`;

const SLABS = [
  { cy: 186, opacity: 0.5, delay: "0s" },
  { cy: 146, opacity: 0.72, delay: "0.3s" },
  { cy: 106, opacity: 1, delay: "0.6s" },
];

const HALF_W = 92;
const HALF_H = 46;
const DEPTH = 20;
const CX = 235;

const CONNECTORS = [
  { path: "M147,128 C128,124 120,72 108,62", dot: [108, 62] },
  { path: "M147,176 C130,180 120,194 108,200", dot: [108, 200] },
  { path: "M323,120 C344,112 350,58 362,45", dot: [362, 45] },
  { path: "M327,146 C342,142 352,132 362,127", dot: [362, 127] },
  { path: "M323,180 C342,188 352,200 362,209", dot: [362, 209] },
];

function topFace(cy: number): string {
  return `${CX},${cy - HALF_H} ${CX + HALF_W},${cy} ${CX},${cy + HALF_H} ${CX - HALF_W},${cy}`;
}

function leftFace(cy: number): string {
  return `${CX - HALF_W},${cy} ${CX},${cy + HALF_H} ${CX},${cy + HALF_H + DEPTH} ${CX - HALF_W},${cy + DEPTH}`;
}

function rightFace(cy: number): string {
  return `${CX + HALF_W},${cy} ${CX},${cy + HALF_H} ${CX},${cy + HALF_H + DEPTH} ${CX + HALF_W},${cy + DEPTH}`;
}

export const HeroGraphic = memo(function HeroGraphic() {
  return (
    <svg
      viewBox="0 0 470 250"
      className="absolute inset-0 size-full"
      role="presentation"
      aria-hidden
    >
      <defs>
        <linearGradient id="kdb-slab-top" x1="0" y1="0" x2="0.9" y2="1">
          <stop offset="0%" stopColor="var(--kdb-accent)" stopOpacity={0.95} />
          <stop offset="55%" stopColor="var(--kdb-accent-strong)" />
          <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0.9} />
        </linearGradient>
        <linearGradient id="kdb-slab-side" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--kdb-accent-strong)" />
          <stop offset="100%" stopColor="var(--kdb-accent-strong)" stopOpacity={0.55} />
        </linearGradient>
        <radialGradient id="kdb-slab-glow">
          <stop offset="0%" stopColor="var(--kdb-accent)" stopOpacity={0.34} />
          <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0} />
        </radialGradient>
        <clipPath id="kdb-logo-clip">
          <rect x={213} y={84} width={44} height={44} rx={10} />
        </clipPath>
      </defs>

      <ellipse cx={CX} cy={214} rx={132} ry={38} fill="url(#kdb-slab-glow)" />

      {CONNECTORS.map((connector) => (
        <g key={connector.path}>
          <path
            d={connector.path}
            fill="none"
            stroke="var(--kdb-accent-line)"
            strokeWidth={1.5}
            strokeLinecap="round"
          />
          <circle cx={connector.dot[0]} cy={connector.dot[1]} r={3} fill="var(--kdb-accent)" />
        </g>
      ))}

      {SLABS.map((slab) => (
        <g
          key={slab.cy}
          className="motion-safe:animate-[kdb-float_6s_ease-in-out_infinite]"
          style={{ animationDelay: slab.delay }}
        >
          <polygon points={leftFace(slab.cy)} fill="url(#kdb-slab-side)" opacity={slab.opacity * 0.8} />
          <polygon points={rightFace(slab.cy)} fill="url(#kdb-slab-side)" opacity={slab.opacity * 0.5} />
          <polygon points={topFace(slab.cy)} fill="url(#kdb-slab-top)" opacity={slab.opacity} />
          <polygon
            points={topFace(slab.cy)}
            fill="none"
            stroke="var(--kdb-surface)"
            strokeOpacity={0.4}
            strokeWidth={1.25}
          />
        </g>
      ))}

      <g
        className="motion-safe:animate-[kdb-float_6s_ease-in-out_infinite]"
        style={{ animationDelay: "0.6s" }}
      >
        <image
          href={logoUrl}
          x={213}
          y={84}
          width={44}
          height={44}
          clipPath="url(#kdb-logo-clip)"
          preserveAspectRatio="xMidYMid meet"
        />
      </g>
    </svg>
  );
});
