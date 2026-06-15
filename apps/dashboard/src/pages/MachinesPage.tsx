import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/ui/badge'
import { fleetApi } from '@/lib/api'

/** The fleet view (ADR 0030 M4): worker machines + live in-flight work.
 *  Operator surface — not workspace-scoped (machines are infra). */
export function MachinesPage() {
  const { data, isLoading } = useQuery({
    queryKey: ['fleet-machines'],
    queryFn: () => fleetApi.listMachines(),
    refetchInterval: 3000,
  })
  return (
    <>
      <h1 className="text-xl font-bold tracking-tight">Machines</h1>
      <p className="text-sm text-muted-foreground">
        Worker machines that dial in to run agent sessions (ADR 0030). With none
        connected, sessions run in the control plane itself.
      </p>
      {isLoading && <p className="text-sm text-muted-foreground">Loading...</p>}
      <div className="space-y-1">
        {data?.machines.map((m) => (
          <div
            key={m.name}
            className="flex items-baseline gap-3 rounded-md px-2 py-1.5 text-sm hover:bg-muted/40"
          >
            <Badge variant={m.status === 'online' ? 'default' : 'outline'}>{m.status}</Badge>
            <span className="font-medium">{m.name}</span>
            <span className="text-xs text-muted-foreground">
              {m.in_flight} in flight
            </span>
            <span className="ml-auto text-xs text-muted-foreground">
              last seen {new Date(m.last_seen_at).toLocaleString()}
            </span>
          </div>
        ))}
      </div>
      {data && data.machines.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No machines have connected. Run one with{' '}
          <code className="rounded bg-muted px-1 py-0.5 text-xs">onsager worker</code>.
        </p>
      )}
    </>
  )
}
