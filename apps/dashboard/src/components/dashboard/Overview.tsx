import { Server, Terminal, AlertCircle, CheckCircle } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import type { Node, Session } from "@/lib/api"

interface OverviewProps {
  nodes: Node[]
  sessions: Session[]
}

export function Overview({ nodes, sessions }: OverviewProps) {
  const onlineNodes = nodes.filter((n) => n.status === "online").length
  const activeSessions = sessions.filter((s) =>
    ["running", "dispatched", "waiting_input"].includes(s.state)
  ).length
  const waitingInput = sessions.filter((s) => s.state === "waiting_input").length
  const completedSessions = sessions.filter((s) => s.state === "done").length

  const stats = [
    {
      title: "Nodes Online",
      value: `${onlineNodes}/${nodes.length}`,
      icon: Server,
      description: `${nodes.length - onlineNodes} offline`,
    },
    {
      title: "Active Sessions",
      value: activeSessions,
      icon: Terminal,
      description: "Currently running",
    },
    {
      title: "Waiting Input",
      value: waitingInput,
      icon: AlertCircle,
      description: "Needs attention",
      highlight: waitingInput > 0,
    },
    {
      title: "Completed",
      value: completedSessions,
      icon: CheckCircle,
      description: "Successfully done",
    },
  ]

  return (
    <div className="grid grid-cols-2 gap-3 md:gap-4 lg:grid-cols-4">
      {stats.map((stat) => (
        <Card key={stat.title} className={stat.highlight ? "border-yellow-500/50" : ""}>
          <CardHeader className="flex flex-row items-center justify-between px-3 pb-1 pt-3 md:px-6 md:pb-2 md:pt-6">
            <CardTitle className="text-xs font-medium text-muted-foreground md:text-sm">
              {stat.title}
            </CardTitle>
            <stat.icon className={`h-4 w-4 ${stat.highlight ? "text-yellow-500" : "text-muted-foreground"}`} />
          </CardHeader>
          <CardContent className="px-3 pb-3 md:px-6 md:pb-6">
            <div className={`text-xl font-bold md:text-2xl ${stat.highlight ? "text-yellow-500" : ""}`}>
              {stat.value}
            </div>
            <p className="text-[10px] text-muted-foreground md:text-xs">{stat.description}</p>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}
