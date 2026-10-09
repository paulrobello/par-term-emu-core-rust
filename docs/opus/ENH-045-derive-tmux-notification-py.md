# ENH-045: Generate `PyTmuxNotification::from` with a declarative macro

> Filed from the 2026-10-08 /opus-audit enhancement pass (cycle `audit-2026-10-08`). Board card: `[ENH-045]`.
> Sequencing:
> - Follows **ENH-041** (done: `empty(kind)` plus struct-update, and the golden test) and the **ARC-006** pattern (generated streaming conversions). Independent of every open card.
> - **Judge before scheduling.** ENH-041 already captured most of the value. See "Remaining value" in Current state. If the measured numbers below do not justify a macro, close this card as won't-do and record the reason. The plan stays valid if a later variant burst changes that.

**Priority**: low · **Estimate**: M

## Goal

`impl From<&TmuxNotification> for PyTmuxNotification` still tops parsight's hotspot list. Replace its 37 hand-written struct-update arms with one `macro_rules!` table: one line per variant, mapping each variant field to a `PyTmuxNotification` field with a conversion mode. The goals are:
- the Python surface stays byte-identical, proven by the existing golden test;
- the wildcard-free exhaustiveness stays, so a new `TmuxNotification` variant is still a compile error in the bindings;
- adding a variant costs one table line.

Use a **local declarative macro** in `notification.rs`, not a proc-macro derive in `derive/`. The reasons are under "Why not a derive".

## Current state

All anchors are at HEAD `2cf0957`.

- **The converter after ENH-041** (`src/python_bindings/types/notification.rs`):
  - `PyTmuxNotification` has 19 fields (`:11-71`): `notification_type` plus 18 `Option`s.
  - `empty(kind)` is `:132-158` (22 lines of field list).
  - `From<&TmuxNotification>` spans **`:161-399` (239 lines)**. It has **37 arms**, one per variant, and every arm already ends in `..Self::empty(kind)`. The arms hold **61** field-assignment lines in total, and no arm writes a `None`. `kind` comes from `notif.notification_type()`, the single kind-string table.
  - `From<TmuxNotification>` (owned) is `:401-405` and delegates.
  - Tests (`:545-809`):
    - `GOLDEN` is a 37-line rendering of every variant over all 19 fields;
    - `all_variants()` builds one value of each variant with distinct field values;
    - `variants_are_exhaustively_converted` (`:754`) is a wildcard-free match over every variant;
    - `golden_matches_pre_refactor_output`.
- **The mapping is mostly name-identical.** Variant fields map to same-named Py fields, with these exceptions, all of which the macro must express:
  - **Renames:**
    - `AgentStateChanged`: `agent` → `name`, `state` → `value`;
    - `AgentReleased` and `AgentTelemetryChanged`: `agent` → `name`;
    - `PaneTitleChanged`: `title` → `name`;
    - `SubscriptionChanged`: `name` → `subscription_name`;
    - `Unknown`: `line` → `raw_line`.
  - **Copy fields:** `timestamp`, `command_number` (Begin, End, Error) and `delay_ms` (ExtendedOutput) use `Some(*x)`.
  - **Optional pass-through:** `ClientLeft`'s `session_id` and `window_id` are already `Option<String>` (`x.clone()`, not `Some`).
  - **Empty string becomes `None`:** `WindowAdd`'s layout triple (`(!x.is_empty()).then(|| x.clone())`).
  - **Derived:** `PaneExited` sets `name` to `exit_code.map(|c| c.to_string())` (deprecated) and `exit_code` to `*exit_code`.
  - **Payload-less:** `SessionsChanged`, `WorkspacesChanged` and `Exit`.
- **Hotspot numbers:**
  - parsight `audit_repository hotspots` (14-day window) puts `notification::PyTmuxNotification::from` at #1 with complexity **38**, churn **244**, score 9272. `tests::variants_are_exhaustively_converted` is #2 (CC 39, churn 125).
  - parsight churn counts save-level episodes. `git log --since=2026-09-24 -- src/python_bindings/types/notification.rs` shows **10 commits**, mostly variant additions such as `ffb7c41` (`%continue` pane id), `b2266a7` (client lifecycle) and `78c0c62` (workspaces).
  - The CC of 38 **is the exhaustive match**, one branch per variant. ENH-041 kept it on purpose ("Cyclomatic complexity will not drop much … That is the intended safety property, not debt"). A macro-generated match keeps the same branches. A parser that scores the invocation may report a lower number, but that is a measurement artefact, not a real reduction.
