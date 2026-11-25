import { memo } from "react";

const SLABS = [
  { cy: 152, opacity: 0.55, delay: "0s" },
  { cy: 112, opacity: 0.75, delay: "0.25s" },
  { cy: 72, opacity: 1, delay: "0.5s" },
];

const HALF_W = 86;
const HALF_H = 43;
const DEPTH = 16;

function topFace(cy: number): string {
  return `160,${cy - HALF_H} ${160 + HALF_W},${cy} 160,${cy + HALF_H} ${160 - HALF_W},${cy}`;
}

function leftFace(cy: number): string {
  return `${160 - HALF_W},${cy} 160,${cy + HALF_H} 160,${cy + HALF_H + DEPTH} ${160 - HALF_W},${cy + DEPTH}`;
}

function rightFace(cy: number): string {
  return `${160 + HALF_W},${cy} 160,${cy + HALF_H} 160,${cy + HALF_H + DEPTH} ${160 + HALF_W},${cy + DEPTH}`;
}

export const HeroGraphic = memo(function HeroGraphic() {
  return (
    <svg
      viewBox="0 0 320 230"
      className="h-[190px] w-[250px] shrink-0"
      role="presentation"
      aria-hidden
    >
      <defs>
        <linearGradient id="kdb-slab-top" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="var(--kdb-accent-strong)" />
          <stop offset="100%" stopColor="var(--kdb-accent)" />
        </linearGradient>
        <radialGradient id="kdb-slab-glow">
          <stop offset="0%" stopColor="var(--kdb-accent)" stopOpacity={0.3} />
          <stop offset="100%" stopColor="var(--kdb-accent)" stopOpacity={0} />
        </radialGradient>
      </defs>

      <ellipse cx="160" cy="186" rx="118" ry="34" fill="url(#kdb-slab-glow)" />

      {SLABS.map((slab) => (
        <g
          key={slab.cy}
          className="motion-safe:animate-[kdb-float_6s_ease-in-out_infinite]"
          style={{ animationDelay: slab.delay }}
        >
          <polygon points={leftFace(slab.cy)} fill="var(--kdb-accent-strong)" opacity={slab.opacity * 0.65} />
          <polygon points={rightFace(slab.cy)} fill="var(--kdb-accent-strong)" opacity={slab.opacity * 0.45} />
          <polygon points={topFace(slab.cy)} fill="url(#kdb-slab-top)" opacity={slab.opacity} />
          <polygon
            points={topFace(slab.cy)}
            fill="none"
            stroke="var(--color-background)"
            strokeOpacity={0.35}
            strokeWidth={1}
          />
        </g>
      ))}
    </svg>
  );
});
