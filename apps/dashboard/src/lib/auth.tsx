import { createContext, useContext, useEffect, useState, useCallback, type ReactNode } from "react"
import { api, ApiError, type User } from "./api"

interface AuthContextValue {
  user: User | null
  loading: boolean
  authEnabled: boolean
  logout: () => Promise<void>
}

const AuthContext = createContext<AuthContextValue>({
  user: null,
  loading: true,
  authEnabled: false,
  logout: async () => {},
})

// eslint-disable-next-line react-refresh/only-export-components
export function useAuth() {
  return useContext(AuthContext)
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null)
  const [loading, setLoading] = useState(true)
  const [authEnabled, setAuthEnabled] = useState(false)

  useEffect(() => {
    api
      .getMe()
      .then((data) => {
        setUser(data.user)
        setAuthEnabled(data.auth_enabled)
      })
      .catch((err) => {
        setUser(null)
        // Only treat as "auth enabled" if the backend explicitly returned 401.
        // Network errors or 404s (e.g. no backend running) mean auth is not available.
        if (err instanceof ApiError && err.status === 401) {
          setAuthEnabled(true)
        } else {
          setAuthEnabled(false)
        }
      })
      .finally(() => setLoading(false))
  }, [])

  const logout = useCallback(async () => {
    try {
      await api.logout()
    } finally {
      setUser(null)
      window.location.href = "/login"
    }
  }, [])

  return (
    <AuthContext.Provider value={{ user, loading, authEnabled, logout }}>
      {children}
    </AuthContext.Provider>
  )
}
