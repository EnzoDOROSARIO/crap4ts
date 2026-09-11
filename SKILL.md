---
name: crap4ts
description: "Calculates cyclomatic complexity and CRAP scores for TypeScript and TSX functions by combining AST analysis with LCOV test coverage, generating sorted reports that identify high-risk under-tested code. Use when the user asks for a CRAP report, cyclomatic complexity analysis, or code quality metrics on a TypeScript project."
---

# crap4ts — CRAP Metric for TypeScript

Computes the **CRAP** (Change Risk Anti-Pattern) score for TypeScript functions,
arrows, and methods. CRAP combines cyclomatic complexity with test coverage to
identify functions that are both complex and under-tested.

## Setup

If `crap4ts` is already on PATH, use it. Otherwise, install it from a local
checkout of the crap4ts repository (not the TypeScript project being analyzed):

```bash
cargo install --path /path/to/crap4ts --locked
crap4ts --help
```

Building requires stable Rust/Cargo and a C compiler for the TypeScript parser.
The installed binary does not need Node.js; generating test coverage usually does.

Inspect the target project's package scripts and test configuration. Use its
existing test runner and package manager to generate an LCOV report. For projects
already using Vitest or Jest, the equivalent npm commands are:

```bash
# Vitest (requires its matching coverage provider, e.g. @vitest/coverage-v8)
npx vitest run --coverage --coverage.reporter=lcov

# Or Jest
npx jest --coverage --coverageReporters=lcov
```

Do not switch test runners or install the latest runner over an existing version.
Configure coverage to include the selected production sources, including files
never imported by tests, and map coverage back to original TypeScript lines.
Verify tests finish successfully and the LCOV report exists before analyzing it.

## Usage

Run from the target TypeScript project directory:

```bash
# Analyze all source files under src/ using coverage/lcov.info
crap4ts

# Analyze specific files or directories (paths, not substring filters)
crap4ts src/combat.ts src/movement

# Analyze another project with an explicit report
crap4ts --root /path/to/project --lcov reports/lcov.info src

# Produce machine-readable results
crap4ts src --format json

# Fail CI for scores above 8 or any missing function coverage
crap4ts --threshold 8 --require-coverage
```

crap4ts **does not run tests or delete coverage reports**. Regenerate coverage
after source changes; do not present stale coverage as current. Relative input
paths and LCOV source paths resolve against `--root`, defaulting to the current
directory. Choose production paths explicitly if tests live alongside sources.

### Output

A table sorted by CRAP score, worst first. This example uses the repository's
illustrative fixture (`cargo run --locked -- --root examples --lcov lcov.info src`):

```text
CRAP Report
===========
Function                       File:line                                  CC   Cov%     CRAP
classify                       src/sample.ts:1                             3   50.0      4.1
identity                       src/sample.ts:7                             1  100.0      1.0
```

JSON reports contain `file`, `name`, `start_line`, `end_line`, `complexity`,
`coverage` (a fraction from 0 to 1), and `crap` (unrounded). Unknown coverage and
scores appear as `N/A` in text or `null` in JSON and sort last.

## Interpreting Scores

| CRAP Score | Guidance |
|-----------|----------|
| 1–5       | Lower risk by this metric |
| >5–30     | Consider refactoring or adding tests |
| >30       | Prioritize investigation of complexity and coverage |
| N/A       | Coverage is unknown; do not treat it as zero or as passing |

These bands are heuristics, not proof of test quality. Inspect both CC and
coverage: even fully covered code can score highly when it is very complex.
Report the highest-risk functions with their file/line, CC, coverage, and score.
Suggest focused tests or simplifications; do not change code merely to lower a
number unless the user requests remediation.

## How It Works

1. Finds `.ts`, `.tsx`, `.mts`, and `.cts` files, excluding declaration files and
   common dependency/build directories.
2. Parses function-like constructs with tree-sitter and records inclusive line
   ranges. Signatures without bodies and top-level non-function code are not scored.
3. Starts CC at 1, adding one for each `if`, loop, `catch`, ternary, switch `case`
   or `default`, and `&&` or `||`. Parameter-default decisions count; nested
   function/class decisions do not inflate enclosing functions. `??`, optional
   chaining, and logical assignments do not add complexity.
4. Reads LCOV `DA` lines in each function's range: covered reported lines divided
   by total reported lines. Exact normalized paths take precedence over unique
   component-suffix matches; ambiguous or missing coverage remains unknown.
5. Applies `CC² × (1 - coverage)³ + CC` and prints sorted results.

Coverage is line-based, not branch or instruction coverage. Nested functions or
functions sharing a line can share coverage data. Parsing is not type checking.

## Troubleshooting

- **Coverage generation fails:** Run the project's existing test command on its
  own; check runner/provider versions and source inclusion. Report failures rather
  than presenting an old report as fresh.
- **Functions show N/A:** Check `--root`, `--lcov`, LCOV `SF` paths, original-source
  line mapping, and whether the runner includes unimported files. N/A also means
  no tracked lines in a function or ambiguous path matching.
- **Coverage shows 0%:** Matching lines exist but all have zero hits. Confirm tests
  exercise those functions; this differs from missing coverage.
- **No source files or parse errors:** Check selected paths and the diagnostic's
  file/line. Declaration-only inputs are excluded; parse errors fail the command.
- **Nonzero exit:** `1` means invalid input, a read/parse error, no source files,
  or missing required coverage. `2` means a known score strictly exceeds
  `--threshold`; equality passes. Unknown scores do not trigger the threshold,
  so use `--require-coverage` for CI. There is no threshold by default.
