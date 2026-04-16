import { useEffect, useRef, useState } from 'react';

export interface LogChunk {
  text: string;
  stream: 'stdout' | 'stderr';
}

interface SSERawChunk {
  chunk: string;
  stream: string;
}

interface SSEData {
  state: string;
  chunks: SSERawChunk[];
}

export function useSessionLogs(sessionId: string | undefined) {
  const [chunks, setChunks] = useState<LogChunk[]>([]);
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
          const newChunks: LogChunk[] = data.chunks.map((c) => ({
            text: c.chunk,
            stream: c.stream === 'stderr' ? 'stderr' : 'stdout',
          }));
          setChunks((prev) => [...prev, ...newChunks]);
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

  return { chunks, state };
}
