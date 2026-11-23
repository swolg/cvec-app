import { Group } from "@visx/group";
import { Pie } from "@visx/shape";
import { Text } from "@visx/text";

export interface WorkspaceIssuesDonutDatum {
  label: string;
  value: number;
  color: string;
}

interface WorkspaceIssuesDonutChartProps {
  data: readonly WorkspaceIssuesDonutDatum[];
  total: number;
  totalLabel?: string;
  size?: number;
  thickness?: number;
}

const FALLBACK_TRACK_COLOR = "var(--hairline)";
const FALLBACK_TEXT_COLOR = "var(--ink)";
const FALLBACK_MUTED_COLOR = "var(--ink-subtle)";

export function WorkspaceIssuesDonutChart({
  data,
  total,
  totalLabel = "Total issues",
  size = 172,
  thickness = 18,
}: WorkspaceIssuesDonutChartProps) {
  const radius = size / 2;
  const innerRadius = Math.max(radius - thickness, 0);
  const hasData = total > 0;

  const normalizedData = data.filter((item) => item.value > 0);

  return (
    <div className="flex items-center justify-center">
      <svg
        aria-label={totalLabel}
        height={size}
        role="img"
        viewBox={`0 0 ${size} ${size}`}
        width={size}
      >
        <Group left={radius} top={radius}>
          <circle
            cx={0}
            cy={0}
            fill="none"
            r={innerRadius + thickness / 2}
            stroke={FALLBACK_TRACK_COLOR}
            strokeOpacity={0.22}
            strokeWidth={thickness}
          />

          {hasData ? <Pie<WorkspaceIssuesDonutDatum>
            cornerRadius={hasData ? 7 : 0}
            data={normalizedData}
            innerRadius={innerRadius}
            outerRadius={radius - 2}
            padAngle={hasData ? 0.02 : 0}
            pieValue={(item) => item.value}
            startAngle={-Math.PI / 2}
            endAngle={(Math.PI * 3) / 2}
          >
            {(pie) =>
              pie.arcs.map((arc) => {
                const path = pie.path(arc);

                if (!path) {
                  return null;
                }

                return (
                  <path
                    key={arc.data.label}
                    d={path}
                    fill={arc.data.color}
                    fillOpacity={hasData ? 1 : 0.35}
                  />
                );
              })
            }
          </Pie> : null}

          <Text
            fill={FALLBACK_TEXT_COLOR}
            fontFamily="var(--font-sans)"
            fontSize={18}
            fontWeight={700}
            textAnchor="middle"
            verticalAnchor="end"
            y={-2}
          >
            {total}
          </Text>
          <Text
            fill={FALLBACK_MUTED_COLOR}
            fontFamily="var(--font-sans)"
            fontSize={11}
            textAnchor="middle"
            verticalAnchor="start"
            y={6}
          >
            {totalLabel}
          </Text>
        </Group>
      </svg>
    </div>
  );
}
