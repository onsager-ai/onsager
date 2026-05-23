import { useCallback, useState } from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { AlertTriangle, Bell, BellOff } from "lucide-react"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import type { Workspace } from "@/lib/api"
import { api } from "@/lib/api"
import {
  describePushSupport,
  isCurrentlySubscribed,
  subscribePush,
  unsubscribePush,
} from "@/lib/push"

// Workspace-scoped HITL push notification toggle (ADR 0020 / spec #472).
//
// Reads the portal config once to decide whether to show the card at
// all. When enabled, the user can opt in/out per-browser; the toggle
// surface also shows current permission state and a test-dispatch
// button (only when dev-login is on portal-side; the call returns 403
// otherwise and we surface the error).

interface PushNotificationsCardProps {
  workspace: Workspace
}

export function PushNotificationsCard({ workspace }: PushNotificationsCardProps) {
  const queryClient = useQueryClient()
  const [error, setError] = useState<string | null>(null)
  const [testStatus, setTestStatus] = useState<string | null>(null)

  const { data: support, isLoading: loadingSupport } = useQuery({
    queryKey: ["push-support"],
    queryFn: describePushSupport,
    staleTime: 60_000,
  })

  // Subscription state is owned by React Query so we never cascade a
  // setState inside an effect. Mutations invalidate this query to
  // re-read after subscribe/unsubscribe.
  const subSupported = !!support?.supported
  const { data: subscribedData } = useQuery({
    queryKey: ["push-subscribed"],
    queryFn: isCurrentlySubscribed,
    enabled: subSupported,
    staleTime: 60_000,
  })
  const subscribed = subSupported ? !!subscribedData : false

  const subscribeMutation = useMutation({
    mutationFn: async () => {
      if (!support?.vapidPublicKey) throw new Error("Push not configured")
      return subscribePush(workspace.id, support.vapidPublicKey)
    },
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ["push-support"] })
      queryClient.invalidateQueries({ queryKey: ["push-subscribed"] })
    },
    onError: (e) =>
      setError(e instanceof Error ? e.message : "Subscribe failed"),
  })

  const unsubscribeMutation = useMutation({
    mutationFn: unsubscribePush,
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ["push-subscribed"] })
    },
    onError: (e) =>
      setError(e instanceof Error ? e.message : "Unsubscribe failed"),
  })

  const onTestDispatch = useCallback(async () => {
    setTestStatus(null)
    try {
      const res = await api.push.testDispatch(
        workspace.id,
        `/workspaces/${workspace.slug}/workflows`,
        "workspace",
      )
      setTestStatus(`Delivered to ${res.delivered} subscription(s).`)
    } catch (e) {
      setTestStatus(
        `Test failed: ${e instanceof Error ? e.message : "unknown"}`,
      )
    }
  }, [workspace.id, workspace.slug])

  // While we're still figuring out support, render a quiet placeholder
  // so the card doesn't flash open and closed.
  if (loadingSupport) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base md:text-lg">
            <Bell className="h-4 w-4" />
            Notifications
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">Checking support…</p>
        </CardContent>
      </Card>
    )
  }

  // The deploy hasn't configured VAPID keys — hide the toggle entirely.
  // We still surface the card so operators see *why* there's no toggle.
  if (!support?.enabled) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base md:text-lg">
            <BellOff className="h-4 w-4" />
            Notifications
          </CardTitle>
          <CardDescription>
            Push notifications aren&apos;t configured on this deployment.
          </CardDescription>
        </CardHeader>
      </Card>
    )
  }

  const browserUnsupported = !support.supported

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base md:text-lg">
          <Bell className="h-4 w-4" />
          Notifications
        </CardTitle>
        <CardDescription>
          Receive a push notification on this browser when an agent asks for
          a decision in <strong>{workspace.name}</strong>.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {browserUnsupported && (
          <div className="flex items-start gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              This browser doesn&apos;t support Web Push. Try a desktop or
              mobile Chrome / Firefox / Edge.
            </span>
          </div>
        )}
        {support.permission === "denied" && (
          <div className="flex items-start gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              Notifications are blocked for this site. Allow them in your
              browser&apos;s site settings and reload.
            </span>
          </div>
        )}
        {error && (
          <p className="text-sm text-destructive">{error}</p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          {subscribed ? (
            <Button
              variant="outline"
              onClick={() => unsubscribeMutation.mutate()}
              disabled={unsubscribeMutation.isPending}
            >
              <BellOff className="mr-1 h-4 w-4" />
              {unsubscribeMutation.isPending ? "Turning off…" : "Turn off on this browser"}
            </Button>
          ) : (
            <Button
              onClick={() => subscribeMutation.mutate()}
              disabled={
                subscribeMutation.isPending ||
                browserUnsupported ||
                support.permission === "denied"
              }
            >
              <Bell className="mr-1 h-4 w-4" />
              {subscribeMutation.isPending ? "Subscribing…" : "Enable on this browser"}
            </Button>
          )}
          {subscribed && (
            <Button variant="ghost" onClick={onTestDispatch}>
              Send test
            </Button>
          )}
        </div>
        {testStatus && (
          <p className="text-xs text-muted-foreground">{testStatus}</p>
        )}
      </CardContent>
    </Card>
  )
}
