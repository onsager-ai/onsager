import path from "path"
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// VITE_HMR_CLIENT_PORT lets the per-slot dev stack (#194) aim the
// browser-side HMR WebSocket at the slot's edge port (e.g. 9010) when
// Vite is reached through Caddy. Falls back to the default Vite behavior
// for the legacy slot-0 / `just dev` flow.
const hmrClientPort = process.env.VITE_HMR_CLIENT_PORT
  ? Number(process.env.VITE_HMR_CLIENT_PORT)
  : undefined

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    host: process.env.VITE_HOST ?? 'localhost',
    hmr: hmrClientPort
      ? { clientPort: hmrClientPort }
      : undefined,
    proxy: {
      '/api': 'http://localhost:3000',
      '/agent': {
        target: 'ws://localhost:3000',
        ws: true,
      },
    },
  },
  build: {
    // Stable vendor chunks — these libs rarely change, so splitting them
    // lets the browser keep them cached across deploys while the app
    // entry invalidates on each release.
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes('node_modules')) return undefined
          if (/\/(react|react-dom|react-router|react-router-dom|scheduler)\//.test(id)) {
            return 'react-vendor'
          }
          if (id.includes('@tanstack/react-query')) {
            return 'query-vendor'
          }
          return undefined
        },
      },
    },
  },
})
