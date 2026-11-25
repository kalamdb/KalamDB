import { useId, useMemo } from "react";
import { cn } from "@/lib/utils";

interface SparklineProps {
  values: number[];
  className?: string;
  stroke?: string;
  height?: number;
}

const VIEW_WIDTH = 100;

function buildPath(values: number[], viewHeight: number): { line: string; area: string } | null {
  const finite = values.filter((value) => Number.isFinite(value));
  if (finite.length < 2) {
    return null;
  }

  const min = Math.min(...finite);
  const max = Math.max(...finite);
  const span = max - min || 1;
  const step = VIEW_WIDTH / (finite.length - 1);

  const points = finite.map((value, index) => {
    const x = index * step;
    const y = viewHeight - ((value - min) / span) * (viewHeight - 2) - 1;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });

  const line = `M${points.join(" L")}`;
  return {
    line,
    area: `${line} L${VIEW_WIDTH},${viewHeight} L0,${viewHeight} Z`,
  };
}

export function Sparkline({
  values,
  className,
  stroke = "var(--kdb-accent)",
  height = 36,
}: SparklineProps) {
  const gradientId = useId();
  const path = useMemo(() => buildPath(values, height), [values, height]);

  if (!path) {
    return <div className={cn("w-full", className)} style={{ height }} aria-hidden />;
  }

  return (
    <svg
      className={cn("w-full overflow-visible", className)}
      style={{ height }}
      viewBox={`0 0 ${VIEW_WIDTH} ${height}`}
      preserveAspectRatio="none"
      aria-hidden
    >
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={stroke} stopOpacity={0.24} />
          <stop offset="100%" stopColor={stroke} stopOpacity={0} />
        </linearGradient>
      </defs>
      <path d={path.area} fill={`url(#${gradientId})`} />
      <path
        d={path.line}
        fill="none"
        stroke={stroke}
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
