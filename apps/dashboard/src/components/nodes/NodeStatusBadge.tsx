import { Badge } from "@/components/ui/badge"

const statusConfig = {
  online: { label: "Online", variant: "default" as const, className: "bg-green-500/10 text-green-500 border-green-500/20" },
  offline: { label: "Offline", variant: "secondary" as const, className: "bg-gray-500/10 text-gray-400 border-gray-500/20" },
  draining: { label: "Draining", variant: "outline" as const, className: "bg-yellow-500/10 text-yellow-500 border-yellow-500/20" },
}

export function NodeStatusBadge({ status }: { status: string }) {
  const config = statusConfig[status as keyof typeof statusConfig] ?? statusConfig.offline
  return (
    <Badge variant={config.variant} className={config.className}>
      {config.label}
    </Badge>
  )
}
