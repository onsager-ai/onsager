import { useEffect, useRef } from "react"
import { useSessionLogs } from "@/lib/sse"

interface SessionLogStreamProps {
  sessionId: string
}

export function SessionLogStream({ sessionId }: SessionLogStreamProps) {
  const { logs } = useSessionLogs(sessionId)
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [logs])

  return (
    <div className="h-[60vh] max-h-[600px] min-h-[300px] overflow-auto rounded-md border bg-black/50 p-3 md:h-[400px] md:p-4">
      <pre className="font-mono text-xs leading-relaxed text-green-400 whitespace-pre-wrap md:text-sm">
        {logs || "Waiting for output..."}
      </pre>
      <div ref={bottomRef} />
    </div>
  )
}
