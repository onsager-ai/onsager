import { NodeTable } from "@/components/nodes/NodeTable"
import { useNodes } from "@/hooks/useNodes"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"

export function NodesPage() {
  const { data, isLoading } = useNodes()
  const nodes = data?.nodes ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="text-xl font-bold tracking-tight md:text-2xl">Nodes</h1>
        <p className="text-sm text-muted-foreground">
          Manage registered agent nodes.
        </p>
      </div>

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">Registered Nodes</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading...</p>
          ) : (
            <NodeTable nodes={nodes} />
          )}
        </CardContent>
      </Card>
    </div>
  )
}
