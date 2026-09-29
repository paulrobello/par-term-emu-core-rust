# ENH-030: Constructor and property coverage in the stub generator and the API_REFERENCE checker

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-030]`.
> Closes the blind spot that let AUDIT DOC-073 (`StreamingConfig(kitty_file_media=…)` undocumented) pass a green 497/497 check.

## Goal

1. `scripts/generate_stubs.py` emits real constructor signatures instead of `__init__(self, *args: Any, **kwargs: Any)` for every class.
2. `scripts/check_api_reference.py` also checks documented constructors and `### Properties` lists against the stub.

## Current state

- `generate_stubs.py:14-16` says PyO3 does not expose `__text_signature__` for `#[new]` constructors. **That is no longer true**, verified 2026-09-28 against the 0.55.0 build:
  - `Terminal.__text_signature__ == '(cols, rows, scrollback=10000)'`
  - `PtyTerminal.__text_signature__ == '(cols, rows, scrollback=10000)'`
  - Under `make dev` (no streaming), `StreamingConfig` is a placeholder with `'()'`. The streaming build exposes the real signature.
  - PyO3 0.29 sets the class-level `__text_signature__` from `#[new]`/`#[pyo3(signature=…)]`.
- `_native.pyi` has 13 classes with the `*args/**kwargs` constructor (DOC-087).
- `check_api_reference.py` parses only `- `name(args)`` bullets under class sections (`METHOD_LINE`), so constructors and properties are unchecked.

## Implementation

1. `generate_stubs.py`: for each class, read `cls.__text_signature__`. When it is present and not `'()'` (or when the class truly takes no args), emit `def __init__(self, <params>) -> None: ...`, reusing the existing method-signature rendering function (params get `Any` annotations, defaults preserved). Fall back to `*args/**kwargs` only when it is absent. Update the module docstring's "Known limitations" to match.
2. The generator must run against the streaming build (`make dev-streaming`), as today (project memory `stub-regen-needs-dev-streaming`). Add an assertion that fails if `StreamingConfig.__text_signature__ == '()'`, with a message pointing at `make dev-streaming`.
3. `check_api_reference.py`:
   - **Constructor check**: under each `CLASS_SECTIONS` heading, find the first fenced code block or bullet of the form `ClassName(args)` and compare parameter names and default presence with the stub `__init__`. The same normalization as methods (DOC-064 rules) applies.
   - **Property check**: under a `### Properties` subheading within a class section, collect backticked names from bullets or table first-columns, and require each to be a stub property. Also require every stub property of that class to be documented. Add an allowlist constant for intentionally undocumented properties (start empty and add only with a comment).
   - Report one line per mismatch, as today.
4. Regenerate the stub (`make dev-streaming && make stubs`), then fix whatever the checker now reports in `docs/API_REFERENCE.md`. At minimum `StreamingConfig.kitty_file_media` (DOC-073) is expected. If DOC-073 already landed, the check is simply green.

## Files to touch

- `scripts/generate_stubs.py`, `scripts/check_api_reference.py`
- `python/par_term_emu_core_rust/_native.pyi` (regenerated)
- `docs/API_REFERENCE.md` (whatever the new checks flag)

## Verify

- `make dev-streaming && make stubs` produces a `_native.pyi` in which `grep -c 'def __init__(self, \*args' python/par_term_emu_core_rust/_native.pyi` is lower than today's 13. Report the number, and have every remaining one correspond to a class whose `__text_signature__` is absent.
- `make stub-check` passes (pyright on the stub, plus the extended checker).
- Negative test: temporarily delete the `kitty_file_media` property line from API_REFERENCE. The checker exits 1 naming it. Restore it afterwards.
- `make checkall` is green.

## Rollback

Revert the two scripts and regenerate the stub. Docs edits made to satisfy the new checks remain correct on their own.
