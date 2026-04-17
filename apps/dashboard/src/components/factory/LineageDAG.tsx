import { Link } from "react-router-dom"
import type { ArtifactDetail } from "@/lib/api"
import {
  buildLanes,
  pairVersions,
} from "@/components/factory/lineage-dag-utils"

interface LineageDAGProps {
  artifact: ArtifactDetail
}

const NODE_WIDTH = 220
const NODE_HEIGHT = 82
const LANE_HEIGHT = 140

type GateEventData = {
  gate_point?: string
  verdict?: string
}

export function LineageDAG({ artifact }: LineageDAGProps) {
  const lanes = buildLanes(artifact.related_events)
  const paired = pairVersions(artifact.versions ?? [], lanes)

  if (paired.length === 0) {
    return (
      <div className="rounded-md border bg-muted/30 px-4 py-6 text-center text-sm text-muted-foreground">
        No shaping runs yet. Lanes appear once Forge dispatches this artifact.
      </div>
    )
  }

  const height = paired.length * LANE_HEIGHT + 40
  const width = NODE_WIDTH * 3 + 120

  return (
    <div className="overflow-x-auto">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        width="100%"
        className="max-w-4xl"
        role="img"
        aria-label="Per-run lineage DAG"
      >
        <defs>
          <marker
            id="arrow"
            viewBox="0 0 10 10"
            refX="8"
            refY="5"
            markerWidth="6"
            markerHeight="6"
            orient="auto-start-reverse"
          >
            <path
              d="M 0 0 L 10 5 L 0 10 z"
              className="fill-muted-foreground"
            />
          </marker>
        </defs>

        {paired.map(({ version, lane }, i) => {
          const y = 20 + i * LANE_HEIGHT
          const prevY = i > 0 ? 20 + (i - 1) * LANE_HEIGHT + NODE_HEIGHT : null
          const cx = 40 + NODE_WIDTH / 2

          const gateEvent = lane?.find(
            (e) => e.event_type === "forge.gate_verdict",
          )
          const gateData = (gateEvent?.data ?? {}) as GateEventData
          const failed = lane?.some(
            (e) => e.event_type === "stiglab.session_failed",
          )
          const completed = lane?.some(
            (e) => e.event_type === "stiglab.session_completed",
          )
          const retried = lane?.some(
            (e) => e.event_type === "forge.retry_requested",
          )

          return (
            <g key={version.version}>
              {prevY !== null && (
                <line
                  x1={cx}
                  y1={prevY}
                  x2={cx}
                  y2={y}
                  className="stroke-muted-foreground"
                  strokeWidth={1.5}
                  markerEnd="url(#arrow)"
                />
              )}

              {/* Version node */}
              <rect
                x={40}
                y={y}
                width={NODE_WIDTH}
                height={NODE_HEIGHT}
                rx={8}
                className="fill-card stroke-border"
                strokeWidth={1}
              />
              <text
                x={52}
                y={y + 22}
                className="fill-foreground font-mono text-[13px] font-semibold"
              >
                v{version.version}
              </text>
              <text
                x={52}
                y={y + 44}
                className="fill-muted-foreground text-[11px]"
              >
                {(version.change_summary || "—").slice(0, 28)}
              </text>
              <text
                x={52}
                y={y + 62}
                className="fill-muted-foreground text-[10px]"
              >
                {new Date(version.created_at).toLocaleString()}
              </text>

              {/* Session node */}
              <rect
                x={40 + NODE_WIDTH + 40}
                y={y}
                width={NODE_WIDTH}
                height={NODE_HEIGHT}
                rx={8}
                className={
                  failed
                    ? "fill-red-50 stroke-red-200 dark:fill-red-950/40 dark:stroke-red-900"
                    : "fill-blue-50 stroke-blue-200 dark:fill-blue-950/40 dark:stroke-blue-900"
                }
                strokeWidth={1}
              />
              <text
                x={40 + NODE_WIDTH + 52}
                y={y + 22}
                className="fill-foreground text-[12px] font-semibold"
              >
                Session
              </text>
              <text
                x={40 + NODE_WIDTH + 52}
                y={y + 44}
                className="fill-muted-foreground font-mono text-[11px]"
              >
                {version.created_by_session.slice(0, 16)}
              </text>
              <text
                x={40 + NODE_WIDTH + 52}
                y={y + 62}
                className="fill-muted-foreground text-[10px]"
              >
                {completed
                  ? "completed"
                  : failed
                    ? "failed"
                    : "in flight"}
              </text>
              <line
                x1={40 + NODE_WIDTH}
                y1={y + NODE_HEIGHT / 2}
                x2={40 + NODE_WIDTH + 40}
                y2={y + NODE_HEIGHT / 2}
                className="stroke-muted-foreground"
                strokeWidth={1.5}
                markerEnd="url(#arrow)"
              />

              {/* Gate node */}
              {gateEvent && (
                <g>
                  <rect
                    x={40 + (NODE_WIDTH + 40) * 2}
                    y={y}
                    width={NODE_WIDTH}
                    height={NODE_HEIGHT}
                    rx={8}
                    className={
                      gateData.verdict === "Allow"
                        ? "fill-emerald-50 stroke-emerald-200 dark:fill-emerald-950/40 dark:stroke-emerald-900"
                        : gateData.verdict === "Deny"
                          ? "fill-red-50 stroke-red-200 dark:fill-red-950/40 dark:stroke-red-900"
                          : "fill-amber-50 stroke-amber-200 dark:fill-amber-950/40 dark:stroke-amber-900"
                    }
                    strokeWidth={1}
                  />
                  <text
                    x={40 + (NODE_WIDTH + 40) * 2 + 12}
                    y={y + 22}
                    className="fill-foreground text-[12px] font-semibold"
                  >
                    Gate
                  </text>
                  <text
                    x={40 + (NODE_WIDTH + 40) * 2 + 12}
                    y={y + 44}
                    className="fill-muted-foreground text-[11px]"
                  >
                    {gateData.gate_point ?? "unknown"}
                  </text>
                  <text
                    x={40 + (NODE_WIDTH + 40) * 2 + 12}
                    y={y + 62}
                    className="fill-muted-foreground text-[11px]"
                  >
                    verdict: {gateData.verdict ?? "—"}
                  </text>
                  <line
                    x1={40 + NODE_WIDTH * 2 + 40}
                    y1={y + NODE_HEIGHT / 2}
                    x2={40 + (NODE_WIDTH + 40) * 2}
                    y2={y + NODE_HEIGHT / 2}
                    className="stroke-muted-foreground"
                    strokeWidth={1.5}
                    markerEnd="url(#arrow)"
                  />
                </g>
              )}

              {retried && (
                <text
                  x={40}
                  y={y + NODE_HEIGHT + 14}
                  className="fill-amber-600 text-[11px] dark:fill-amber-400"
                >
                  ↺ retry requested
                </text>
              )}
            </g>
          )
        })}
      </svg>

      <div className="mt-3 flex flex-wrap gap-4 text-xs text-muted-foreground">
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-3 rounded bg-blue-100 dark:bg-blue-950/60" />
          session
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-3 rounded bg-emerald-100 dark:bg-emerald-950/60" />
          gate allow
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-3 rounded bg-red-100 dark:bg-red-950/60" />
          deny / fail
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-3 rounded bg-amber-100 dark:bg-amber-950/60" />
          escalate / modify
        </span>
      </div>

      <div className="mt-4 space-y-1 text-xs">
        {paired.map(({ version }) => (
          <div
            key={version.version}
            className="flex items-center gap-2 font-mono text-muted-foreground"
          >
            <span>v{version.version}</span>
            <span>·</span>
            <Link
              to={`/sessions/${version.created_by_session}`}
              className="hover:text-foreground hover:underline"
            >
              {version.created_by_session.slice(0, 12)}
            </Link>
            <span>·</span>
            <span className="truncate">{version.change_summary || "—"}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

