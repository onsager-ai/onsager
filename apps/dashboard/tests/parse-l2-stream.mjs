#!/usr/bin/env node
// parse-l2-stream.mjs — Parses Claude Code `--output-format stream-json` NDJSON
// and prints human-readable execution logs to stderr while capturing the final
// result JSON to a file.
//
// Usage:
//   claude -p "..." --output-format stream-json | node parse-l2-stream.mjs /tmp/l2-result.json

import { createInterface } from "node:readline";
import { writeFileSync } from "node:fs";

const outFile = process.argv[2];
if (!outFile) {
  console.error("Usage: node parse-l2-stream.mjs <output-file>");
  process.exit(1);
}

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });

let resultEvent = null;
let turnNumber = 0;

for await (const line of rl) {
  if (!line.trim()) continue;

  let event;
  try {
    event = JSON.parse(line);
  } catch {
    // Not valid JSON — print as-is for debugging
    console.error(line);
    continue;
  }

  const type = event.type;

  if (type === "system" && event.subtype === "init") {
    console.error(`[L2] Session started (model: ${event.model || "unknown"})`);
    continue;
  }

  if (type === "assistant") {
    turnNumber++;
    const content = event.message?.content || [];
    for (const block of content) {
      if (block.type === "text" && block.text) {
        console.error(`\n[Turn ${turnNumber}] ${block.text}`);
      }
      if (block.type === "tool_use") {
        const inputPreview = JSON.stringify(block.input || {}).slice(0, 200);
        console.error(`[Turn ${turnNumber}] Tool: ${block.name}(${inputPreview})`);
      }
    }
    continue;
  }

  if (type === "user") {
    // Tool results — log a compact summary
    const content = event.message?.content || [];
    for (const block of content) {
      if (block.type === "tool_result") {
        const status = block.is_error ? "ERROR" : "ok";
        const preview =
          typeof block.content === "string"
            ? block.content.slice(0, 150)
            : JSON.stringify(block.content || "").slice(0, 150);
        console.error(`  -> [${status}] ${preview}`);
      }
    }
    continue;
  }

  if (type === "result") {
    resultEvent = event;
    console.error(`\n[L2] Finished — cost: $${event.cost_usd?.toFixed(4) || "?"}, turns: ${event.num_turns || "?"}`);
    continue;
  }
}

// Write the result to the output file
if (resultEvent) {
  writeFileSync(outFile, JSON.stringify(resultEvent, null, 2));
  console.error(`[L2] Result written to ${outFile}`);
} else {
  console.error("[L2] WARNING: No result event received");
  writeFileSync(outFile, JSON.stringify({ error: "No result event in stream" }));
  process.exit(1);
}
