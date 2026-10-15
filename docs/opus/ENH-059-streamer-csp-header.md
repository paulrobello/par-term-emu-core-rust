# ENH-059: Full Content-Security-Policy on the streamer's HTTP responses

> Filed from the 2026-10-12 /opus-audit enhancement pass (Security posture, "Hardening ideas"). Board card: `[ENH-059]`.
> Sequencing: independent. Touches only `src/streaming/server/http.rs`, `origin_tests.rs` and docs. No frontend rebuild is needed.

**Priority**: low · **Estimate**: S

## Goal

The streamer's HTTP app (`--enable-http`) serves a live shell UI. Today the only CSP directive it sends is `frame-ancestors 'none'` (SEC-010). Replace that with a full policy that limits where the page can load scripts from, where it can connect (WebSocket only), and which objects, base URIs and form targets it can use. The page drives a PTY, so a script-injection foothold is equivalent to shell access. The policy must keep the **shipped** Next.js static export in `web_term/` fully working: fonts, xterm's image addon (WASM and blob images), its runtime `<style>` elements, and the WebSocket to `/ws`.

**Deviation from the brief, on purpose.** The brief's starting point, `default-src 'self'; connect-src 'self' ws: wss:`, breaks the shipped page in five measured ways (see Current state). The final policy below adds exactly the sources the export uses. It uses `'unsafe-inline'` for scripts instead of hashes, for the reason given in step 1.

## Current state

- **Header site.** `src/streaming/server/http.rs:260-283` defines `add_security_headers`. It inserts `x-frame-options: DENY` and `content-security-policy: frame-ancestors 'none'` with `HeaderValue::from_static` (`:277-280`).
- **Wiring.** `src/streaming/server/listen.rs:15-46` defines `build_http_app()`. It builds `ServeDir::new(&self.config.web_root)` as the fallback (`:42`), then `.layer(build_cors_layer(..))` and `.layer(axum::middleware::from_fn(add_security_headers))` (`:44-45`) at the outermost layer, so static files and API routes both get the header. Both `start_with_http` (`:56`) and `start_with_https` (`:90`) call `build_http_app()`. The WebSocket-only listeners serve no HTML and are out of scope.
- **Existing test.** `src/streaming/server/origin_tests.rs:227-293` (`http_app_responses_carry_anti_framing_headers`) asserts the CSP **equals** `"frame-ancestors 'none'"` twice (`:268-273`, `:287-292`), once for the ServeDir `/` and once for `/sessions`. Its helper `origin_request()` is at `:60-70`. The module is `#[cfg(test)] mod origin_tests;` (`server/mod.rs:650-651`). The file is compiled with the `streaming` feature.
- **What the shipped export loads.** Measured against `web_term/` (32 tracked files, built by `make web-build-static`, `Makefile:912-922`, from `web-terminal-frontend/`, `next.config.js` `output: 'export'`). Chunk filenames under `web_term/_next/static/chunks/` are content-hashed and change on every frontend build, so the `.js` chunk names below identify the measured build only; locate the same code in a fresh build by content, not by name:
  1. **Inline scripts.** `index.html`, `404.html` and `_not-found.html` each have 3 inline `<script>` blocks: the Turbopack bootstrap `id="_R_"`, `(self.__next_f=self.__next_f||[]).push([0])`, and the RSC payload `self.__next_f.push([1,…])`. They have no `on*=` handlers. Their sha256 values are `GLh7i/Wh…3to=` and `OBTN3RiyCV4B…Keo=`, which are shared, and the payload hash differs per page (`3ReRXuLr…cI=` for index, `zzRbTgS+…biE=` for 404). The payload changes on every frontend content change.
  2. **External styles and fonts.**
     - `app/layout.tsx:40,66,71` and `app/globals.css:2` load `https://fonts.googleapis.com/css2?…` (stylesheet), whose font files come from `https://fonts.gstatic.com`.
     - `app/globals.css:6` loads `https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.3.0/…/SymbolsNerdFont-Regular.ttf`. It is present in the built stylesheet chunk under `web_term/_next/static/chunks/` (a `*.css` file whose name is content-hashed per build).
  3. **Inline styles.**
     - Server-rendered `style="…"` attributes: 5 in `index.html`, 4 in each 404 page. They come from `app/page.tsx:202,249,269,343` and `components/OnscreenKeyboard.tsx`.
     - A `<style>` tag in `404.html` and `_not-found.html`.
     - xterm/addons create `<style>` elements at runtime: `createElement("style")` appears 3 times in `2c2d7ch-c65p9.js` and once in `3dmm3v_fq28gc.js`.
  4. **WASM.** `@xterm/addon-image` (`components/Terminal.tsx:9,364`) compiles its decoder with `new WebAssembly.Module(…)` and `WebAssembly.instantiate(…)` (`2c2d7ch-c65p9.js`). Under a CSP without `'wasm-unsafe-eval'`, these throw.
  5. **Blob images.** The same chunk does `URL.createObjectURL(r)` followed by `new Image` when `createImageBitmap` is missing, so `img-src` needs `blob:`.
  - **WebSocket.** `app/page.tsx:93-110` derives `ws:`/`wss:` from `window.location`. The UI also accepts a user-typed URL (placeholder `ws://127.0.0.1:8080`, `:208`), so `connect-src` must allow any `ws:`/`wss:` host, as the brief says.
  - **Absent.** No `fetch(` in app code, no `new Worker`, no `eval(`, no `new Function(`.
