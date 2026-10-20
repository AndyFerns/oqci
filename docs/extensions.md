# Extensions: adding to OQCI without modifying it

`final-deliverables-spec.md` §18 asks that "new components can be added
without modifying the compiler core". This page describes how, as of `0.6.0`.
The implementation is [`src/extension.rs`](../src/extension.rs). The
demonstration is [`examples/plugin-demo/`](../examples/plugin-demo/).

## Source-level, process-local — on purpose

§18.4 allows a dynamic plugin mechanism such as `libloading`, but only once a
safety, versioning and ABI contract is specified. It also forbids claiming
binary compatibility across Rust compiler versions. No such contract exists,
so OQCI takes the alternative §18.4 names: a **source-level** API.

An extension is an ordinary crate that depends on `oqci`. It implements public
traits and registers them on a `Registry`, and a compilation sees them through
`CompilerConfig::extensions`.

- The shipped `oqci` binary cannot load extensions.
- An extension crate builds its own binary with `oqci::cli::run_with(registry)`.
- No Python registration API is provided. The SDK reaches built-ins and
  runtime target files only.

## What can be added

| Component | Trait | Registered with | Spec |
|---|---|---|---|
| A source language | `extension::FrontendPlugin` (`id`, `description`, `file_extensions`, `parse`) | `Registry::register_frontend` | §18.1 |
| An optimization or analysis pass | `pass::Pass` (`id`, `description`, `run`) | `Registry::register_pass(origin, placement, pass)` | §18.2 |
| A backend | `backend::Backend` (profile, cost model, lowering, preparation, execution) | `Registry::register_backend` | §18.3 |
| A device described in a file | a serialized `BasisProfile` | `Registry::register_target_json` | §9.2 |
| A lint rule | `analysis::lint::LintRule` | `Registry::register_lint_rule` | §13.5 |

### Frontends

A frontend follows the same contract as every built-in one
([`adapters.md`](adapters.md)):

- it builds only through `CircuitBuilder`, so invariants I1–I8 are checked;
- it resolves gate names through `frontend::map_gate`;
- it reports what it cannot represent as `FrontendError::Unsupported` rather
  than approximating it.

`compile_with_frontend(source, name, frontend_id, config)` reads with any
registered frontend. The CLI selects one with `--frontend ID`, or by a file
extension the frontend claims.

### Passes

`PassPlacement` decides where an extension pass goes in the default pipeline:

- `BeforeFirst(id)` or `AfterLast(id)`, relative to an existing pass;
- `End`;
- the default is `BeforeFirst("schedule")`, so the read-only scheduler
  measures the extension's output.

An extension pass builds its result through the public `CircuitBuilder` and
reports failures with `PassError::from_ir`.

Under `--verify`, extension passes are verified exactly like built-in ones:
the in-compiler checker sees every transformation through
`PassManager::run_observed`.

### Backends and target files

Registration runs the same checks every built-in backend is tested against:

- the backend's cost model must be the one its profile names;
- its decomposition rules must reach its own basis (`RuleSet::new`).

`backend::from_target_json(json, kind, id)` loads a profile as an `ibm` or
`simulator` backend. The kind defaults to the profile's `backend_id`. This is
how a real IBM device reaches the compiler (see [`hardware.md`](hardware.md)).

## What registration refuses

All of these are refused at registration time, not when the component is
first used:

- **An empty id.**
- **An id already taken**, by a built-in or another extension. Two passes
  sharing an id would make `--disable` apply to both. A target file named
  `simulator` would silently change what that id means.
- **A backend whose cost model or rule set is inconsistent.**
- **A pass placed relative to a pass that does not exist.**

## Provenance

Every extension that took part in a compilation is recorded in
`CompilationArtifacts::extensions` and in the executable's
`Provenance::extensions`, as `crate@version kind:id`.

- This covers the frontend that read the source, every extension pass that
  ran, and the backend.
- A disabled extension pass is not listed.
- The field is omitted when empty, so a compilation without extensions
  produces exactly the record it always did.

A result therefore can never hide that part of it came from outside the core.

## The demonstration

[`examples/plugin-demo/`](../examples/plugin-demo/) adds three extensions
through the public API only:

- the pass `hzh-to-x`;
- the frontend `qtoy`, a line-based language;
- the backend `demo-ring`, a synthetic CZ-native ring.

`tests/no_core_modification.rs` then checks, for a `qtoy` program compiled end
to end:

- the extension pass fires;
- the circuit lowers legally onto the extension backend;
- the in-compiler checker verifies every transformation, the extension pass
  included;
- provenance names the three extensions with their crate version.

The crate links against `oqci` like any third party, so it structurally
cannot reach a crate-private item. This is the demonstration Definition of
Done item 9 asks for.

```bash
cargo run -p oqci-plugin-demo -- lower my.qtoy --backend demo-ring --verify
```
