import { useEffect, useRef } from "react"
import { useSessionLogs } from "@/lib/sse"

interface SessionLogStreamProps {
  sessionId: string
}

export function SessionLogStream({ sessionId }: SessionLogStreamProps) {
  const { chunks } = useSessionLogs(sessionId)
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [chunks])

  return (
    <div className="h-[60vh] max-h-[600px] min-h-[300px] overflow-auto rounded-md border bg-black/50 p-3 md:h-[400px] md:p-4">
      <pre className="font-mono text-xs leading-relaxed whitespace-pre-wrap md:text-sm">
        {chunks.length === 0 ? (
          <span className="text-green-400">Waiting for output...</span>
        ) : (
          chunks.map((chunk, i) =>
            chunk.stream === "stderr" ? (
              <span key={i} className="text-yellow-500/70">{chunk.text}</span>
            ) : (
              <span key={i} className="text-green-400">{chunk.text}</span>
            )
          )
        )}
      </pre>
      <div ref={bottomRef} />
    </div>
  )
}