- **Docs.** `docs/SECURITY.md:906-940` ("Streaming Server Security"). It has subsections for Threat Model, Authentication, TLS and Origin Validation (`:940`), and none for response headers. `CHANGELOG.md:446` is the historical SEC-010 entry.

## Implementation

1. **Policy constant** in `src/streaming/server/http.rs`, above `add_security_headers`:
   ```rust
   /// Content-Security-Policy on every HTTP-app response (ENH-059, extends
   /// SEC-010). Sources are exactly what the shipped `web_term/` export
   /// loads: Google Fonts CSS + font files and the jsDelivr Nerd Font,
   /// xterm's runtime `<style>` elements and React's SSR `style=` attributes
   /// (`'unsafe-inline'` styles), the image addon's WASM decoder
   /// (`'wasm-unsafe-eval'`) and blob-URL images, and any `ws:`/`wss:`
   /// endpoint (the UI accepts a user-typed server URL). Scripts are
   /// same-origin plus `'unsafe-inline'`: Next.js inlines a per-page,
   /// per-build RSC payload, so a hash list would break every
   /// `--download-frontend` version but the one it was computed for.
   pub(super) const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
       script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; \
       style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
       font-src 'self' data: https://fonts.gstatic.com https://cdn.jsdelivr.net; \
       img-src 'self' data: blob:; \
       connect-src 'self' ws: wss:; \
       object-src 'none'; base-uri 'self'; form-action 'self'; \
       frame-ancestors 'none'";
   ```
   - The `\`-continued literal yields single spaces between directives, so `HeaderValue::from_static` accepts it (no control characters).
   - `frame-ancestors 'none'` must stay. This header replaces the SEC-010 one rather than adding a second.
   - **Hashes rejected.** `'unsafe-inline'` is ignored when a hash is present, so hashes would have to cover every inline script. The RSC payload hash differs between `index.html` and `404.html` and changes on every frontend build. The streamer also serves arbitrary `--web-root` / `--download-frontend --frontend-version` exports (`src/bin/streaming_server/main.rs:201-210`), so a compiled-in hash list goes stale silently. Computing hashes at startup by scanning `web_root` would need an HTML scanner plus `sha2`, and `sha2` is optional and only enabled under `streaming-bin` (`Cargo.toml:102,207`). That is out of scope for an S card. Record it in the CHANGELOG as a possible follow-up.
2. **Use it.** In `add_security_headers` (`:277-280`), replace `HeaderValue::from_static("frame-ancestors 'none'")` with `HeaderValue::from_static(CONTENT_SECURITY_POLICY)`. Update the doc comment at `:260-265` to say the header now carries the full policy, not just anti-framing.
3. **Tests** in `src/streaming/server/origin_tests.rs`:
   - In `http_app_responses_carry_anti_framing_headers`, change both CSP asserts to `Some(CONTENT_SECURITY_POLICY)`. Bring it in with `use super::http::CONTENT_SECURITY_POLICY;` or the module's existing `use super::*` path. Also assert `.contains("frame-ancestors 'none'")`, so SEC-010's guarantee is pinned by name.
   - New `#[tokio::test] async fn http_app_csp_restricts_scripts_connections_and_objects()`. Build the app as in `:236-253` (TempDir web root with `index.html`), GET `/`, and assert that the header contains each of `"default-src 'self'"`, `"connect-src 'self' ws: wss:"`, `"object-src 'none'"`, `"base-uri 'self'"`, `"script-src 'self'"`. Also assert that it does **not** contain `"'unsafe-eval'"`. That needle is quoted, so `'wasm-unsafe-eval'` does not match it.
   - New `#[test] fn csp_allows_every_external_origin_the_shipped_frontend_loads()`. This is the drift guard for the next frontend change.
     - Read `concat!(env!("CARGO_MANIFEST_DIR"), "/web_term")`. Skip with an early `return` if the directory is absent.
     - Collect `https://<host>` origins from every `*.html` (attributes `href="https://…"` on `<link rel="stylesheet"|"preload">` and `src="https://…"` on `<script>`) and every `_next/static/**/*.css` (`url(https://…)` and `@import url('https://…')`).
     - Use plain `str::find` scanning, because the root crate has no `regex` dependency (only `par-term-emu-core` has one).
     - Exclude `preconnect` links, which CSP does not govern, and `<a href>`, which is navigation.
     - Assert that each collected origin appears in `CONTENT_SECURITY_POLICY`.
     - Today the set is `{fonts.googleapis.com, cdn.jsdelivr.net}`. `fonts.gstatic.com` is only reached from the Google CSS, so the policy lists it explicitly and the test does not need to find it.
