// Preview-env UI screenshots (ADR 0030 / #655). Drives the live preview
// deploy with a dev-login cookie and captures the fleet evidence pages at
// desktop + mobile. The CI job seeds a connected machine + a completed run
// first, then runs this; output PNGs are published inline on the PR.
//
// Env:
//   PREVIEW_URL  — the pr-<N> base URL (required)
//   RUN_ID       — a completed run to screenshot (optional; skipped if unset)
//   OUT_DIR      — output directory (default: ./shots)
import { chromium } from 'playwright'
import { mkdir } from 'node:fs/promises'

const BASE = process.env.PREVIEW_URL
const RUN_ID = process.env.RUN_ID || ''
const OUT = process.env.OUT_DIR || 'shots'
if (!BASE) {
  console.error('PREVIEW_URL is required')
  process.exit(1)
}

// Dev-login (previews enable it) and lift the session cookie for the browser.
const res = await fetch(`${BASE}/api/auth/dev-login`, { method: 'POST' })
if (!res.ok) {
  console.error(`dev-login failed: ${res.status}`)
  process.exit(1)
}
const setCookie = res.headers.get('set-cookie') || ''
const m = setCookie.match(/onsager_session=([^;]+)/)
if (!m) {
  console.error('no onsager_session cookie in dev-login response')
  process.exit(1)
}
const sessionValue = m[1]
const host = new URL(BASE).hostname

const pages = [
  ['workflows', '/workflows'],
  ['machines', '/machines'],
  ['artifacts', '/artifacts'],
  ['activity', '/activity'],
]
if (RUN_ID) pages.push(['run', `/runs/${RUN_ID}`])

const viewports = [
  ['desktop', 1280, 800],
  ['mobile', 390, 844],
]

await mkdir(OUT, { recursive: true })
const browser = await chromium.launch()
try {
  for (const [label, width, height] of viewports) {
    const ctx = await browser.newContext({ viewport: { width, height } })
    await ctx.addCookies([
      {
        name: 'onsager_session',
        value: sessionValue,
        domain: host,
        path: '/',
        httpOnly: true,
        secure: BASE.startsWith('https'),
        sameSite: 'Lax',
      },
    ])
    const page = await ctx.newPage()
    for (const [name, path] of pages) {
      await page.goto(`${BASE}${path}`, { waitUntil: 'networkidle', timeout: 30000 })
      // Let react-query settle (lists poll on an interval).
      await page.waitForTimeout(2000)
      const file = `${OUT}/${label}-${name}.png`
      await page.screenshot({ path: file, fullPage: true })
      console.log(`captured ${file}`)
    }
    await ctx.close()
  }
} finally {
  await browser.close()
}
