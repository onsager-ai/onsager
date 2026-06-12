import { Badge } from "@/components/ui/badge"

const stateConfig = {
  pending: { label: "Pending", className: "bg-gray-500/10 text-gray-400 border-gray-500/20" },
  dispatched: { label: "Dispatched", className: "bg-blue-500/10 text-blue-400 border-blue-500/20" },
  running: { label: "Running", className: "bg-green-500/10 text-green-500 border-green-500/20" },
  waiting_input: { label: "Waiting Input", className: "bg-yellow-500/10 text-yellow-500 border-yellow-500/20" },
  done: { label: "Done", className: "bg-gray-500/10 text-gray-400 border-gray-500/20" },
  failed: { label: "Failed", className: "bg-red-500/10 text-red-500 border-red-500/20" },
}

export function SessionStateBadge({ state }: { state: string }) {
  const config = stateConfig[state as keyof typeof stateConfig] ?? stateConfig.pending
  return (
    <Badge variant="outline" className={config.className}>
      {config.label}
    </Badge>
  )
}
