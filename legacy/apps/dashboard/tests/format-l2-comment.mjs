#!/usr/bin/env node
// format-l2-comment.mjs — Formats L2 structured results into a rich PR comment
//
// Generates a visually evident markdown comment with:
// - Clear pass/fail verdict banner
// - Per-page results with desktop/mobile viewport breakdown
// - Screenshot references
// - Issues and crystallized tests sections
//
// Usage:
//   node format-l2-comment.mjs /tmp/l2-result.json /tmp/l2-comment.md

import { readFileSync, writeFileSync } from "node:fs";

const [resultFile, outputFile] = process.argv.slice(2);
if (!resultFile || !outputFile) {
  console.error(
    "Usage: node format-l2-comment.mjs <result-json> <output-md>"
  );
  process.exit(1);
}

const data = JSON.parse(readFileSync(resultFile, "utf-8"));
const so = data.structured_output;

if (!so) {
  // Fallback: no structured output
  const raw = data.result || "No result found";
  writeFileSync(outputFile, `## L2: AI PR Testing\n\n${raw}\n`);
  process.exit(0);
}

const verdict = so.verdict || "UNKNOWN";
const summary = so.summary || "";
const pages = so.pages_tested || [];
const issues = so.issues || [];
const crystallized = so.crystallized || [];

const statusIcon = (s) =>
  s === "PASS" ? "\u2705" : s === "FAIL" ? "\u274C" : "\u23ED\uFE0F";
const verdictBanner =
  verdict === "PASS"
    ? `> **\u2705 VERDICT: PASS** — All pages verified at Desktop + Mobile`
    : `> **\u274C VERDICT: FAIL** — Issues found (see details below)`;

const lines = [];

lines.push(`## L2: AI PR Testing`);
lines.push(``);
lines.push(verdictBanner);
lines.push(``);
if (summary) {
  lines.push(summary);
  lines.push(``);
}

// Per-page results with viewport breakdown
if (pages.length > 0) {
  lines.push(`### Pages Tested`);
  lines.push(``);

  for (const page of pages) {
    const icon = statusIcon(page.status);
    lines.push(
      `<details${page.status === "FAIL" ? " open" : ""}>`,
    );
    lines.push(
      `<summary>${icon} <code>${page.route}</code> — ${page.status}${page.notes ? ` — ${page.notes}` : ""}</summary>`,
    );
    lines.push(``);

    const viewports = page.viewports || [];
    if (viewports.length > 0) {
      lines.push(`| Viewport | Status | Screenshot | Notes |`);
      lines.push(`|----------|--------|------------|-------|`);
      for (const vp of viewports) {
        const vpIcon = statusIcon(vp.status);
        const vpLabel = vp.name === "desktop" ? "\uD83D\uDDA5\uFE0F Desktop" : "\uD83D\uDCF1 Mobile";
        const screenshotNote = vp.screenshot
          ? `\`${vp.screenshot.split("/").pop()}\``
          : "—";
        lines.push(
          `| ${vpLabel} | ${vpIcon} ${vp.status} | ${screenshotNote} | ${vp.notes || "—"} |`,
        );
      }
    } else {
      lines.push(`_No viewport breakdown available_`);
    }

    lines.push(``);
    lines.push(`</details>`);
    lines.push(``);
  }
}

// Issues
if (issues.length > 0) {
  lines.push(`### Issues Found`);
  lines.push(``);
  for (const issue of issues) {
    lines.push(`- \u274C ${issue}`);
  }
  lines.push(``);
}

// Crystallized
if (crystallized.length > 0) {
  lines.push(`### Crystallized into L1`);
  lines.push(``);
  for (const item of crystallized) {
    lines.push(`- \u2728 ${item}`);
  }
  lines.push(``);
}

// Summary stats
const desktopCount = pages.reduce(
  (n, p) => n + (p.viewports || []).filter((v) => v.name === "desktop").length,
  0,
);
const mobileCount = pages.reduce(
  (n, p) => n + (p.viewports || []).filter((v) => v.name === "mobile").length,
  0,
);
lines.push(
  `> Tested **${pages.length}** page(s) — **${desktopCount}** desktop, **${mobileCount}** mobile viewport(s)`,
);
lines.push(``);

writeFileSync(outputFile, lines.join("\n"));
console.error(
  `[L2] Comment formatted: ${pages.length} pages, verdict=${verdict}`,
);
