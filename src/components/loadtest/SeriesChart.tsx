interface Series {
  label: string;
  color: string;
  values: number[];
}

interface Props {
  series: Series[];
  height?: number;
  unit?: string;
  emptyText?: string;
}

const MAX_POINTS = 300;

function sample(values: number[]): number[] {
  if (values.length <= MAX_POINTS) return values;
  const step = values.length / MAX_POINTS;
  return Array.from({ length: MAX_POINTS }, (_, index) => {
    const value = values[Math.min(values.length - 1, Math.floor(index * step))];
    return Number.isFinite(value) ? value : 0;
  });
}

/** 轻量内联 SVG 折线：不引入图表依赖。 */
export function SeriesChart({
  series,
  height = 96,
  unit,
  emptyText = "暂无数据",
}: Props) {
  const prepared = series.map((item) => ({
    ...item,
    values: sample(item.values).map((value) =>
      Number.isFinite(value) ? Math.max(0, value) : 0,
    ),
  }));
  const max = Math.max(
    1,
    ...prepared.flatMap((item) => item.values),
  );
  const hasData = prepared.some((item) => item.values.length > 0);

  return (
    <div className="rounded border border-slate-200 bg-white p-2">
      <div className="mb-1 flex items-center justify-between text-[10px] text-slate-400">
        <div className="flex flex-wrap gap-2">
          {prepared.map((item) => (
            <span key={item.label} className="inline-flex items-center gap-1">
              <span
                className="inline-block h-1.5 w-3 rounded"
                style={{ backgroundColor: item.color }}
              />
              {item.label}
            </span>
          ))}
        </div>
        <span>
          {unit ? `${unit} ` : ""}0–{max.toFixed(max < 10 ? 1 : 0)}
        </span>
      </div>
      {hasData ? (
        <svg
          viewBox="0 0 100 30"
          preserveAspectRatio="none"
          style={{ height }}
          className="w-full"
          role="img"
          aria-label={prepared.map((item) => item.label).join("、")}
        >
          {[7.5, 15, 22.5].map((y) => (
            <line
              key={y}
              x1="0"
              x2="100"
              y1={y}
              y2={y}
              stroke="#e2e8f0"
              strokeWidth="0.3"
            />
          ))}
          {prepared.map((item) => {
            if (item.values.length === 0) return null;
            const points = item.values
              .map((value, index) => {
                const x =
                  item.values.length === 1
                    ? 0
                    : (index / (item.values.length - 1)) * 100;
                const y = 29 - (value / max) * 28;
                return `${x.toFixed(2)},${y.toFixed(2)}`;
              })
              .join(" ");
            return (
              <polyline
                key={item.label}
                points={points}
                fill="none"
                stroke={item.color}
                strokeWidth="0.8"
                vectorEffect="non-scaling-stroke"
              />
            );
          })}
        </svg>
      ) : (
        <div
          className="flex items-center justify-center text-xs text-slate-400"
          style={{ height }}
        >
          {emptyText}
        </div>
      )}
    </div>
  );
}
