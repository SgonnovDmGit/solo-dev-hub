# User-only addon: `docs/benchmarks.md` spec

**Not bundled in the app's `claude.md.global.tmpl`** — this is a personal convention of the user, not enforced/parsed by Solo Dev Hub. Copy-paste the block below into your `~/.claude/CLAUDE.md` manually if you want it as a global rule.

Reason for exclusion: benchmarks are language-specific and not every project has them. Keeping the app's global template lean; personal standards live here as opt-in addons.

---

## Block to paste

```markdown
# `docs/benchmarks.md`

## Target metrics

Table with columns `Metric | Target | Criticality`. Targets are project-specific — fill with actual SLOs. Example:

| Metric | Target | Criticality |
|---|---|:-:|
| ... | ... | critical / major / medium / minor |

## Rules

- Run benchmarks before every release and record the result.
- Regression > 10% flags the result for review; > 15% blocks the release.
- Use a suitable tool for the language (`benchstat` for Go, `criterion` for Rust, etc.).
- Benchmark functions live next to the code they test (language-specific convention).

## Results

### v0.1.0 — YYYY-MM-DD

Example (Go `go test -bench`):

​```
BenchmarkAuth-8          10000       45320 ns/op      2048 B/op      32 allocs/op
BenchmarkGetProfile-8    20000       23100 ns/op      1024 B/op      16 allocs/op
​```

Conclusion: all within targets.
```
