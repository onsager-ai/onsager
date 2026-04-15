import { SessionTable } from "@/components/sessions/SessionTable"
import { CreateSessionSheet } from "@/components/sessions/CreateSessionSheet"
import { useSessions } from "@/hooks/useSessions"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Plus } from "lucide-react"

export function SessionsPage() {
  const { data, isLoading } = useSessions()
  const sessions = data?.sessions ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">Sessions</h1>
          <p className="text-sm text-muted-foreground">
            View and manage agent sessions.
          </p>
        </div>
        <CreateSessionSheet>
          <Button size="sm" className="hidden md:inline-flex">
            <Plus className="h-4 w-4" data-icon="inline-start" />
            New Session
          </Button>
        </CreateSessionSheet>
      </div>

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">All Sessions</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading...</p>
          ) : (
            <SessionTable sessions={sessions} />
          )}
        </CardContent>
      </Card>
    </div>
  )
}
