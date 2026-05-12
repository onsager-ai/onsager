# Developer setup

Notes on tooling that isn't required to build or run Onsager but is useful for day-to-day dev workflows. Pairs with the user-facing setup in `README.md` and the contributor-facing rules in `CLAUDE.md`.

## DAG rendering

The `plan-dag` skill (`.claude/skills/plan-dag/SKILL.md`) and `scripts/plan-dag-render.py` use `graph-easy` to lay out DAGs from a JSON IR. Install once:

```
cpan -T -i Graph::Easy  # pure-Perl, no system deps
```

On Debian/Ubuntu, `apt install libgraph-easy-perl` is an equivalent path.
