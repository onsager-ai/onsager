import { createContext, useContext, useEffect, useState, useCallback, type ReactNode } from "react"
import { api, type SessionKind, type User } from "./api"

interface AuthContextValue {
  user: User | null
  loading: boolean
  /**
   * How the session was minted, surfaced by `/api/auth/me`. `"github"` for
   * a real OAuth session, `"dev"` for a debug-build dev-login session.
   * `null` while loading or when the user is signed out (issue #193).
   */
  sessionKind: SessionKind | null
  logout: () => Promise<void>
}

const AuthContext = createContext<AuthContextValue>({
  user: null,
  loading: true,
  sessionKind: null,
  logout: async () => {},
})

// eslint-disable-next-line react-refresh/only-export-components
export function useAuth() {
  return useContext(AuthContext)
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null)
  const [sessionKind, setSessionKind] = useState<SessionKind | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    api
      .getMe()
      .then((data) => {
        setUser(data.user)
        setSessionKind(data.session_kind)
      })
      .catch(() => {
        setUser(null)
        setSessionKind(null)
      })
      .finally(() => setLoading(false))
  }, [])

  const logout = useCallback(async () => {
    try {
      await api.logout()
    } finally {
      setUser(null)
      setSessionKind(null)
      window.location.href = "/login"
    }
  }, [])

  return (
    <AuthContext.Provider value={{ user, loading, sessionKind, logout }}>
      {children}
    </AuthContext.Provider>
  )
}
