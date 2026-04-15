import { useEffect, useRef, useState } from 'react';

interface SSEChunk {
  chunk: string;
  stream: string;
}

interface SSEData {
  state: string;
  chunks: SSEChunk[];
}

export function useSessionLogs(sessionId: string | undefined) {
  const [logs, setLogs] = useState<string>('');
  const [state, setState] = useState<string>('');
  const eventSourceRef = useRef<EventSource | null>(null);

  useEffect(() => {
    if (!sessionId) return;

    const es = new EventSource(`/api/sessions/${sessionId}/logs`);
    eventSourceRef.current = es;

    es.onmessage = (event) => {
      try {
        const data: SSEData = JSON.parse(event.data);
        if (data.state) {
          setState(data.state);
        }
        if (data.chunks && data.chunks.length > 0) {
          const newText = data.chunks.map((c) => c.chunk).join('');
          setLogs((prev) => prev + newText);
        }
      } catch {
        // ignore parse errors
      }
    };

    es.onerror = () => {
      es.close();
    };

    return () => {
      es.close();
    };
  }, [sessionId]);

  return { logs, state };
}
