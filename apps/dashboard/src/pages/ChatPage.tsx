import { MessageSquare } from "lucide-react"
import { Card, CardContent } from "@/components/ui/card"
import { usePageHeader } from "@/components/layout/PageHeader"

// Top-level Chat surface per spec #289. PR 1 (sidebar collapse) ships
// the route as a stub so the new sidebar nav links somewhere live;
// PR 4 (chat surface promotion) replaces this with the real MCP-backed
// chat builder from #288.
export function ChatPage() {
  usePageHeader({ title: "Chat" })

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">Chat</h1>
        <p className="text-sm text-muted-foreground">
          R&amp;D mode. Design workflows and run ad-hoc one-shots by talking
          to the agent.
        </p>
      </div>
      <Card>
        <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
            <MessageSquare className="h-6 w-6" />
          </div>
          <div>
            <p className="text-base font-medium">Chat surface coming soon</p>
            <p className="max-w-md text-sm text-muted-foreground">
              The agent-backed chat builder lands with the MCP backbone in a
              follow-up PR. Until then, design workflows from the Workflows
              page.
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