- **Remaining value**, the basis for judging the card:
  - Today a new variant costs one arm of about 4-8 lines in `from`, plus a `GOLDEN` line, an `all_variants` entry and an exhaustiveness-test line.
  - The macro makes the `from` part one line, cutting `from` from 239 lines to an estimated 70-80: a table of about 45 lines plus a macro of about 25. No behaviour changes.
  - The test-side lines (3 per variant) stay, because they are the safety net.
  - Net saving: about 160 lines once, and about 4-7 lines per future variant.
- **Why not a derive** (the ARC-006/`PyDictConvert` route):
  - `TmuxNotification` is defined in `crates/par-term-emu-core/src/tmux_control.rs`. `par-term-emu-core`'s `[dependencies]` (`crates/par-term-emu-core/Cargo.toml:21+`) does **not** include `par-term-emu-derive`, and the core crate is published separately.
  - A `#[derive]` with `#[py(...)]` attributes on the enum would add a proc-macro dependency to a published crate and write Python field names into the parser crate, which is the wrong direction for the boundary.
  - The existing derives (`derive/src/lib.rs`: `pyo3_get_all` `:23`, `PyDictConvert` `:90`, `ProtoConvert` `:426`) all annotate types owned by the crate that uses them.
  - A `macro_rules!` in the bindings file needs no new dependency and **does not change `derive/`**. So `derive/Cargo.toml` (`version = "0.47.0"`) and the root's `par-term-emu-derive = { …, version = "0.47.0" }` pin (`Cargo.toml:62`) stay untouched, and `make derive-version-check` (CLAUDE.md "Derive crate exception") needs no bump.
  - If an implementer still chooses a proc-macro in `derive/`, they **must** bump `derive/Cargo.toml`'s version and the root `Cargo.toml:62` pin to the same new version in the same commit, and `make derive-version-check` must pass. This plan does not take that route.

## Implementation

1. **Baseline.** Run `cargo test --lib --no-default-features --features python-test notification` and confirm `golden_matches_pre_refactor_output` and `variants_are_exhaustively_converted` pass before any edit. Do **not** edit `GOLDEN`, `all_variants()` or the exhaustiveness test in this card. They are the oracle.
2. **The macro** (`src/python_bindings/types/notification.rs`, directly above the `From` impl). Field modes:
   - `own` → `Some(x.clone())`
   - `copy` → `Some(*x)`
   - `opt` → `x.clone()`
   - `nonempty` → `(!x.is_empty()).then(|| x.clone())`
   ```rust
   /// One table row per `TmuxNotification` variant: `Variant { src => dst: mode, … }`.
   /// Every unlisted Py field stays None via `empty(kind)`. Trailing `raw:` arms are
   /// pasted into the same match verbatim for conversions no mode expresses. The
   /// generated match has no wildcard, so a new variant fails to compile here.
   macro_rules! tmux_to_py {
       (@val own, $x:ident) => { Some($x.clone()) };
       (@val copy, $x:ident) => { Some(*$x) };
       (@val opt, $x:ident) => { $x.clone() };
       (@val nonempty, $x:ident) => { (!$x.is_empty()).then(|| $x.clone()) };
       // Rows sit inside `[...]`: a bare repetition of `$variant:ident` followed
       // by the `raw` keyword would be a macro_rules local ambiguity.
       ($notif:expr, $kind:expr;
        [ $( $variant:ident $({ $($src:ident => $dst:ident : $mode:ident),* $(,)? })? ;)* ]
        raw: { $($raw:tt)* }) => {
           match $notif {
               $( TmuxNotification::$variant $({ $($src),* })? => PyTmuxNotification {
                   $($( $dst: tmux_to_py!(@val $mode, $src), )*)?
                   ..PyTmuxNotification::empty($kind)
               }, )*
               $($raw)*
           }
       };
   }
   ```
   Payload-less variants (`SessionsChanged`, `WorkspacesChanged`, `Exit`) are written as bare `Variant;`, which expands to a unit pattern. If the optional `{}` group makes macro matching ambiguous on the nightly or stable toolchain in use, split the macro into two repetition kinds, `unit Variant;` and `Variant { … };`. That is an expected adjustment, not a design change.
