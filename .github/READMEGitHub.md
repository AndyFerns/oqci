# `.github/` — CI/CD Configuration

GitHub-specific automation. Currently a single workflow.

## `workflows/docs.yml`

Builds the [mdBook](https://rust-lang.github.io/mdBook/) documentation site
from [`../docs/`](../docs/) and publishes it to GitHub Pages.

- **Triggers** on a push to `master`/`main` that touches `docs/**` or the
  workflow file itself, and can also be run manually
  (`workflow_dispatch`). Rust source changes do **not** trigger it.
- Installs a pinned `mdbook` version, runs `mdbook build docs`, and deploys
  `docs/book` via the standard `actions/{configure-pages,upload-pages-artifact,deploy-pages}` sequence.

## What's not here yet

There is currently **no CI workflow that runs the Rust or Python test
suites** (`cargo test`/`clippy`/`fmt`, or `pytest`) — those checks are run
locally via [`../scripts/build.sh`](../scripts/build.sh) /
[`build.bat`](../scripts/build.bat), not automatically on push or PR. Per
`final-deliverables-spec.md` §26 (CI/CD Deliverable), that gate is expected
eventually; it just doesn't exist yet. Do not assume a passing docs deploy
implies the code itself was checked.
