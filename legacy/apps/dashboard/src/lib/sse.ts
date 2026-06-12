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

/** Merge adjacent chunks that share the same stream type. */
function coalesce(chunks: LogChunk[]): LogChunk[] {
  if (chunks.length === 0) return chunks;
  const out: LogChunk[] = [{ ...chunks[0] }];
  for (let i = 1; i < chunks.length; i++) {
    const last = out[out.length - 1];
    if (chunks[i].stream === last.stream) {
      last.text += chunks[i].text;
    } else {
      out.push({ ...chunks[i] });
    }
  }
  return out;
}

export function useSessionLogs(sessionId: string | undefined) {
  const [chunks, setChunks] = useState<LogChunk[]>([]);
  const [state, setState] = useState<string>('');
  const [prevSessionId, setPrevSessionId] = useState<string | undefined>(undefined);
  const eventSourceRef = useRef<EventSource | null>(null);

  // Reset state when sessionId changes (React-idiomatic prop-change pattern)
  if (sessionId !== prevSessionId) {
    setPrevSessionId(sessionId);
    setChunks([]);
    setState('');
  }

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
          setChunks((prev) => coalesce([...prev, ...newChunks]));
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