3. **The table** replaces the body of `From<&TmuxNotification>` (`:162-398`):
   ```rust
   let kind = notif.notification_type();
   tmux_to_py!(notif, kind; [
       Begin { timestamp => timestamp: copy, command_number => command_number: copy, flags => flags: own };
       End { timestamp => timestamp: copy, command_number => command_number: copy, flags => flags: own };
       Error { timestamp => timestamp: copy, command_number => command_number: copy, flags => flags: own };
       Output { pane_id => pane_id: own, data => data: own };
       PaneModeChanged { pane_id => pane_id: own };
       WindowPaneChanged { window_id => window_id: own, pane_id => pane_id: own };
       WindowClose { window_id => window_id: own };
       UnlinkedWindowClose { window_id => window_id: own };
       WindowAdd { window_id => window_id: own, window_layout => window_layout: nonempty,
                   window_visible_layout => window_visible_layout: nonempty,
                   window_raw_flags => window_raw_flags: nonempty };
       // … one row per remaining variant, transcribed from the current arms (:161-399)
       //   with the renames listed in Current state …
       SessionsChanged; WorkspacesChanged; Exit;
   ] raw: {
           // Deprecated name-as-exit-string beside the typed exit_code.
           TmuxNotification::PaneExited { pane_id, exit_code } => PyTmuxNotification {
               pane_id: Some(pane_id.clone()),
               name: exit_code.map(|code| code.to_string()),
               exit_code: *exit_code,
               ..PyTmuxNotification::empty(kind)
           },
       }
   )
   ```
   - Transcribe all 37 variants: 36 table rows plus the one `raw` arm. Keep the two explanatory comments that exist today, for the `WindowAdd` bare-id rule and the `AgentTelemetryChanged` identity-only rule, as `//` lines above their rows.
   - Keep `use crate::tmux_control::TmuxNotification;` in scope for the expansion.
   - `rustfmt` does not reformat macro-invocation bodies. Keep one row per line, wrapping long rows by hand as `WindowAdd` shows.
4. **No other edits.** Leave `empty()`, the owned `From`, the struct, `#[pymethods]`, the tests and `docs/API_REFERENCE.md` unchanged, since the Python surface is unchanged.
5. **Measure and record.** After the change, record in the PR description:
   - `wc -l` of `from`'s span;
   - parsight `audit_repository` `hotspots` for the file after reindex, noting that a lower CC there is a parser artefact of the macro and not a real reduction (see Current state);
   - the per-variant cost (one table line).

## Files to touch

- `src/python_bindings/types/notification.rs`: the macro, plus the `From<&TmuxNotification>` body.
- `CHANGELOG.md`: nothing. This is an internal refactor with no user-visible change. Add a `Changelog: skip` trailer to the commit per ENH-036's release-notes check.

## Verify

- Before the edit, `cargo test --lib --no-default-features --features python-test notification` passes (the baseline).
- After the edit, `cargo test --lib --no-default-features --features python-test notification` passes with `golden_matches_pre_refactor_output` and `variants_are_exhaustively_converted` green.
- `git diff -- src/python_bindings/types/notification.rs` touches no line inside `mod tests` (`GOLDEN`, `all_variants`, and the exhaustiveness match are unchanged).
- The `From<&TmuxNotification>` body contains no `_ =>` arm: `grep -n "_ =>" src/python_bindings/types/notification.rs` shows no match inside the converter or the macro.
- `wc -l` of the `From<&TmuxNotification>` span (macro plus impl) is at most 110 lines, down from 239 (`:161-399` at `2cf0957`).
- `git diff --stat -- derive/ Cargo.toml crates/par-term-emu-core/Cargo.toml` is empty: no derive-crate change, no new dependency, no version bump.
- `make derive-version-check` exits 0.
- Adding a throwaway variant to `TmuxNotification` locally (not committed) makes `cargo check --lib --no-default-features --features python-test` fail with a non-exhaustive-match error pointing into `notification.rs`. Revert it after the check.
- `make checkall` exits 0.

## Rollback

Revert the commit. The change is confined to one function body, the golden test pins its output, and no wire, Python API or dependency surface changes.
