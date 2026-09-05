# Upstream contribution: workflow-terminal input-handling fixes

Prepared 2026-09-05, to be filed as a PR against [workflow-rs](https://github.com/aspectron/workflow-rs) (`terminal/` crate) once this repository is public. The fixes ship today in this repo's vendored copy at [`extern/workflow-terminal/`](../../extern/workflow-terminal/) (registered via `[patch.crates-io]`); every change is marked `Marigold fix` in the source. Base version: `workflow-terminal 0.18.0` from crates.io. The complete diff against the pristine crate is [`fix.patch`](fix.patch). Also affects downstream `rusty-kaspa`'s `kaspa-cli`, which is where we found it — a companion note to kaspanet may be warranted since the mitigation on the CLI side (prompt-redraw throttling, empty-password re-ask; our commit `fe7e256a`) lives outside this crate.

## Suggested PR title

terminal: don't let buffered input defeat secret prompts; fix ghost intake loops

## The bug, as experienced

On a busy terminal (our network produces ~10 wallet notifications/second, each triggering a prompt redraw), a user typed a `send` command with an extra Enter — natural when the screen is churning and it looks like nothing registered. The result:

1. The stale Enter sat in the tty buffer. `Terminal::ask(secret=true, ...)` opened the password prompt and the spawned intake loop immediately consumed the buffered Enter as the entire password. The prompt closed within milliseconds — visually it never existed.
2. Wallet decrypt failed with the empty secret; the CLI printed the error and returned to the normal prompt.
3. The user — still watching a scrolling screen — typed their password. `user_input` was no longer enabled, so the characters went through the **normal line editor**: echoed in cleartext, **pushed into the Up-arrow history**, and executed as a command (`command not found: <password>`).

Net effect: pressing Up-arrow later shows the wallet password in cleartext. This is a real-world secret leak, reproduced deterministically (see "Reproduction" below).

## Root causes found during investigation

All in `src/terminal/mod.rs` and `src/terminal/crossterm.rs` of 0.18.0:

1. **No input flush on modal entry.** `Terminal::ask()` calls `reset_line_buffer()` (clears the *line* buffer) but nothing drains pending tty input, so type-ahead answers secret prompts.
2. **The normal editor accepts input while a command is running.** `Terminal::ingest()` checks `user_input.is_enabled()` but never `is_running()`. Keys typed during command execution are echoed, recorded in history, and executed — the leak path in step 3 above.
3. **Ghost intake loops.** `UserInput::capture()` spawns a second `intake()` reader per prompt; the loop's terminate check sits *after* the blocking `event::read()`, so a reader whose closing Enter was consumed elsewhere blocks forever, and `UserInput::open()` resets the shared `terminate` flag on the next prompt — the ghost then steals keystrokes from every later prompt. (Two concurrent readers also share a single unbounded channel, so a secret can in principle be delivered to the wrong `capture()` caller. The `cfg_if` around the spawn carries an upstream `TODO - refactor` acknowledging the workaround.)
4. **`refresh_prompt()` is not modal-aware.** It repaints the normal `$`-prompt (and line buffer) over an open secret prompt; it only checks `is_running()`.
5. **Output dropped during kbhit prompts.** `Terminal::writeln()` with `user_input` enabled but `get_prompt() == None` (the `kbhit(None)` case) silently discards the line.
6. **Panic-capable arithmetic.** `writeln()`/`refresh_prompt()` compute `data.buffer.len() - data.cursor` with no ordering guarantee between the two reads; with `overflow-checks = true` (as rusty-kaspa sets in its release profile) an interleaving underflow panics the terminal.

## Second find (2026-09-05): Ctrl+C during a secret prompt

Three more defects, found in live use the day after the first batch:

7. **Ctrl+C at a secret prompt is indistinguishable from an empty answer.** `UserInput::ingest`'s `Ctrl('c')` arm closed the prompt, so `capture()` returned `Ok("")` — a caller re-asking on empty input (a sensible policy after root cause #1) greets a user who just cancelled with "try again". Fixed with an `aborted` flag on `UserInput`; `capture()` now returns an error on cancellation.
8. **Ctrl+C at a secret prompt killed the whole application.** The same arm called `term.abort()` — changing your mind about entering a password tore down the CLI. Now it cancels the prompt only (crlf + close); the normal-prompt Ctrl+C behavior is unchanged.
9. **Duplicate prompt-close panics the runtime.** `close()` unconditionally `try_send().unwrap()`-ed into the shared channel; a stray second close (ghost reader, or Ctrl+C racing Enter) eventually panicked a tokio worker with `Channel TrySend Error: Full(..)`. `close()` is now idempotent (guards on `enabled.swap(false)`) and the send result is not unwrapped.

Reproduction for 7-9: open any secret prompt and press Ctrl+C — observe empty-answer semantics and app teardown; repeat the cycle several times to accumulate the channel panic.

## Also in the patch: Ctrl+D and the Ctrl+C hint

`Key::Ctrl('d')` at an empty line exits (EOF, like every modern REPL); with text on the line it does nothing. The idle-prompt `Ctrl+C` (which clears the line rather than exiting — see the Ctrl+C section) now prints a one-line hint pointing at `exit`/Ctrl+D so the exit path is discoverable.

## Also in the patch: Tab completion (feature, not fix)

The `Cli::complete` hook has existed in the trait all along but nothing drove it: `KeyCode::Tab` was never mapped to a `Key` variant, so Tab never reached `Terminal::ingest`. The patch adds `Key::Tab`, maps it in the crossterm backend, and adds a completion driver to `ingest` (single candidate completes in place; several extend to the common prefix or list alternatives). Contract: `complete(line)` returns full-line candidates. Offered together since it touches the same files.

## The fixes (see fix.patch)

- `crossterm.rs`: new `flush_pending_input()` (drains via `event::poll(Duration::ZERO)`); `intake()` polls with a 50 ms timeout and checks `terminate` *before* reading, so orphaned readers exit.
- `mod.rs` `ask()`: flush pending input immediately before the prompt opens **and** after it closes (keys typed a beat too late must not leak into the next command line).
- `mod.rs` `ingest()`: while `is_running()`, discard everything except `Ctrl+C`.
- `mod.rs` `refresh_prompt()`: return early while `user_input.is_enabled()`.
- `mod.rs` `writeln()`: print the line (without prompt redraw) in the kbhit-no-prompt case instead of dropping it.
- Both length computations switched to `saturating_sub`.
- `termion.rs` / `xterm.rs`: no-op `flush_pending_input()` so the per-platform `Interface` alias keeps a uniform surface (the wasm/xterm backend has no OS-level type-ahead buffer to drain; termion could implement the real thing analogously to crossterm if desired).

Behavioral change worth flagging in the PR: type-ahead of the *next command* while one is executing no longer works — keys are discarded rather than buffered. We judged silent discard safer than the leak; if upstream values type-ahead, an alternative is buffering keys without echo/execute until `running` clears.

## Reproduction (manual)

1. Any CLI on workflow-terminal with a secret prompt (e.g. `kaspa-cli` `open` against a wallet).
2. Type `open` then Enter **twice** in quick succession.
3. Unpatched: "Enter wallet password:" appears and instantly resolves with an empty answer → decrypt error → normal prompt. Type anything now and press Up-arrow afterward: your input is in history.
4. Patched: the prompt waits; the second Enter is discarded. An automated pty reproduction (pexpect: send `open\r\r`, assert the prompt still waits, answer it, assert Up-arrow recalls `open`) is described in our commit `fe7e256a`.

## Licensing

workflow-terminal is MIT OR Apache-2.0; the vendored copy and this patch retain the license. Attribution intact; the diff is ours to contribute (same dual license).
