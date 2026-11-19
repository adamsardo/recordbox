# Contributing to Recordbox

Keep changes small enough to review independently. Preserve existing rekordbox
libraries and original audio files, and use generated or redistributable test
data instead of personal music, databases or credentials. Explain the behavior
being fixed, the relevant regression tests and any remaining limitations.

## Toolchain and dependencies

Install Rust through [rustup](https://rustup.rs). The root
`rust-toolchain.toml` selects **1.100.0**, rustfmt and Clippy for local commands,
matching CI. This is the tested toolchain, not an independently validated minimum
Rust version. Native prerequisites are in the
[build and packaging guide](docs/BUILD-AND-PACKAGING.md).

Run normal Cargo commands with `--locked`. Commit `app/Cargo.lock`; do not run
`cargo update` as part of an ordinary build. Review intentional dependency
changes separately, including the complete manifest/lockfile diff and native
compatibility. GitHub Actions must remain pinned to reviewed commit SHAs.

## Local checks

From the repository root:

```sh
cargo build --manifest-path app/Cargo.toml --release --locked
python packaging/run_tests.py --output reports
python -m unittest discover -s packaging -p 'test_*.py' -v
cargo fmt --manifest-path app/Cargo.toml --all -- --check
cargo clippy --manifest-path app/Cargo.toml --locked --all-targets -- --cap-lints warn
```

The application runner executes the complete serial suite and requires a
successful, complete result with no failed tests. It retains the raw output and
ignored-test count in `reports/`; filtered or interrupted runs do not qualify.
Targeted regressions are useful while developing, then run the complete suite.
Serial execution avoids existing shared test-fixture races.

Formatting and Clippy debt remains visible but advisory in CI. The commands
above expose that debt; a successful aggregate does not mean formatting is
clean or that there are no Clippy findings. Format touched Rust code and avoid
adding warnings. Keep broad mechanical cleanup separate from behavior changes;
after the baseline is clean, make formatting and
`cargo clippy --manifest-path app/Cargo.toml --locked --all-targets -- -D warnings`
blocking. Do not suppress findings just to make a check green.

## What CI covers

The full serial application suite and Clippy run on Linux. All four native
package jobs (macOS ARM64/Intel, Windows x64 and Linux x64) compile in release
mode, run backup security regressions and smoke the extracted application.
Smoke proves startup/rendering, not physical audio playback or USB-player
compatibility. Broader portable tests can be added to the native matrix as
their platform requirements are made explicit.

Ignored benchmarks, external format oracles and Metal-device tests have separate
requirements. The Metal test can return early without a device, so Cargo's `ok`
alone does not prove GPU execution. State which fixtures/devices were available
and preserve execution evidence. See the
[Metal check](docs/BUILD-AND-PACKAGING.md#build-checks-worth-keeping),
[format scope](docs/ANLZ-IMPORT-INTEGRITY.md) and
[opt-in CPU measurements](docs/PLAYBACK-PERFORMANCE.md).

The dependency gate compares cargo-audit output against the exact committed
[advisory baseline](.github/security/advisory-baseline.json). New, changed or
stale findings and audit execution errors fail. Remaining baseline entries are
unresolved findings, not a clean audit; remove them only after verifying the
resolved dependency graph and remediation. Keep the raw audit reports.

### Optional format oracles

Generated USB regressions run in the ordinary suite. The real-export
`export::usb_fix_test::oracle_` group is explicitly ignored and requires an
existing rekordcrate checkout or equivalent fixture directory containing
`data/`, selected with `RECORDBOX_TEST_ORACLES`. This group has no
developer-specific home-directory fallback. In a POSIX shell, run it with:

```sh
RECORDBOX_TEST_ORACLES=/path/to/rekordcrate cargo test --manifest-path app/Cargo.toml --locked --bin recordbox export::usb_fix_test::oracle_ -- --ignored --test-threads=1
```

On PowerShell, set `$env:RECORDBOX_TEST_ORACLES` before the same Cargo command.
An explicitly requested oracle test fails if its required fixture is missing.
Use fixtures you can legally obtain and share; these commands do not download
music or reference exports. The beatgrid oracle checks two fixed demo DAT/EXT
pairs from that fixture tree. An optional `RECORDBOX_TEST_ANLZ_PAIRS` TSV file
adds analysis pairs (`audio-path<TAB>extensionless-analysis-base`); the first
column is unused by this check, and relative analysis bases resolve from the
TSV directory. Use the specific oracle filter above; other ignored tests can
require different research fixtures.

## Repository settings and releases

CI does not change GitHub protection settings. The reviewed desired configuration
is [.github/rulesets/rust-main.json](.github/rulesets/rust-main.json). **Merging
that file does not activate it.** It targets the current default branch and
requires a PR, resolved review threads, an up-to-date branch, **Build and security
checks** from GitHub Actions (app ID `15368`) and **CodeQL** from GitHub Advanced
Security (app ID `57789`). It also blocks deletion/force pushes and CodeQL errors
or high/critical security findings. Zero approving reviews are required, allowing
a solo maintainer to merge their own PR after checks pass; no bypass actors are
configured. Up-to-date checks mean a PR may need rebasing after another merge.

A maintainer with repository administration access should review the JSON, then
update the existing rule in **Settings → Rules → Rulesets**. In `fantuz/recordbox`,
the existing `rust-branch-rules` ruleset is ID `27548091`; update it rather than
creating a duplicate. An API alternative is shown below; replace `OWNER/REPO`
and `RULESET_ID` with the target repository and its existing rule ID:

```sh
gh api repos/OWNER/REPO/rulesets
gh api --method PUT repos/OWNER/REPO/rulesets/RULESET_ID --input .github/rulesets/rust-main.json
```

Confirm the required check names and apps against an actual PR, then verify that
a missing/failed result prevents merging. A disabled ruleset enforces nothing,
and a code-scanning rule alone does not require the build aggregate. The JSON
omits Copilot and license-dependent code-quality rules; CodeQL must be enabled
and available for the target repository before activation.

Keep Dependabot alerts, security updates and CodeQL enabled in repository
settings. The committed Dependabot schedule and workflow do not enable those
settings themselves. Build jobs remain read-only; only the gated release
publisher receives narrowly scoped write permissions.

Follow the [release process](docs/BUILD-AND-PACKAGING.md#fork-release-process) for
version changes, notes, immutable tags and verified native packages. Disclose
actual test/security/platform limitations and keep published releases intact.
