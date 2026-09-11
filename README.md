# crap4ts

A Rust CLI that finds risky TypeScript functions by combining cyclomatic
complexity with test coverage. Inspired by
[crap4java](https://github.com/unclebob/crap4java) and
[crap4clj](https://github.com/unclebob/crap4clj).

```text
CRAP = CC² × (1 − coverage)³ + CC
```

Coverage is a fraction from 0 to 1. A function with complexity 4 scores 20
without coverage, 6 at 50% coverage, and 4 at 100% coverage. Lower is better.

## Install and run

Install stable Rust with [rustup](https://rustup.rs); a C compiler is also
required to build the bundled tree-sitter grammar. Then, from this repository:

```sh
cargo install --path . --locked
crap4ts --help
```

Run in a TypeScript project after generating LCOV coverage with your existing
test runner:

```sh
# Examples: use whichever runner is already installed in your project.
npx vitest run --coverage --coverage.reporter=lcov
# or:
npx jest --coverage --coverageReporters=lcov

crap4ts                                  # src + coverage/lcov.info
crap4ts src packages/shared --format json
crap4ts --root /path/to/project --lcov reports/lcov.info src
crap4ts --threshold 8 --require-coverage   # CI gate
```

Vitest requires its matching coverage provider package (such as
`@vitest/coverage-v8`). Configure the runner to include **all source files**,
including files never imported by tests, and to emit coverage mapped back to
the original TypeScript lines. crap4ts does not run tests, transpile code,
resolve source maps, or validate coverage freshness. Regenerate coverage after
source changes. It never deletes existing coverage.

Try the included illustrative fixture without Node.js:

```sh
cargo run --locked -- --root examples --lcov lcov.info src
```

The example's `classify` function has CC 3, coverage 50%, and CRAP 4.1
(unrounded: 4.125). Its `identity` function has CC 1, coverage 100%, and CRAP 1.

## Analysis rules

- Scans `.ts`, `.tsx`, `.mts`, and `.cts`; excludes `.d.ts`, `.d.mts`, and
  `.d.cts`. Directory traversal skips `node_modules`, `.git`, `dist`, `build`,
  `coverage`, and `target`. Symlinked child directories are not followed.
- Reports declarations, function expressions, arrows, generators, and methods
  (including constructors and accessors). Signatures without bodies are skipped.
  Top-level code outside functions is not scored. Test files are included if
  they are under a selected input path; choose production source paths explicitly.
- CC starts at 1, plus one for each `if`, loop (`for`, `for…in`, `for…of`,
  `while`, `do…while`), `catch`, ternary, switch `case` or `default`, and
  `&&` or `||`. `else`, `??`, optional chaining, and logical assignment operators
  do not add complexity. Parameter-default decisions count.
- Nested functions and classes do not inflate their enclosing function's CC;
  their functions are reported separately. Comments and string contents do not
  count. Parsing is syntactic, not TypeScript type checking. Parse errors fail
  the command rather than silently producing partial results.
- Names come from declarations or bindings when available; anonymous functions
  have line/column names. File and source range disambiguate repeated names,
  including getters/setters. Ranges are one-based and inclusive.

## Coverage and output

Coverage is **covered LCOV `DA` lines / reported `DA` lines** within the
function's inclusive declaration-to-end range. Duplicate records are merged;
any positive hit marks a line covered. `FN`, branch counters, and summary
counters are not used. This is line coverage, not branch/instruction coverage,
so scores need not match the Java tool's JaCoCo instruction-based scores.
Nested functions and multiple functions on one line can share coverage lines.

Relative source and LCOV paths resolve against `--root` (default: current
directory). Exact normalized paths take precedence; a unique path-component
suffix match supports reports from another build directory. Ambiguous matches
and functions with no reported lines have unknown coverage, never assumed zero.
Windows separators and percent-escaped `file://` paths are normalized.

Text reports show function, file/line, CC, coverage percent, and CRAP, rounded to
one decimal. JSON is an array of objects containing `file`, `name`, `start_line`,
`end_line`, `complexity`, `coverage` (fraction), and `crap` (unrounded).
Unknown values are `N/A` in text and `null` in JSON. Both formats sort by
descending score, unknowns last, then file and start line. Diagnostics go to stderr.

Exit statuses:

| Code | Meaning |
| --- | --- |
| 0 | Success (or help/version); threshold not exceeded |
| 1 | Invalid input/options, parse/read error, no source files, or missing required coverage |
| 2 | At least one known CRAP score **strictly exceeds** `--threshold` |

There is no threshold by default. Unknown scores do not exceed a threshold;
use `--require-coverage` to prevent missing coverage from passing CI. A missing
auto-detected report warns; an explicitly requested missing report fails.
Coverage errors take precedence over threshold failures.

## Development and tests

```sh
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

Tests cover the formula, AST decision rules and scope isolation, TypeScript/TSX
parsing, LCOV range/path matching and malformed input, and the actual CLI's
sorting, JSON, discovery, diagnostics, and exit-code boundaries. The example
LCOV file is hand-authored to make its scores reproducible, not a claim of
coverage from an executed TypeScript test suite.