4. **Docs.**
   - In `docs/SECURITY.md`, after the Origin Validation subsection (`:940`), add `### Response Headers (SEC-010, ENH-059)`. Give the full policy, the reason for each source, the `'unsafe-inline'` script trade-off, and the instruction: "a frontend change that loads a new external origin must extend `CONTENT_SECURITY_POLICY` (`src/streaming/server/http.rs`); `csp_allows_every_external_origin_the_shipped_frontend_loads` fails until it does."
   - `CHANGELOG.md` `[Unreleased]` → `### Security`: one bullet citing ENH-059, `src/streaming/server/http.rs`, and the follow-up idea of startup-computed script hashes.

## Files to touch

- `src/streaming/server/http.rs` (`CONTENT_SECURITY_POLICY`, `add_security_headers`)
- `src/streaming/server/origin_tests.rs` (update one test, add two)
- `docs/SECURITY.md` (new Response Headers subsection)
- `CHANGELOG.md` (`[Unreleased]` Security bullet)

## Verify

Acceptance criteria:
- `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming streaming::server::origin_tests > /tmp/enh059-t.log 2>&1; echo EXIT=$?` prints `EXIT=0`. The `test result:` line shows a **nonzero** passed count, and `grep -cE 'http_app_responses_carry_anti_framing_headers|http_app_csp_restricts_scripts_connections_and_objects|csp_allows_every_external_origin_the_shipped_frontend_loads' /tmp/enh059-t.log` prints `3`.
- **Live check: the header is served and the shipped page runs under it with zero CSP violations.**
  - Run `PATH="$PWD/.venv/bin:$PATH" make streamer-build-release`.
  - Start `./target/release/par-term-streamer --port 8099 --enable-http --web-root ./web_term` in a Herdr pane. If 8099 is taken, pick a free port listed in `~/.claude/used_ports.md`.
  - `curl -sI http://127.0.0.1:8099/ > /tmp/enh059-h.txt; grep -ci "^content-security-policy: default-src 'self'" /tmp/enh059-h.txt` prints `1`.
  - Run `agentchrome navigate http://127.0.0.1:8099/` (headless, GPU flags per the browser rules) and wait about 3 s. Then `agentchrome console read > /tmp/enh059-c.json 2>&1; grep -ci 'content security policy\|refused to' /tmp/enh059-c.json` prints `0`.
  - `agentchrome page text` shows the terminal connected, with a shell prompt rendered and no "disconnected" banner. Finish with `agentchrome connect --disconnect` and stop the server.
- **The drift guard bites.** Temporarily delete `https://cdn.jsdelivr.net` from the constant. `csp_allows_every_external_origin_the_shipped_frontend_loads` then fails and names `cdn.jsdelivr.net`. Restore the constant, and the test passes again.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh059-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

## Rollback

Restore the `from_static("frame-ancestors 'none'")` line and the old equality asserts. That brings back exact SEC-010 behavior. If a browser feature breaks in the field, the targeted fix is to extend the single constant with the missing source. Reverting the whole card is not needed.

## Dependencies / sequencing

- None blocking.
- Any future `web-terminal-frontend` change that adds an external origin, a Worker (`worker-src`), or `fetch` to another host must update the constant in the same change. The new drift test enforces this for stylesheet, script and CSS `url()` origins, but not for `connect-src` hosts.
- If the frontend later self-hosts its fonts, drop the three font/style hosts in the same change.
