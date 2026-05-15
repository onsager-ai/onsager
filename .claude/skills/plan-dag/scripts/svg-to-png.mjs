#!/usr/bin/env node
// svg-to-png — rasterise a graphviz SVG to PNG via headless Chromium.
//
// Usage:
//   dot -Tsvg | node svg-to-png.mjs <out-path>
//
// Why not `dot -Tpng`? Going through the browser gives sharper text and
// correct anti-aliasing at deviceScaleFactor=2, which matters when the
// PNG is shown inline in chat surfaces.

import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const out = process.argv[2];
if (!out) {
  process.stderr.write('usage: svg-to-png.mjs <out-path>\n');
  process.exit(2);
}

const svg = readFileSync(0, 'utf8');
if (!svg.trim()) {
  process.stderr.write('svg-to-png: no SVG on stdin\n');
  process.exit(2);
}

let chromium;
try {
  ({ chromium } = await import('playwright'));
} catch {
  try {
    const req = createRequire(import.meta.url);
    ({ chromium } = req('/opt/node22/lib/node_modules/playwright/index.js'));
  } catch (e) {
    process.stderr.write(
      'svg-to-png: cannot load Playwright. Install with `npm i -g playwright` '
      + 'and `npx playwright install chromium`.\n'
    );
    process.exit(2);
  }
}

const html = `<!doctype html><html><head><meta charset="utf-8"><style>
  html,body{margin:0;padding:0;background:#fff}
  body{padding:24px;display:inline-block}
  svg{display:block}
</style></head><body>${svg}</body></html>`;

const browser = await chromium.launch();
try {
  const ctx = await browser.newContext({ deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  await page.setContent(html, { waitUntil: 'load' });
  const box = await page.locator('body').boundingBox();
  await page.setViewportSize({
    width: Math.ceil(box.width),
    height: Math.ceil(box.height),
  });
  await page.screenshot({ path: out, omitBackground: false, fullPage: true });
} finally {
  await browser.close();
}

process.stderr.write(`svg-to-png: wrote ${out}\n`);
