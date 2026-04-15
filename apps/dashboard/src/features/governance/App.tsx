/* eslint-disable react-hooks/set-state-in-effect -- governance feature from synodic-ui, cleanup in PR-B */
import { useCallback, useEffect, useState } from 'react'
import type { Event, Stats } from './api'
import { fetchEvents, fetchStats, resolveEvent } from './api'

const SEVERITY_COLORS: Record<string, string> = {
  critical: '#ef4444',
  high: '#f97316',
  medium: '#eab308',
  low: '#22c55e',
}

const TYPE_LABELS: Record<string, string> = {
  tool_call_error: 'Tool Error',
  hallucination: 'Hallucination',
  compliance_violation: 'Compliance',
  misalignment: 'Misalignment',
}

export function App() {
  const [events, setEvents] = useState<Event[]>([])
  const [stats, setStats] = useState<Stats | null>(null)
  const [filter, setFilter] = useState<string>('')
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    setLoading(true)
    const params: Record<string, string> = {}
    if (filter) params.type = filter
    const [ev, st] = await Promise.all([fetchEvents(params), fetchStats()])
    setEvents(ev)
    setStats(st)
    setLoading(false)
  }, [filter])

  useEffect(() => { load() }, [load])

  const handleResolve = async (id: string) => {
    const notes = prompt('Resolution notes:')
    if (notes === null) return
    await resolveEvent(id, notes)
    load()
  }

  return (
    <div style={{ maxWidth: 1200, margin: '0 auto', padding: '24px 16px' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 32 }}>
        <div>
          <h1 style={{ fontSize: 24, fontWeight: 700 }}>Synodic</h1>
          <p style={{ color: '#888', fontSize: 14 }}>AI Agent Governance Dashboard</p>
        </div>
        {stats && (
          <div style={{ display: 'flex', gap: 24, fontSize: 14 }}>
            <Stat label="Total" value={stats.total} />
            <Stat label="Unresolved" value={stats.unresolved} color="#ef4444" />
            <Stat label="Resolution" value={`${stats.total > 0 ? Math.round(((stats.total - stats.unresolved) / stats.total) * 100) : 0}%`} />
          </div>
        )}
      </header>

      <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
        {['', 'tool_call_error', 'hallucination', 'compliance_violation', 'misalignment'].map(t => (
          <button
            key={t}
            onClick={() => setFilter(t)}
            style={{
              padding: '6px 12px',
              borderRadius: 6,
              border: 'none',
              background: filter === t ? '#333' : '#1a1a1a',
              color: filter === t ? '#fff' : '#888',
              cursor: 'pointer',
              fontSize: 13,
            }}
          >
            {t ? TYPE_LABELS[t] || t : 'All'}
          </button>
        ))}
      </div>

      {loading ? (
        <p style={{ color: '#666' }}>Loading...</p>
      ) : events.length === 0 ? (
        <p style={{ color: '#666', textAlign: 'center', padding: 48 }}>No events found. Submit events via CLI or API.</p>
      ) : (
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 14 }}>
          <thead>
            <tr style={{ borderBottom: '1px solid #333', textAlign: 'left' }}>
              <th style={{ padding: '8px 12px', color: '#888', fontWeight: 500 }}>Type</th>
              <th style={{ padding: '8px 12px', color: '#888', fontWeight: 500 }}>Severity</th>
              <th style={{ padding: '8px 12px', color: '#888', fontWeight: 500 }}>Title</th>
              <th style={{ padding: '8px 12px', color: '#888', fontWeight: 500 }}>Source</th>
              <th style={{ padding: '8px 12px', color: '#888', fontWeight: 500 }}>Status</th>
              <th style={{ padding: '8px 12px', color: '#888', fontWeight: 500 }}>Created</th>
              <th style={{ padding: '8px 12px' }}></th>
            </tr>
          </thead>
          <tbody>
            {events.map(e => (
              <tr key={e.id} style={{ borderBottom: '1px solid #1a1a1a' }}>
                <td style={{ padding: '10px 12px' }}>
                  <span style={{ background: '#1a1a1a', padding: '2px 8px', borderRadius: 4, fontSize: 12 }}>
                    {TYPE_LABELS[e.event_type] || e.event_type}
                  </span>
                </td>
                <td style={{ padding: '10px 12px' }}>
                  <span style={{ color: SEVERITY_COLORS[e.severity] || '#888', fontWeight: 600, fontSize: 12, textTransform: 'uppercase' }}>
                    {e.severity}
                  </span>
                </td>
                <td style={{ padding: '10px 12px' }}>{e.title}</td>
                <td style={{ padding: '10px 12px', color: '#888' }}>{e.source}</td>
                <td style={{ padding: '10px 12px' }}>
                  {e.resolved
                    ? <span style={{ color: '#22c55e' }}>Resolved</span>
                    : <span style={{ color: '#ef4444' }}>Open</span>
                  }
                </td>
                <td style={{ padding: '10px 12px', color: '#888', fontSize: 12 }}>
                  {new Date(e.created_at).toLocaleString()}
                </td>
                <td style={{ padding: '10px 12px' }}>
                  {!e.resolved && (
                    <button
                      onClick={() => handleResolve(e.id)}
                      style={{ padding: '4px 10px', borderRadius: 4, border: '1px solid #333', background: 'transparent', color: '#888', cursor: 'pointer', fontSize: 12 }}
                    >
                      Resolve
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

function Stat({ label, value, color }: { label: string; value: string | number; color?: string }) {
  return (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: 20, fontWeight: 700, color: color || '#fff' }}>{value}</div>
      <div style={{ color: '#888', fontSize: 12 }}>{label}</div>
    </div>
  )
}
