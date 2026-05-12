#!/usr/bin/env python3
"""plan-dag-render — JSON DAG IR → ASCII / box-art / mermaid / DOT."""

import argparse
import json
import subprocess
import sys
from pathlib import Path

STATUS_MARKER = {"done": " ✓", "in_progress": " …", "open": ""}
VALID_STATUS = set(STATUS_MARKER.keys())
VALID_SOURCES = {"sub-issue", "depends-on", "pr-link", "closes", "part-of"}


def validate(ir):
    errors = []
    nodes = ir.get("nodes", [])
    if not isinstance(nodes, list) or not nodes:
        return ["ir.nodes is missing or empty"]
    ids = set()
    for i, n in enumerate(nodes):
        if "id" not in n:
            errors.append(f"nodes[{i}] missing id")
            continue
        nid = str(n["id"])
        if nid in ids:
            errors.append(f"duplicate node id: {nid}")
        ids.add(nid)
        status = n.get("status", "open")
        if status not in VALID_STATUS:
            errors.append(f"node #{nid}: invalid status {status!r}")
        if not n.get("label"):
            errors.append(f"node #{nid}: missing label")
    ids.add("close")
    for i, e in enumerate(ir.get("edges", [])):
        for end in ("from", "to"):
            if end not in e:
                errors.append(f"edges[{i}] missing {end}")
            elif str(e[end]) not in ids:
                errors.append(f"edges[{i}].{end}={e[end]!r} not in declared nodes")
        if not e.get("source"):
            errors.append(f"edges[{i}] missing source (citation required)")
        elif e["source"] not in VALID_SOURCES:
            errors.append(
                f"edges[{i}].source={e['source']!r} not in {sorted(VALID_SOURCES)}"
            )
    return errors


def render_dot(ir):
    lines = ["digraph plan {", "  rankdir=LR;", "  node [shape=box];", ""]
    for n in ir["nodes"]:
        nid = str(n["id"])
        marker = STATUS_MARKER[n.get("status", "open")]
        label = f'#{nid} {n["label"]}{marker}'
        lines.append(f'  "{nid}" [label="{label}"];')
    if ir.get("close"):
        lines.append(f'  "close" [label="close #{ir["close"]}"];')
    lines.append("")
    for e in ir.get("edges", []):
        lines.append(f'  "{e["from"]}" -> "{e["to"]}";')
    lines.append("}")
    return "\n".join(lines)


def render_mermaid(ir):
    lines = ["graph LR"]
    for n in ir["nodes"]:
        nid = str(n["id"])
        status = n.get("status", "open")
        marker = STATUS_MARKER[status]
        cls = {"done": ":::done", "in_progress": ":::wip", "open": ""}[status]
        lines.append(f'  N{nid}[#{nid} {n["label"]}{marker}]{cls}')
    if ir.get("close"):
        lines.append(f'  CLOSE[close #{ir["close"]}]')
    for e in ir.get("edges", []):
        a = "CLOSE" if e["from"] == "close" else f'N{e["from"]}'
        b = "CLOSE" if e["to"] == "close" else f'N{e["to"]}'
        lines.append(f"  {a} --> {b}")
    lines.append("  classDef done fill:#cfc,stroke:#3a3")
    lines.append("  classDef wip fill:#ffd,stroke:#aa3")
    return "\n".join(lines)


def render_via_graph_easy(dot, mode):
    try:
        res = subprocess.run(
            ["graph-easy", "--from=graphviz", f"--as={mode}"],
            input=dot, capture_output=True, text=True, timeout=10,
        )
    except FileNotFoundError:
        sys.exit("graph-easy not on PATH. Install: cpan -T -i Graph::Easy")
    if res.returncode != 0:
        sys.stderr.write(res.stderr)
        sys.exit(res.returncode)
    return res.stdout


def main():
    ap = argparse.ArgumentParser(description="Render plan DAG IR.")
    ap.add_argument("ir", help="path to JSON IR, or '-' for stdin")
    ap.add_argument(
        "--as", dest="target", default="boxart",
        choices=["boxart", "ascii", "mermaid", "dot"],
        help="output target (default: boxart)",
    )
    args = ap.parse_args()

    text = sys.stdin.read() if args.ir == "-" else Path(args.ir).read_text()
    try:
        ir = json.loads(text)
    except json.JSONDecodeError as e:
        sys.exit(f"invalid JSON: {e}")

    errors = validate(ir)
    if errors:
        sys.stderr.write("IR validation failed:\n")
        for err in errors:
            sys.stderr.write(f"  - {err}\n")
        sys.exit(1)

    if args.target == "mermaid":
        print(render_mermaid(ir))
    elif args.target == "dot":
        print(render_dot(ir))
    else:
        sys.stdout.write(render_via_graph_easy(render_dot(ir), args.target))

    cp = ir.get("critical_path")
    if cp and args.target in ("boxart", "ascii"):
        path = " → ".join("close" if n == "close" else f"#{n}" for n in cp)
        print(f"\nCritical path: {path}")


if __name__ == "__main__":
    main()
