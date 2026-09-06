# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.21.7] - 2026-09-06

### Fixed

- **Copy in an embedded VNC session did nothing and said nothing** — the button was wired to a handler that read the *local* clipboard, logged its length at debug level, and stopped. Nothing was transferred, no status appeared, and the button was enabled from the moment the toolbar was built, so pressing it on a fresh session was indistinguishable from a button with no handler. The RDP toolbar next to it had the full version — cache the text the server pushed, put it in the local clipboard on demand, report "Copied N chars" or "No remote clipboard data" — and VNC had none of it. It does now. The one structural difference from RDP is worth stating, because it decides what the button can mean: RFB clipboard transfer is push-only, so there is no request a client can send to read the remote clipboard. The text the server volunteered through `ServerCutText` is the only copy that exists locally, so `EmbeddedVncWidget` retains it and Copy re-asserts it into the local clipboard — which is what you want when something else has since overwritten it. The automatic mirror on arrival stays, since without it a remote copy would never reach the local clipboard at all. The button now starts insensitive with a tooltip saying it is waiting for remote data, exactly as the RDP one does, and becomes active when the first text arrives. It also gained the accessible label it was missing. `show_status_briefly` moved from `embedded_rdp::clipboard` to `embedded_trait`, where both toolbars can reach it — the absence of a shared helper is part of why the VNC side had no feedback to give.

- **The wizard threw away the icon when you pressed "Advanced…"** — typing an emoji or icon name on the wizard's Authentication page and then continuing into the full editor lost it, so the field opened empty and the connection was saved without it. Save and Save & Connect kept the icon, which is what made this hard to spot: the icon was read in `build_connection`, the method those two buttons use, and `collect_partial` — the one that feeds `WizardResult::OpenAdvanced` — never asked for it. `PartialConnection` had no `icon` field at all, so there was nowhere for it to go even if it had been read. The field exists now and the fallback to the selected template's icon is resolved once, inside `collect_partial`, rather than separately in `build_connection`; both paths read the same value, so they cannot disagree about it again. `from_connection` carries the icon in the other direction too, which fixes the same omission in "Duplicate via Wizard…" — it dropped the original's icon on the way *into* the wizard, and `AuthPage` had a getter for the field but no setter, so nothing could have pre-filled it.

- **The connection editor kept calling a URL a Host, and hid fields by leaving an empty row behind** — selecting the Web protocol was supposed to retitle the Host row to **URL**, and the protocols that have no host or port — Serial, Kubernetes, and every Zero Trust provider but Custom Command — were supposed to drop those rows entirely. Neither worked. The General tab created seven `gtk::Label` widgets for Host, Port, Username, Domain, Tags, Password Source and Value, stored them on `BasicTabWidgets`, and never added a single one to any container. The visible text of a boxed-list row is the row's own `AdwActionRow::title`, set separately when the row was built, so the labels were orphans from the moment they were constructed. `apply_general_field_visibility` then spent its whole body acting on them: `host_label.set_text("URL")` retitled a widget nobody could see, and `host_label.set_visible(false)` hid the same one. What made the second half look like it worked is that the *entries* were being hidden alongside — so a hidden field left its row on screen, titled "Host" or "Port", with nothing in it to type into. Visibility and titles now act on the rows, which is also why the paired `*_entry.set_visible()` calls are gone: an entry is a suffix *inside* the row, so hiding the row already takes the entry with it, and toggling both invited the two to disagree. The orphaned labels are deleted rather than wired up, along with the `Value` one that had no reader at all. Three of them turned out to be the only thing keeping `username_entry`, `tags_entry` and `domain_entry` alive in the visibility struct, and the whole `#[expect(dead_code)]` on `BasicTabWidgets` went with them — an attribute that had been silently absorbing this bug, since "these fields are never read" was the compiler pointing straight at it.

## [0.21.6] - 2026-09-05

### Added
- **An optional second confirmation before a snippet runs (issue [#315](https://github.com/totoshko88/RustConn/issues/315))** — a **Confirm before running** switch in the snippet editor. With it on, running the snippet opens an `adw::AlertDialog` showing the fully substituted command and waits; with it off, which is the default and what every existing snippet deserializes to, nothing changes. The field is `Snippet::confirm_before_run`, serialized only when true, so `snippets.toml` written by this version still loads on older ones. The interesting part was not the dialog but the count of ways a snippet can already be started: the picker, the Execute button in the snippet manager, the variable-input dialog's own Execute button, the inline snippet items in the terminal's right-click menu, and the Scripts menu of an embedded RDP session. The first four each reached VTE through their own copy of the resolve-then-send sequence — `execute_snippet`, `execute_snippet_direct` and the variable dialog's handler all called `send_text_to_focused` independently, and that helper takes a `&str`, so it never knew which snippet it was sending. A gate in any one of them would have left the other three open. They now funnel through one `send_snippet_command`, which is the only thing in the module that appends the newline that makes a shell run the text, so a future call site that forgets the flag would also have to reinvent that. `execute_snippet_direct` gained a parent-window parameter to make this possible: it had none, on the stated grounds that a context-menu action has no window to offer, but `setup_snippet_actions` holds the window and the three sibling actions in the same function were already downgrading it. RDP is a genuinely separate delivery mechanism — clipboard-plus-Ctrl+V or per-character autotype, not a VTE write — so it gets its own gate around the same command text, on the reasoning that where the keystrokes come from does not change whether the user meant to send them. The confirmation from the variable dialog is parented on that dialog and closes it only once the command has gone out, so cancelling returns the values you typed instead of discarding them. The button is styled destructive: the user turning this on for a specific snippet is a statement that running it by accident is expensive, and the default response stays Cancel either way. Both gates log at debug level, which the snippet execution path did not do at all before — running the app with `RUST_LOG=debug` and executing a snippet produced no output whatsoever, so a snippet that failed to arrive was indistinguishable from a click that never reached the app, which is the same blind spot the 0.21.5 tray-menu fix was found through. The command itself is deliberately not logged, only its length: substitution has already happened by that point, so the text can carry values resolved from vault-backed global variables. The cancel branch is logged too, since declining is a state the flag creates and nothing else would record it.
- **`rustconn-cli snippet` honours the same flag** — `--confirm` on `add`, `--confirm [<true|false>]` on `edit` (the value is optional and defaults to `true`, and omitting the flag entirely leaves the current setting alone), and `snippet show` reports it. The part that matters is `snippet run --execute`, which runs the command through `sh -c`: it now prompts on stderr, and with no terminal on stdin it refuses outright rather than prompting into a pipe, so a snippet marked for confirmation is never executed unattended by accident. `--force` is the opt-out for scripts that mean it, mirroring `connection delete --force`. Without this the flag would have been advice the GUI took and the CLI ignored, on the one path in the workspace that actually spawns a shell. Declining and having nobody to ask now say different things — one is a decision, the other is a script that needs `--force`.

### Fixed

- **Bitwarden auto-unlock did nothing at all in every interface language but English (issue [#312](https://github.com/totoshko88/RustConn/issues/312), reported by [@vh45f](https://github.com/vh45f))** — the vault could be unlocked by hand from Preferences ▸ Secrets and then worked, but the unlock RustConn performs for itself at startup, from the master password in the keyring, was skipped. `check_bitwarden_status_sync` returned the *display* string for the vault state, so it had already been through `i18n()`, and both callers then decided whether to attempt an unlock by comparing that string against the literal `"Locked"`. In Italian the string is `Bloccato`, in Ukrainian `Заблоковано`; neither equals `"Locked"`, so the guard read "not locked", returned early, and set the status row to the locked text it had just declined to act on. English was the one locale where that comparison could be true, which is why this survived testing on both the system and Flatpak builds. The reporter's log shows it exactly: the auto-unlock step "completed" in 2472 ms, which is the cost of one `bw status` in that same log, where a real unlock takes about 4.5 s. The state is now an enum, `BwVaultStatus`, and the decision is `should_try_unlock()` on the variant, with the label rendered separately at the point it is shown. Two other places in the same paths pushed a bare `"Unlocked"` or `"Locked"` into that status row untranslated; they go through the same renderer now. The decision also fires on an *inconclusive* probe, which is a second door onto the same silence: `bw status` gets five seconds and cost 2.5 s in that same log, so a slower link answers `ProbeFailed`, and reading that as "nothing to do" skips the unlock exactly as the string comparison did. An attempt against a vault that turns out to be unlocked costs one `bw unlock` and returns a fresh session key, so guessing wrong there is cheap; `Unauthenticated` still declines, because no password unlocks a CLI with no account. Tests pin the decision per variant and pin the shape of the original bug — that a decision taken from the rendered label is wrong even in English, because the label is what a translator is free to change.

- **The startup banner announced that Bitwarden could not store passwords while Bitwarden was storing them (issue [#312](https://github.com/totoshko88/RustConn/issues/312))** — "Bitwarden is selected, but it cannot store passwords yet: Locked", on a vault that had unlocked, synced and was answering lookups seconds earlier in the same log. The readiness probe ran `bw status` as a bare `std::process::Command`, and RustConn deliberately keeps the session key in process memory rather than in its own environment — so the child was launched without `BW_SESSION`, `bw status` could not see the session, and it answered `locked`, correctly. That answer became `BackendReadiness::NeedsAction`, which is what the banner renders. The probe now assembles its command the way `BitwardenBackend::build_command` does: the session key from `get_session_key()` passed through the environment rather than argv, the extended `PATH` a sandboxed `bw` needs to find the tools it shells out to, and `--nointeraction`, so a call with a five-second budget cannot stall on a prompt or an implicit network fetch. Worth recording because 0.21.3 was reported as fixing this and did not: that release corrected the banner's *wording*, which until then claimed a missing keyring client for a product that has no keyring client. The verdict behind it was still wrong.

- **A Bitwarden password was written to the vault and reported as refused at the same time (issue [#312](https://github.com/totoshko88/RustConn/issues/312))** — "Failed to store credentials: Vault store timed out after 10s", after which the password was in fact present in the vault and RustConn offered to save it somewhere else instead. One Bitwarden store is three `bw` processes in sequence — `list folders`, then `list items`, then `create item` or `edit item` — each a fresh `node` plus a round trip to the vault server. Measured in the reporter's log against `bitwarden.eu`: 2.9 s, 5.0 s, and `create item` still in flight when the budget expired at 10.008 s. The ten seconds came from a comment about a hung keyring blocking a GTK callback, which is a real concern for a D-Bus call and not the same order of magnitude as a CLI over a network. Dropping the future is also not the same as stopping the work — `tokio::process` does not kill on drop — so `bw` ran to completion and the write landed: the failure was in the reporting, not in the store. The budget is now chosen per backend by `vault_op_timeout`, 45 s for the four CLI-backed backends and the previous 10 s for everything that answers from this machine, and it applies to reads and deletes too: a `bw list items` alone took 5 s there, so a saved password could as easily have gone unfound on connect and been asked for again. Each `bw` invocation keeps its own 30 s ceiling inside `rustconn-core`, and none of these calls run on the GTK thread, so the longer wait costs a slow save rather than a frozen window. The timeout message now states the budget that was actually applied instead of a hardcoded "10s". The credential-transfer loop keeps its own, smaller per-entry budget: that one is spent forty times across a batch and is a different trade-off.

- **Add SSH Key did nothing, and there was no way to find out why** — Preferences ▸ Secrets ▸ SSH Agent ▸ **Add Key** opens a `GtkFileDialog`, and its callback matched the outcome with `if let Ok(file) = result`, so every failure was discarded without a word. That is also how a *dismissed* chooser arrives, which is why the shape looked deliberate, but it swallowed real errors identically — including a desktop portal that refuses the request, which is what a `GtkFileDialog` failing to open usually is. The whole path had no logging either, so the button was indistinguishable from a button with no handler at all. Dismissal is now told apart from failure: closing the chooser logs at debug and does nothing, while an actual failure logs a warning and opens a dialog carrying the system's own message, pointing out that keys already in `~/.ssh` are listed under Available Key Files and can be added without the chooser. A chosen location with no local path — a remote mount, which `ssh-add` cannot use — is reported rather than ignored, and the two "no root window" paths now say which one happened instead of both logging the same line. This makes the failure diagnosable; the underlying reason it fails on a given system is whatever the new message names.

- **The Add Key passphrase dialog had no visible way out** — Preferences ▸ Secrets ▸ SSH Agent ▸ Available Key Files ▸ **+** opened a dialog with a single **Add Key** button, no Cancel, and the header's close button explicitly hidden. Escape and clicking outside did dismiss it, so it was never a trap, but nothing said so and it read as stuck. The cause is that this dialog did not use any of the house patterns: `dialogs/widgets.rs::dialog_header` hides the title buttons and puts the action in the *header*, which is why hiding them is normally fine, whereas this one hid them and put its only button in the body. It now matches `portable_passphrase_change` and `credential_transfer`, the two other dialogs that take input and hide the title buttons — Cancel at the start of the header, the action at the end. Pressing Enter in the passphrase field also submits now, as it already did in the connection password dialog. The two dialogs mentioned above were checked for the same defect and do not have it.

- **Global variables that could not be written to disk were reported as saved** — the Variables dialog updated the settings in memory, so the edits looked applied and survived until the next start, at which point they were simply gone; the only record was a `tracing::error!`. This is a save failure on user data, so it now opens a dialog naming what went wrong, with a toast as the fallback for when the window has already closed. The same file was already doing this correctly forty lines above, for a failed vault write, and with the same reasoning written out — the disk write was the one path that had been left as a log line.

- **A standalone SSH tunnel could fail in complete silence, and the diagnosis was already being built and then thrown away** — pressing Start with `ssh` absent logged a warning, redrew the row as Stopped and told the user nothing; a tunnel that died later simply left the Active group, which looks exactly like having stopped it on purpose. The information existed the whole time. `TunnelManager` captures the ssh process's stderr in a background thread, and `health_check` formatted it into `TunnelStatus::Failed(msg)` — then removed the entire process record one loop later, discarding the message, after which `status()` answered `Stopped`. `tunnel_builder::path_diagram` has a `TunnelStatus::Failed` branch already written to draw exactly that text, which could therefore never fire, and the public `TunnelManager::stderr()` had no callers at all. `health_check` now returns what it found as `TunnelFailure { id, reason }` and records the reason, so `status()` reports `Failed` for a tunnel that exited on its own and `Stopped` only for one never started or stopped deliberately; both clear on the next explicit start or stop. The row shows the difference with a warning icon, an `error` style and an accessible label rather than colour alone, plus a selectable **Last Error** row carrying ssh's own words — "Permission denied", "Address already in use" — placed in the expanded body instead of the subtitle, because stderr is arbitrarily long and would wreck the collapsed row. A failed start now opens a dialog, since a start the user asked for and did not get is a half-finished action rather than a background event, while a tunnel dying on its own raises an error toast. A missing binary gets its own error variant carrying the program name, because an MPTCP-enabled connection runs `mptcpize` rather than `ssh` and a message naming `ssh` would have sent the user to install something they already had. Auto-reconnect also stopped discarding its `Result` — that is what let a reconnect which could never succeed retry in silence until the attempt counter ran out — and giving up after the final attempt now says so instead of only logging it. Two follow-ups from auditing that fix are below: the remedy it printed named the wrong package, and none of it was visible while the tunnel manager was open.

- **A failed tunnel start named the wrong package to install** — the entry above added `ProgramNotFound` carrying the program name for one stated reason: an MPTCP-enabled connection runs `mptcpize` rather than `ssh`, so a message naming `ssh` would send the user to install something they already have. The remedy sentence then said "Install the OpenSSH client" whatever the name held, producing "needs mptcpize … install the OpenSSH client" — the same misdirection, one line below the comment explaining it. The advice follows the program now: `ssh` points at the OpenSSH client, `mptcpize` at the Multipath TCP tools with the package name most distributions use, plus the alternative of turning Multipath TCP off for that connection. An unrecognised name gets generic advice rather than a guess, because the string arrives from `rustconn-core` and a third carrier added there would otherwise show up with a confidently wrong package name.

- **The tunnel manager did not notice a tunnel dying while it was open** — the health check runs on a five-second timer and had no way to reach the dialog, so a tunnel that exited kept its row in the **Active** group saying Running. The warning icon, the accessible label and the **Last Error** row — the entire visible half of the fix above — appeared only after the dialog was closed and reopened, or after some unrelated button happened to trigger a refresh. The one window built to show a tunnel failure was the one place it did not show. The health check now redraws an open manager, through a handle the dialog publishes when it is presented: held weakly, since a strong reference would keep a closed dialog alive for the life of the process, and a no-op when nothing is open, which is what makes it safe on a timer.

- **Running a snippet from the terminal's right-click menu could do nothing at all** — a snippet with a variable that neither a global variable nor its own default could supply was dropped without a dialog, a toast or a log line. The comment explaining that pointed at the "Execute Snippet…" picker, because the variable dialog needs a parent window and a context-menu action was said to have none — but it has had one since the confirmation gate above needed somewhere to anchor. The reason outlived the constraint it described, and what remained was the one route where choosing a snippet from a menu silently did nothing. It opens the variable dialog now, with whatever did resolve pre-filled. Fixing it exposed a divergence between the two resolution loops: `execute_snippet` collected every unresolved name while `execute_snippet_direct` stopped at the first, so the same snippet would have reached the same dialog with different fields pre-filled depending on which menu you came from. They share one `resolve_snippet_variables` now.

- **The embedded RDP confirmation showed the command untruncated** — the terminal gate caps the preview at 400 characters because an `adw::AlertDialog` grows with its body until it stops being readable; the RDP gate passed the command whole, so a generated one-liner made the dialog unreadable on exactly the snippets a confirmation is worth having for. Same dialog, same failure, same helper.

- **Add Key could be left permanently unable to open a file chooser** — the guard against a second click while a chooser was open disabled the button and re-enabled it in the chooser's callback. That callback fires when the user picks a file or closes the chooser, so it can be minutes away while they browse, and there is no "the chooser appeared" signal to time out against instead. In the environment recorded in the previous entry — a portal that refuses the request — the callback never fires at all, so the button stayed dead for the rest of the session with no message, which is a worse version of the report this path exists to fix. A stacked pair of choosers is recoverable; a dead button is not. The button stays live now and a new click cancels the request in flight, which arrives at the callback and is recognised there.

- **Every `bw unlock` ran without a deadline** — `unlock_vault_sync` spawns up to three children and bounded none of them, so an unlock stalled on a network sync blocked its caller indefinitely, and every caller is a worker thread with a status row waiting on it. The 30-second ceiling that `run_command` already had was a literal in one place, which made "each `bw` invocation has a deadline" true of exactly one invocation; it is a named constant now and all four honour it, through the helper that also reaps the child it kills. The two `--passwordenv` attempts additionally pass `--nointeraction`, matching how the backend builds every other command; the stdin fallback deliberately does not, since it exists for older CLI builds and works by answering the prompt that flag suppresses. Two smaller things a security pass found while this was written, both pre-existing: the unlock logged the master password's *length*, which is bruteforce metadata that the GUI's own handler explicitly declines to log before sending all four of its call sites here; and raw `bw` stderr flowed into a debug log and into the error the Secrets page renders, while the session-key parser only ever reads stdout — so a build emitting its `export BW_SESSION="…"` banner on stderr would have leaked the vault's bearer token down the one path that never looks for it. Both are closed, with the reason text ("Invalid master password", "not logged in") deliberately preserved because two callers match on it.

- **`RUST_LOG=debug` wrote the RDP account password into the log in clear** — found while reading a debug log attached to a bug report, which is exactly where it does the most damage. `sspi` logs the encoded CredSSP `TSRequest` at debug level, and field `[2]` of `TSPasswordCreds` is the password as plain UTF-16LE, so the log carried a decodable byte array: an ASN.1 walk over the array from that report yielded a 26-byte `userName` and a 64-byte `password`, which is 32 printable characters. What made it easy to miss is that the *same log line* prints `password: Secret` in its span fields — the secrecy wrapper was working perfectly on the field RustConn owns while the crate underneath it serialised the whole structure a few characters later. A `sspi=warn` directive now joins the three `ironrdp*` ones, matching how those are already treated. It is added **after** `EnvFilter::from_default_env()` and is therefore deliberately not overridable: a `RUST_LOG=sspi=debug` no longer re-enables it, because nobody should be able to turn a credential leak back on by accident. Anyone who has run 0.21.5 or earlier with `RUST_LOG=debug` against an RDP host should treat that account's password as disclosed.

- **A clean shutdown reported that every terminal child had ignored SIGTERM** — quitting with sessions open logged "child ignored SIGTERM, killing process group" once per session, which reads as a hung `ssh` needing a `SIGKILL`, and is not what happened. `still_our_group_leader` exists to prevent signalling a pid that has been recycled onto somebody else's process, and it answers *true* for a zombie — a child that has already exited and is waiting to be reaped. On shutdown that is the normal state: the reaper is GLib's child watch, and its main loop is gone by then, so a child that obeyed the signal promptly is still in the process table when the grace period ends. The check now tells a corpse from a live process by reading state `Z` from `/proc/<pid>/stat`, and the two escalation sites report which one they found. The `SIGKILL` to the process group stays in both branches, deliberately: an `ssh` with a `ProxyCommand` shares its group with helpers that the parent's exit does not reach, so sweeping the group is right even when the leader is already dead — only the message was wrong. Extending the grace period was considered and rejected, since nothing was ignoring anything; so was reaping with `waitpid(WNOHANG)`, which would steal the child from a GLib watch that may still be running. The original reading of this log was wrong in a way worth recording: `status=65280` is `WIFEXITED` with code 255, which is `ssh`'s own exit, not a signal at all.

- **An unset console keymap made every RDP session US English** — `localectl status` prints `VC Keymap` above `X11 Layout`, and on a desktop the console keymap is normally unconfigured, so the output opens with `VC Keymap: (unset)` and only then gives `X11 Layout: de`. The parser returned the first line matching either label, answered `(unset)`, found no Windows keyboard-layout identifier for it, and logged "Keyboard layout detection failed, using US English". Detection had not failed. It had found the layout and discarded it one line later. The consequence is silent and hard to attribute — the server interprets every scancode against the wrong table, so a German or French keyboard types the wrong characters in an embedded RDP session with nothing in the UI pointing at the layout. On a `us,*` machine the wrong answer happens to equal the right one, which is why this survived. `X11 Layout` now wins wherever it appears, and the placeholders systemd prints for an unset value are rejected rather than read as layout names — `(unset)` today, `n/a` in older releases. Both sources also report the whole group list rather than a single name, so a machine whose *primary* layout is unknown to the table no longer falls back to US English while naming a layout that is in the table one entry later. The parse moved out of the spawn, so those shapes are tested without a `localectl` on the machine running the tests, and the fallback message now carries what each source answered and points at the connection's explicit keyboard-layout setting.

- **With LibSecret, a password saved on a connection in no group was never found again (issue [#316](https://github.com/totoshko88/RustConn/issues/316), reported by [@Xiaol1n173](https://github.com/Xiaol1n173))** — connecting prompted for a password that was sitting in the keyring, and the connection dialog's 📂 load and ✓ test buttons both reported it missing. Putting the connection into a group fixed everything, which is the shape of the clue: `generate_store_key_with_group` takes an `Option<&str>` group path with *three* meanings, not two. `Some("Production")` is a grouped connection, `Some("")` is an ungrouped connection — still prefixed `RustConn/`, because that is what its entry is written with — and `None` is the flat pre-0.19.18 key that belongs to no connection at all. Four call sites built that `Option` themselves, and they disagreed: saving and deleting passed `Some("")`, while resolving and the two dialog buttons wrote `connection.group_id.map(…)`, which is `None` when there is no group. So the password went to `RustConn/{name} ({protocol})` and was looked for at `{name} ({protocol})`. The Secret Service matches attributes exactly, so the two never met. A grouped connection escaped it for a reason worth stating: there the two keys differ, so the resolver tried its legacy second key as well and that one hit — the bug needed the hierarchical and flat keys to collapse onto the same wrong string, which happens only when there is no group. The reporter's own conclusion is the fix: the ungrouped decision now lives in one function, `generate_store_key_for_connection`, which takes the group *id* rather than a pre-joined path, and all four sites call it. Save, resolve, delete, migrate and the credential transfer therefore cannot drift apart again the way the previous half of this fix did in 0.21.0, which corrected the resolver for grouped connections and left the ungrouped case behind. What was missing was the test, not the insight: nothing compared the key the GUI writes against the key the resolver reads, and every existing test used a grouped connection. Four now pin it, including that `Some("")` and `None` are not interchangeable. One asymmetry found while reading and deliberately left alone: the store key does not run the connection name through `sanitize_imported_value` and the resolver's does, so an imported name ending in a literal escape can still produce two strings — the delete path already carries both keys for exactly that reason, and narrowing it belongs with the import code rather than here.

- **The shutting-down flag was set after the session exits it exists to explain** — `APP_SHUTTING_DOWN` is how a session-exit callback tells an exit the application caused from one that is a fault, and it was set in `connect_shutdown`. GTK runs that from `app.quit()`, which `do_quit` calls *after* `shutdown_sessions_for_exit` has already signalled every session child, synchronously — so on the ordinary quit path both guards that ask the question got `false`. One of them decides whether a failed `terminate_session` is a debug line or a warning, and was left resting on its second clause alone; the other is the early return that stops a reconnect banner and a failure toast being raised for a session the teardown had just killed, and it was simply unreachable. The flag is now set at the top of `shutdown_sessions_for_exit`, the single teardown every exit path funnels through, and one a tray-minimize never reaches — which is what makes it the right place, since the flag must not be set by a close that leaves the application running. `connect_shutdown` still sets it as well, for an exit that never reaches the helper. The post-disconnect task also gets a guard it never had: it runs on a detached worker thread and reports through a 16 ms main-loop callback, so at shutdown the process exits from under the thread mid-command and the loop that would deliver the result is already stopped — neither the success nor the failure arm can log. It was being started and abandoned, an arbitrary user command interrupted halfway with nothing recording the attempt. It is skipped now, and says so, because a task that silently did not run is the same puzzle from the other side. Three comments blamed `close_all_control_sockets()` for the expected exits; it closes SSH ControlMaster sockets from `connect_shutdown`, after the children are already dead, so they name the teardown instead. One thing checked and left alone: `Session already gone; nothing to terminate` appearing once per open session is correct — the handler is keyed by session *and* hook since issue [#297](https://github.com/totoshko88/RustConn/issues/297), so one exit runs it once, and three lines meant three sessions rather than three teardown paths.

- **A second Quit stacked a second confirmation instead of raising the first** — the log from a tray quit shows `msg=Quit` at 22:40:44, `msg=Quit` again at 22:40:54, and the teardown only at 22:40:58: the first click looked ignored. It was not. It raised the close confirmation, and the second click built another one on top of it. `close_confirmation_dialog` is called from two independent places — the main window's `close_request`, and the `app.quit` action, which is where Ctrl+Q, the primary menu and the tray's Quit item all arrive — and neither asked whether the question was already on screen, so every activation constructed and presented a fresh dialog. They stack across call sites too: the window's × followed by Ctrl+Q produced one from each. Both now go through one helper that returns the dialog to wire up, or nothing when a confirmation is already up — in which case it raises the window carrying it, since an `AdwDialog` is drawn inside its host window and raising that window is what puts the question back in front of a user whose first ask went to a window sitting behind another, on another workspace, or hidden to the tray. Nothing is ever read as consent: `close_request` still stops the close and the quit action still returns without quitting, so whichever dialog is pending owns the decision. The guard holds the dialog *weakly* — a strong handle has to be released by a signal, and a guard that can outlive what it guards is how a control ends up permanently dead, which is why the click guard came back off Add Key two entries above. The quit action also had no logging whatsoever, so the log could not distinguish a raised dialog from an action that did nothing; all three outcomes now say which one happened.

### Improved

- **The four hand-rolled `bw unlock` invocations on the Secrets page are gone** — each built its own command, and between them: none passed the extended `PATH` a sandboxed `bw` needs, none passed `--nointeraction`, none had a deadline, and only two of the four implemented the verbose fallback for CLIs that do not support `--raw`. They all call `unlock_vault_blocking` now, so the fix above applies to every one of them and a fifth call site cannot reintroduce the gaps. The session key travels back as a `SecretString` instead of a bare `String` that each site re-wrapped, the GUI's duplicate copy of the session-key parser is deleted — it never sees the key as text at all now — and a failed unlock is logged, which is what previously made it indistinguishable from one that was never attempted.

- **One confirmation prompt in `rustconn-cli` instead of three** — adding the snippet prompt made a third near-copy of the same fifteen lines, and they had already drifted: `history clear` printed its question to stdout, which is why it needed an explicit flush the other two did not, and which put a prompt into whatever pipe was collecting the command's output. What kept the copies apart was not the prompt but the answer — `connection delete` treats a non-interactive stdin as a silent abort, while `history clear` and `snippet run --execute` fail and name `--force`. A `bool` cannot carry that, so each call site kept its own copy to keep its own behaviour; the shared helper reports three outcomes instead, and "nobody to ask" is never consent.

- **OpenH264 was probed again on every RDP connection** — the embedded client does not link a decoder, it `dlopen`s one, and each connection walked the whole candidate list from scratch: `OPENH264_PATH`, the well-known system paths, then the versioned-soname scan, `stat`ing each and `dlopen`ing every file that existed. On a machine with a distribution `libopenh264` the loader refuses it on the hash check, so each walk also re-emitted the same warnings — three connections in one log produced nine identical lines explaining that the library is not one of Cisco's published binaries. That is a property of the machine, it cannot change while answering the same question, and it was being repeated at the one severity users actually read, which is how a real warning gets lost. The decoder itself still cannot be shared, since each session needs its own — so what is cached is the *outcome*, the path that loaded or nothing, and a fresh decoder is built from it per session. A failure to build one from an already-proven path is now a warning rather than a silent fall back to RemoteFX, because it means a second session failed where the first succeeded. The trade is that installing a Cisco blob into a running RustConn is not picked up until restart; the answer depends on files and an environment variable that do not change under a running process, and the alternative is paying the walk forever.

- **A KeePass lookup paid for one database open that could never match** — connecting to a host whose password is not in the database took 2.9 s before the prompt appeared, and the log accounts for all of it: three `keepassxc-cli` invocations against three candidate entry paths, roughly 700 ms each. That cost is inherent, because every run of the CLI reopens the KDBX and pays its Argon2 cost again — the only thing available to reduce is the number of opens. One of the three could never have succeeded. For a connection inside a group the candidates were `RustConn/Production/nginx-01 (rdp)`, then the same without the protocol suffix, then `Production/nginx-01 (rdp)` "without the RustConn prefix" — but `KeePassHierarchy::build_entry_path` starts every path it builds at `RustConn`, so no release has ever written a grouped entry at the database root. The un-prefixed form can only match the ungrouped case, where the name is a bare entry name, and there it stays. A path the *user* chose does not come through this function at all; that is `get_password_from_kdbx_exact`, which queries it as-is. The construction moved into a pure function so the order is finally pinned by tests: exercising the loop needs a `keepassxc-cli` and a real database on the machine running the tests, so until now the sequence was only ever verified by reading it. Separately, `find_keepassxc_cli` remembers where it found the binary — every reader and writer in the module called it and each call redid the search, a PATH walk plus up to six `stat`s natively and, inside a Flatpak sandbox, an extra child process running `sh -lc 'command -v …'` on the host. Only a *successful* find is cached: the tidier `OnceLock<Option<_>>` would also remember failure, and since the Flatpak probe has a two-second budget, one slow probe would leave the rest of the session convinced KeePassXC is not installed with a restart as the only cure.

### Documentation

- **Three claims that were not true of the code** — `docs/CLI_REFERENCE.md` said `snippet run` resolves `${VARIABLE}` from Global Variables before execution. It does not, and cannot usefully: a global variable may be vault-backed and the CLI has no session to unlock it with, so a name it cannot fill is left in the command text for the shell to expand, usually to nothing. `rustconn-cli/AGENTS.md` required every user-facing string to go through `i18n()`, in a crate that has never contained a single `i18n` call — so the rule was being "followed" by nobody, and following it for one new string would have produced output in two languages. It now says the crate is English-only, that this is a gap rather than a decision, and what translating it would actually involve. And the comment listing the reachable routes to the snippet picker omitted the terminal context menu's own "Execute Snippet…" entry, which is that action's main caller.

- **The debug-logging instructions did not say what a debug log discloses** — `docs/USER_GUIDE.md` told users to run with `RUST_LOG=debug` and attach the output to a bug report, which is good advice and was incomplete: a debug log names every host, username and connection you opened, and until the password leak fixed above it also carried the RDP account password itself. The section now says so before the command rather than after it, so the warning arrives while there is still a decision to make.

### Dependencies

- **Updated**: cc 1.4.4→1.4.5, find-msvc-tools 0.1.11→0.1.12, indexmap 2.14.1→2.14.2, js-sys 0.3.104→0.3.105, syn 3.0.4→3.0.5, tinyvec 1.12.0→1.13.2, tokio-rustls 0.26.4→0.26.5, wasm-bindgen 0.2.127→0.2.128 together with its `-futures`, `-macro`, `-macro-support` and `-shared` crates, web-sys 0.3.104→0.3.105, zstd-safe 7.2.4→7.3.0, zstd-sys 2.0.16→2.1.0. Six of those fifteen are the wasm-bindgen family, and nothing this project ships compiles them: `cargo tree --invert wasm-bindgen` finds no path to it on the host target and answers "nothing to print", so they are reachable only behind a `wasm32` target and are listed here because `Cargo.lock` moved, not because a binary did. `indexmap` is the one with real reach — it arrives three separate ways, through `h2`/`hyper`/`reqwest`, through `serde_yaml_ng`, and through `toml_edit` behind the GTK macro crates. Thirteen further crates remain behind their latest release because the newer versions are semver-incompatible, so `cargo update` does not reach them; that count is unchanged by this run. `cargo deny check advisories` is clean against the new lockfile, and both `cargo-sources.json` files were regenerated from it, so the Flatpak and Flathub builds fetch the same crate set the workspace resolves to.
- **FreeRDP (Flatpak) 3.31.0 → 3.31.1** — upstream calls it a papercut release after the security run that 3.31.0 belonged to, so there is no advisory behind this one, but three of its fixes land on the exact client this project builds. RustConn compiles FreeRDP with `-DWITH_CLIENT_SDL3=ON`, and 3.31.1 fixes flickering in that client on displays with a scale factor other than 1.0, maps a set of keys the SDL client previously dropped (including the Japanese YEN/RO/EISUU/KANA keys), and makes WinPR read JSON files in binary mode — that last one is the `sdl-freerdp.json` path the bundled cJSON module exists to enable, which is how the RDP hotkey configuration is applied at all. The `sha256` was taken from upstream's own published checksum beside the tarball and matches the archive as downloaded. Only the two Flatpak manifests bundle FreeRDP; the deb and RPM depend on the distribution's copy.
- **The Flathub build had no H.264 decoder at all, so the embedded RDP client fell back to RemoteFX on every session that offered H.264** — the client does not link a decoder, it `dlopen`s one at runtime: `gfx_handler.rs` probes `/app/lib/libopenh264.so` and then scans `/app/lib` and `/app/lib64` for a versioned soname. The openh264 module supplying it was added to `packaging/flatpak/io.github.totoshko88.RustConn.yml` when the IronRDP GFX handler landed, and to neither of the other two Flatpak manifests — so the probe found nothing on Flathub, which is the build most users install, and it also found nothing in a local `flatpak-builder` run, which is the build that exists to reproduce what Flathub ships. Silent degradation to RemoteFX is the exact failure `gfx_handler.rs`'s own comments warn about for a runtime-only distro install; nobody looked for it in the packaging. All three manifests now carry the same openh264 2.6.0 module and the same comment explaining what it is for, because an H.264 module sitting next to FreeRDP in the module list reads like a FreeRDP dependency and is not one — flatpak-builder builds modules in order and openh264 comes after FreeRDP, so FreeRDP's configure step never sees it. Bundling is the only route left: freedesktop dropped `org.freedesktop.Platform.openh264` from the runtime in 2025, so there is no extension to depend on.
- **CLI downloads** — unchanged. TigerVNC is the only pinned component and its pin, 1.16.2, is upstream's current release; the other twelve resolve the latest version at runtime and never need a bump.
- **Checked and already current**: the other nine bundled Flatpak modules — fast_float 8.2.10, VTE 0.80.5 (newest in the 0.80 series the pin allows), inetutils 2.8, picocom 3.1, S-Lang 2.3.3, mc 4.8.33, waypipe 0.11.2, cJSON 1.7.19 and openh264 2.6.0 — are each at upstream's latest release, so only FreeRDP moved.

## [0.21.5] - 2026-09-03

Two features and an audit that turned into the larger half of the release.

The features: terminal colours can now follow the desktop's light/dark preference,
and a monitoring mode that fires on an actual shell event — the command finished,
with its exit code — rather than on a timing heuristic over raw output.

The audit began as a question about raising the GTK, libadwaita and VTE baselines
for 0.22.0 and produced an uncomfortable answer: the version features this project
already had were reaching almost nobody. The Flatpak passed one of them, the OBS
Debian rules computed three and then discarded two through a `make` quirk, the RPM
spec chose from a hand-written distro table that had gone stale, the release RPM and
the Homebrew formula passed none or one, and the three `gtk-4-*` features had zero
consumers the day they landed. Every channel now asks `pkg-config` what it can
actually back. Nothing here raises a baseline; it makes the existing tiers work,
which is what the 0.22.0 question was really waiting on.

### Added

- **A monitoring mode that fires when the remote shell reports a command finished** — Preferences ▸ Monitoring ▸ Default Mode and the connection editor's Activity Monitor section gain **Command finished**, alongside Off, Activity and Silence, and the per-tab **Monitor** menu cycles into it. The three existing modes are timing heuristics over raw output: "Activity" means bytes arrived after a quiet period, which cannot tell a finished build from a progress bar. This one is an actual event — VTE's `vte.shell.postexec` termprop, which the remote shell sets through OSC 133 and which carries the command's exit code, so the notification distinguishes "Command finished" from "Command failed with status 1". It is delivered through the same three channels the other modes use, including the toast landing on the window a detached session actually lives in (issue [#236](https://github.com/totoshko88/RustConn/issues/236)). Two conditions, both stated in the mode row's subtitle and the debug log: shell integration has to be sourced on the *remote* host, and the build needs VTE 0.78 (the new `vte-0-78` feature — `debian.rules` now detects the version with `pkg-config`, the way it already did for libadwaita). Consecutive commands each notify rather than being collapsed, which is deliberate: several commands finishing while you are on another tab is the case the mode exists for.

- **Tabs keep a persistent mark after a notification** — every monitoring notification now also sets `AdwTabPage:needs-attention`, so libadwaita draws a line under the tab, highlights the tab-bar edge when the tab is scrolled out of view, and puts a dot on the Tab Overview thumbnail and the tab-switcher button. This also repairs a latent gap in the existing Activity and Silence modes: their only tab-level signal was `indicator-icon`, a single slot that five different meanings write to — split-pane colour, protocol colour, offline, pinned, and the notification itself — with a priority guard between only two of them, so a notification's icon could be overwritten by `apply_protocol_color` moments later and the user would never learn anything had happened. `needs-attention` is a separate property nothing else touches. It is cleared when the tab is selected, since libadwaita does not clear it and looking at the tab is the acknowledgement.

- **Terminal colours can follow the desktop's light/dark appearance** — a new **Follow System** entry at the top of Preferences ▸ Terminal ▸ Theme. Until now the "System" colour scheme reached only the GTK chrome: `apply_color_scheme()` handed the choice to `AdwStyleManager`, but VTE's palette was looked up by name with a hardcoded `dark_theme()` fallback, and nothing connected the two. A light desktop therefore produced light window decoration around a dark terminal, with no setting that could pair them — the app never once asked libadwaita what it had resolved to, so the information was not available to the terminal at all. `TerminalTheme::resolve(name, system_dark)` in `rustconn-core` now resolves the new sentinel against that *resolved* state, which the GUI reads through `app::system_is_dark()` (`AdwStyleManager::is_dark()`, and so the settings portal on Wayland and in Flatpak). A theme picked by name is unaffected and stays put whatever the desktop does.

- **Terminals repaint when the desktop switches light/dark mid-session** — with **Follow System** selected the resolved palette changes while `settings.toml` does not, so no existing path repainted: switching the desktop to dark left every open terminal light until it was reconnected. A `notify::dark` handler on the process-wide `AdwStyleManager` now drives `TerminalNotebook::reapply_colors()` across every live terminal, layering per-connection colour overrides back on for the same reason `reapply_theme_overrides()` already does after a settings save (issue [#99](https://github.com/totoshko88/RustConn/issues/99)). It touches colours only, so unlike `apply_settings()` it cannot undo a per-session Backspace/Delete choice and needs no `reapply_erase_modes()` chaser (issue [#271](https://github.com/totoshko88/RustConn/issues/271)). Both the notebook and the app state are held weakly, mirroring the neighbouring `gtk-fontconfig-timestamp` handler: the signal lives on a manager that outlives every window, so a strong reference would leak the window and its sessions.

### Fixed

- **Every tray menu item that opens a session did nothing — Local Shell, Quick Connect and all of Recent Connections** — the item highlighted on click and then nothing happened, on KDE's StatusNotifier and on the macOS tray alike, since both feed the same dispatch in `setup_tray_handling`. Show/Hide, About and Quit worked throughout, which is what made the shape of this hard to see: they take different routes — the first calls `present()`/`set_visible()` directly, the other two activate an action on the *application*, where names are unprefixed by definition. The three broken items went through `WidgetExt::activate_action`, which resolves a name through the widget action muxer by splitting it on the first `.` to pick a group. They passed `"local-shell"`, `"quick-connect"` and `"connect"` with no prefix, so there was no group to find, the call returned `FALSE`, and `let _ =` discarded it. Recent Connections was broken twice over: even with the prefix corrected it named `connect`, which is declared with no parameter and acts on the *sidebar selection*, so the connection the user picked in the tray had nowhere to arrive and GTK would have rejected the activation for supplying a parameter at all. All three now activate on the window's own `GActionGroup` with verbatim names, matching what `window/mod.rs` already did for `connect-to` and the command palette, and the connect path uses the parameterised `connect-to` that takes a connection id. The names are constants with a test pinning both decisions, because the two activation functions cannot share one spelling and the difference is invisible at the call site. A `tracing::debug!` now records each tray message on arrival: this path had no logging whatsoever, so an item that silently failed was indistinguishable from a click that never reached the app, and the absence of log output proved neither.

- **Local shell tabs never took part in activity monitoring, in any mode** — `resolve_activity_config` looked the session's connection up with `get_connection(id)?` and gave up when there was none, which is every local shell and anything else opened without a connection record. The caller reads that as "do not monitor this session", so it returned *before* wiring `connect_command_finished`: the new Command mode could not fire on a local shell whatever the shell emitted, Activity and Silence were equally absent, and because the whole setup aborted there was no `Activity monitoring started` line either — the one signal that would have shown any of it. The global defaults exist precisely for a session with no per-connection override, so a connection-less session now takes them, and notifications use the tab's own name when there is no connection to name. The resolution moved into a free `effective_activity_config` so all three branches are covered by tests that need no display. Found while trying to exercise Command mode from a Local Shell tab, which is also why the per-tab Monitor menu's comment is corrected here: it still described the pre-Command cycle `Off → Activity → Silence → Off`, and since the menu advances one mode per activation, reaching Command from Off actually takes three — a stale list makes that look like a menu that does nothing.

- **Five icon names had been dropped by adwaita-icon-theme 50, and one of them was the new Command mode's success mark** — the tab indicator for a command that finished cleanly asked for `emblem-ok-symbolic`, which no longer exists in the theme, so it drew as a missing-image placeholder on the very notification the mode exists to deliver. There is no fallback behind it: the app forces the Adwaita theme at startup for consistent availability, so a name Adwaita has dropped resolves nowhere, and GTK reports nothing — the failure surfaces only as a broken glyph at draw time. Auditing every `*-symbolic` literal in both crates against the installed theme found four more in the same position: `emblem-synchronizing-symbolic` on all four sync indicators (Cloud Sync preferences, the sidebar, the welcome panel and the group editor), `chart-line-symbolic` on the Statistics empty state, and `utilities-system-monitor-symbolic` / `preferences-desktop-peripherals-symbolic` on the Task Manager and Device Manager RDP quick actions. All nine call sites now use names verified present in the theme — `object-select-symbolic` for the success mark, `view-refresh-symbolic` for sync, `view-list-ordered-symbolic` for statistics, `view-list-symbolic` and `input-mouse-symbolic` for the two quick actions. Theme 50 ships no chart or graph glyph at all, which is why the statistics page settles for an ordered list.

- **Closing a tab logged a warning about a teardown the app had just performed itself** — the child-exited handler asks the session manager to terminate the session whose exit it is reacting to, but on tab close the widget side has already killed the process group, so the manager answers "Session not found". That outcome was recognised as benign only when the app was shutting down, so an ordinary tab close produced `WARN Failed to terminate session` two milliseconds after the `Killed VTE child process group on tab close` line that caused it. Both routes are now recognised by the session already being absent, rather than by matching the error text — it crosses that boundary as a formatted `String`, and classifying an error by its prose is the bug fixed in the KeePassXC handling below. Any other failure still warns.

- **KeePassXC reported "Could not read the password" for a database that was open and healthy, in every non-English interface language** — `keepassxc-cli` exits 1 for "entry not found", "wrong database key" and "database unreadable" alike, so `classify_show_failure` in `rustconn-core/src/secret/status.rs` tells them apart by matching the CLI's English prose. But it is a Qt program and translates that prose, and RustConn itself exports `LANGUAGE` at startup to honour its own language setting — so with the interface in Ukrainian the CLI answered `Неможливо знайти запис із шляхом …`, no needle matched, and a merely missing entry was classified as an unreadable database. Two consequences, and the misleading wording was the smaller one: the connection got the modal "it may be locked, not logged in, or not set up on this computer" while Preferences ▸ Secrets correctly showed the backend **Ready**, and because that path returns `Err` instead of `Ok(None)` it also skipped **Also read from the encrypted file**, so a password sitting in `credentials.enc` became unreachable. The child now gets `LC_MESSAGES=C` with `LANGUAGE` cleared, set in `keepassxc_command` — the one place all three readers, the save path, the group-create path and the rename path build their command, so the four other stderr matchers in that file which had the same latent bug are fixed by the same two lines. The character encoding is deliberately left as the user had it: entry paths and the database path travel as argv and a Qt 5 build takes its codec from the locale charset, so pinning `LC_ALL=C` would have traded this bug for a worse one, mangling a non-ASCII group name or a database named `Паролі.kdbx`. Where `LC_ALL` is set its value is copied to `LC_CTYPE` before it is dropped, since it would otherwise outrank `LC_MESSAGES`.

- **openSUSE Slowroll had no package at all, because it was being handed a Rust built for a newer base than its own** — the OBS build reported `unresolvable: nothing provides libm.so.6(GLIBC_2.44)(64bit) needed by rust1.98`. Slowroll and Tumbleweed take Rust from the same place, `devel:languages:rust/openSUSE_Tumbleweed`, but their bases are not the same: Tumbleweed sits on openSUSE:Factory with glibc 2.44, while Slowroll is a deliberately delayed Factory snapshot and is on 2.43. The resolver picks the newest Rust it can see, that Rust was linked against Factory, and Slowroll cannot satisfy its glibc symbol version — which is structural rather than a one-off, since it recurs whenever Factory bumps glibc. Slowroll's own repository has `rust1.97.1`, comfortably above the 1.95 MSRV and needing only `GLIBC_2.4`, but the path could not simply be dropped: `cargo-packaging`, which provides the `%cargo_build` macro the spec uses on openSUSE, exists only in `devel:languages:rust`. Slowroll now builds with the bundled toolchain instead, the same way Fedora, Debian and Ubuntu already did, which removes the need for `rust`, `cargo` and `cargo-packaging` there and takes the target out of that coupling for good. The four sites that had to agree — the build requirements, the toolchain unpack in `%prep`, the `PATH` export in `%build` and the choice of build invocation — are now driven by a single `bundled_rust` flag rather than a distro test repeated four times, because a mismatch between them would be silent. The condition is OBS's `%_repository` macro, which names the repository being built for and is exported into the build root. Verified locally by parsing the spec in three colours: Tumbleweed still takes the `cargo_build` macro and still build-requires `cargo-packaging` and does not unpack the toolchain; Slowroll takes the plain `cargo build`, drops both requirements, keeps `alsa-devel`, and unpacks the toolchain; Fedora is unchanged. Breaking Tumbleweed to fix Slowroll was the risk, and that check was not sufficient to rule it out: the first upload broke Tumbleweed and Leap anyway, with `error: unexpected argument 'comes' found`. The cause is worth knowing, because it is a trap rather than a mistake in the logic — `cargo_build` expands to several lines, RPM expands macros in `%build` before the shell sees the text, and naming the macro in a `#` comment therefore hides only its first line while the rest becomes live script, taking the tail of the comment with it. The comment now says so, in a form that does not reproduce it. What the three-colour check does cover is the conditional logic, and that part was right; what it missed is that parsing a spec is not the same as running it. Landed on `main` after the `v0.21.5` tag, like the browser fix above, and reaches users through the separately rebuilt OBS packages.

- **No OBS Debian or Ubuntu package had ever contained the in-tab browser, because one build-dependency list was checked against another that did not have it** — for a Debian-style build, OBS assembles the chroot from the `Build-Depends` in `packaging/obs/debian.dsc`, and `dpkg-checkbuilddeps` inside that chroot then validates `packaging/obs/debian.control`. Those are two separate lists and nothing keeps them in sync. `libwebkitgtk-6.0-dev` was in neither, and `debian.control` asked for `libwebkitgtk-6.0-dev | libglib2.0-dev` — an alternative whose second branch is already present through `libgtk-4-dev`, so the check passed while WebKitGTK was never installed. `debian.rules` then probes with `pkg-config --exists webkitgtk-6.0`, that probe failed, and `web-embedded` was compiled out of a build that reported success. Measured across all three deb targets in the 0.21.5 logs: `libwebkitgtk-6.0-dev` appears zero times, `libglib2.0-dev` twice, and the detection line printed `web:` empty. Because the UI is honest about a build without the feature — it offers only the System and Custom browser modes — the symptom was an option quietly not existing rather than an error, which is why it survived two releases. It is now listed in both files, so Debian 13, Ubuntu 24.04 and Ubuntu 26.04 get the embedded browser for the first time; `rustconn.dsc`, which OBS does not build from, was synced too rather than left as a trap. Two earlier explanations of this are worth recording as wrong, since the second was written into the source and believed: that a plain entry would be unsatisfiable on repositories not shipping the package, and that apt installs the first satisfiable branch of an alternative so each repository opts in by itself. OBS does not use apt to resolve build dependencies, and the branch was never what decided this. Fixing only `debian.control` proves the mechanism: the build then fails with `dpkg-checkbuilddeps: unmet build dependencies: libwebkitgtk-6.0-dev`, which is precisely what happened when it was first attempted in 0.20.10 — and it was "fixed" then by weakening `debian.control` instead of correcting the `.dsc`. Availability was checked on all three repositories before the change, because a wrong answer breaks three working builds: 2.52.6 on Ubuntu 26.04, 2.52.6-0ubuntu0.24.04.1 on noble amd64 via universe, 2.52.6-1~deb13u1 on trixie amd64 in main, all above the 2.40 floor `webkit6-sys` asks for. A repository that genuinely lacks it should have the feature gated for it in the OBS project config; weakening the entry to an alternative again would hide the loss rather than report it. The RPMs were never affected — the spec has always required `pkgconfig(webkitgtk-6.0)` outright. One note on provenance, since this entry sits under 0.21.5 and the `v0.21.5` tag does not contain it: the change landed on `main` just after the tag was cut, and reaches users through the OBS packages, which are rebuilt from `main` separately. Nothing attached to the GitHub release is affected either way — that channel bundles WebKitGTK through `linuxdeploy` for the AppImage and declares it from the binary for the `.deb` and `.rpm`.

- **The `.deb` and `.rpm` attached to a GitHub release named three fewer libraries than the binary loads, so the `.deb` died in the dynamic linker before reaching `main()` (issue [#313](https://github.com/totoshko88/RustConn/issues/313), reported by [@philclifford](https://github.com/philclifford))** — `rustconn: error while loading shared libraries: libwebkitgtk-6.0.so.4`, on a package that installed without complaint. Both artifacts are assembled by hand in the release workflow — the `.deb` with a control file written inline and packed by `dpkg-deb --build`, the `.rpm` by `fpm` with an explicit `--depends` list — so neither ran the dependency machinery a normal build provides, and both lists fell behind the binary when `web-embedded` entered the crate's `default` features. Measured against the recursive closure of the old `Depends` (266 packages), three were unreachable: `libwebkitgtk-6.0-4`, `libjavascriptcoregtk-6.0-1` and `libasound2t64` — the last from the RDP audio feature, which had simply not bitten anyone because ALSA is usually already installed. The lists are now derived rather than remembered: the `.deb` runs `dpkg-shlibdeps` over the staged binaries, which maps each soname to its package and takes a minimum version from the symbols actually referenced, and the `.rpm` passes `--rpm-autoreqprov`, since `fpm` writes `AutoReqProv: no` unless told otherwise — the published 0.21.4 RPM carried zero soname requires. Both steps then fail the build if WebKitGTK is absent from what they derived, so this cannot regress quietly. Two things stay declared by hand because neither tool can know them: `openssh-client` is a program rather than a library, and `gtk4 >= 4.14` is the floor the `v4_14` bindings assert while no referenced symbol proves it. `dpkg-shlibdeps` reads `debian/control` — which is what makes that file matter again: a version in its `Build-Depends` raises the derived floor, and `libgtk-4-dev (>= 4.14)` there is why the package asks for `libgtk-4-1 (>= 4.14)` rather than the 4.12 the symbols alone would justify. The other channels were never affected and for a reason worth recording: OBS builds its Debian package through `dh`, so `dh_shlibdeps` derives the list, its RPM gets rpmbuild's generator, and the AppImage bundles what `linuxdeploy` finds through `ldd`. Every channel that hand-maintained a list had this bug; no channel that derived one did. The released `.deb` also gains the two `Recommends` its inline list had dropped, `picocom` among them — the helper serial connections need.

- **Five of this release's own fixes were missing from every distro changelog, and every release gate was green while they were** — the tray dispatch, the local-shell monitoring, the dropped icon names, the KeePassXC locale misread and the 0.21.4 untranslated strings all reached `CHANGELOG.md` after the release-prep commit had already derived the five other formats from it. `release.sh` checks that `debian/changelog`, both OBS changelogs, the spec's `%changelog` and the metainfo `<release>` *lead with* the release version, which they did — the header is identical whether the body has thirteen entries or eighteen. So the notes shown by `apt changelog`, by `dnf`, and as the AppStream description in GNOME Software would have omitted five user-facing fixes, including the one where a stored password became unreachable. All five are now propagated. A new gate compares commits rather than prose: if the last commit touching a derived changelog is an ancestor of the last commit touching `CHANGELOG.md`, that file was written from an older CHANGELOG and the release stops. Matching wording across five formats would have meant a fuzzy comparison that can be argued with, and a gate nobody trusts is the one that gets skipped; a derived file touched *later* is deliberately not flagged, since fixing a typo in one of them alone is legitimate.

- **The RPM on every GitHub release, and every Homebrew install on macOS, shipped the libadwaita 1.5 baseline** — the last two channels that chose features by hand. The `build-rpm` job runs in a `fedora:44` container, which carries GTK 4.20+ and libadwaita 1.9, and passed no version feature at all; the Homebrew formula wrote out `adw-1-8` and selected no GTK or VTE feature, so the Command monitoring mode could not appear on macOS whatever VTE Homebrew had installed. Both now ask `pkg-config --atleast-version`, the same comparator and the same newest-first ladders as the OBS spec, with package-qualified feature names because both invocations select two packages. Both fail safe: a missing `.pc` file yields nothing and the build is what it was. The formula asks pkg-config rather than Homebrew's formula metadata so the answer comes from the files the compiler will read — verified as syntactically valid Ruby, which is worth doing by hand because the release workflow copies this file into the tap without parsing it.

- **Every OBS Debian and Ubuntu package was built without any libadwaita feature and without the in-tab browser, and the build log said so all along** — `debian.rules` detected libadwaita and WebKitGTK correctly and then threw the answers away. In a `make` recipe a comment line without a trailing backslash ends the continuation chain, and each chain gets its own shell, so `ADW_FEATURES` and `WEB_FEATURES` — assigned above the comment that introduced the VTE block — were empty by the time the `cargo` line ran. Running the recipe as committed on a host with libadwaita 1.9.1 and WebKitGTK 6.0 installed printed `=== libadwaita  => features:  | web:  | vte 0.84 =>,vte-0-78 ===`: three empty fields and one populated one, VTE having survived only because it was detected after the last comment and so shared a shell with the build. Debian 13 and Ubuntu 26.04 therefore shipped the libadwaita 1.5 baseline — `GtkSpinner` instead of `AdwSpinner`, linked buttons instead of `AdwToggleGroup`, the legacy shortcuts dialog — and Web connections had no embedded browser on any of them. All four detections now reach cargo, verified by running the recipe with `cargo` swapped for `echo`. The comment block is now anchored at the top of the recipe with a note saying why nothing may follow it, since the failure is invisible: the build succeeds, and only that one log line ever said anything was wrong.

- **The RPM spec chose features from a hand-written distro table that had gone stale, and never enabled VTE termprops at all** — `rustconn.spec` mapped `%fedora` and `%suse_version` to a libadwaita version by hand, so it needed an edit for every new distro release, and it had no VTE branch whatsoever: the Command monitoring mode could not appear in any RPM regardless of the VTE installed. It now asks `pkg-config` the way `debian.rules` does, for libadwaita, GTK, VTE and WebKitGTK together. Both files use `--atleast-version` rather than a glob over `--modversion`, which closes a second trap in the same area: a `case` pattern of `1.8*|1.9*` does not match libadwaita `1.10`, so the first distro to ship 1.10 would have dropped to the 1.5 baseline for an unrelated reason. Checked on this host (GTK 4.22.4, libadwaita 1.9.1, VTE 0.84.0, WebKitGTK 6.0 present): both files select `adw-1-8,gtk-4-22,vte-0-78,web-embedded`, and `rpmspec --parse` still parses the spec.

- **The Flatpak and Flathub builds could not reach the Command monitoring mode, or anything else gated behind a version feature** — the manifests passed `--features adw-1-8` and nothing more, so `vte-0-78` was never enabled and the mode was hidden from the picker and inert, in the one channel most users install from, from the moment it was added. The `gtk-4-18` / `gtk-4-20` / `gtk-4-22` features were in the same position but worse: no packaging file in the repository passed any of them, so all three had zero consumers the day they landed. Both manifests now build with `adw-1-8,vte-0-78,gtk-4-22`, which the runtime backs — measured with `pkg-config` inside `org.gnome.Sdk//50` rather than taken from documentation: GTK 4.22.4, libadwaita 1.9.3, GLib 2.88.3, WebKitGTK 6.0 2.52.5, with VTE coming from the manifest's own 0.80.5 module because the runtime ships none. `vte-0-80` is deliberately left out: by the feature ladder it implies `vte-0-78` and adds only the image termprops, which `vte4` 0.10 does not bind and VTE never populates, so naming it would claim a capability nothing reads. The VTE ceiling below 0.81 is unchanged.

- **A KDE and XFCE workaround disappeared depending on which GTK the build targeted** — the one-shot clear of `gtk-application-prefer-dark-theme` before `adw::init()` was compiled out on any build with `gtk-4-20` or newer, on the reasoning that such a build links libadwaita 1.8+, which no longer warns about the legacy property. That reasoning holds for the warning and not for the behaviour: what the clear actually buys is that libadwaita starts from a property it did not set, which is true of every libadwaita. Tying it to a GTK feature meant enabling `gtk-4-22` for the Flatpak would have silently dropped the workaround on the desktops it exists for, and this project tests neither. The guard is now the condition the workaround was written for — the property being true this early, which only a desktop that set it through xsettings or `gtk-4.0/settings.ini` does, and never GNOME, which expresses the preference through `org.gnome.desktop.interface color-scheme`. The deprecation is suppressed instead, narrowly, on the builds that raise it.

- **Four declarations in the application stylesheet had never applied, and the app was hiding the reason** — `.monitoring-bar` and its compact variant set `margin-start` / `margin-end`, which GTK's CSS does not have; it uses the physical `margin-left` / `margin-right`, and the rule ten lines further down in the same file used those correctly. GTK discarded all four as unknown properties, so the monitoring bar's horizontal margins were absent in both layouts. What kept this invisible is `install_glib_css_warning_filter` in `main.rs`: it drops every GLib message containing "Theme parser" or "gtk.css", which was added for a real flood from a libadwaita stylesheet newer than the GTK parser reading it — but the predicate matches on wording, not on origin, so it silenced complaints about our own file too. Fixing the spelling makes the margins appear, which is a small visible change and the one originally intended. Two things now stop it recurring: `RUSTCONN_CSS_WARNINGS=1` disables the filter for a live session, and `scripts/check-css.sh` loads the stylesheet through the installed GTK and fails on any parser complaint — no display needed, so it runs as a CI gate and in `release.sh` alongside the i18n checks. Measured on this stack, incidentally, the flood the filter exists for no longer occurs at all: GTK 4.22.4 with libadwaita 1.9.1 emits nothing, so the filter was suppressing only our own errors.

- **The window rendered light on a dark desktop whenever the theme was set to System** — `gtk-application-prefer-dark-theme` is how `AdwStyleManager` tells GTK it has resolved the colour scheme to dark. RustConn cleared that property from a `notify::gtk-application-prefer-dark-theme` handler installed at startup, and cleared it a second time in `build_ui()`, so every time libadwaita set it the app unset it again within milliseconds — on a dark desktop the window came out light and stayed light. Both clears were written for an "xsettings race" on KDE/XFCE, where older libadwaita warned when it found the legacy property already true; the flaw is that a `notify` handler cannot tell that daemon apart from libadwaita, and libadwaita is what sets the property on every dark desktop. Reproduced on GNOME 50 with the desktop set to dark: the debug log showed `Re-cleared deprecated gtk-application-prefer-dark-theme (xsettings race)` twice during startup. Only the one-shot clear *before* `adw::init()` is kept, which is the case the workaround was genuinely for — libadwaita has no opinion at that point and sets the property itself immediately afterwards. On KDE this trades a possible libadwaita log warning for a window that matches the desktop. The startup log now carries a `dark=` field with what libadwaita actually resolved, since the stored preference never said what came out of it — which is a large part of why this went unnoticed.

- **The monitoring mode picker no longer needs editing when a mode is added** — seven hand-written index maps across four files translated between `MonitorMode` and a combo-row position, in the shape `match combo.selected() { 1 => Activity, 2 => Silence, _ => Off }`. A copy that was missed silently mapped the new mode onto Off, and nothing failed until a user noticed their setting would not stick — which is exactly what happened while adding Command mode. The list now comes from `MonitorMode::all()` and the mapping from one helper (`crate::monitor_mode`), and the visibility rules for the quiet-period and silence-timeout rows are keyed on the mode rather than on a bare index, so a future variant cannot land in a `_` arm. Two property tests were rewritten for the same reason: `mode_cycling_is_three_cycle` hard-coded the number of variants, and the `prop_oneof!` strategy listed them by hand, so every property in that file would have gone on passing without ever seeing the new mode.

- **Two strings added in 0.21.4 shipped untranslated in all 17 languages** — the **Login Timeout (seconds)** row and its subtitle reached the UI without `po/rustconn.pot` being regenerated, so the catalogues never learned about them and `scripts/check-po-complete.sh` passed by comparing against a template that was missing the strings too. Both are now translated everywhere. Worth noting for next time: `scripts/check-pot-current.sh` exists and does catch exactly this, so the gap is that it did not run, not that it is absent.

### Changed

- **Packagers can build against GTK 4.18, 4.20 and 4.22** — new `gtk-4-18`, `gtk-4-20` and `gtk-4-22` features, opt-in per channel like `adw-1-8` and `vte-0-78`, with GNOME 50 as the target platform. Worth being precise about what they do: a newer GTK's *runtime* behaviour — portals for file dialogs by default in 4.18, the accessibility work on entries and file choosers in 4.20, `GtkSvg` in 4.22 — arrives with the linked library and needs no feature at all. What the feature adds is access to API introduced in that version and deprecation warnings for what it retired, and the second half is the immediate value: enabling `gtk-4-22` surfaced exactly three deprecations, all now resolved. `gdk::Texture::for_pixbuf` became `gdk::MemoryTexture::new`, which takes the RGBA buffer the split-colour tab icons already build and drops the intermediate `GdkPixbuf` entirely — shorter than it was, and off the gdk-pixbuf image path GTK 4.20 moved away from. The other two were the remaining `gtk-application-prefer-dark-theme` accessors, where the deprecation is now suppressed on the builds that raise it rather than the code being compiled out: the clear is guarded at runtime by whether the desktop had already set the property, which only KDE and XFCE do, and that is true of every libadwaita regardless of which GTK the bindings target. No `adw-1-9` feature was added — libadwaita 1.9's automatic gains already apply through the linked library, and `AdwSidebar`, the one widget that would justify the bindings, does not fit a sidebar built on `TreeListModel` with drag-and-drop and multi-selection.

- **New installations default to the Follow System terminal theme** — `default_color_theme()` returned `"Dark"`. It is reached only when `color_theme` is absent from `settings.toml`, so this applies to a fresh install or a config written before the field existed; anyone who has ever saved Preferences has an explicit value stored and keeps it. An unrecognised theme name still falls back to Dark rather than starting to track the desktop, which is deliberate: a custom theme the user deleted is a different situation from one they asked to follow the system, and quietly conflating the two would repaint terminals nobody asked about.

### Documentation

- **`packaging/obs/README.md` no longer keeps a table of other projects' version numbers** — it listed a GTK4 and a libadwaita version for each of eight distros, needed an edit whenever any of them moved, and was wrong about half of them: it claimed GTK 4.18 for Fedora 43/44, Tumbleweed and Ubuntu 26.04, which carry 4.20 or 4.22. Both columns are gone. In their place the feature-flag table states the `pkg-config` condition that actually selects each flag, since that is now what the spec and the rules file do, and the two traps that had already fired in this area — the glob that misses libadwaita 1.10, and the comment that breaks a `make` continuation chain — are written down next to the flags rather than left to be rediscovered.

### Dependencies

- **Updated**: mio 1.2.2 → 1.2.3, open 5.4.2 → 5.4.3, toml 1.1.4 → 1.1.5. Patch
  releases behind existing requirements, taken with `cargo update`; 13 further
  dependencies are behind their latest but held by a semver requirement and were
  left alone. The GTK stack is already at the newest published bindings — gtk4
  0.11.4, libadwaita 0.9.2, vte4 0.10.0, webkit6 0.6.1 — and their version features
  are what this release finally routes to the channels that can use them.

## [0.21.4] - 2026-09-02

A documentation-and-consistency pass driven by a user-guide audit: the CLI
reference and user guide are brought back in line with the code, two CLI flags
that were silently dropped now fail loudly, and the automatic-login timeout that
was reachable only by editing `connections.toml` gains a control in the
connection editor.

### Added

- **Login timeout is now editable in the connection editor** — `login_timeout_secs` (how long the automatic-login watcher waits for the device's prompt) was fully wired through the model and group inheritance but had no UI, so it could only be set by hand-editing the `automation` section of `connections.toml`. The connection editor's **Automation** tab now carries a **Login Timeout (seconds)** row in the Automatic Login group; `0` means the built-in default (10 s), matching the stored `None`.

### Fixed

- **A SPICE connection with a stored password failed outright in Flatpak with "connection type cannot be detected from URI" (issue [#308](https://github.com/totoshko88/RustConn/issues/308))** — a regression from the 0.21.2 fix in the same issue. That fix delivers the password to `remote-viewer` through a `.vv` connection file written to `$XDG_RUNTIME_DIR`, on the assumption that the sandbox and the host see that directory at the same path. They do not: Flatpak gives the sandbox its own runtime directory and keeps it on the host under `/run/user/<uid>/.flatpak/<app-id>/xdg-run/`. virt-viewer is a desktop application in its own right and is not bundled in the manifest, so it is found on the host and launched through `flatpak-spawn --host` — where the path it was handed does not exist. `remote-viewer` then falls back to reading the argument as a URI, cannot type it, and aborts. Setting the password source back to **None** was the only way to reconnect, since without a password no file is written and the plain `spice://` URI is used.

  The path handed to a host viewer is now translated to the host's own view of the file, and the translation is *verified* with a readability probe rather than assumed, so a Flatpak that arranges its runtime directory differently is detected instead of silently producing another unusable path. When no host-visible path can be confirmed, the launch drops back to the URI: the viewer asks for the password, as it did before 0.21.2, rather than failing to connect at all. Requires no new Flatpak permissions — the existing `--talk-name=org.freedesktop.Flatpak` is what makes both the probe and the launch possible. Native and Snap installs were never affected, since there is no sandbox boundary to cross. A related leak is fixed with it: a connection file that was written but never handed over is now removed immediately, where previously a successful spawn released ownership to a viewer that had no way to read the file, leaving the password on disk for the rest of the session.

- **`--key` and `--auth-method` were silently dropped for non-SSH protocols** — `rustconn-cli add`/`update` accepted these flags for every protocol but only applied them to SSH (and, for `add`, SFTP), logging a warning and discarding them otherwise. A typo such as `-P vnc -k id_rsa` produced a connection that quietly ignored the key. Both commands now reject the flags for any protocol other than SSH and SFTP with a clear error, and `update` now also honours them for SFTP connections (previously ignored — a latent bug).

- **`--window-mode` reported SPICE as a supported protocol while ignoring it** — `Connection::supports_window_mode()` returned `true` for SPICE, but SPICE always uses an external viewer, so the setting has no observable effect. The docstring, CLI help, and reference all said "RDP and VNC only" while the code disagreed. SPICE is now excluded from `supports_window_mode()`, so the code, help text, and documentation agree.

### Documentation

- **CLI reference and user guide realigned with the code (0.21.4 audit):** version headers corrected to 0.21.4 (the CLI reference still read 0.18.11); `--audio-mode` and `--printer` RDP flags documented in the `add`/`update` tables; `web` added to the `--protocol` value list (help text and reference); the differing `--mptcp` / `--skip-port-check` semantics on `update` (which accept an explicit `true`/`false`) now explained; `sync inventory` cross-linked from the Cloud Sync subcommand table; the `--backend` help text expanded to the full list of eight backends; and the Split View shortcut table gained the missing **Ctrl+Shift+R** (Pop Pane to Tab) and **Ctrl+Shift+J** (Unsplit).

- **`.kiro` development rules audited and repaired** — `cargo-security-scan` could not report the one thing it existed for: its inline `cargo deny check advisories 2>/dev/null || cargo audit 2>/dev/null || echo 'Neither … installed'` conflated "the tool found an advisory" with "the tool is absent" (cargo-deny exits non-zero *because* it found one), while `2>/dev/null` discarded the report itself — measured at 0 bytes on stdout against 96 on stderr. The logic moved to `bin/cargo-advisory-scan.sh`, which probes with `command -v`, keeps both streams, logs to `target/cargo-advisories.log`, and invokes the bare `cargo-deny` binary so `rust-toolchain.toml` is not asked to resolve a toolchain for a lockfile parse — the reason `ci.yml` already calls `cargo-machete` directly. `scripts/check-ai-docs.sh` gained a third gate asserting every hook file has a row in `hooks-map.md`, after that map was found covering 15 of 16 (`session-baseline` had been missing since 2026-08-26 in a file whose first line promises all of them). `bash-serialization-guard`'s four block messages pointed at `/tmp` in seven places while the always-loaded `shell-environment.md` requires `target/`; they now agree, and the message's own `nohup` example carries the `timeout` the guard demands of it. `hooks-map.md` also lost a stale false-positive claim (`pgrep -f cargo` is *not* blocked — the guard needs `cargo` plus a build verb, verified by probe) and a worked example built on `rustconn/src/secret/`, a directory that does not exist. `bugfix-workflow.md` became `inclusion: auto`; the four runbooks that must stay `manual` are now listed with the reason in `docs/AI_DEVELOPMENT.md`. Removed two unreferenced tracked files from the repository root: `gitlog.txt` (a `git log` dump) and `package-lock.json` (an empty npm lockfile in a Rust workspace).

- **Contributor-facing community files added** — the four items GitHub's community-standards checklist reported as missing now exist: `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1, enforcement contact and a pointer to Security Advisories for vulnerabilities), `CONTRIBUTING.md` (development setup, the local equivalents of the twelve CI jobs, the crate-boundary and `unsafe` rules that get a PR sent back, commit and changelog conventions, and the translation workflow with its three gates), three issue forms under `.github/ISSUE_TEMPLATE/` (bug report, feature request, translation) with a `config.yml` routing vulnerabilities to Security Advisories and questions to Discussions, and `.github/PULL_REQUEST_TEMPLATE.md` mirroring the Definition of Done. Three labels were created to go with them — `i18n`, applied automatically by the translation form, plus `dependencies` and `ci`, which `.github/dependabot.yml` has been requesting since it was written without ever getting them: Dependabot drops a label that does not exist in the repository, so all sixteen of its pull requests from `#226` onwards landed unlabelled. Nothing in the build or the shipped application changes.

### Dependencies

- **Updated**: aws-lc-rs 1.18.0 → 1.18.1, aws-lc-sys 0.44.0 → 0.45.0. Semver-compatible updates from `cargo update`; `cargo check --all-targets` is clean against the refreshed lock file.

## [0.21.3] - 2026-09-01

A release spent on making the secret backends honest: a password the selected
backend refused is no longer redirected to a local file behind the user's back,
where a password went is now a question the user is asked and an answer the
connect path honours, every backend reports the same set of failures the same
way, and the Secrets page and startup banner say which backend is in force and
whether it can actually store a password. Two unrelated fixes ride along — an
embedded RDP session that a Windows 11 keepalive could kill, and network toasts on
a window with nothing open.

### Fixed

- **A password the selected backend refused was moved into the encrypted file without asking, where the connect path never looks** — saving went through `SecretManager::store_reported` with `allow_fallback` from the **Enable fallback** setting (on by default), so any primary failure walked the chain and wrote to `credentials.enc`, while connect-time resolution queries the selected backend alone. A locked Bitwarden vault therefore produced a password that was saved and, from the connection's view, missing — "Vault entry not found. You will be prompted for a password" for a password on disk the whole time. Saving now targets the selected backend only; a refusal opens a dialog naming the backend and its cause, and offers the encrypted file as a deliberate choice.

- **A password in the encrypted file was still not found at connect time, so the offer above had nowhere to put one** — the write half of the fix above is only half. `resolve_credentials_blocking` asks the selected backend for both of its lookup keys and, on a miss, returns `VaultEntryMissing` — it never consulted the encrypted file, whatever **Enable fallback** was set to, because that setting only ever reached the `SecretManager` chain and a `Vault` password source does not go through the chain. A password put in the encrypted file by the old silent redirect, or by the new dialog's **Save to This Computer**, was therefore still reported missing: the same "saved and missing at the same time", now with the user's consent on it. Both miss paths — the KeePass one and the non-KeePass one — now read the encrypted file when **Also read from the encrypted file** is on, under *the same lookup keys the write used* rather than keys derived independently, since a key computed twice is a key that can disagree. `encrypted_file_fallback_enabled` is one predicate governing the read and the write offer together, and applies the same test `SecretManager::build_from_settings` applies before appending `EncryptedFileBackend`, so the setting cannot mean one thing per password source. Only the *miss* path falls back: the `Err` arms still report `BackendNotConfigured` and consult nothing, because a store that could not be read has not said the password is absent.

- **Secret backends disagreed on how to report a failure, and three of them called an unreadable store a missing entry** — a group of fixes making every backend answer the same way:
  - *KeePassXC* turned every non-zero `keepassxc-cli` exit except `Invalid credentials`/`wrong password` into "entry not found", so a corrupt or unsupported database, a wrong `--key-file`, or a hardware key awaiting a touch all read as "no such password". The three KDBX readers in `secret/status.rs` now share `classify_show_failure`, which separates "opened, entry absent" from "did not open"; the second is an error that names the database instead of prompting.
  - *`pass`* treated any non-zero `pass show` as a miss and discarded stderr, so an uninitialised store, a missing GPG key and a locked `gpg-agent` were indistinguishable from an empty store — and it was the only backend with no readiness check (`pass --version` succeeds on an uninitialised store). Only pass's own "is not in the password store" is a miss now.
  - *KeePass at resolve time* logged a failure and fell through to the generic `SecretManager` chain, so a locked database could serve a password out of libsecret or `credentials.enc` with nothing saying the chosen database was never consulted. KeePass now answers like every other backend on that path, and the two cases are separated rather than collapsed: a database that could not be *read* is reported and nothing else is consulted, while a database that opened and does not hold the entry falls back to the encrypted file exactly as the other backends do. That distinction is the fix — what was wrong was the locked database, not the miss. The narrowing that remains is that libsecret is no longer consulted on a KeePass miss, only the encrypted file; **Copy Passwords…** in Settings ▸ Secrets is what moves entries between stores.

- **Secret status was reported to the wrong words, the wrong store, or not at all** — a cluster of Secrets-page and banner fixes:
  - "Secret Backend Not Configured" was shown to people whose backend was merely locked — the dialog discarded the `required_backend` the result carried. It now names the backend and says the password could not be *read*.
  - The startup banner was built from `BackendAvailability` (three keyring-shaped variants), so a Bitwarden vault that was not logged in came out as "keyring client is not installed" and printed Rust variant names like `MacOsKeychain`. It now uses the same readiness verdict the Secrets page shows, with `SecretBackendType::display_name` for the name.
  - The Passbolt and Pass status lines stayed on "Detecting…" forever, because they set a label only when the probe returned a status — and a missing CLI returns nothing.
  - The `pass` readiness probe read the ambient `$PASSWORD_STORE_DIR` while the backend uses the *configured* directory, so a custom store was reported "Not initialized" while healthy. The probe now resolves the directory in the backend's own order, keyed into the detection cache.

- **The Bitwarden master password was demoted out of its `Zeroizing` wrapper before use** — the unlock handler wrapped it to be wiped on drop, then made two bare `String` copies that moved into closures and dropped unwiped, leaving it in freed heap. Both copies stay wrapped now. The handler also logged the resulting session key's length — bruteforce metadata it had declined to log two statements earlier for the password; the field is gone.

- **An embedded RDP session against a Windows 11 host could be killed by the server's own keepalive** — `ironrdp-session` 0.11 decodes *every* PDU on the MCS message channel as an auto-detect request, but that channel also carries Heartbeat PDUs, which Windows 11 sends every one to two seconds. The first failed the security-header check, came back as a decode error, and RustConn turned any `ActiveStage::process` error into a dead session — so it ended seconds after connecting as a plain "Session error". Reachable on every embedded RDP connection to such a host, since RustConn always registers `drdynvc` and one static channel is enough for Windows to allocate a real message channel. A PDU the session layer cannot decode is now survivable *when it arrived on the message channel* (auto-detect is advisory; a client implementing neither Heartbeat nor multitransport need not answer). The guard re-derives the channel from the frame rather than matching on the error kind, so a failure to parse the MCS framing itself stays fatal; five tests pin the narrowness. Credit to issue [#262](https://github.com/totoshko88/RustConn/issues/262). Temporary: [IronRDP#1814](https://github.com/Devolutions/IronRDP/pull/1814) merged the real fix upstream but is unpublished, so the guard carries the gap until the next `ironrdp-session` bump. Still open and not covered: [IronRDP#1629](https://github.com/Devolutions/IronRDP/issues/1629), an auto-detect request during the licensing exchange, which happens in `ironrdp-connector` before this session loop exists.

- **An idle window with no connections showed network warnings during a Wi-Fi flap** — a four-signal flap produced three toasts in two seconds on a window with nothing open, the first reading "Network disconnected — active sessions may be interrupted" when there were none. The debounce is right and unchanged; what was wrong is that the `Down` and limited-connectivity paths announced without checking whether anything was at stake, where the `Up` path already did. All three now go through `should_announce`, counted through `open_session_count` (a detached window or external viewer counts, not just a tab). The socket sweeps still run on every transition, since a stale `ControlMaster` is worth closing regardless. This also silences the launch-time "network down" toast that affects nobody.

### Changed

- **"Enable fallback" is now "Also read from the encrypted file", and governs reads only** — the description said "Use libsecret if the primary backend is unavailable", naming a store that stopped being the fallback when `build_from_settings` moved to `EncryptedFileBackend` (#201) and was never updated. It now names the encrypted file, is no longer platform-conditional, and says what it does: look there as well when resolving a password, so entries saved before a backend change keep working. Two caveats worth stating, since the old label's problem was a description that had drifted from the code. Making the new one true took a code change, not just a rewording — the `Vault` password source never reached the chain the setting governed, and that is the second **Fixed** entry above. And where a *write* goes is no longer part of this setting at all, which is the first.

- **The Secrets page reports whether the selected backend can actually store a password** — one **Status** line for every backend, replacing an Availability row shown only for the system keyring (four of eight backends had none). It distinguishes "ready", "needs something from you" (not logged in, locked, no database chosen, no keyring answering) and "not installed" — the distinction the **Version** row cannot make, since a `bw` that is installed but not logged in has a version. Selecting a not-ready backend is no longer accepted in silence: a new `win.recheck-secret-backend` action re-checks after settings are saved, so the banner reflects the backend in force rather than the one selected at launch.

- **`SecretManager::retrieve` logs when a fallback backend answered instead of the selected one** — "my backend works" and "my backend is broken and everything is quietly coming from a local file" were indistinguishable in a log. A read served by anything other than the first entry in the chain now warns with both backend ids. A field on the return type was considered and dropped: nothing in the interface is ready to surface it, and the remaining chain reads go through `CredentialResolver`, which would have to carry it too.

- **The `pass` row in the backend selector is spelled `pass`** — `display_name()` returned "Pass", so the row read as a product by that name while its own description, its status label and the documentation all say `pass`, which is the program's actual name.

- **The startup backend check no longer probes seven command-line clients to answer a question that has no probe in it** — the check now shares the Secrets page's readiness verdict, and that verdict comes from `detect_secret_backends`, which spawns a `--version` or `status` call per backend with a 5-second ceiling. The two file backends answer from local state alone, so `backend_needs_probe` skips the probe for them entirely; two tests pin it against `backend_readiness` so a backend cannot be exempted from a probe it actually needs, or made to wait for one it does not.

### Documentation

- **`docs/BITWARDEN_SETUP.md` told Flatpak users to log in somewhere RustConn does not read (issue [#312](https://github.com/totoshko88/RustConn/issues/312))** — the guide said to run `bw login` in a Local Shell tab, but that tab is a *host* shell spawned through `flatpak-spawn --host`, which does not carry the sandbox environment across. `bw` resolves its state directory from `$XDG_CONFIG_HOME`, which Flatpak sets per-application, so `bw` run by RustConn and `bw` run from a Local Shell tab used different `data.json` files — the terminal reported the vault unlocked while Settings → Secrets said "You are not logged in." The guide gains a "Where the CLI keeps its login state" section, two Flatpak login recipes that write where RustConn reads (run `bw` inside the sandbox, or pin `BITWARDENCLI_APPDATA_DIR`), a `bw status` check, and a troubleshooting entry. Snap and native installs were never affected and are called out as such. Also removed a contradiction in Step 3, which printed `export BW_SESSION=…` and then said session keys are automatic — the export cannot work, since RustConn reads `BW_SESSION` from its own environment fixed at launch.

- **`docs/BUILD.md` records that the headless crates run on Android under Termux** — `rustconn-core` and `rustconn-cli` build and run there with default features off (issue [#129](https://github.com/totoshko88/RustConn/issues/129)). Known-to-work rather than supported, since no CI job covers it: what works is connection management, not opening sessions — the GTK4 GUI does not target Android.

- **`docs/ARCHITECTURE.md`, `docs/USER_GUIDE.md` and `docs/BITWARDEN_SETUP.md` updated for the read/write asymmetry** — the architecture section described a single "fallback chain" and quoted a `get_available_backend` helper that does not exist; it now states why reads may fall back and writes may not. The user guide's Secrets page listing gained the **Status** row and lost the claim that fallback means libsecret. The Bitwarden guide gained a Status table mapping each state to what to do, and its "Enable fallback" section was rewritten with a note about how earlier releases behaved, since anyone with passwords the old silent redirect put in the encrypted file needs to know where to look.

### Dependencies

- **Updated**: libredox 0.1.21 → 0.1.23, ppmd-rust 1.4.0 → 1.4.1, smallvec 1.15.2 → 1.16.0. All semver-compatible transitive updates from `cargo update`; `cargo check --all-targets` is clean against the refreshed lock file.

## [0.21.2] - 2026-08-31

### Fixed

- **SSH through a jump host could hang at the target password prompt when the proxy step was slow (issue [#301](https://github.com/totoshko88/RustConn/issues/301))** — the target host's password was typed by a watcher that read the terminal and recognised the `password:` prompt, and that watcher gave up after a fixed 10-second window measured from the moment SSH was spawned. The whole `ProxyCommand` handshake to the bastion ate into the same budget, so when the proxy step ran long the watcher had already stopped by the time the target prompt appeared and the session sat at `password:` doing nothing.

  The deadline is not the fix. SSH no longer types the target password into the terminal at all: it is handed to OpenSSH itself through `SSH_ASKPASS`, which has no deadline to miss because OpenSSH asks the helper exactly when it needs the credential, however long the proxy chain took to get there. See **Changed** below for the shape of that mechanism.

  The terminal watcher is still what logs Telnet and Serial in, and its deadline was the real defect there too — a device that spends a long time on its banner could outlast a window that started at spawn. It is now measured from the last terminal activity: as long as output keeps arriving the watcher keeps waiting, and it gives up only after the terminal has been genuinely idle for the timeout (default 10 s, still overridable per connection or group), with a 120-second absolute ceiling so a device printing a heartbeat cannot keep it alive forever.

- **A SPICE connection asked for the password every time, whatever the password source (issue [#308](https://github.com/totoshko88/RustConn/issues/308))** — SPICE runs in an external viewer (`remote-viewer`), and RustConn resolved and cached the connection's password at connect time but then never handed it to the viewer, which prompted on its own. The password source only affected how the secret was resolved, so changing it made no difference — the launch discarded the secret regardless. RustConn now passes it through a virt-viewer `.vv` connection file, mirroring how RDP delivers a password: written to a mode-0600 file in the user's runtime directory, carrying `delete-this-file=1` so the viewer removes it after reading, and never placed on the command line or in the environment where another process could read it. The file is used for host/port and TLS connections; a unix-socket SPICE connection, which the `.vv` format cannot address and which needs no password in practice, is unchanged and still relies on the viewer. Connection-independent options (USB redirection, shared folders, window title) continue to be passed as flags alongside the file.

- **The embedded web browser lost its login on every restart (issue [#309](https://github.com/totoshko88/RustConn/issues/309))** — a Web connection opened in Embedded mode asked for credentials again after RustConn was closed and reopened, while the same connection in System mode stayed logged in. Each connection already gets a persistent `NetworkSession` with its own data directory, which makes the website data manager persistent — but WebKitGTK keeps cookies in a separate subsystem that stays in memory only until `set_persistent_storage` is called on the cookie manager, and that call was missing. Cookies are now written to `cookies.sqlite` in the connection's data directory, so a session survives a restart. Existing connections gain this automatically the next time they are opened; there is nothing to migrate, since there were no persisted cookies before.

- **A new Local Shell tab in Flatpak could open at the wrong size when the window had changed size since the last tab (issue [#294](https://github.com/totoshko88/RustConn/issues/294))** — the host shell runs through `flatpak-spawn --host` and `script`, which copies the window size exactly once at startup and never sees a later `SIGWINCH` (`flatpak-spawn` does not forward it). The spawn already waited for the terminal to be laid out before starting, but the test was a non-zero pixel allocation, and that arrives one frame before VTE recomputes its row and column count for that allocation. A tab opened while the window was a different size than the previous one therefore spawned on the very first poll tick with the *previous* grid still in place, freezing the host shell at that stale size (the reported 18×80). The spawn now waits until the grid has *settled* — a non-zero allocation plus two consecutive polls reporting the same row/column count — so `script` inherits the size the user is actually looking at. SSH and other sessions were never affected: they run on RustConn's own PTY, which tracks resizes normally.

### Changed

- **A stored SSH password is now given to OpenSSH instead of typed into the terminal** — this replaces the auto-fill watcher that has answered SSH password prompts since 0.17.5, and it is the mechanism behind the #301 fix above. The old arrangement worked by reading terminal output and calling `feed_child`, which meant RustConn had to guess, from text, both *what* was being asked and *who* was asking. That guess is what issues #191, #203, #254 and #301 were all variations of.

  What happens now: the credential is written to a mode-0600 file in `$XDG_RUNTIME_DIR` and only its *path* is placed in the SSH process environment, so the secret is no longer visible in `/proc/<pid>/environ`. An `SSH_ASKPASS` helper opens that file, unlinks it, and prints it — once. The helper answers only the prompt OpenSSH generates itself for the `password` method (`<user>@<host>'s password: `) and exits without printing for a private-key passphrase, a host-key confirmation, a keyboard-interactive or OTP challenge, or a password-change prompt. Bastion passwords moved to the same file-path indirection (the environment variable is `_RC_JH_PW_FILE` where it used to be `_RC_JH_PW`), and every generated proxy hop now starts with an `env` boundary that blanks the credentials that hop must not see. Files are removed when the session's child exits, and on any spawn that fails.

  Nothing about *which* authentication method runs is pinned, and that is deliberate: what keeps the credential away from the wrong question is the helper declining to answer it, not a narrowed method list. A method the helper declines simply fails without consuming the secret file, and OpenSSH moves on to the next one — so a key inherited from a group is still offered and still wins when it works, and a server that tries keyboard-interactive before `password` still reaches the prompt the helper answers. The only two options added are about the shape of the launch rather than the method: `NumberOfPasswordPrompts=1`, which keeps the one-shot posture that stops a wrong stored password from walking an account into a lockout, and `StrictHostKeyChecking=accept-new` unless the connection sets its own, so a forced helper is never handed a host-key question. A *changed* host key is still refused.

  Where askpass steps aside, the terminal watcher still does the typing, exactly as it did before: an authentication method whose point is an interactive second factor (security key, keyboard-interactive), a PKCS#11 token, a custom option that redirects authentication or routing, or a proxy route RustConn did not build. That last one is detected by asking `ssh -G`, which resolves `~/.ssh/config` the way a real connection would — a bastion declared only there would otherwise spawn a nested `ssh` that inherits the helper and gets asked for a password with the very shape the helper answers, which would hand the target's credential to the bastion. Costs one short-lived `ssh -G` (~4 ms, no network) on connections that are otherwise eligible. The per-connection and per-group **expected password prompt** override from issue #254 and the login timeout apply to the watcher, so they keep working for every connection that uses it.

### Dependencies

- **Updated**: gtk-rs stack (glib, gio, cairo-rs, pango, gdk-pixbuf-sys, graphene-sys and their `-sys` crates) 0.22.8 → 0.22.9, aes 0.9.2 → 0.9.3, hyper 1.11.0 → 1.11.1, indexmap 2.14.0 → 2.14.1; system-deps 9.0.0 pulled in transitively. All are semver-compatible patch updates from `cargo update`.

## [0.21.1] - 2026-08-28

### Fixed

- **Connections that authenticate with a key waited on a vault lookup before every connect, then warned about the password they never wanted (issue [#307](https://github.com/totoshko88/RustConn/issues/307))** — a user whose hosts are all key-authenticated, with no stored passwords at all, paid a full vault round trip each time. Against the Bitwarden CLI that is several seconds, because it decrypts on every read, and the answer was the same every time: nothing here. The wait then ended in "Vault entry not found. You will be prompted for a password", which is not true of a key-authenticated connection — nothing prompts for an account password there — so the reporter reasonably read it as a fault rather than a notice.

  Two changes, and neither touches which credential a connection uses. The empty answer is now remembered for five minutes, so a burst of connections pays for the lookup once instead of once each; and the notice is only shown when the connection is actually set up to want a password.

  Both are deliberately kept away from the decision of *whether* a password is needed. The cache sits after that decision and only skips repeating a question already asked, so a stale record can cost a connection the password it would have found — bounded by the five minutes, and cleared immediately when one is saved, when an edit or group move changes the lookup key, and after a bulk transfer between backends — but it can never hand a connection the wrong one. The notice predicate feeds nothing but the toast.

  Worth recording why the "does this want a password" test is keyed on the key configuration and not on the authentication method: `SshAuthMethod` defaults to `Password`, so a connection imported from an `ssh_config` or created without touching that dropdown reads as password auth however it actually connects. A check on the method alone would have kept showing the notice to precisely the people who complained about it. A key path, an agent key source, or an explicit key method — any of the three is enough.

  Not fixed here, and the reason the wait existed at all: the fast path that skips the vault entirely still keys on the connection's Password Source, which is a stored setting rather than a description of how the connection authenticates. A connection migrated from an older release carries `Vault` because that is what `keyring` became, not because anyone chose it. Deriving that from the connection is the better fix and a larger one; setting Password Source to None on such a connection is the workaround today.

- **Local Shell failed to open in Flatpak on a host without `script` (issue [#306](https://github.com/totoshko88/RustConn/issues/306))** — the button reported `Failed to start command: script` and nothing opened. The Local Shell path wraps the host shell in `script` (util-linux) because that is what allocates a real PTY on the host and so gives the shell job control: Ctrl-Z, `fg`, `bg`. It called it unconditionally, and `script` is not present everywhere. Fedora moved the binary out of `util-linux` into a package of its own, `util-linux-script`, in F42, so a host that has `util-linux-core` — which is what a minimal install carries — does not have `script`, and installing `util-linux-core` does not help.

  The host is now probed and the shell is run directly when `script` is missing. Job control is lost in that fallback, which is a real downgrade and worth stating rather than glossing: a shell that opens without Ctrl-Z is still better than a button that does nothing. On a host that does have `script`, nothing changes.

  The same probe already existed one module away, on the Generic-command path in `window/protocols.rs`, added when a missing `script` broke that path. It was never applied here, so the two copies had drifted and only one of them survived a host without the binary — which is why a Generic command worked on exactly the systems where Local Shell did not.

- **Quitting from the tray put the confirmation dialog on a window the user was not looking at** — quitting with sessions still open asks for confirmation, which is correct, and 0.20.11 made the tray route present the window first so that dialog could not be drawn on a tray-hidden surface and lost. The guard it used was `!win.is_visible()`, which answers a different question than the one that matters: a window can be visible and still not be the window in front of the user, behind others, on another workspace, or simply unfocused. In each of those cases the present was skipped, the confirmation was drawn on a surface nobody was watching, and quitting from the tray meant going to look for it. The window is now presented whenever there is something to confirm; on an already-visible window that raises and focuses it, which is what asking to quit should do. Nothing is touched when there is nothing to confirm, which is what the guard was really there to prevent.

- **The embedded browser was missing from every OBS `.deb`** — Web connections there offered only System and Custom, with no Embedded row and nothing to say why. The interface was describing the build accurately: `debian.rules` decides the feature for itself with `pkg-config --exists webkitgtk-6.0`, that probe can only succeed if `libwebkitgtk-6.0-dev` is installed in the build chroot, and nothing installed it. The package was named plainly in `Build-Depends` until 0.20.10, which made all three deb targets unsatisfiable the first time the file actually reached OBS, and the answer at the time was to drop it altogether — trading a build that failed loudly for one that succeeded while quietly producing a binary without the feature, which is why this went unreported for a whole minor release.

  It is now requested as `libwebkitgtk-6.0-dev | libglib2.0-dev`. Apt installs the first branch it can satisfy, so a repository that ships WebKitGTK 6.0 opts in and one that does not resolves to a package `libgtk-4-dev` already pulls in, at no cost. All three current targets — xUbuntu_24.04, xUbuntu_26.04 and Debian_13 — do ship it, so all three gain the embedded browser; the second branch is there so that adding an older repository later degrades instead of breaking the build. Runtime linkage needs no entry of its own, because `${shlibs:Depends}` derives it from the binary exactly when the feature was compiled in.

  Neither of the other two package families was affected, and it is worth saying which so the fix is not looked for there: the spec guards its WebKit `BuildRequires` with the same condition that selects the feature, so Tumbleweed and Fedora 43+ RPMs always had it; and the `.deb` attached to the GitHub release builds with default features, which include `web-embedded`, against a workflow that installs the dev package.

## [0.21.0] - 2026-08-28

A minor version spent on what twelve patch releases in fourteen days left
behind: waits with no deadline, a bastion setting that was stored and then
ignored at connect time, and nine dependency advisories reported for code that
was never compiled. Two of the fixes are requirement bumps a patch release would
have refused, and one is a breaking `rustconn-core` signature.

### Added

- **CI builds and tests the macOS-specific code** — a `macos-sys` job clippies and tests the four `rustconn-*-sys` crates on a macOS runner. Every other job in the matrix is Linux, and until now nothing outside the maintainer's own machine had ever compiled the macOS side. That had already cost something: the `rustconn-pty-sys` contract test proving the `pre_exec` hook runs in the forked child — the thing that makes SSH password prompts work ([#175](https://github.com/totoshko88/RustConn/issues/175)) — had never passed there, because it accepted `ENOTTY` and `EPERM` while macOS answers `ENODEV`, and it took until 0.20.11 for anyone to notice. A test guarding `unsafe` that no job executes is a test whose state nobody knows.

  Scoped to the four helper crates deliberately: they hold every `unsafe` block in the repository, and between them they need one Homebrew package. Building the GUI there would mean gtk4, libadwaita, vte3 and WebKitGTK from Homebrew — and `web-embedded`, a *default* feature, cannot build on macOS at all, which is worth fixing before the scope grows.

### Fixed

- **A Jump Host set on a group or in Preferences → Network was ignored at connect time ([#301](https://github.com/totoshko88/RustConn/issues/301))** — the three-tier resolver shipped in 0.20.9 and is correct; what was never wired is the launchers. A bastion picked from the dropdown above connection level was stored, shown in the editor as inherited, synced between machines — and then dropped at the moment it was needed. The 0.20.9 notes said inheritance "reads the same for every protocol", which was true of neither the picker for SSH nor of RDP, VNC and SPICE at all.

  Seven call sites now resolve the first hop through one function: the SSH terminal, the RDP, VNC and SPICE tunnel gates, two `has_jump_host` guards that drive the sidebar status and monitoring, and the check deciding whether the *bastion's* own password may be prompted for. That last one matters most — it was reading the raw field, so it answered "no bastion" for an inherited one and suppressed nothing, which is how the target's password could be fed to the bastion prompt ([#191](https://github.com/totoshko88/RustConn/issues/191)).

  Precedence, in order: the connection's own Jump Host field wins unconditionally; otherwise `Network Mode: Direct` refuses a bastion outright; otherwise the nearest group in the chain that has one; otherwise the global setting. Only the *first* hop resolves this way — a bastion's own bastion stays a property of that bastion, not something the target's group can redirect.

  **What this does not fix**: a *text* ProxyJump at group or global level still does not reach RDP, VNC or SPICE. Those protocols reach a bastion by opening an SSH tunnel, and a tunnel needs a saved connection with its own port, identity file and credentials — a `user@host:port` string has none of that. For them the bastion must be a picked connection, which now inherits.

- **The Secrets page in Settings could stay empty instead of listing the password managers it found** — each manager is probed by running its command-line tool, the probes run together, and the page waited for the slowest, so one tool that never answered held all of them. The three most likely to do that are the three that reach out over the network or wait on a biometric prompt: `bw status`, `op whoami`, `passbolt list`. Every probe now gives up after five seconds and reports that manager as unavailable, which is already how an errored probe reads and is the honest answer when a tool will not say otherwise.

- **A KeePass operation could freeze the window with no way out** — all twelve `keepassxc-cli` call sites waited indefinitely, in a project that already bounds a credential resolution at 30 s and every Secret Service call at 10 s. A database on a network share that had gone away, or one locked by another program, left the calling thread never coming back. Reads and probes now give up after ten seconds and writes after thirty: the consequence differs rather than the expected duration, since a kill delivered mid-write lands inside a KDBX rewrite. The timeout message names heavy key-derivation settings as a cause alongside a locked file, because that cost is paid on *every* invocation — `keepassxc-cli` reopens the database each run, and a single save is four of them.

- **Printing from an RDP session could stall the session itself** — RustConn asks the local print system for the queue list when the connection opens and sends the document through it when the guest prints, on the connection's own thread, with no limit on any of it. Bounded at two seconds, where losing the answer costs the printer list rather than the session. One limitation is now stated rather than implied: the document is written to `lp`'s stdin *before* the wait, so a page larger than the pipe buffer blocks in that write, which no deadline on the wait can cover.

- **Nine advisory warnings came from code that was never compiled** — the macOS tray took `tray-icon` and `muda` with their default features, which pull `libappindicator`, `gtk` 0.18 and the rest of the GTK3 binding stack, unmaintained since 2024, plus `libxdo`, `x11` and their build machinery: 39 crates. None of it was ever built — those dependencies are already Linux-gated inside the two crates, and the crates are only enabled by the `tray-macos` feature that the macOS bundle alone passes, since the Linux tray is `ksni` and pure D-Bus. But `cargo audit` reads `Cargo.lock`, which is target-agnostic, so the advisories were reported on every platform regardless. That is also why target-gating the declarations would have fixed nothing and turning the feature off does: the feature is what pulled the optional dependency into the graph.

  Measured before and after on the same tree: ten warnings down to one. The nine that went are RUSTSEC-2024-0412, -0413, -0415, -0416, -0418, -0419, -0420 (GTK3 bindings), -0429 (`glib` unsoundness) and -0370 (`proc-macro-error`). The one that remains is RUSTSEC-2023-0089, `atomic-polyfill`, which arrives by another route. Worth stating plainly because it was assumed otherwise while planning this release: **none of the nine was ever in an allow-list.** `.cargo/audit.toml` and `deny.toml` ignore exactly one advisory between them and still do — the warnings were being counted and tolerated rather than named and accepted.

- **A Flatpak build could be stopped by any one of seven download hosts having a bad day** — and one was: the 0.20.11 release job failed twice with `Failed to download sources: module inetutils`, because `ftp.gnu.org` was in the middle of an outage. inetutils is the third of twelve modules, so nothing was compiled; the deb, RPM and AppImage jobs all succeeded, and the Flatpak job alone held back the GitHub release, OBS, Snap and Homebrew, which depend on it. Not one source in the three manifests used `mirror-urls`, which is the flatpak-builder feature for exactly this.

  inetutils now leads with `ftpmirror.gnu.org` — GNU's own redirector, and the primary rather than a fallback, because while `ftp.gnu.org` is down a primary pointing at it costs every build the full 60-second timeout first — with `ftp.gnu.org` and `mirrors.kernel.org` behind it. `mc` keeps its canonical URL and gains `ftp.osuosl.org`, the host actually behind the Midnight Commander FTP and HTTPS where the primary is plain HTTP. Both mirrors were verified to serve a byte-identical tarball before being trusted. `slang` deliberately gets none: jedsoft.org has no alternative that could be verified, and an unverified mirror trades a download failure for a checksum failure.

- **Both Flatpak `cargo-sources.json` manifests were three lock-file changes behind** — `Cargo.lock` went 733 → 695 crates across the tray feature change and the quick-xml and argon2 bumps, and neither manifest was regenerated, so both still listed 1455 entries including 15 GTK3 crates that were no longer in the graph. A sources file behind the lock makes `flatpak-builder` vendor crates the build then cannot find. Regenerated to 1377 entries, identical in both manifests.

- **A translatable string could ship untranslated with every check reporting the catalogues complete** — every i18n check read the *committed* template, so a string that was never extracted was invisible to all of them: the file was listed as a source, the literal was extractable, and all 17 catalogues reported 100% — complete with respect to a template that was itself incomplete, and the string rendered in English in every locale. It had happened in three releases running, eighteen strings between them, each time with a note saying the gap was real and would outlive the fix. The template is now regenerated and compared against the sources, before a release and in CI.

### Changed

- **`KeePassStatus::save_password_to_kdbx` takes the entry password as `&SecretString` instead of `&str`** — a breaking change to a public `rustconn-core` signature, which is why it waited for a minor version. No caller was leaking today: all seven passed either a borrow of a `Zeroizing` buffer or an `expose_secret()` directly. The `&str` simply did not *stop* the next one handing it a bare `String`, and this is the API a GUI, a CLI and any other consumer of the crate reach for when they store a password.

  Two of the call sites get strictly better out of it: they are synchronous, so they now borrow the caller's secret and their intermediate plaintext copy is gone entirely. Three pass the secret into a `move` closure on another thread and need an owned value; `SecretString` is `SecretBox<str>` and `str` is not `Clone`, so they share it through an `Arc` rather than cloning — a clone would duplicate the `Box<str>`, which is the second plaintext the change is trying to remove.

### Documentation

- **The VTE version ceiling in the Flatpak manifests now says why it exists, or admits that nobody wrote it down** — three release notes across the 0.20 series stated that VTE "stays pinned below 0.81 by design", and not one of them, nor any comment or commit message, said what the design was. An assertion repeated until it looks settled is not a reason. What is recorded instead is the evidence: `vte4` is at 0.10 with its `v0_76` feature selected, so the API in use is VTE 0.76's and 0.84 still provides it; the maintainer's macOS build runs against Homebrew's 0.84.1 daily, which is the only empirical datum and points at the ceiling being unnecessary. What is *not* known is how a bundled 0.81+ behaves inside the GNOME 50 runtime, and whether the `mc` SGR-mouse workaround beside it is sensitive to the version — those two are why it is still pinned rather than lifted. The steps to lift it, and the instruction to write down which of them failed if it cannot be, are in the manifest.

- **`adw-1-6` cannot be retired yet, and the reason is now beside the feature** — its comment invites "retire this once no supported target is below 1.6", which prompts the question every release, and the snap's `core24` platform is usually cited as the blocker, making a `core26` GNOME extension look like the thing to wait for. It is not: the binding constraint is **Ubuntu 24.04 LTS**, which ships libadwaita 1.5.0, is the baseline tier in `packaging/obs/README.md`, and is supported to 2029. Checking on core26 answers nothing while 24.04 is a target.

### Dependencies

- **argon2 0.5 → 0.6 — the key derivation behind the encrypted credential stores, and the question was whether existing files still open.** Deferred from 0.20.11 for that reason. They do, and it is now proved rather than assumed: the derivation is byte-identical, so no migration is needed and no stored credential is affected. Nothing in RustConn's own code had to change, since the algorithm, version and cost parameters were already passed explicitly.

  Proving it needed a fixture that did not exist. `derive_settings_key` has long had a captured blob that fails to decrypt if the format moves; `derive_passphrase_key`, which is what opens the *portable* store, had none. That gap mattered at exactly this moment: the round-trip tests create a store and open it with the same build, so they pass whether or not the derivation changed, while every store already on disk would have become unopenable with nothing going red. A fixed Argon2id vector captured under 0.5.3 is now pinned and asserted to still hold, alongside a second test pinning the default cost parameters. Cheap parameters on purpose — Argon2id does not special-case cost, so a change in output shows up at 1 MiB as clearly as at 64 MiB. Brings `password-hash` 0.6.1, `blake2` 0.11 and `phc` 0.6.1; drops `rand_core` 0.6.4.

- **quick-xml 0.41 → 0.42** — deferred from 0.20.11 because it needed a requirement widened, which a patch release would not take. The break is that decoding moved from the caller to the parser: `QName` and the three text event types now dereference to `str`, and `Attribute::value` is a `Cow<str>` rather than `Cow<[u8]>`. Every `String::from_utf8_lossy` and every `.decode()` on the reader path in the libvirt and RoyalTS importers was therefore redundant rather than merely renamed, so twelve call sites got shorter. Semantics were preserved deliberately at the one place they could have drifted — the text event still yields the event's own content rather than the newline-normalised form, which is what the byte-level code did. All 31 importer tests pass unchanged.

- **Updated**: cpufeatures 0.3.0 → 0.3.1, flate2 1.1.9 → 1.1.10 (bringing miniz_oxide 0.9.1). `cargo audit` reports one allowed warning across 695 dependencies — RUSTSEC-2023-0089, `atomic-polyfill`, unmaintained and reached transitively; `cargo deny check` is clean, and `cargo machete` finds no unused dependency. Every auto-resolving CLI download endpoint answered: kubectl 1.37.0, Tailscale 1.102.3, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.150.2, Bitwarden CLI 2026.8.0, 1Password CLI 2.39.0, and TigerVNC — the only pinned one — is current at 1.16.2.

## [0.20.11] - 2026-08-27

### Fixed

- **A telnet session left its process running after RustConn was closed (issue [#304](https://github.com/totoshko88/RustConn/issues/304))** — closing the window, pressing Ctrl+Q or quitting from the tray with session tabs still open ended the application and left every `telnet`, `ssh`, `picocom` and shell child alive. The device on the other end therefore stayed occupied, which for the reporter's serial-console server — one connection per port — meant the next attempt was refused until the orphan was killed by hand. The per-tab kill that closed issue [#172](https://github.com/totoshko88/RustConn/issues/172) works and always has; it lives in the `close-page` handler, and *quitting never closes the pages*. Every exit path went from "save state" straight to "proceed", tearing down some of external viewers, tunnels and detached windows — and in no case the children of the tabs themselves. Nothing downstream covered for it either: RustConn opens the PTY itself now, so dropping the notebook closes the master and does no more, and `telnet` is precisely the client that ignores the `SIGHUP` that follows. A detached session was killed on quit and an identical tabbed one was not, which is the shape of the bug in one sentence. The kill is now a shared function with two escalations — a GLib timeout while the application is running, and a blocking one on the way out, because a timeout registered during shutdown never fires, and that is why the omission survived a fix that looked complete. Quitting with sessions open costs one 100 ms grace period, once, however many are open; quitting with none costs nothing.

- **Quitting from the tray tore nothing down at all (issue [#304](https://github.com/totoshko88/RustConn/issues/304), and [#209](https://github.com/totoshko88/RustConn/issues/209) / [#236](https://github.com/totoshko88/RustConn/issues/236) with it)** — the tray's Quit item called `app.quit()` directly, so it reached neither the window's close handler nor the quit action, and skipped the close confirmation, the external-viewer shutdown, the detached-window close and the session children in one go. That is the exit path a tray user actually takes: someone who has minimise-to-tray on does not close the window, which means fixing #304 in the window handler alone would have missed exactly the people most likely to hit it. Tray Quit now goes through the same action as Ctrl+Q. It presents the window first when there is something to confirm, because a confirmation dialog parented to a tray-hidden window is never drawn and the quit would have looked like it did nothing.

- **A `pre_exec` contract test in `rustconn-pty-sys` had never passed on macOS** — it asserts that claiming a non-terminal stdin as controlling terminal fails, which is what proves the hook runs in the forked child at all. It accepted `ENOTTY` and `EPERM`; macOS answers `ENODEV` for `/dev/null`, so the one test guarding the `unsafe` block behind SSH password prompts (issue [#175](https://github.com/totoshko88/RustConn/issues/175)) failed on the maintainer's own platform and passed in CI, which has no macOS runner. `ENODEV` is now accepted, and it is not a loosening: nothing else on the spawn path produces it, so an unrelated failure is still rejected.

- **SPICE refused to launch with "Install virt-viewer" on a machine that had virt-viewer (issue [#303](https://github.com/totoshko88/RustConn/issues/303))** — and the Settings → Clients tab called it not installed for the same reason. Detection spawned `/usr/bin/which`, so the answer depended on a program that is not in POSIX, is not guaranteed to be installed, and is being retired by distributions in favour of the shell builtin `command -v` — which is no use to a `Command::spawn` with no shell in the loop. When `which` is absent the spawn fails with `ENOENT` and every candidate reports missing, however many viewers are installed. Lookup is now resolved in process: a few `stat` calls, no helper binary, no child. The same change fixes the same silent failure everywhere it could occur, because SPICE was not the only place — every client probe in the application went through `which`, including FreeRDP, the VNC viewers, `secret-tool`, `keepassxc-cli`, `bw`, `op`, `pass`, `passbolt`, `mc` and the "open my password manager" dispatch. SPICE was additionally the only protocol whose detection bypassed the shared resolver, so it alone missed the Flatpak and snap search paths; it now uses them, and falls back to `flatpak-spawn --host` for a host viewer, since virt-viewer is a desktop application in its own right and is not bundled in the Flatpak. Three copies of the SPICE probe — the launcher's, the Settings tab's and the core helper's — are now one.

### Improved

- **Binary lookup gained the paths a macOS `.app` and a sandbox actually use** — the resolver searches `/app/bin` under Flatpak, `$SNAP/{usr/bin,bin,usr/local/bin}` under snap, the writable per-application CLI directories of either, and the Homebrew prefixes a bundle launched from Finder does not inherit from the environment. Two of those were already honoured by *most* probes and by none of the three that had their own copy. The host probe is now bounded at two seconds and kills the child on expiry, where the KeePassXC one it replaced could block the main thread indefinitely on an unresponsive Flatpak session helper.

- **One teardown for every way the application can exit** — the window's close handler, the quit action and the tray's Quit item each carried their own list of what to shut down, and no list was complete: external viewers were in two of the three, detached windows in two, session children in none. They are one function now, so the next exit path added inherits the whole list instead of a subset of it. This is the fix for #304 rather than an aside — three divergent copies is what the bug *was*.

- **The CI clippy gate was passing on a cache hit rather than on a check.** The job caches `target/` keyed on `Cargo.lock`, so with the lock unchanged it restored a tree clippy had already linted and reported zero warnings without looking at anything — the failure mode the project's own steering warns about, in the gate itself. Nineteen warnings had accumulated behind it across `rustconn-core`, `rustconn`, `rustconn-cli` and `rustconn-pty-sys`: `chunks_exact` on constant sizes now that `as_chunks` exists, two `async fn` test doubles that never await, a `.ok().is_some_and()` on a `Result`, and a widening cast in the PTY crate. All fixed, because this release changes both `Cargo.lock` and `rustconn-core/src/lib.rs` — which means the next CI run is a real one, and would have failed on every one of them. That is the same shape as the 0.20.9 failure: a gate agreeing with a tree the packaging jobs would reject. The cache key itself is left alone; it is a build-time trade-off, and what made it dangerous was believing the result.

### Dependencies

- **FreeRDP (Flatpak) 3.30.0 → 3.31.0 — a security release, and the reason this one should be installed rather than skipped.** Upstream describes [3.31.0](https://github.com/FreeRDP/FreeRDP/releases/tag/3.31.0) as a "huge bugfix and security release", lists 22 security advisories against it, and asks distributors to update immediately because of "severe issues being addressed". Only the Flatpak and Flathub builds are affected: they bundle FreeRDP so external RDP works with no host package, while the deb and RPM merely *recommend* the distribution's own copy, and the snap bundles none at all. RustConn's embedded RDP client is IronRDP and is untouched by this, but external mode and every embedded fallback go through the bundled binary. Also brings a faster YUV decoder, which is the AVC/H.264 path an embedded session uses.

- **waypipe (Flatpak) 0.11.0 → 0.11.2** — the Wayland forwarding helper for SSH sessions.

- **Cargo**: chacha20 0.10.1 → 0.10.2, libredox 0.1.20 → 0.1.21, rtoolbox 0.0.5 → 0.0.6, uuid 1.25.0 → 1.26.0, wide 1.6.1 → 1.7.0; windows-sys 0.59.0 dropped from the lock file. `cargo audit` and `cargo deny check` report no vulnerabilities across 733 dependencies — the ten remaining warnings are the allow-listed unmaintained/unsound advisories already recorded in `.cargo/audit.toml` and `deny.toml`.

- **snap: rebuilt against the current Ubuntu 24.04 archive** — the Snap Store reported revision r427 as built with a `libssl3t64` that has since been superseded ([USN-8678-1](https://ubuntu.com/security/notices/USN-8678-1)). Nothing in `snapcraft.yaml` pins it: `stage-packages` resolves at build time, so publishing this release is itself the fix. The other archive packages the snap stages — `openssh-client`, `mc`, `picocom`, `inetutils-telnet`, `waypipe`, `libvte-2.91-gtk4-0`, `libasound2t64`, `python3` — are refreshed by the same rebuild.

## [0.20.10] - 2026-08-26

**0.20.9 shipped no packages.** Its tag exists, but every build job failed and no
GitHub release, no artifact and no channel update was ever produced — so
everything listed under 0.20.9 below reaches users with this release. The tag was
left in place rather than moved: rewriting a published tag is destructive for
anyone who already fetched it, and a patch release costs nothing by comparison.

### Fixed

- **`rustconn-cli` did not compile with the features every package builds it with** — 0.20.9 gave `build_sftp_browser_uri` a fourth parameter, the `NetworkSettings` carrying the new global jump-host tier, and one of its two call sites was not updated: `cmd_sftp`'s file-manager branch still passed three arguments. Nothing local caught it, because `rustconn-cli` declares `default = []` and that branch sits behind `#[cfg(feature = "client-launch")]` — `cargo clippy --all-targets`, `cargo test --workspace` and the CI clippy job all compile the crate with no features, where the code does not exist. Every package builds it as `--features full`, so all four release jobs — deb, RPM, AppImage and Flatpak — failed on the same `E0061`, and the jobs that publish (GitHub release, OBS, Homebrew, snap) were skipped behind them. CI does cover this, in `cargo test -p rustconn-cli --features full`, but that job runs on the push that carries the tag rather than before it, and on release day GitHub Actions was in a [major outage](https://www.githubstatus.com/incidents/y1t7p9fzrlj2) — a database primary failing over — so of the three workflows the tag should have started, one was created and stalled in `queued` with zero jobs and two were never created at all. The fix went in with the file-manager branch extracted into its own function: adding the missing argument pushed `cmd_sftp` past `clippy::too_many_lines`, which CI treats as an error.

### Improved

- **`verify.sh` now compiles the CLI the way the packages do** — the gate that was supposed to be the mechanical half of the Definition of Done ran `cargo clippy --all-targets` and nothing else, so it agreed with a tree four packaging jobs would reject. It gained a `-p rustconn-cli --features full` clippy gate, the matching test run under `--tests`, and `-- -D warnings` on the existing clippy gate — without it clippy exits 0 on a pedantic warning and the gate reported `ok` for a tree the CI clippy job, which does pass `-D warnings`, would fail.

## [0.20.9] - 2026-08-26

*Tagged but never published — see 0.20.10.*

### Added

- **A jump host can be set once, on a group or for the whole application (issue [#301](https://github.com/totoshko88/RustConn/issues/301))** — the centralised proxy management the request asks for, without repeating the bastion on every connection. Three tiers are consulted, nearest first: the connection's own Jump Host / ProxyJump, then the group chain, then a new **Settings → Connection → Network** page holding a Global Jump Host and a Global ProxyJump. The global tier is there because no group can stand in for "everything" — a top-level group is still just a group, and an ungrouped connection has no chain to walk. A **Network Mode** row chooses between *Inherit from group or global* and *Direct*, where `Direct` refuses an inherited bastion but keeps one set on the connection itself. It sits on the connection editor's **Basic** page, beside Host and Port, rather than on the SSH page: the choice is a property of the connection and applies to RDP, VNC and SPICE as much as to SSH, and the protocol pages are a stack that only ever shows one of them — a row on the SSH page would have been unreachable for exactly the protocols that could not refuse an inherited bastion before. The ProxyJump row's subtitle names whichever bastion is in force, resolving the whole chain: a bastion picked from a dropdown is stored as a reference rather than as `user@host`, and the global tier is where this request asked for it to be set, so naming only a group's free-text field would have stayed silent in the two likeliest cases. The subtitle also follows Network Mode as it is changed, instead of reporting the mode the connection was opened with. Both tiers offer a Jump Host picker as well as a text field, because a saved connection also carries its port, its identity file and its own bastion chain, none of which fit in `user@host`; setting both chains them, the picker's hop first. `rustconn-cli add`/`update` gained `--network-mode inherit|direct`.

- **The external RDP client is finally told how to size its window (user report)** — a new **External Window** row in the connection editor's Display group, with four modes: *Fit to screen* (the default, `/size:100%`), *Fullscreen* (`/f`), *Custom resolution* (`/w:` + `/h:`) and *All monitors* (`/multimon`). It is shown for both client modes on purpose, because it governs the standalone window in `External` mode *and* the window an embedded session hands over to when IronRDP cannot serve the server — the case the report arrived from, and the one where nothing the editor collected reached the client. `rustconn-cli add`/`update` gained `--rdp-display-mode fit|fullscreen|custom|multimon` and `--rdp-resolution WIDTHxHEIGHT`; a resolution on its own implies the custom mode, since asking for a specific size and then not using it is never what was meant, and `custom` without a resolution is refused rather than silently falling back.

### Fixed

- **An external RDP window opened at about a quarter of a 4K screen (user report)** — reported as "the parameters do not reach FreeRDP", but they did reach it; they were the wrong ones. When an embedded session hands over to the external client, the launcher built `/w:` and `/h:` from the config's `width`/`height`, and by that point those hold the embedded viewer's own `DrawingArea` geometry in **logical** pixels. On a 4K display at 200%, a maximised window's drawing area is under 1700x1000 once the sidebar, header bar and tab bar are subtracted, so FreeRDP opened a window of that many *device* pixels on a 3840x2160 screen. The embedded IronRDP path never noticed because it discards those values and recomputes the desktop from the widget size times the fractional display scale; the numbers only ever mattered on the path that had no business reading them. A standalone window is now sized by the new display mode, which defaults to filling the monitor, and the widget geometry is not consulted at all.

- **Every RDP profile carried a resolution nobody chose** — the editor's Resolution row is hidden unless a fixed size is wanted, but the save path wrote `Some(1920x1080)` — the spin buttons' own defaults — regardless of whether the row had ever been visible. So a connection created and saved entirely in embedded mode still stored a FullHD resolution, and switching it to `External` later applied that value with nothing on screen to explain where the number came from. Worse, because the field was never `None`, there was no way to express "size this to the display" at all. The resolution is now stored only when the display mode reads it, and the row's visibility follows the display mode rather than the client mode. The template editor had the same defect and the same fix, keyed on its client-mode selector since it has no display-mode picker: External stores the size that was on screen, Embedded stores none. **A connection that deliberately used a fixed resolution needs *Custom resolution* selected once** — a stored `1920x1080` is indistinguishable from the phantom, so it is not guessed.

- **`External` client mode ignored an RD Gateway completely** — of the two argument builders, only the fallback one emitted `/gateway:`. A connection configured with a gateway and set to `External` therefore dialled the target host directly, which cannot work for a host that only exists behind the gateway. The same builder also appended the user's Custom Arguments with no filtering, so a `/p:` or `/shell:` there failed the launch on the args-file validator instead of being dropped with a warning the way the other builder did it. Both were fixed by deleting the builder, not by patching it — see below.

- **Display Scale and Color Depth were collected and never sent** — `Display Scale` was read only by the embedded viewer and the row was hidden in `External` mode, so a HiDPI session in an external window had no way to ask for legible text; `Color Depth` was written to every profile and emitted by nobody. The external client now receives the scale as `/scale-desktop:` paired with the nearest accepted `/scale-device:` — the pair matters, because MS-RDPEDISP discards the desktop scale factor unless the device scale factor is exactly 100%, 140% or 180%, so the desktop value alone changes nothing — and the colour depth as `/bpp:`. Both rows are now visible for both client modes, since both clients honour them.

- **The Uzbek catalogue labelled the RDP resolution rows "Permission"** — `Resolution` was translated as `Ruxsat` and `Dynamic Resolution` as `Dinamik ruxsat`, which is the machine-translation trap on the other sense of the English word: `ruxsat` means permission or consent and has nothing to do with a display. It was the catalogue's own outlier — everywhere resolution appears inside a sentence the same translator wrote `o'lcham`, as in `qat'iy o'lchamlar` for "fixed resolution" and `oyna o'lchamiga` for "window size". All three labels now share one head phrase: `Ekran o'lchami`, `Dinamik ekran o'lchami` and `Maxsus ekran o'lchami`. The thirteen remaining uses of `ruxsat` in the file are the correct ones, about permissions.

- **An embedded session warned about custom FreeRDP arguments the user had never written** — the security layer, TLS level and "Disable NLA" were carried to the fallback client by being appended to the connection's *custom arguments* list. So a connection whose only unusual setting was `NLA`, a TLS level, or that switch arrived at the embedded viewer with a non-empty argument list, and the viewer duly announced that custom FreeRDP arguments are ignored and to switch the client mode to External. There were no custom arguments. All three are now fields the shared builder reads directly, so the list holds only what the user typed into it.

- **A jump host set on a group was never used (issue [#301](https://github.com/totoshko88/RustConn/issues/301))** — the group editor stored the choice and showed it again when reopened, but nothing read it at connect time. Three further defects around it: a group configured with *only* a jump host opened with the SSH switch off, and because that branch clears all five SSH fields, the next save threw the host away; the Jump Host and ProxyJump rows stayed hidden until an authentication method was picked, though how a group is reached and what credential its connections present are separate questions; and whether a connection inherited the bastion at all depended on where it got its SSH key, which made a group bastion and a connection-level key file mutually exclusive. RDP, VNC and SPICE meanwhile inherited unconditionally, with no way to refuse. Inheritance now follows Network Mode and reads the same for every protocol.

- **`rustconn-cli group set --ssh-proxy-jump ""` produced `ssh -J ""`** — a blank value was stored verbatim and reached the command line, breaking it rather than disabling the proxy. An empty value now clears the setting, `--ssh-agent-socket` included, and a blank is treated as unset at every tier it can be stored in: the connection, the group chain and the new global one. The dialogs normalise blanks away, but `config.toml` and `connections.toml` are editable by hand, so the check belongs in the resolver rather than only in the writers.

- **A pinned connection's real row never updated (adjacent to issue [#302](https://github.com/totoshko88/RustConn/issues/302))** — a pinned connection is in the sidebar twice, under the virtual "Favorites" group and in its real place, as two row objects that share an id. The update walkers stopped at the first match, always the Favorites copy, so the row in the group or at the root had no status icon, no recording dot, no external-viewer emblem and no split marker. The context menu reads the connected state from the row that was clicked, so right-clicking the real row offered *Connect* for a session that was already open and hid *Stop Recording* while it was recording.

- **The recording dot, the external-viewer emblem and the split marker vanished on any sidebar reload** — for every connection, not only pinned ones. Connected status was restored from the sidebar's own map when the tree was rebuilt; those three had nowhere to be restored from, so a rename, a duplicate, a pin toggle, a re-sort or a drag-drop cleared them until the next session event. They now survive a rebuild.

- **The Simplified Chinese translation has never loaded, in any package** — since it was contributed in PR [#94](https://github.com/totoshko88/RustConn/pull/94). Every install path derives the locale directory from the catalogue's filename, so `zh-cn.po` landed in `share/locale/zh-cn/` while gettext looks up `zh_CN` and never the hyphenated form. Ten paths were affected, from the OBS spec and the Debian rules to the Flatpak manifests, the Nix package, the Homebrew formula and `install-desktop.sh`; the application had the right locale all along, which is why nothing in the code looked wrong. The catalogue is now `po/zh_CN.po`, and a stored `zh-cn` choice keeps working.

- **`Settings` filled its connection dropdowns before it had the connection list**, so the startup-action picker had nothing to choose from.

- **The openSUSE package declared dependencies rpm already derives** — `Requires: libadwaita` and `libasound2` are gone; `libadwaita` is not even a package name there. The `.mo` files now carry `%lang()` markers, generated by `%find_lang` instead of a hand-maintained file list, and the RPM `Summary` fits the 79-character limit it had been 25 characters over.

### Changed

- **The sidebar's "Open new session" is offered whenever there is a session to duplicate (issue [#302](https://github.com/totoshko88/RustConn/issues/302))** — the action force-launches a second, independent session, bypassing the double-click that focuses the existing tab. It was only in the menu for external-viewer sessions, so for an embedded session the only alternative was the global "Open a new session on every double-click" preference, which changes what *every* double-click does. Now shown for embedded and external alike.

- **A server that runs RDS licensing is named as such instead of "incompatible"** — a Windows host with Remote Desktop licensing fails the embedded client's licensing exchange and the session is handed to the external FreeRDP client, which connects. That fallback is unchanged; the banner now says the server requires RDS licensing, and the log names the phase it failed in. The cause is upstream and open ([IronRDP #1629](https://github.com/Devolutions/IronRDP/issues/1629)): the host sends an auto-detect probe during the licensing exchange and the connector feeds it to the licensing decoder without checking the channel. Only markers that name the licensing exchange itself select the new wording — not `securityHeaderFlags`, the field the decoder rejects, which belongs to a header every Standard RDP Security PDU carries and so would have reported an unrelated decode failure as a licensing server.

- **An OpenH264 library that is installed but refused now says why, and can be overridden** — `Invalid hash: 55c8052d…` was the whole explanation. The decoder compares the library's SHA-256 against the binaries Cisco itself publishes and refuses anything else, because Cisco pays the H.264 patent royalties only for what it distributes. No distribution build can pass that check: `libopenh264-8`, Fedora's `libopenh264` and a local build from the Cisco *source* tarball are all refused. The message now says so, and `RUSTCONN_OPENH264` names a library to try before anything else, so a user who fetches the Cisco binary can enable H.264 without root. The 0.20.6 entry claiming Flatpak was never affected because it builds its own copy had it backwards — building its own copy is exactly why Flatpak fails the same check. The external FreeRDP fallback is unaffected either way and still accepts a distribution build, which is why `libopenh264` remains a recommended package. Documented in `docs/INSTALL.md`.

### Improved

- **Every Secret Service call has its own deadline** — reading a secret from a locked keyring makes the Secret Service raise an unlock prompt and answer only once it is dismissed, with no deadline of its own. The GUI bounded the whole credential resolution at 30 s, one budget shared by five lookup keys and every round-trip they make, and could not say which step stalled; `rustconn-cli` and any other consumer of `rustconn-core` had no bound at all and could wait forever. Each call now gets the standard 10 s vault budget and names itself in the log and in the error. This covers the helpers carrying the Bitwarden master password, the 1Password token, the Passbolt passphrase, the KDBX password and the portable-store passphrase, whose Linux branch had no budget while the macOS branch did.

- **The Bitwarden client ID is wiped even when the second keyring lookup fails** — it was held as a plain string across that call, so a failure dropped it without zeroizing.

- **One FreeRDP argument builder instead of three** — what an external FreeRDP client is told is now decided in exactly one place, `rustconn_core::protocol::build_freerdp_args`. There were three: the `External` client mode had its own list built from fifteen loose parameters, the embedded client's fallback had a second, and `rustconn-core` already held a third — the canonical one, with the property tests and the argument-sanitisation helpers, and with no production caller at all. Every defect fixed above is a symptom of that: a capability added to one list did not appear in the others, so `/gateway:` existed in one, sanitisation in one, and the display scale in none. The two GUI builders are gone; both paths now fill in a `FreeRdpConfig` and hand it over, which is also what let the display mode, the DPI and the colour depth arrive on both paths from a single change rather than three parallel ones. The property tests moved with it: the invariant is no longer "`/w:` and `/h:` are always present" but "some argument always decides the session size", plus the converse — that a fixed resolution is sent *only* when one was asked for, which is the bug that started this.

### Documentation

- **Per-directory `AGENTS.md`, so a rule loads where it applies** — nine nested files, one per crate plus `po/` and `packaging/`, alongside the root one. This is what the root file could not express: `rustconn-cli` *inverts* the workspace rule against `println!`, because printing is its interface and `main.rs` carries three crate-level allows saying so, and an agent reading only the root rules would "fix" working code. The four `rustconn-*-sys` crates each state their own FFI contract, including why `rustconn-dock-sys` target-gates its dependencies rather than itself. The root file loses the crate table and the widget and accessibility rules, which now live in the trees they govern.

- **Four runbooks became scripts, because they had no judgement in them** — `scripts/verify.sh` runs the mechanical half of the Definition of Done as one command with one log and one exit code, and warns when clippy reported zero warnings without compiling anything, the cached false green this repo has been caught by before. `scripts/bump-version.sh` writes the release version into the workspace `Cargo.toml` and all sixteen version-only packaging files, reading the canonical list out of `release.sh` at runtime instead of keeping a second copy: the previous route was an agent walking a sixteen-bullet list in steering that mirrored `PKG_FILES` and asked, in its own text, to be kept in sync with it by hand. Every rule in it is line-anchored, because a global version replace corrupts `rustconn.spec`, which records dependency bumps like `cfg-expr 0.20.8→0.20.9`, and `docs/USER_GUIDE.md`, which says "Changed in 0.20.9" about behaviour. `scripts/ponytail-ledger.sh` collects the 36 `// ponytail:` markers, reassembling the ones that wrap across comment lines, and flags those naming a ceiling without an upgrade path. `scripts/dep-audit.sh` runs the three command-shaped steps of the dependency audit and classifies what cargo holds back, separating a pre-release pin from a patch-level hold that something pins on purpose.

- **The post-session hook now knows what the session changed** — it asked the agent to run `git diff --name-only HEAD`, which reports the whole dirty working tree, then to call `getDiagnostics` on up to ten files and scan a second diff for debug macros, on every stop. In a session whose only edit was a markdown file that cost three consecutive round-trips to conclude nothing had changed. A `SessionStart` hook now records content hashes — immune to a commit moving `HEAD` mid-session — and the stop hook compares against them, leaving the agent the one step a script cannot take.

- **`docs/AI_DEVELOPMENT.md` counted 14 steering files and 20 hooks against an actual 27 and 16**, in a document whose opening paragraph warns that hand-maintained inventories drift. It also listed five steering runbooks as though they were hooks, which is where the 20 came from, and used trigger names from the pre-v2 hook format. `scripts/check-ai-docs.sh` now gates both counts and runs as part of `verify.sh`.

### Dependencies

- **Updated**: combine 4.6.7→4.6.8, the only crate `cargo update` could move inside the declared requirements. `cargo deny check advisories` reports no known vulnerabilities. Seventeen crates sit behind their latest release and none of them can move from the lockfile alone — the same seventeen as 0.20.8: eleven are pre-release pins carried in transitively through ironrdp, picky and sspi (`aes-gcm`, `ecdsa`, `p256`, `p384`, `p521`, `primeorder`, `rfc6979`, `curve25519-dalek`, `ed25519-dalek`, `x25519-dalek` and picky itself), five are patch-level holds something pins explicitly (`crypto-mac`, `generic-array`, `toml`, `toml_datetime`, `toml_edit`), and one, `quick-xml` 0.41→0.42, needs the requirement in `rustconn-core` widened rather than a lockfile change.

- **Audited, no change needed**: every auto-resolving CLI download endpoint answered — kubectl 1.36.4, Tailscale 1.102.3, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.149.0, Bitwarden CLI 2026.8.0, 1Password CLI 2.39.0 — and TigerVNC, the only pinned CLI download, is current at 1.16.2.

## [0.20.8] - 2026-08-24

### Fixed

- **Sidebar context menu did not open when it had no room below the pointer (issue [#298](https://github.com/totoshko88/RustConn/issues/298))** — the third and last cause behind this report, found after 0.20.7 fixed the two selection bugs: moving the main window higher up made the same right-click work, which is the signature of a popup that cannot be placed rather than a handler that never ran. A `GtkPopover` can never be smaller than its child, and the menu's child is a plain box of up to twenty rows and six separators — around 700 logical pixels. That was the popover's *minimum* height, so where neither anchoring below the pointer nor flipping above it left that much room, GTK had nowhere to put the surface and it never mapped: no menu, no error, nothing in the log. The menu now lives in a `ScrolledWindow` capped at two thirds of the monitor's height, which keeps room for the flip in either direction and scrolls whatever does not fit; a short menu is still allocated exactly as tall as its items, so nothing changes for a group or the empty-space menu. Keyboard navigation is unaffected — a `ScrolledWindow` scrolls to whichever item takes focus. Separately, a menu that closes within 300 ms of opening with no user interaction now says so at debug level whether or not it was holding the input grab; the existing message only covered the grab-less case, which is why both this and #299 arrived as "the menu just does not open" with nothing to go on.

- **Embedded mode on Web connections reverted to System, permanently (reported for 0.20.7)** — `WebBrowserMode::Embedded` only existed as an enum variant when the `web-embedded` feature was compiled in, so a build without it parsed a stored `browser_mode = "embedded"` as `System`. The connection then sat in memory as `System`, and because `connections.toml` is rewritten whole from that in-memory copy, the next save of *any* connection destroyed the choice for every Web bookmark in the file — and updating `last_connected` on a single connect is enough to trigger one. Nothing warned: the downgrade was logged at debug level and the rewrite not at all. This needs no exotic setup. `rustconn-cli` takes `rustconn-core` with `default-features = false` and never enables the feature, so a CLI built on its own did it too, and any distribution package built without WebKitGTK 6.0 does it to a configuration written by a Flatpak build that has it — the three can share one `~/.config/rustconn`. All three modes now exist in every build, and the feature decides whether Embedded can be *run* rather than whether it can be *stored*: a build with no WebView opens the URL in the system browser and leaves the stored mode alone, saying so at info level instead of silently. Round-tripping `"embedded"` through save and load is now asserted in both feature configurations.

- **`rustconn-cli --browser-mode embedded` set the opposite of what it asked for** — the same root cause seen from the CLI: `"embedded"` fell through to the compile-time default, which for this crate is `System`. Both `rustconn-cli add` and `rustconn-cli update` now store the mode that was requested.

- **Editing a Web connection reset its page zoom and its certificate exception** — the Web page of the connection editor has a widget for six of `WebConfig`'s eight fields, and `build_web_config` rebuilt the other two from literals: `zoom_level: 1.0` and `accept_invalid_certs: false`. So changing a bookmark's URL threw away the zoom the embedded browser had persisted from its own zoom controls, and cleared a certificate exception that only `rustconn-cli` can set — neither of which the dialog ever showed, so there was nothing on screen to suggest either was at stake. Saving now starts from the config the dialog was populated with and overlays what the widgets know; a new connection has no such value and correctly gets the defaults. In the same function, the custom browser command and user agent were only written into their rows when the stored value was `Some`, so editing a connection that had neither left whatever the *previously* edited connection had in those rows — and saving adopted it. Both are now set unconditionally, empty string included.

- **Config writes fought each other inside a single process** — `flock(2)` is held per open file description, so two writes contend even within one process, and `.lock` covers the whole config directory. RustConn has four independent writers: three debounce workers (connections, groups, trash) and the history flusher on its own thread, plus synchronous `save_settings` calls from GTK callbacks. One connect starts two of those two-second debounces at the same instant, so they woke together and one found the lock taken — which is why "Waiting for another rustconn instance to release config lock…" appeared roughly two seconds after every connect with no other instance running. In-process writers now queue on a mutex, leaving the advisory lock to do the job it is actually for, and the message no longer names a cause it cannot observe. Three further problems in the same path: the wait was unbounded, so a lock genuinely held by another process could freeze the window forever from a GTK callback — it is now bounded and reports a timeout instead; `save_toml_file_async` was a second, hand-maintained copy of the write sequence that opened with a *synchronous* `flock`, parking a runtime worker and defeating the caller's `tokio::time::timeout` (a timer only fires when the future yields, and a future stuck in a syscall never does), so the write now goes to a blocking-pool thread through the same single code path as the synchronous version; and `flush_persistence` took each pending snapshot out of its channel *before* awaiting the write and then used `?`, so one failure on `connections.toml` discarded that snapshot and returned before groups and trash were written at all — three files lost to one error, on the shutdown path, with a single log line to show for it. All three are now attempted, and each snapshot is cleared only once its write succeeds.

- **A single credential lookup opened 25 encrypted Secret Service sessions** — `LibSecretBackend` connected separately for every field of a credential, and `oo7::dbus::Service::new()` performs a DH key exchange each time (~12 ms, one "Starting an encrypted Secret Service session" line apiece). One `retrieve` therefore cost five connections — an availability probe plus one per field — and because resolving a credential tries five different lookup keys, a single click spent ~330 ms opening 25 of them. Visible in any debug log as a wall of identical oo7 lines whenever a password was not found. Each operation now opens one connection and passes it down, and the availability probe is cached briefly: `SecretManager` probes every backend before every lookup, for an answer that cannot meaningfully change inside one resolution. Storing a credential drops from four connections to one as well.

- **Embedded RDP and VNC toolbars were barely readable on a light theme** — the floating toolbar carried a fixed dark scrim (`rgba(36, 36, 36, 0.85)`) and no foreground colour, on the reasoning that it floats over a remote desktop whose colours are not ours. Its *contents* are ours and take their colour from the local theme regardless, so under a light theme Copy, Paste, Autotype and the toolbar's icon buttons were near-black text on a near-black bar; only Ctrl+Alt+Del stayed legible, because `suggested-action` brings its own colour pair. The embedded browser had already been split out of that shared rule one release earlier for the same reason — a light URL bar inside a dark bar — which was this defect showing a different symptom. All three toolbars now follow the local theme the way a header bar does, and a drop shadow rather than a contrasting fill is what separates the bar from the content underneath. VNC and SPICE share the class and are fixed with it.

### Dependencies

- **Updated**: h2 0.4.18→0.4.19, open 5.4.1→5.4.2, syn 3.0.3→3.0.4. Seventeen further crates sit behind their latest release but cannot move: they are held at `-rc` or exact versions by constraints elsewhere in the graph (the RustCrypto stack pinned through ironrdp, picky and sspi), so they need an upstream release rather than a lockfile change.

- **Audited, no change needed**: every auto-resolving CLI download endpoint answered — kubectl 1.36.4, Tailscale 1.102.3, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.148.0, Bitwarden CLI 2026.8.0, 1Password CLI 2.39.0 — and TigerVNC, the only pinned CLI download, is current at 1.16.2.

## [0.20.7] - 2026-08-23

### Fixed

- **SSH sessions offered no way to reconnect once the connection ended (issue [#297](https://github.com/totoshko88/RustConn/issues/297))** — when a session exited or dropped, the "Session disconnected / Reconnect" banner never appeared, and neither did the sidebar's offline indicator, the history record, the post-disconnect task, nor auto-reconnect. `TerminalNotebook::connect_child_exited` keeps one handler per session and disconnects the previous one, which is what stops an in-place reconnect from running the whole disconnect path once per reconnect the session has ever had (0.20.0). But four unrelated parts of the app watch that same signal — the disconnect path, the activity monitor, and two session-logging flushes — so keying by session alone left only whichever registered last. Session logging wires its handlers from an async callback, i.e. *after* the disconnect path, so on any connection with logging enabled the sole surviving handler was a `logger.flush()`: the tab looked alive and nothing offered a reconnect. Handlers are now keyed by purpose as well as by session, so a re-registration replaces its own predecessor and leaves the others attached. A handler that cannot be attached at all (no terminal for the session) now warns instead of returning silently, which is how this stayed invisible for two releases.

- **Cloned connections could not be edited and their right-click menu did not open (issue [#298](https://github.com/totoshko88/RustConn/issues/298))** — three silent failures in the sidebar's per-row right-click handler, all reachable after a clone because duplicating rebuilds the entire connection tree and therefore recycles every row widget. A recycled `GtkListItem` reports `GTK_INVALID_LIST_POSITION`, and that value was handed straight to the selection model, which *clears* the selection rather than moving it — after which Edit, Rename, Duplicate and Delete all returned without doing anything, since each of them re-resolves its target through the selection. The row is now located in the model by identity whenever the list item does not know its own position, and the row itself is resolved from the `GtkTreeExpander` when the list item carries no data. Beyond that, a press the handler could not resolve produced no menu and no log line, yet still claimed the event sequence — which cancelled the `ListView`-level fallback gesture that resolves the row from the pointer position and would very likely have succeeded. Unresolved presses are now left unclaimed and every failure path says what it could not resolve.

- **Sidebar context menu opened and vanished within a frame on GNOME Wayland (issue [#299](https://github.com/totoshko88/RustConn/issues/299))** — reported on Fedora 44 / GNOME Shell 50.4 / GTK 4.22, where right-clicking a saved connection flashed the menu and lost it 7–27 ms later, for every protocol, at every nesting depth, with the set of working rows changing between runs. Two independent causes. The popover was parented to the row's `GtkTreeExpander` — a widget `ListView` recycles, re-realizes and re-allocates — and a popover is a native surface tied to its parent, so any of that unmaps the menu roughly one frame after it opened. That is why forcing `GSK_RENDERER=cairo` or software GL changed how often it happened without fixing it: it changed the frame timing, not the anchor. Menus are now parented to the enclosing `ScrolledWindow`, which is never recycled, with the click point translated into its coordinates. Separately, the per-row menu used `autohide=false` so that moving the menu between rows costs one right-click instead of two ([#87](https://github.com/totoshko88/RustConn/issues/87)); a Wayland compositor cancels a grab-less `xdg_popup` on the focus change that follows the click, and the deferred `autohide=true` retry can never rescue it because a grab has to be requested against the serial of a live input event ([#157](https://github.com/totoshko88/RustConn/issues/157)). Wayland sessions therefore take the grab from the start. Any environment that still cancels a grab-less menu now switches to a grab for the rest of the session after the first attempt, instead of retrying the same doomed re-popup on every menu.

### Changed

- **Moving the sidebar context menu between rows takes a second right-click on Wayland** — a direct consequence of the #299 fix: the menu now holds the input grab, so the click that dismisses it is consumed by the compositor instead of reaching the next row. This is how GTK 4 context menus behave on Wayland generally, and it replaces a menu that could not be opened at all. X11 sessions keep the one-click behaviour.

### Dependencies

- **Updated**: cc 1.4.3→1.4.4, cfg-expr 0.20.8→0.20.9, crc32fast 1.5.0→1.5.1, font-types 0.12.3→0.12.4, icu_provider 2.3.0→2.3.1, keccak 0.2.1→0.2.2, log 0.4.33→0.4.34, rustls-webpki 0.103.14→0.103.15, uuid 1.24.1→1.25.0, zerovec-derive 0.11.5→0.11.6. `cargo deny check advisories` reports no known vulnerabilities. Seventeen further crates sit behind their latest release but cannot move: they are held at `-rc` or exact versions by constraints elsewhere in the graph (the RustCrypto stack pinned through ironrdp, picky and sspi), so they need an upstream release rather than a lockfile change.

- **Audited, no change needed**: GNOME runtime 50 is still the newest branch published on Flathub, and every bundled pinned source is at its latest upstream release — FreeRDP 3.30.0, cJSON 1.7.19, openh264 2.6.0, mc 4.8.33, inetutils 2.8, picocom 3.1. VTE stays pinned below 0.81 by design. The Flathub manifest matches the local one, `cargo-sources.json` was regenerated from the updated lockfile and both copies are identical. TigerVNC — the only pinned CLI download — is current at 1.16.2, and every auto-resolving CLI endpoint answered. Snap stays on core24 + gnome-46-2404: the GNOME extension is still only available for core22 and core24, so the core26 base that Snapcraft 9 now supports remains out of reach (issue #174).

### Internal

- **A property test asserted something the code deliberately does not guarantee** — `move_to_root_always_succeeds` claimed that moving a group to the root always succeeds, but `move_group` refuses a destination that already holds a folder of the same name, compared case-insensitively. A generated parent/child pair differing only in case ("q" and "Q") therefore made it fail, which it finally did while validating this release. Renamed to `move_to_root_succeeds_unless_the_name_is_taken`, and the expected outcome now comes from the manager's own `sibling_group_name_exists` instead of being assumed — a refused move is asserted to leave the group where it was, and the hierarchy is checked in both branches. The behaviour under test is unchanged.

## [0.20.6] - 2026-08-21

### Fixed

- **RDP used the RemoteFX path even with OpenH264 installed** — the H.264 probe looked only for the unversioned `libopenh264.so`, which a distribution ships in its `-dev` package. A runtime-only install carries `libopenh264.so.8` and the real `libopenh264.so.2.6.0` and nothing else, so RustConn reported "OpenH264 not found" on machines that had the library, skipped the EGFX pipeline, and ran every session over RemoteFX — noticeably more bandwidth and CPU against a modern Windows host. The known library directories are now scanned for versioned sonames, newest ABI first, and `libopenh264` is listed as a recommended package for the .deb and RPM builds. Flatpak was never affected: it builds its own copy.

- **Portable file passphrase could be discarded without saying why** — a store that does not exist yet requires the confirmation entry, and the notice saying so was outranked by the passphrase-strength advice. A passphrase that was both weak and unconfirmed therefore showed only the advice, which is phrased as advice and never refuses anything, and the passphrase was then dropped with nothing on screen having warned that it would. Three changes: the requirement is shown whenever it applies, with the strength advice below it rather than instead of it; editing the file path re-checks it, where previously the path could be moved to a not-yet-created file — making the confirmation mandatory — without the notice updating; and because Preferences saves when it *closes*, a dialog now reports the discarded passphrase afterwards, since the inline notice is destroyed along with the window that carried it.

- **Warning about a portable passphrase that was never entered** — with no portable file on disk, the "a new store requires the confirmation entry" rule was applied to two empty fields, so simply opening Preferences and closing it again logged a warning about a passphrase the user had not typed. The check now treats an empty field as nothing to save.

- **Spurious warnings when quitting with sessions open** — shutdown closes the SSH control sockets first, so a session's child is normally reaped before the teardown handler asks the session manager to terminate it. The resulting "Session not found" is the expected outcome at that point and is now logged at debug level, rather than leaving warnings in a log the user is about to attach to a bug report.

- **macOS: Dock icon changed to generic "exec" when app was pinned and closed** — the `.app` bundle was not registered with LaunchServices during `brew install`, so macOS could not associate the correct icon with the pinned Dock tile. `post_install` now runs `lsregister` to register the bundle, as a best-effort step that cannot fail the installation. Updated caveats with explicit Dock pinning instructions (symlink to `/Applications` first, then pin from there).

### Improved

- **Sidebar status is set once per change instead of twice** — both the outer action handler and the connection-start path marked a connection "connecting", so every status change walked the connection tree twice and re-rendered the row twice. The outer call is the one kept: it runs before credential resolution, which can take a second against a vault, and is what puts the status on screen while the user waits.

- **Header bar Shell button restyled** — replaced the oversized pill-shaped button with a flat accent-colored button that blends better with the header bar chrome, especially on macOS where libadwaita pill buttons inflate the toolbar height. The button retains full functionality and keyboard shortcut (Ctrl+Shift+T).

- **macOS: increased font size for Retina displays** — the default libadwaita font (11pt) appeared disproportionately small on macOS where the system font is 13pt. UI text is now scaled up 10% on macOS so labels, buttons, and list rows look natural next to native AppKit applications. The scaling is carried by a `.macos` class on each top-level window, which includes the "Move" and "Edit Connections" dialogs — a separate top-level does not inherit it, and those two would otherwise have kept the unscaled font.

- **macOS: Settings dialog uses icon-only tabs** — switched from a wide (1000px) dialog with text labels to a narrower (600px) dialog where the ViewSwitcher shows only icons, matching macOS native sheet proportions.

- **macOS: Welcome screen rows more compact** — reduced vertical spacing between list rows on the welcome page so all content (Features, Keyboard Shortcuts, Quick Access, Import Formats) fits without scrolling on standard MacBook displays.

## [0.20.5] - 2026-08-21

### Fixed

- **Flatpak: terminal still started at 24×80 on some hosts (issue [#294](https://github.com/totoshko88/RustConn/issues/294))** — 0.20.4 sized RustConn's own PTY correctly, but inside Flatpak the shell runs on a *second* one: `flatpak-spawn` forwards our PTY to the host, where `script` creates its own and copies the window size across once, at startup. The host shell was started before the terminal widget had been allocated, so that copy captured VTE's 24×80 default and nothing corrected it afterwards — the later `TIOCSWINSZ` reaches the sandbox PTY only, and the `SIGWINCH` it raises is not among the eight signals `flatpak-spawn` forwards to the host. The host shell now starts once the widget has a size, which is all `script` needs in order to inherit the right one, so programs that read their geometry at startup (`mc`, `htop`, shells) open at the real size.

  Known limitation: resizing the window mid-session still does not reach a Flatpak host shell. The `stty` call that was supposed to handle this (added with the host shell itself, issue #122) could never have worked — it ran against RustConn's own stdin, which is `/dev/null` under a desktop launcher and the launching terminal when started from a shell, but never the session's PTY. Its exit status was discarded, so it either failed silently or resized the wrong terminal. It has been removed rather than left in place looking functional.

- **Shortcut recorder did not warn about conflicts with fixed shortcuts (issue [#295](https://github.com/totoshko88/RustConn/issues/295))** — recording e.g. `Ctrl+V` for terminal paste showed no conflict even though the sidebar's "Paste connection" handler occupies that key whenever the sidebar has focus. The checker now consults all eleven non-rebindable shortcuts — sidebar row actions, terminal zoom, the primary menu — reading the same list the keyboard-shortcuts dialog shows, and names the conflicting action in the warning.

### Documentation

- **WSL guide: added Flatpak install option and single-line Ubuntu command** — `docs/WSL.md` now offers three install paths (OBS repo, .deb, Flatpak) with a copy-paste one-liner for Ubuntu 24.04. Flatpak section includes sandbox permission notes and portal integration requirements.

### Dependencies

- **Updated**: either 1.17.0→1.18.0, h2 0.4.16→0.4.18, zerovec 0.11.7→0.11.8

## [0.20.4] - 2026-08-19

### Added

- **Portable encrypted file backend — cloud-syncable credential storage (issue [#293](https://github.com/totoshko88/RustConn/issues/293))** — a new secret backend keyed by a passphrase you choose instead of by the machine, so one file holds your passwords and opens on every device you own, Linux and macOS alike. Configured in Settings ▸ Secrets: pick a path inside whatever folder your cloud client syncs, choose a passphrase, and optionally have it remembered locally. Existing passwords can be copied into the portable file in one step, keeping the originals as the local fallback. `rustconn-cli secret get`/`set`/`delete` work with it too.

  The file uses AES-256-GCM with a key-encryption-key / data-encryption-key split derived from Argon2id. Concurrent edits resolve as last-writer-wins; there is no passphrase recovery. Both limits are documented in the user guide.

- **Settings ▸ Secrets ▸ Copy Passwords… — move stored passwords between any two backends** — a dialog with From/To selectors covering all eight backends, progress reporting, and the ability to stop mid-batch. Reports what arrived, what had no stored password, and what failed, by name. Refuses when two entries would collide in the destination rather than silently overwriting.

- **Settings ▸ Secrets ▸ Change Passphrase…** — re-encrypts every credential in the portable file under a new passphrase. All-or-nothing: a single unreadable entry aborts and leaves the file unchanged.

- **Passphrase strength indicator** — the setup row warns when a passphrase is weak. Advice only, never a refusal; not shown when opening an existing file.

- **Create File button in Settings** — creating the portable file is now an explicit step with confirmation, rather than a side effect of the first credential save. On a second machine it verifies that the passphrase opens the file the sync client delivered.

### Fixed

- **Terminal starts at correct size instead of 24×80 (issue [#294](https://github.com/totoshko88/RustConn/issues/294))** — programs that check geometry only at startup (`mc`, `htop`) now see the real window dimensions immediately.

- **Keyboard shortcuts help dialog shows user overrides (issue [#295](https://github.com/totoshko88/RustConn/issues/295))** — the window was built from a compile-time array and always showed defaults. It now reads the effective accelerators, including remapped bindings, and lists eleven non-rebindable shortcuts that were previously missing.

- **macOS: Dock showed a generic tile instead of RustConn's icon** — set at runtime via AppKit when no `.app` bundle is behind the process. Lives in a new `rustconn-dock-sys` crate.

- **macOS: Homebrew formula's `.app` had no bundle identity** — the wrapper script lost the bundle context, causing a generic Dock tile and missing tray icon. The bundle now holds the real binary directly.

- **`~` in Settings path fields was taken literally** — `~/Dropbox/rustconn.enc` now expands correctly for the KeePass database, key file, pass store and portable file paths.

- **Empty row visible in the Portable Encrypted File settings group** — the status row was always present but hidden wrong, leaving a blank band. Also fixed: status messages from different sources (keyring warning, file creation, copy result) no longer erase each other.

- **Portable file could be overwritten during setup on a second machine** — a file arriving from the sync client during the half-second Argon2 derivation was discarded. The path is now re-checked before the rename.

- **Saved passphrase reported as stale when it no longer opens the file** — previously said "Incorrect passphrase", now says "The saved passphrase no longer opens the portable file" with guidance to update it in Settings.

- **Selecting the portable backend made the machine-bound store unreachable** — credentials in `credentials.enc` became unreadable immediately after switching, before the user could run Copy Credentials. Fixed.

- **Wrong passphrase read as "no stored password"** — a mistyped passphrase in Settings now surfaces as an explicit error instead of silently falling through to a password prompt.

### Improved

- **Backend selector redesigned** — now an `AdwComboRow` with descriptions explaining each option's trade-offs. Both file backends show file status (path, credential count, or error state).

- **Fallback toggle is now an `AdwSwitchRow`** — was a checkbox naming "libsecret" on all platforms; now says "macOS Keychain" on macOS.

- **`rustconn-cli --backend` accepts `encrypted-file` and `portable`** — the machine-bound file backend was previously unreachable from the CLI without changing Settings first.

- **Transfer failures named in a dialog, not a disappearing toast** — entries that could not be copied are listed by name with guidance, rather than reported as a count in a transient notification.

### Security

- **Passbolt CLI error messages no longer contain the submitted password** — `go-passbolt-cli` quotes its arguments in error output; those are now scrubbed before logging.

- **Bitwarden CLI no longer logs saved passwords at debug level** — the base64-encoded item (containing the plaintext) was logged on every `bw create/edit`. Only the verb is logged now.

- **Vault parse errors no longer quote credential values** — serde's `Display` included the rejected token, which could be a password stored as a JSON number. Parse errors now report position and category only.

- **Intermediate plaintext copies are wiped on drop** — five code paths across Passbolt, 1Password, KeePass and the portable store left `expose_secret().to_string()` in bare `String` instead of `Zeroizing`. Fixed.

- **Portable store data key no longer left in an unwiped stack slot** — the `[u8; 32]` is `Copy`, so wrapping the copy in `Zeroizing` left the original unprotected. Now filled in place.

- **Credential store temp files are created `0600`** — previously chmod'ed after the write, leaving a window where the KDF salt and wrapped key were world-readable. Temp names are also randomised and opened with `O_EXCL`.

- **Concurrent writes no longer lose entries** — a mutex now spans the full read-modify-write cycle. Cross-machine writes remain last-writer-wins.

- **Portable file size and entry count capped before parsing** — 8 MiB / 10 000 entries. Argon2 cost ceilings reduced to 256 MiB / 12 iterations.

- **KeePass bulk transfer has a per-entry timeout** — a hung `keepassxc-cli` no longer blocks the entire batch indefinitely.

### Localisation

- **Georgian (ka) added — [PR #296](https://github.com/totoshko88/RustConn/pull/296)** — full catalogue contributed by Ekaterine Papava. RustConn now ships 17 locales.

- **Eighty-five new strings for the Secrets rework translated in all 17 locales** — merged with `--no-fuzzy-matching` to avoid incorrect guesses that render as English while counting as translated. Ukrainian reviewed against the project style guide.

### Documentation

- **`docs/USER_GUIDE.md`** — new "Portable Encrypted File" section covering setup, passphrase choices, and limitations.
- **`docs/ARCHITECTURE.md`** — key hierarchy, untrusted-header handling, and cloud-sync ceiling.
- **`docs/CLI_REFERENCE.md`** — new `--backend` aliases and passphrase prompting behaviour.

### Dependencies

- **Updated**: zvariant 5.14.0→5.15.0, zvariant_derive 5.14.0→5.15.0, zvariant_utils 4.1.0→4.2.0

## [0.20.3] - 2026-08-17

### Added

- **Keyboard passthrough state is now saved across restarts (issue #274 follow-up)** — the global keyboard passthrough toggle (Ctrl+Shift+Backspace) previously reset to off on every launch, forcing users who work primarily in TUI applications (nvim, tmux, mc) to re-enable it each time. The state is now persisted in `config.toml` under `[ui] keyboard_passthrough` and restored on the next start. A new switch in Settings ▸ Interface ("Remember keyboard passthrough") controls the behavior. Default remains off so existing workflows are unchanged.

### Improved

- **Sidebar tooltips now show full tree path for nested items** — hovering over a deeply nested connection or group in a narrow sidebar reveals the complete ancestor path (e.g. "Client › UK / WORK01 › server.example.com"), making truncated names readable without widening the panel.

### Dependencies

- **Updated**: cpal 0.18.1→0.18.2, h2 0.4.15→0.4.16, uuid 1.24.0→1.24.1, zvariant_utils 4.0.0→4.1.0

## [0.20.2] - 2026-08-15

A polish release that merges all outstanding community pull requests and closes
the last rough edges from 0.20.0/0.20.1. Special thanks to
[Felipe Schneider](https://github.com/sch-felipe) — his first contribution to
RustConn brought three well-researched fixes that made this release possible.

### Added

- **Settings ▸ Interface — reveal session toolbar on hover or click only ([PR #286](https://github.com/totoshko88/RustConn/pull/286))** — a new switch under Settings ▸ Interface ▸ Appearance controls whether the floating RDP/VNC toolbar opens on pointer proximity or only on an explicit click. Default: hover (existing behaviour unchanged). The handle at the top centre of the remote view sits right where the remote window's own title bar and close button are, so dragging the pointer there to reach the remote UI opens a panel over the exact spot being aimed at. Turning the switch off keeps the handle, its focus ring and every toolbar action — only the accidental trigger goes away. The preference is read per event rather than captured when the view is built, so toggling it takes effect on sessions already open. Contributed by Felipe Schneider.

### Fixed

- **Duplicating a connection with vault credentials left the duplicate without a stored secret ([PR #280](https://github.com/totoshko88/RustConn/pull/280))** — `duplicate_selected_connection` copied every field of the source connection, including `password_source = Vault`, but never copied the vault entry itself. The duplicate looked fully configured while having no stored secret: connecting to it prompted for a password or failed to authenticate. Every other name-changing path (paste, rename, move-to-group) already migrated the credential; duplicate was the only one that did not. The secret is now copied on a background thread immediately after the duplicate is created. Contributed by Felipe Schneider.

- **A reconnected terminal session stacked `child-exited` handlers, running the disconnect path N+1 times after N reconnects ([PR #283](https://github.com/totoshko88/RustConn/pull/283))** — `connect_child_exited` connected a fresh handler on every reconnect (Reconnect button, auto-reconnect, network-change sweep) without disconnecting the previous one. After two reconnects the next disconnect ran three post-disconnect tasks, decremented the sidebar session count three times, and spawned three host-probe polls. The handler id is now stored per session and the previous one is disconnected before connecting the new. Contributed by Felipe Schneider.

- **Duplicate `http-body-util` entry in 0.20.0 CHANGELOG Dependencies section** — the first bullet listed only `http-body-util 0.1.4→0.1.5`, while the second included it alongside `clap_mangen` and `font-types`. The redundant first bullet is removed.

### Documentation

- **Group name uniqueness rule documented in `docs/USER_GUIDE.md`** — 0.20.1 changed group names from globally unique to unique per parent folder (issue #291), but the user guide did not mention the naming constraint at all. The "Create Group" section now states that names must be unique among siblings, and that identically named groups under different parents (e.g. `Site A/RDP` + `Site B/RDP`) are allowed.

### Localisation

- **Two strings from PR #286 translated in all 16 locales** — the "Open session toolbar on hover" switch title and its subtitle shipped untranslated; now in the POT and all catalogues.

## [0.20.1] - 2026-08-14

### Added

- **Settings ▸ Interface ▸ Window — show connection name in split panes (issue [#277](https://github.com/totoshko88/RustConn/issues/277))** — a new toggle adds a compact colored header at the top of each split-view pane displaying the connection name and protocol. Off by default; useful when 3+ panes are open side by side and the color indicator alone is not enough to identify which pane belongs to which connection at a glance. The header background matches the panel's color (15% opacity), shrinks in compact mode, and updates live when a session is moved between panes via drag-and-drop or Select Tab.

### Fixed

- **Group names are now unique per parent folder, not globally (issue [#291](https://github.com/totoshko88/RustConn/issues/291))** — RustConn rejected creating a group whose name existed anywhere in the tree, even under a different parent. Hierarchies like `Site A/RDP` + `Site B/RDP` were impossible, and CSV imports silently merged unrelated branches. Uniqueness is now enforced only among siblings sharing the same parent. Moving or renaming a group into a parent that already contains a child with the same name is still rejected.

- **A split pane could not be closed at all when its connection had the floating toolbar switched off (issue [#260](https://github.com/totoshko88/RustConn/issues/260) follow-up)** — 0.20.0 gave RDP, VNC and the embedded browser a switch that removes the session toolbar, and marked the viewer with a `no-floating-overlays` CSS class so the split view would leave it alone too. The split view read that marker before adding its own corner-button overlay, so **Remove from Split** and **Close session** disappeared along with the toolbar. Those two buttons are the split view's, not the viewer's, and they are the only discoverable way to dismantle a pane: the right-click menu still carried the actions, but a pane the user cannot see a way out of is a pane they cannot close. The corner buttons are unconditional now, and the marker — which had exactly one reader — is gone with the gate rather than left behind as state nothing consumes. What #260 asked for is unaffected: the viewer's own toolbar and its 44×44 reveal handle are still switched off inside the viewer, before it ever reaches the split view.

- **RDP: xrdp hosts no longer stop at their own greeter after a successful NLA exchange ([#290](https://github.com/totoshko88/RustConn/pull/290))** — the connector hardcoded `autologon: false`, so `INFO_AUTOLOGON` was never set in the Client Info PDU. Windows ignored this (session established via CredSSP/NLA), but xrdp only skips its login screen when the flag is present. Connecting to a Linux host got past authentication and then paused at the xrdp greeter asking for the same credentials again. FreeRDP sets the flag whenever credentials are supplied, which is why the same host logged straight in from Remmina. The flag is now set when both a username and a non-empty password are available.

- **Opening the connection dialog crashed the application if an SSH agent key had a non-ASCII comment ([#278](https://github.com/totoshko88/RustConn/pull/278))** — `format_agent_key_short` sliced the comment at byte offsets, but agent key comments are free text from `ssh-keygen -C`. An accented letter or emoji at the 10-byte boundary panicked inside a GTK signal handler, aborting the entire process and taking every open session with it. The function now measures and cuts in characters.

- **Vault credentials were not found for connections inside a group ([#289](https://github.com/totoshko88/RustConn/pull/289))** — saving stored the credential under the hierarchical key `RustConn/{group}/{name} ({protocol})`, but resolving searched only the flat `{name} ({protocol})`. Any connection in a folder prompted for a password on every connect attempt despite the secret sitting in the keyring. The hierarchical key is now tried first, with a fallback to the flat key for credentials written by older releases.

- **Minimizing to tray silently destroyed port forwards, recordings and external viewers ([#279](https://github.com/totoshko88/RustConn/pull/279))** — the close handler ran `flush_active_recordings()`, `tunnel_manager.stop_all()` and `external_session_registry().shutdown()` unconditionally, before the minimize-to-tray decision was reached. Closing the window with tray enabled hid it and then tore down everything behind it. The tray decision now fires first, and the session snapshot is saved on that path so a subsequent force-kill does not lose session state.

- **A post-disconnect automation task froze the entire application for up to 60 seconds ([#281](https://github.com/totoshko88/RustConn/pull/281))** — the task ran via `block_on()` directly in the `child-exited` handler on the GTK main thread. An arbitrary user command (script, ssh, curl to an unreachable host) blocked every other terminal, RDP and VNC tab until it finished or the 60-second ceiling fired. The work now runs on a background thread via `spawn_blocking_with_callback`.

- **Remote session recordings were lost when quitting the application ([#282](https://github.com/totoshko88/RustConn/pull/282))** — `flush_active_recordings` used `spawn_blocking_with_callback`, whose 16 ms poll callback is never reached once the main loop stops on the quit path. The SCP result was dropped, the `.meta.json` sidecar was never written, and the recording disappeared. A new `stop_recording_blocking` variant runs the retrieval inline on shutdown, bounded by `ConnectTimeout=5` so an unreachable host cannot hang the window.

- **Web embedded: links requesting a new view (target="_blank", window.open) did nothing ([#288](https://github.com/totoshko88/RustConn/pull/288))** — nothing was connected to WebKit's `create` signal, so every request for a second web view was silently dropped. On SAPUI5-like pages whose links use `window.open()`, every link appeared broken. The requested URI is now loaded in the same view, preserving the authenticated session.

- **Network monitor reported a false outage on every Flatpak launch ([#284](https://github.com/totoshko88/RustConn/pull/284))** — `GNetworkMonitorPortal` emits an initial `network-changed` signal ~6 ms after creation as it learns the host's real state. This was treated as a real transition, showing "reconnecting affected sessions" with nothing wrong. Additionally, the debounce erased a down→up flap shorter than 3 s, preventing the recovery sweep from running. The first signal is now treated as a baseline, and debounce only collapses signals that classify the same way.

- **A session closed with `exit` was silently reconnected on the next Wi-Fi roam ([#285](https://github.com/totoshko88/RustConn/pull/285))** — the reconnect sweep picked victims by asking whether the reconnect banner was visible, but a banner is not consent: it is also shown for a shell the user closed with `exit`, for a failed login and for a rapid crash. The disconnect path now records its verdict, and the sweep requires explicit eligibility alongside the visible banner.

- **Web embedded: the toolbar's reveal handle was unreachable in a split pane, buried under the split view's own panel arrow** — 0.20.0 moved the Web handle to the top *trailing* corner, reasoning that a web page keeps its logo, primary navigation and search across the top centre, where a 44×44 button swallows clicks meant for the page. That corner was already taken: `SplitViewAdapter` overlays the panel's reveal arrow at exactly the same spot, so in a split layout the two stacked and the page's toolbar could not be summoned at all. Neither widget could notice the other — they are added to different overlays by different modules, one keyed to the viewer and one to the pane. The handle is back at the top centre for every viewer, which is what the centre position's own documentation had given as its reason all along: it "keeps the handle clear of the window controls and split-view buttons that live in the corners". The page-content cost is real but recoverable — a connection that wants an unobstructed page switches **Navigation Toolbar** off (issue [#260](https://github.com/totoshko88/RustConn/issues/260)) and gets no handle at all — whereas a handle underneath another widget is not recoverable. With all three viewers agreeing again, the `RevealHandle` enum and the `attach` parameter that carried it are gone.

### Improved

- **`scripts/release.sh` now runs `typos`, the last CI gate it did not** — the CI `Hygiene` job spell-checks the tree and fails the build on it, and the release script did not, so this release reached a pushed tag and a published GitHub release with a red CI over one word: `cliente`, Spanish for "client", in the SSH agent key comment `backup-señor-cliente-final` that the #278 regression tests use. The fixture has to be text a real person would type, because that is the entire bug — the comment is free text from `ssh-keygen -C` and slicing it at a byte offset panicked inside a GTK signal handler — so the word is allowed in `typos.toml` rather than anglicised, which would have cost the test its point. Exactly the gap 0.20.0 closed for the three i18n checks, in the one place it was not closed; the gate resolves `~/.cargo/bin/typos` explicitly when it is not on `PATH`, because a gate that silently skips reports nothing and is believed.

### Localisation

- **Three strings added this release reached only Ukrainian** — the two "Show connection name in split panes" labels were translated in `uk` alone, so the other 15 locales rendered them in English; the resume toast "Reconnecting sessions after sleep" was worse off still, never having been extracted into `po/rustconn.pot` at all. Now in the POT and translated in all 16 locales. This is the third release in a row to find strings that shipped untranslated while `scripts/check-po-complete.sh` reported every catalogue at 100%, because that gate reads the committed `.po` files and never regenerates the POT to compare against — the gap 0.20.0's changelog recorded as "real and outliving this fix", and it still is.

### Dependencies

- **Updated**: cc 1.4.2 → 1.4.3, find-msvc-tools 0.1.10 → 0.1.11, inotify 0.11.4 → 0.11.5, libredox 0.1.19 → 0.1.20, pkg-config 0.3.33 → 0.3.34, safe_arch 1.1.0 → 1.2.0
- **ICU / zerovec family**: icu_collections, icu_locale_core, icu_normalizer, icu_normalizer_data, icu_properties, icu_properties_data and icu_provider 2.2.0 → 2.3.0, alongside litemap 0.8.2 → 0.8.3, potential_utf 0.1.5 → 0.1.6, tinystr 0.8.3 → 0.8.4, writeable 0.6.3 → 0.6.4, zerotrie 0.2.4 → 0.2.5, zerovec 0.11.6 → 0.11.7 and zerovec-derive 0.11.3 → 0.11.4. All transitive, reached through `idna` → `url`; no direct dependency changed.

## [0.20.0] - 2026-08-13

### Added

#### Sessions and tabs

- **Opening a cluster labels its tabs with a group named after the cluster** — a cluster used to dissolve into anonymous tabs the moment it opened. Nothing on screen said which tabs belonged together, and the only thing that could act on the set as a whole was the Disconnect button in the Clusters dialog. Each member tab now joins a tab group named after the cluster, so it reads `[cluster] host` and every existing tab-group operation applies to the cluster for free: **Close All in Group** on any member closes the cluster, **Close All Ungrouped** closes everything except your open clusters, and the tab switcher (`Ctrl+%`) shows the cluster name beside each member. No new command, no new setting — the two features simply meet.

  The group name is the cluster's name verbatim, so renaming a cluster affects the next opening rather than tabs already open, and a cluster sharing a name with a hand-made group merges into it, which is the rule two hand-labelled tabs already follow. Per-tab overrides still work: **Remove from Group** or **Set Group…** move one member out without removing it from the cluster, so Disconnect still reaches it. Closing a cluster retires its name from the group chooser unless another tab still wears it — otherwise every cluster ever opened would accumulate there, which is a new problem created by naming groups automatically rather than by hand.

  This is also the first caller `TerminalNotebook::set_tab_group` and `TabGroupManager::remove_group` have ever had: the assignment mechanism was already written and wired to nothing, with the context-menu dialog duplicating its body inline instead.

- **Web embedded mode: auto-hide floating toolbar with reveal zone** — the navigation toolbar (Back, Forward, Reload, Home, URL bar, Zoom, Menu) now floats as a semi-transparent overlay above the WebView, matching the behavior of RDP and VNC embedded sessions. The toolbar appears briefly when the page connects, then auto-hides after 2 seconds of inactivity. A small arrow indicator at the top center acts as the reveal zone — hover or click it to show the toolbar. The toolbar stays visible while the pointer is over it, any control has focus, or a menu is open. This gives the web content the full viewport height, identical to how graphical remote sessions work.

- **A connection can now switch its floating toolbar off entirely, hot zone and all ([#260](https://github.com/totoshko88/RustConn/issues/260))** — RDP, VNC and the embedded browser each gained a **Session Toolbar** / **Navigation Toolbar** switch in the connection editor, plus `--vnc-toolbar` and `--web-toolbar` on `rustconn-cli add` and `update`. Turned off, there is no toolbar, no 44×44 reveal arrow and no revealer left to intercept a click — and the split view stops adding its own corner buttons over that session too. #260 asked for this and was closed by 0.19.14's auto-hide toolbar, which the reporter then explained was not the same thing: with a full-screen browser behind the session they kept catching the barely visible arrow by accident, and the toolbar it summoned covered the tab strip they were aiming for. Auto-hide reduces how long the obstruction lasts; it cannot remove one for someone who never wanted the toolbar.

  What goes with it is stated in the switch's own subtitle rather than left to be discovered: for RDP and VNC that is Ctrl+Alt+Del, which has no keyboard route of its own, along with Fit resolution, autotype, scripts and the quick actions — Copy and Paste survive, since Ctrl+C/V reach the remote session directly. The embedded browser loses less: Alt+←/→, Ctrl+R, Ctrl+L and Ctrl+±/0 keep working, so only the URL bar itself is gone. In a split pane the panel's Remove from Split and Close session stay on the right-click menu, which lives on the pane and not on the overlay being suppressed.

  Stored as `hide_floating_toolbar`, phrased negatively on purpose: `false` is both what `Default::default()` produces and what a profile written before this release deserialises to, so no existing connection and nothing built by an importer, template or wizard changes behaviour. The setting travels into a split pane by itself — the split view reparents the live viewer rather than rebuilding it, so the controller and its state move with the widget, including across the layout rebuild that follows closing a pane. Quick Connect, which has no stored profile, keeps the toolbar.

#### Settings

- **Settings ▸ Interface ▸ Rendering — pick the GTK renderer (issue #274)** — Automatic, Hardware (GPU) or Software (Cairo), applied on the next start because GTK reads `GSK_RENDERER` while it opens the first window. Automatic keeps GTK's GPU renderer everywhere except the two environments where it is known to be worse than software rasterisation (see the macOS guest-VM fix below and the X11 popover workaround from #85). The two explicit values exist because both of those are heuristics about the environment rather than facts about the user's hardware: Software is the escape hatch for a slow environment the probe does not recognise, and Hardware is the way back for an X11 session whose driver is fine — until now every X11 session was downgraded unconditionally. An explicit `GSK_RENDERER` in the environment still overrides all three.

#### Release and distribution

- **Signed build provenance for every release artifact** — the `.deb`, `.rpm`, AppImage and Flatpak bundle attached to a GitHub release now carry a SLSA build-provenance attestation, signed through Sigstore with a short-lived certificate issued to the release workflow itself. A download can be checked against the repository and the commit it was built from with `gh attestation verify <file> --repo totoshko88/RustConn`, which fails if the artifact was rebuilt elsewhere or altered after the run. The attestation binds a file's digest together with its name, so it is generated after the Flatpak bundle is renamed to its versioned filename. The snap is not covered here: it is built by a separate post-release job and the Snap Store signs it on its own.

### Fixed

#### Tabs, clusters and split view

- **An RDP, VNC, SPICE or Web member of a cluster was never registered in it, so "Disconnect all cluster sessions" could not close it** — the notebook resolves a cluster membership when the member's tab appears, and that resolution was called from the terminal creation path only. Every other protocol opened its tab and stayed in the pending map forever: `get_cluster_sessions` never listed it, Disconnect skipped it, and the stale pending entry survived until the cluster was unregistered. The call moved to `notify_tab_added`, the one function all five creation paths (terminal, VNC, embedded RDP, embedded Web, external process) already go through, so a cluster of mixed protocols now behaves like a cluster of shells. Found while wiring the cluster tab groups above — the same single point that fixes this is what labels the tabs.

- **A tab returning from a split pane lost its group label** — `restore_session_tab` rebuilds the tab for a session leaving a split layout and set the bare connection name as the title, while the `[group] ` prefix is only re-applied by the detach path. The tab stayed a member of its group for every operation and still showed the group in its tooltip, so the label was missing from the one place the user reads it. The prefix is now composed in a single place (`tab_title`) that the creation, rename, group-change and restore paths all use, with a matching `strip_group_prefix` for the one case that has to recover the base name from the rendered title.

#### The floating viewer toolbar

- **The floating viewer toolbar was revealed but inert for its first two seconds — RDP, VNC and Web alike** — every state change routes through `ToolbarAutoHide::show_briefly`, which revealed the toolbar without making it targetable. Only the private `show()`, reached from the arrow handle, ever called `set_can_target(true)`, and the viewers construct their revealer with `can_target(false)` so it does not swallow input while hidden. So the toolbar that appears on connect could not be clicked, and the pointer-motion controller that is supposed to hold it open while hovered never fired either — it became usable only after auto-hiding once and being re-revealed from the arrow. Targetability now follows revealed-ness in one place, which is what the comment beside `can_target(false)` had claimed all along. Present since the auto-hide toolbar arrived; 0.20.0 extended it to Web.

- **The toolbar overflow breakpoints had been overtaken by the touch-target rule** — the three collapse thresholds were eyeballed pixel counts, and `ToolbarAutoHide` later began applying the GNOME HIG 44×44 minimum to every toolbar button, which the numbers predated. The Web toolbar felt it worst: nine buttons at a 44 px floor plus a URL entry need more width than the 520 px breakpoint and its 48 px hysteresis grant, so between roughly 570 and 620 px it expanded and then clipped anyway — the failure the overflow controller exists to prevent. `ToolbarOverflow` now measures the toolbar (`WidgetExt::measure`) instead of comparing against a constant, and reports the same required width whether expanded or collapsed by adding back the parked actions and subtracting the "⋯" button that replaced them. `RDP_OVERFLOW_THRESHOLD_PX`, `SPICE_VNC_OVERFLOW_THRESHOLD_PX` and `WEB_OVERFLOW_THRESHOLD_PX` are gone, and with them a `ponytail` note asking for them to be retuned whenever a toolbar gains a button.

- **Web embedded mode: the reveal handle sat on top of the page, and the toolbar ignored the local theme** — the arrow that summons the hidden toolbar is a 44×44 button pinned to the top centre of the viewer, which is right for a remote desktop and wrong for a web page: the top centre is where a site puts its logo, primary navigation and search, and the button swallowed clicks there. Its position is now the caller's choice (`RevealHandle::TopCentre` for RDP and VNC, `TopTrailing` for Web, which is also where the toolbar's own menu button appears). Separately, `.web-toolbar-overlay` had been merged with `.rdp-toolbar-overlay` to remove a byte-identical duplicate, which also locked the Web toolbar to a fixed dark scrim — fine over a remote desktop drawn in someone else's theme, wrong for local chrome containing a `GtkEntry`, which produced a light URL bar inside a dark bar on a light desktop. The two classes now share their geometry and differ in exactly the one declaration that has to. The handle's `min-width`/`min-height` have been dropped from CSS, where they disagreed with the 44×44 size request that was silently winning.

- **Web embedded mode: the toolbar clipped instead of collapsing in a narrow split panel** — making the toolbar float (see Added) replaced its own responsive rule, a tick callback that hid Home and the secondary actions below 500 px, with nothing. A floating toolbar spans the panel, so in a pane narrower than the assembled width the box overflowed its allocation and GTK clipped the last children — the secondary actions and, worst of all, the menu button, which is the only route to Copy URL, Open in System Browser, Zoom Reset and Clear Session Data. The Web toolbar now uses the same `ToolbarOverflow` controller as RDP and VNC: below `WEB_OVERFLOW_THRESHOLD_PX` (520 px, between the RDP and VNC breakpoints because this is the only toolbar carrying a text entry) Home, Autofill, Zoom In and Zoom Out are *reparented* into a "⋯" popover rather than hidden, so every action stays reachable at any width — which is more than the pre-0.20 rule managed, since the menu it hid them behind never contained them. Back, Forward, Reload, the URL bar and the menu stay in place. `ToolbarOverflow` gained `attach_to_widget` for this: its existing `attach` watches `GtkDrawingArea::resize`, and a WebView has no drawing area, so the new variant reads the allocated width from a tick callback and only acts when the number changes.

#### Embedded Web viewer

- **Web zoom shortcuts (Ctrl+/-/0) did not work in split view** — keyboard shortcuts for zoom were intercepted by WebKitGTK's internal handlers before reaching the application's EventControllerKey. Fixed by attaching the key controller to the container widget with PropagationPhase::Capture, which intercepts events before they propagate to the WebView.

- **…and still only after clicking the toolbar, and then in the wrong panel** — the capture-phase controller above sits on the panel's container, so it only sees a key press that GTK routes into that container, which requires something inside it to hold the keyboard focus. Clicking a toolbar button gave the focus to the button as a side effect of the press, which is why the shortcuts came alive only after using the toolbar; in a split view they kept reaching whichever panel had last been clicked, because a click on the page moved no focus at all. Two things were in the way. A click on the page never reached WebKit: `SplitViewAdapter`'s panel gesture recognises buttons, VTE terminals and the RDP/VNC drawing surfaces as interactive and steps aside for them, but a `WebKitWebView` matched none of those, so the gesture was *claimed* on what it took for empty panel background — which also means links, form fields and text selection were being swallowed in a split panel, not just the focus. It is now recognised like its siblings. And the WebView now takes the focus itself on a press, so any click in the page area routes that panel's shortcuts to that panel's page. A `set_focus_child` chain was tried alongside this and has been removed: nothing calls `grab_focus()` on that container, `gtk_widget_set_focus_child` is documented as an API for widget *implementations*, and GTK overwrites the value during focus navigation — the gesture is what works, and it is the same thing the RDP and VNC surfaces already do.

- **Web embedded mode: a failed page load left the toolbar logic unrun, and a load timeout said nothing at all** — `set_state` grew the toolbar handling for the Error state, but nothing reached it: the only caller passing `Error` was a `report_error` helper with no callers of its own, while the two paths that actually fail — the `load-failed` signal and the 60-second load timeout — assigned the state field directly from closures that hold a few `Rc` clones rather than the widget. So the state and its presentation could disagree, which is the kind of split that quietly outlives the release that introduced it. The presentation now lives in one associated function every writer calls, `report_error` is gone, and the timeout fills the reconnect banner the way a load failure already did — before this it only fired the callbacks, leaving a page that never loaded with no explanation and no Reload button. On Error the toolbar is now *shown* briefly rather than switched off: unlike a remote desktop, an error page is something the user navigates away from, and Back, the URL bar and the menu are the way out.

#### Connection defaults

- **`RdpConfig::default()` and `VncConfig::default()` disagreed with their own serde defaults, and every importer used the wrong one** — `#[derive(Default)]` cannot see `#[serde(default = "…")]`, so five RDP fields and three VNC fields had two different "defaults" depending on how the config was created. A stored profile missing the keys deserialised to `clipboard_enabled: true`, `show_local_cursor: true`, `script_paste_via_clipboard: true`, `jiggler_interval_secs: 60` and `autotype_delay_ms: 20`; `RdpConfig::default()` handed back `false`, `false`, `false`, `0` and `0`. That second set is what every Remmina, RoyalTS, RDM, SecureCRT, CSV, Ásbrú and libvirt import arrived with, along with `models::template`, `sync::inventory`, the connection wizard and RDP quick-connect — so an imported RDP connection had no clipboard, no local cursor, and a 0 ms inter-character autotype delay, fast enough to drop characters on a Citrix or gateway session. VNC imports lost scale-to-fit as well. Both types now have hand-written `Default` impls that call the same `default_true()` / `default_jiggler_interval()` / `default_autotype_delay()` functions the serde attributes name, which is what `SpiceConfig` already did. A new test compares whole structs — `T::default()` against deserialising `{}` — for all four protocol configs, so the next field added with a non-`Default` serde default fails the suite instead of quietly repeating this. Surfaced while adding `hide_floating_toolbar` above: that field is negative precisely so this trap could not catch it, and then the trap was worth closing.

#### Startup and platform

- **A non-system interface language cost macOS users the tray icon (issue [#158](https://github.com/totoshko88/RustConn/issues/158))** — applying a configured language re-executed the process with `LANGUAGE` set in the child, and that re-exec was not platform-gated. On macOS replacing the process image destroys the LaunchServices scene registration `NSStatusItem` needs, which is the exact defect the renderer fallback was moved off `exec()` to avoid this release — diagnosed for `GSK_RENDERER`, missed for `LANGUAGE`, in the same file tree. The variable now goes through `rustconn-env-sys::set_startup_var` like the renderer does, and the `setlocale` call that follows is what makes gettext re-read it. The `_RUSTCONN_LANG_SET` loop sentinel is gone with the re-exec, and startup spawns two processes fewer than before 0.20.0 rather than one.

- **macOS inside a virtual machine: input lag, late frames and stuttering scroll ([#274](https://github.com/totoshko88/RustConn/issues/274))** — Apple's paravirtualised GPU gives a macOS guest Metal but no accelerated OpenGL, and Homebrew builds `gtk4` with `-Dvulkan=disabled`, so GSK has only its GL renderer and Cairo to choose from and the GL one lands on a software path inside a guest: slow, and busy enough to keep a core occupied. The reporter had found `GSK_RENDERER=cairo` themselves; RustConn now reaches the same conclusion on its own, asking `sysctl -n kern.hv_vmm_present` — Darwin's own "am I a guest" answer — and selecting Cairo when the answer is `1`. A probe that cannot be answered counts as "no hypervisor", never as "guest", so a failed `sysctl` on bare metal cannot cost anyone their GPU renderer. The chosen renderer and the reason for it are logged at `info` level. Two details worth recording for the next report: `GDK_SCALE`, which the same workaround set, is an X11-only variable and does nothing on the macOS backend (which takes its scale from `NSWindow.backingScaleFactor`); and the fix had to work for `/opt/homebrew/bin/rustconn` launched directly, not just the `.app`, so it could not live in a bundle wrapper script.

#### Accessibility

- **The header bar's busy spinner lost its accessible name** — the `gtk4::Spinner` → `adw::Spinner` swap dropped the `update_property(Property::Label)` call, because libadwaita 0.9 does not implement `IsA<gtk::Accessible>` for `adw::Spinner` and the call does not compile on the concrete type. Every `GtkWidget` is a `GtkAccessible` in C, so `crate::spinner::set_accessible_label` sets it through a `GtkWidget` upcast and both build paths keep the label a screen reader needs.

#### Localisation

- **POTFILES.in did not list the two modules extracted from `terminal/mod.rs`** — `scripts/check-potfiles.sh` is a CI gate and it fails on an unlisted source that calls `i18n()`, so the terminal split (see Improved) would have turned the release commit's CI red. Extraction itself was never affected — `po/update-pot.sh` globs `rustconn/src` rather than reading the manifest — so no msgid was lost and no catalogue changed; the manifest was simply out of step with the sources it is there to describe. `scripts/release.sh` now runs this check plus `check-i18n-escapes.sh` and `check-po-complete.sh`, the same three the CI `i18n` job runs, so the next drift is caught before a tag instead of after one.

- **Seven translatable strings were in the source but in no catalogue** — regenerating `po/rustconn.pot` for this release surfaced them: the KeePass unlock dialog's four strings, the Backspace/Delete hint, and the two `Automatic (^?)` / `Automatic (\e[3~)` erase-mode labels (the latter in 15 locales; Ukrainian had them). They rendered as English in every locale while `scripts/check-po-complete.sh` reported all 16 catalogues at 100%, because that gate reads the committed `.po` files and never regenerates the POT to compare against — the same class of silent rot `check-potfiles.sh` was written for, in the one direction it does not cover. Now translated in all 16 locales. **Nothing yet checks that the committed POT matches the sources**; that gap is real and outlives this fix.

- **Six strings in the Web connection panel were in no catalogue, for the same reason as the seven above and a different mechanism** — `SwitchRowBuilder::build()` calls `i18n()` on a *variable*, so `xgettext` finds nothing at the call site: "JavaScript", "Private / Incognito Mode" and their subtitles rendered through gettext at runtime with msgids that had never been extracted, which is to say they rendered in English in all 16 locales. `check-potfiles.sh` cannot catch this — the file is listed, it is the string that is invisible. They now go through the `_i18n_markers()` function the same file already keeps for `dialog_header()` labels, and are translated in all 16 locales. The Web panel is the builder's only caller.

### Changed

- **The 60-second Web load timeout now reports itself in the reconnect banner** — the visible half of the state-machine fix above, called out separately because it changes what the user sees: a page that never finishes loading now shows "Connection timed out. Check that the host is reachable." with a Reload button, where before the tab simply sat there.

### Improved

#### Lints, toolchain and CI gates

- **The three crates allowed to write `unsafe` were the only three with no lints at all** — `[workspace.lints.rust]` set `unsafe_code = "forbid"`, and `forbid` cannot be overridden at any level, so `rustconn-pty-sys`, `rustconn-locale-sys` and `rustconn-env-sys` could not inherit the workspace lint table and each declared its own `[lints.rust] unsafe_code = "allow"` instead. A crate-local `[lints]` table *replaces* the inherited one rather than adding to it, so the effect — visible in what cargo actually passes to `clippy-driver` — was that the FFI crates compiled with `--allow=unsafe_code` and nothing else: no `clippy::all`, no `pedantic`, no `nursery`, no `unwrap_used`, no `dbg_macro`. The workspace lint is now `deny`, each helper writes `[lints] workspace = true` and re-opens the one lint with a crate-level `#![expect(unsafe_code, reason = "…")]`, and `expect` rather than `allow` so a `-sys` crate that loses its last `unsafe` block says so instead of keeping a stale exemption. One step weaker on paper, considerably stronger in practice; introducing `unsafe` elsewhere is still a hard error, and the pre-write hook still blocks it before the compiler is reached. Turning the lints on found three real things in `rustconn-pty-sys`, all fixed: two `clippy::borrow_as_ptr` sites where `&ws as *const _` and an implicit `&mut` coercion created a reference and immediately discarded it — asserting to the compiler an aliasing guarantee that `ioctl(2)` and `poll(2)` never agreed to — now `&raw const` and `&raw mut`, and one redundant import in the test module.

- **`rustfmt.toml` stopped configuring two options it could not apply** — `imports_granularity = "Module"` and `group_imports = "StdExternalCrate"` are nightly-only, so on the pinned stable toolchain they did nothing except print a warning apiece per crate: 22 lines of noise on every `cargo fmt` and on the CI format gate, for rules that never ran. Pinning the toolchain to stable 1.97.1 this release made "run it under nightly occasionally" stop being a plan at all. They are gone, replaced by the exact command to apply them deliberately (`cargo +nightly fmt --all -- --config …`). Same class of problem as the `typos.toml` that had no runner, in the other direction.

- **Two configured-but-unexecuted quality tools became CI gates, and the toolchain is pinned** — `typos.toml` had been in the repository fully configured, down to ignore patterns for UUIDs and commit hashes, with nothing ever running it; a config with no runner is decoration. Its first run produced 73 findings and every one was a false positive: HashiCorp, the `flate2` and `writeable` crate names, Ásbrú's own `Parrallels` wire-format spelling, the `bottons` field vnc-rs misspells upstream, a deliberate `prodction` in the CLI docs demonstrating the "Did you mean" suggestion, and base64 certificate fixtures. The vocabulary was therefore recorded with a reason per entry instead of "correcting" the code — several of those corrections would have introduced bugs. `cargo machete` joined it and found one genuinely dead dependency (see Dependencies). Both now run in a new toolchain-free `hygiene` job together with a gate asserting that the copies of the pinned toolchain version agree across `rust-toolchain.toml` and both workflows. The toolchain is pinned in `rust-toolchain.toml`, so a new stable release can no longer turn CI red with no change to this repository — the project already carried an escape hatch for that exact failure, which is the evidence it had happened. MSRV is untouched and still 1.95: the `msrv` job, the MSRV-built RPM and the snap each override the pin with `RUSTUP_TOOLCHAIN` and now *assert* the compiler they actually got, because a pin that silently overrides a deliberately older toolchain is worse than no pin at all.

#### Startup

- **The X11 Cairo fallback no longer re-execs the process** — the #85 workaround used to set `GSK_RENDERER=cairo` by replacing the process image with a copy of itself carrying the variable, because `std::env::set_var` is `unsafe` in edition 2024. That is unavailable on macOS, where an `exec()` destroys the LaunchServices scene registration `NSStatusItem` needs and the tray icon disappears — which is why the guest-VM case above had no fix until the write moved in-process. The environment write now lives in a new `rustconn-env-sys`, the third sanctioned FFI crate, guarded exactly like `rustconn-locale-sys`: `set_startup_var` refuses to run from any thread but the one `main()` started on, refuses once the process has spawned a thread of its own (counted from `/proc/self/task` where the OS offers it), and refuses after `seal_env()` closes the startup window. Both platforms now share one renderer decision in `rustconn/src/renderer.rs`, and startup spawns one process fewer than it did. The crate is an unconditional workspace member rather than a macOS-only dependency on purpose: no CI job builds macOS, so gating it would leave the workspace's newest `unsafe` block compiled by nothing.

- **The single-key `config.toml` scan has one implementation instead of two** — the language and the renderer are both needed before GTK exists, long before the application's settings are loaded, and each was reading its own key out of the file with its own hand-rolled scan. They now share `rustconn/src/startup_config.rs`, which is tested rather than assumed: it no longer confuses a longer key with the same prefix (`renderer_debug` for `renderer`), it stops a value at its closing quote so a hand-added trailing comment is not read as part of the setting, and a value that is not quoted at all reads as unset rather than aborting the scan.

#### Code structure

- **`terminal/mod.rs` split into three modules, shrinking it by 30%** — the 4365-line file has been divided along lifecycle lines. Note what this is and is not: the *file* was split, not the type. `TerminalNotebook` still has 156 methods, now spread over three files with the moved ones widened from private to `pub(super)`, so coupling did not go down — it became visible. The god object is the per-tab state the methods share, and splitting it out is the next step, recorded as a `// ponytail:` note at the top of `mod.rs`.
  - `tab_lifecycle.rs` (889 lines): welcome tab creation, terminal/VNC/RDP/Web tab creation, tab parking for split view, tab restore, widget reparenting
  - `session_lifecycle.rs` (500 lines): reconnect preparation, VTE reset with history preservation, disconnect/connect status indicators, reconnect banner UI, poll cancellation, font refresh after fontconfig changes
  - `mod.rs` reduced to 3052 lines (−1313 lines, −30%)

  Every method moved verbatim; only the visibility widened from private to `pub(super)`. No public API and no behaviour changed.

- **`adw::Spinner` where the runtime has it, `gtk4::Spinner` where it does not — decided in one place** — libadwaita 1.6 introduced `AdwSpinner`, and the six construction sites each carried their own `#[cfg(feature = "adw-1-6")]` pair to choose between it and `GtkSpinner`. The choice now lives in `crate::spinner`, which hands out a `Spinner` type alias, so no call site mentions the feature. Nothing outside that module mentions `spinning` either: `AdwSpinner` animates whenever it is mapped and cannot be stopped, so the fallback hands out a `GtkSpinner` that is already spinning, and GTK advances a CSS animation only for a mapped widget — which makes "show it" the way to start a spinner and "hide it" the way to stop one on both paths.

  The `adw-1-6` feature stays opt-in and out of `default`, so the workspace baseline remains libadwaita 1.5. Ubuntu 24.04 ships 1.5.0 and the snap's `core24` `gnome-46-2404` platform ships 1.5, and `libadwaita-sys` turns a version feature into a hard `system-deps` failure in the build script rather than an unsatisfied dependency — so raising the baseline breaks those builds with an error that names pkg-config, not RustConn. Flatpak and the OBS tiers that have 1.6 or newer already pass `adw-1-7`/`adw-1-8`, which now imply `adw-1-6` and therefore get `AdwSpinner`. Retire the feature once no supported target is below 1.6.

- **One CSS rule for the floating viewer toolbar instead of two identical copies** — `.web-toolbar-overlay` was a byte-for-byte duplicate of `.rdp-toolbar-overlay`, which meant a change to one silently stopped applying to the other. They now share one selector for their geometry and differ in the single declaration that has to (see Fixed: the shared background was wrong for Web).

#### Packaging

- **`packaging/obs/rustconn.dsc` said less than the file OBS actually builds from** — it named neither `libasound2-dev` nor `gettext`, and until this release named no libadwaita at all. It is also not read by anything: `scripts/obs-publish.sh` rewrites `debian.dsc` only. Its `Build-Depends` is now identical to `debian.dsc`'s, and which of the two is live is written down in `packaging/obs/README.md` — a `.dsc` is strict deb822 with no comment syntax, so the note cannot live in the file that needs it.

- **Build dependencies state the versions the crate features actually require** — `libadwaita-1-dev (>= 1.5)` and `libvte-2.91-gtk4-dev (>= 0.76)` in `debian/control`, `packaging/obs/debian.control`, `debian.dsc` and `rustconn.dsc`, and the matching `pkgconfig()` floors in `rustconn.spec`. `rustconn.dsc` did not name libadwaita at all. These mirror the `v1_5` and `v0_76` feature selections, so a too-old system now fails in the dependency solver with a legible message instead of inside a build script.

#### Credential handling

- **The password dialog stopped making an extra copy of the password to zeroize** — 0.20.0 development wrapped the entry's text in `Zeroizing` before handing it to `SecretString`, which allocated a second plaintext copy and then scrubbed the copy it had just made: `SecretString` already owns its `Box<str>` and zeroizes it on drop, and the one plaintext that does escape unscrubbed is the `GString` the entry returns, which GTK frees without zeroing and which no Rust-side wrapper can reach under `unsafe_code = "forbid"`. The intermediate is gone, so the dialog holds one copy where it briefly held two.

### Documentation

- **How to verify a downloaded release artifact is now written down** — the build-provenance attestation described above existed only in this file, which meant nobody downloading a `.deb` or an AppImage would learn that `gh attestation verify` applies to it. `docs/INSTALL.md` gains a section with the command, the offline variant via `gh attestation download --bundle`, and what is *not* covered (the snap, which the Snap Store signs itself, and anything installed from Flathub, OBS, AUR, nixpkgs or Homebrew, which those repositories sign). `SECURITY.md` states what a pass proves and, more usefully, what it does not: provenance answers "who built this, from what source", never "is this source trustworthy". `README.md` points at both.

- **`docs/ARCHITECTURE.md` stopped describing a guard rule that 0.19.21 removed** — three places, plus the crate table in steering, still said the `setlocale` guard "refuses once the process has a second thread". That is precisely the rule that aborted RustConn at startup on Fedora 44 ([#271](https://github.com/totoshko88/RustConn/issues/271)) and was replaced by baseline-growth, and 0.19.21's notes claimed the documentation had been corrected — it had been, in `deny.toml`, `.cargo/audit.toml` and the crate docs, but not here. Worse, the new `rustconn-env-sys` paragraph in this release restated it for a crate that never behaved that way. Both paragraphs now describe the baseline rule, say that the thread count is a Linux-only check, and say plainly that this one clause is a judgement rather than a proof. The dependency diagram is also redrawn: after `rustconn-env-sys` was added to it, the arrow into the `-sys` block appeared to come from `rustconn-cli`, contradicting the caption directly beneath it.

- **The floating toolbar is documented, including how to switch it off and what that costs** — `docs/USER_GUIDE.md` gains the **Session Toolbar** switch under RDP (with the list of actions that have no keyboard route, Ctrl+Alt+Del first), a pointer to it from VNC, the **Navigation Toolbar** switch under the embedded browser next to the keyboard routes that survive it, and a note in Split View that the panel's corner indicator goes with it while the right-click menu does not. `docs/CLI_REFERENCE.md` gains `--vnc-toolbar` and `--web-toolbar`. The Web section's "secondary buttons are hidden below 500px" was also corrected: they are reparented into the "⋯" menu rather than hidden, and the threshold has been measured rather than fixed since the overflow rewrite in this release.

- **The `unsafe` policy is documented as what it now is** — `unsafe_code = "deny"` plus a crate-level `expect` in each helper, with the reason `forbid` was given up written down so it is not "tightened" back. Updated in `docs/ARCHITECTURE.md`, `AGENTS.md`, both steering files, the power's reference table and the crate-boundary guard hook, whose description had also never been updated to mention `rustconn-env-sys`.

### Dependencies

- **Removed**: `tracing-subscriber` as a direct dependency of `rustconn-core`. It had no reference anywhere in that crate — the subscriber is installed by the application entry points, which declare it themselves, and `rustconn-core/src/tracing/mod.rs` said as much in its own doc comment. Found by `cargo machete` on its first run. A library pulling in a subscriber is also an invitation to end up with two in one process, so it is documented as "do not re-add" rather than merely deleted. `md-5`, `vnc-rs`, `gettext-rs` and `native-tls` are declared as machete exceptions in the same pass: the first three are used under an import path that differs from the package name, and `native-tls` is present only to pin a version away from 0.2.17's `Tlsv13` compile bug.
- **Updated**: http-body-util 0.1.4→0.1.5, clap_mangen 0.3.2→0.3.3 (`rustconn-cli`'s man-page generator, build-time only), font-types 0.12.2→0.12.3 (transitively, through read-fonts for `harfrust` and `skrifa`). The GTK stack is already at the newest published bindings (gtk4 0.11.4, libadwaita 0.9.2, vte4 0.10.0, webkit6 0.6.1), and their version features cannot rise while Ubuntu 24.04 (libadwaita 1.5.0, VTE 0.76.0) and snap `core24` are supported targets.

  Both Flatpak and Flathub `cargo-sources.json` manifests were regenerated for the two patch bumps. That is not optional bookkeeping: those files list every crate by exact version and digest, so a `Cargo.lock` ahead of them makes `flatpak-builder` vendor crates the build then cannot find. 0.19.22 had to catch up on a resvg bump that landed without them, which is the same omission one release earlier.
## [0.19.22] - 2026-08-12

### Fixed

- **KeePassXC "Don't save" mode did not unlock the database on demand, reporting credentials as missing (issue [#273](https://github.com/totoshko88/RustConn/issues/273))** — when the KDBX master password storage is set to "Don't save", credential resolution failed with a misleading "Variable Not Configured" dialog even when the variable and its KeePass entry were correctly configured. The actual problem was that the database password was not available in memory — there was no mechanism to prompt for it at connection time, and a successful "Check" in Settings did not keep the password for the session either. Three changes fix this: (1) when "Don't save" is selected and the user verifies the password in Settings, it is now kept in the runtime-only `kdbx_password` field for the remainder of the session (never persisted to disk or keyring); (2) if a connection needs a KeePass credential and no session password is available, the resolver returns a new `KdbxLocked` signal instead of failing with the wrong error; (3) the UI shows an on-demand "KeePass database is locked" dialog with a password entry, verifies the password against the database, stores it in memory for the session, and automatically retries the connection. Subsequent connections in the same session reuse the in-memory password without re-prompting. The password is forgotten when RustConn exits.

- **`kdbx_unlock.rs` was missing from `POTFILES.in`** — the KeePassXC unlock dialog added above calls `i18n()`, and `scripts/check-potfiles.sh` is a CI gate that fails on an unlisted source that does. Extraction itself was never affected, because `po/update-pot.sh` globs `rustconn/src` rather than reading the manifest, so no msgid was lost. (Recorded here rather than under 0.20.0, where it first appeared: the fix commit is part of this tag.)

### Improved

- **Sidebar search uses a cached search engine with result caching** — `filter_connections` no longer allocates a fresh `SearchEngine` on every keystroke. A `DebouncedSearchEngine` stored in `ConnectionSidebar` provides automatic result caching with TTL: repeated or backspace-retype queries return instantly from cache instead of recomputing fuzzy scores across the entire connection list. The cache is invalidated whenever connections are added, edited, deleted, or imported (via `reload_sidebar`), so results are never stale. For large connection databases (hundreds of entries), search-as-you-type is noticeably more responsive.

- **Command palette reuses its stored search engine instead of allocating per-mode** — `filter_commands` previously created a new `SearchEngine::new()` on each invocation even though `CommandPaletteDialog` already held a stored engine passed to every other mode. The redundant allocation is removed; `filter_commands` now receives the existing engine as a parameter.

- **Embedded RDP: suppressed `unused_variables` warning for `frame_stats` without `gfx-h264`** — the `FrameStatistics` variable in the RDP session loop is only used when the H.264 decode path is active. The existing `#[cfg_attr(not(feature = "gfx-h264"), expect(unused_mut, ...))]` annotation now also covers `unused_variables`, eliminating the warning when building with `--features rdp-embedded` alone.

- **Keyboard group dropdowns show what "Automatic" actually sends (issue [#271](https://github.com/totoshko88/RustConn/issues/271))** — the reporter confirmed Backspace works after 0.19.20, but Delete still does not on their device. The dropdown said *Automatic* with no indication that it puts a five-byte VT220 escape sequence (`\e[3~`) on the wire rather than the single byte the other two options name — a device that cannot parse escape sequences silently ignores it, and the user has no cue that *Delete (^?)* is what they need. The labels now read *Automatic (^?)* for Backspace and *Automatic (\e[3~)* for Delete, mirroring the parenthesised notation the non-default choices already use. The group description on both the SSH/MOSH and Telnet pages also changes from a generic "change what these keys send" to an explicit hint: devices that reject one default usually reject both, so the user knows to check Delete after fixing Backspace.

## [0.19.21] - 2026-08-11

- **RustConn did not start at all on Fedora 44: the `setlocale` guard aborted the process (issue [#271](https://github.com/totoshko88/RustConn/issues/271#issuecomment-5258089991))** — 0.19.20 shipped the `rustconn-locale-sys` startup guard described below, and on Fedora 44 Workstation it refused the very first call and panicked before the window ever appeared: `setlocale in a process with 2 threads`. The guard required the process to have exactly one live thread, which is a state an application cannot actually arrange. A shared library's ELF constructor runs before `main()` is entered and may spawn a thread there — Fedora 44 pairs glibc 2.43 with OpenSSL 3.5, and a constructor-spawned thread is present by the time the first statement of `main()` runs. So the check was not catching a `setlocale` call that had drifted out of the startup window; it was reporting a precondition the platform had already made unattainable, and turning it into a hard abort. The guard now samples the thread count on its first call and treats that as the baseline, refusing only when the count *grows* past it. That is the condition the call site actually controls — a thread this program started between the two `init_locale` calls, which is what a regression in `main()`'s ordering would look like — while a library thread that was there before `main()` no longer stops the application from starting. `Refusal::MultiThreaded` becomes `Refusal::ThreadCountGrew { baseline, current }`, so the panic message now names both numbers instead of only the total.

  Not Fedora-specific despite being reported there. Nothing in the mechanism is: any distribution whose shared libraries spawn a thread from an ELF constructor reaches the same abort, and that is a property of the installed libraries rather than of the distribution. Fedora 44 is where the combination landed first; a rolling release picking up the same OpenSSL or p11-kit would follow, as would any system with an `LD_PRELOAD` library that starts a thread.

### Changed

- **The thread-count clause of the `setlocale` contract is documented as a judgement, not a proof (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — 0.19.20's notes in `deny.toml`, `.cargo/audit.toml` and the crate docs all said the guard "refuses to run once a second thread exists". That is no longer what it does, and the notes would have been read as describing current behaviour. They now describe the baseline rule and say why it is a baseline. The SAFETY comment in `init_locale` is explicit that this is the one clause the crate cannot establish by inspection: a pre-`main()` library thread is not reachable from there, so whether it reads locale state is unknown, and tolerating it is a deliberate trade against aborting at startup on a correct call site. What makes the call sound remains the call site — `main()`, ahead of GTK, tokio and the tracing subscriber — with the guard as defence-in-depth against that ordering regressing, which is what the 0.19.20 note already said and the code now matches.

## [0.19.20] - 2026-08-11

### Added

- **SSH connections can choose what Backspace and Delete send (issue [#271](https://github.com/totoshko88/RustConn/issues/271))** — the reporter has devices that do not recognise the codes RustConn sends and need `Ctrl+H` for Backspace, and asked for the switch the Telnet panel already has. The SSH panel of the connection editor gains the same **Keyboard** group: Backspace and Delete can each send *Automatic*, *Backspace (^H)* or *Delete (^?)* — the labels name the control character, and the byte each one puts on the wire (`0x08`, `0x7F`) is documented on the enum variants rather than shown in the dropdown. It is a property of the terminal rather than of the `ssh` command, so it is applied to the session's VTE widget after the tab's terminal settings — which is also why it is re-applied on reconnect, where the same terminal is reused. *Automatic* is the previous behaviour (`^?` for Backspace, the VT220 `\e[3~` for Delete), so stored connections are unaffected and the field defaults on configurations written by earlier releases; neither choice ever hands the decision back to VTE, which would abort the process for want of a PTY to read `VERASE` from (issue [#247](https://github.com/totoshko88/RustConn/issues/247)). The two option types are shared with Telnet rather than duplicated — `TelnetBackspaceSends`/`TelnetDeleteSends` are now `BackspaceSends`/`DeleteSends`, since the remote side's disagreement about the erase byte is not specific to either protocol.

- **MOSH connections get the same Keyboard group (issue [#271](https://github.com/totoshko88/RustConn/issues/271))** — MOSH draws into the same VTE widget as SSH and Telnet, so a host that wants `^H` wants it over MOSH too. `MoshConfig` gains `backspace_sends`/`delete_sends` with the same `Automatic` default, applied both when the session starts and when it reconnects in place. The three protocols now read their pair through one `ProtocolConfig::erase_modes()`, so the terminal side has a single place to ask rather than a match arm per protocol at every call site.

### Fixed

- **Saving Preferences threw away the Backspace/Delete choice on a live session (issue [#271](https://github.com/totoshko88/RustConn/issues/271))** — the setting is installed on the session's VTE widget, and `TerminalNotebook::apply_settings` reinstalls the global erase bindings over every open terminal when the Terminal page is saved. So opening Preferences and clicking Save on an unrelated setting silently put the session back to `^?` until the next reconnect — the same shape as the per-connection theme override that issue [#99](https://github.com/totoshko88/RustConn/issues/99) fixed, and it now gets the same treatment: `reapply_erase_modes` runs right after `apply_settings` and restores each tab's pair from its connection.

- **SFTP connections offered a Keyboard group that did nothing and could overwrite the stored value (issue [#271](https://github.com/totoshko88/RustConn/issues/271))** — SFTP reuses the SSH options tab, so it inherited the new group, but an SFTP session opens a file manager or `mc` rather than a terminal that applies erase modes. Worse, the shared dropdowns sit at whatever the last-shown protocol left them at, so saving an SFTP connection could write another protocol's choice into it. The group is now hidden for SFTP by `apply_general_field_visibility`, and `build_sftp_config` writes the value the connection was loaded with, kept aside in `sftp_erase_modes`, instead of reading the widgets.

- **RDM JSON import still aborted on an integer field, and one bad entry cost the whole file (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — a third report of `invalid type: integer 25, expected a string`. 0.19.8 and 0.19.13 made `ConnectionType`, `Port` and every other string field tolerant, but `Name` was left as a plain `String` on both connection and folder entries, so an entry named after a number was the one remaining field that could still produce exactly that message; `ViewOnly` had the same problem with `0`/`1` under a different message. Chasing fields one at a time cannot end, because RDM's add-on architecture lets any add-on contribute entry fields and there is no schema that covers every export — so the structural cause is fixed too: entries are now decoded one by one instead of the whole document in a single `serde_json::from_str`, and an entry that still cannot be read is reported in the skipped list while the rest of the file imports. It is labelled by its `Name` when it has one, and by its position (`Connection #3`) when it does not — which is in practice the only remaining case, since every field now has a tolerant deserializer and the one thing left that can fail to decode is an array element that is not an object at all, and such an element has no name to report. `Name` and `ViewOnly` accept any scalar form, and a `Credentials` value that is not an object no longer maps onto the credential fields by position. `Port` is now parsed by one `parse_port_text` for both the quoted and the unquoted form: it accepts the `3389.0` that .NET serializers write for an integral value, which previously failed to deserialize and — because the document was decoded in one `serde_json::from_str` — took the whole file down with it. An out-of-range value was never that destructive: the old `u16::try_from(i64)` path already yielded the protocol default. What changed for it is that the rejected value is now visible as a warning naming the connection and the raw text, instead of a silent fallback. A bare array of entries and a single entry from RDM's **Clipboard > Copy** are accepted alongside the usual `{"Connections": [...]}` export, and a JSON file that is not an RDM export now says so rather than reporting a successful import of nothing.

- **Royal TS Telnet sessions were imported as SSH on port 22 (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — Royal TS has no Telnet object: its Terminal object is `RoyalSSHConnection` for Telnet, SSH, RAW, rlogin and serial alike, and its own `ConnectionType` field (`telnet;Telnet`, `ssh;SSH`) picks the protocol. That field was never read, so every Telnet session became an SSH connection on the SSH port. Telnet sessions now import as Telnet with a default port of 23; RAW, rlogin and serial have no RustConn equivalent and stay on the object's default. Object element names are also matched case insensitively, and a connection whose target host is under `ComputerName`, `HostName` or `Host` rather than `URI` is imported instead of being skipped as "Missing host".

- **Royal TS import gave no hint why every connection came in without a password (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — passwords genuinely cannot be imported, and that is a property of the format rather than a gap in RustConn: Royal TS never stores a password in clear text, encrypting it under the document's encryption password when one is set and under a key built into the application when it is not. The importer already marked such connections "prompt for password" but said nothing, leaving the user to discover the empty password on the first connect. The import result now carries a warning naming the limitation, the import dialog renders warnings at all (`ImportResult::warnings` was populated by some importers and never displayed), and `docs/USER_GUIDE.md` explains the limitation and the two ways around it.

- **Import warnings were permanently English, and the list mixed translated and untranslated text (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — `ImportResult::warnings` was a `Vec<String>` filled in by importers in `rustconn-core`, which `po/update-pot.sh` does not scan, so every warning reached the dialog as a finished English sentence no locale could touch. Rendering them next to the dialog's own translated headings produced a result page in two languages at once. The warnings are now a typed `ImportWarning` enum carrying the reason and its arguments; `ImportDialog::format_warning` matches on the variant and calls `i18n_f` with a literal `xgettext` can extract, while `ImportWarning::message` returns that same literal so the two cannot drift and `Display` still renders English for the CLI, logs and tests.

- **An import that produced nothing claimed it had "Successfully imported 0 connection(s)" (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — with warnings displayed for the first time, the summary line above them turned out to be wrong in exactly the case that now matters: nothing imported and nothing failed is neither a success nor an error, yet the success wording was printed anyway, directly above a list explaining why nothing arrived. That case now says "Nothing was imported. See the {} warning(s) below."

- **Royal TS entries with their own encrypted password were left with neither a password nor a prompt (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — the prompt fallback keyed off the assigned credential, so an object that carried its own `<Password>` element and no credential kept `PasswordSource::None`. Since the ciphertext cannot be decrypted from outside Royal TS, that connection had no password and never asked for one. Such objects now get `PasswordSource::Prompt` like the rest and are counted in the "passwords cannot be imported" warning.

- **Several RDM shapes were guessed at instead of reported (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — an empty `ConnectionType` was silently turned into RDP, which is RDM's documented default only for an *absent* field; a blank one names no protocol and is now listed as skipped, while an absent one still means RDP. `Port` `0` and `"0"` are rejected alongside out-of-range values, because zero means "any free port" to a listener and nothing at all to a client. And `looks_like_entry` now requires `ConnectionType` plus one of `ID`/`Name` before treating a top-level object as a single copied entry: `{"Name": …, "Host": …}` is the shape of countless unrelated inventory files, and reading one as a one-connection import was worse than saying the file is not an RDM export.

- **The KeePass database password was never written to the system keyring (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — with KeePassXC configured and its unlock password set to "System keyring" storage, the password never reached the keyring: Seahorse showed no entry and every start logged `KeePass password not found in system keyring`. The keyring key rename in 0.19.18 was not the cause — storing and reading both use `rustconn/kdbx-password`, and legacy entries still migrate on first retrieval. The break came from 0.19.17, which moved the keyring write out of the settings-collect phase into a deferred `save_pending_keyring_credentials()` that runs after the dialog closes. That function reads the password from the collected `SecretSettings`, but `collect_secret_settings()` returns `kdbx_password: None` for keyring storage by design — it must not write an encrypted blob to disk in that mode — so the deferred save found nothing to store and silently did nothing. The collect step now carries the typed password as the runtime-only `SecretString` it already is (`#[serde(skip)]`, so still never serialized) and continues to leave the encrypted blob empty; `AppState::update_settings` correspondingly stops encrypting that password to disk when keyring storage is selected (through `SecretSettings::apply_storage_persistence`, see Improved), so the secret is never duplicated against the user's explicit choice. The password also reaches the secret manager immediately, so the database unlocks without waiting for a restart.

- **Re-entering a vault password in Settings was discarded as "no change" (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — the second half of the same report: on an already-configured backend, typing the password and closing the dialog logged `Settings unchanged — skipping save` and did nothing. The dirty check added in 0.19.17 compares the collected settings against a snapshot taken when the dialog opened, and `PartialEq` for `SecretSettings` deliberately ignores the six runtime-only `SecretString` fields because they are not persisted. A freshly typed password is invisible to that comparison, yet for a keyring-backed backend the save path is the only thing that hands it to the keyring — so the one case where the password mattered most was the one that skipped the save. The check now also asks `SecretSettings::has_new_runtime_secret`, which reports a runtime secret that is newly present or different. A field left `None` never counts, so an untouched open/close round trip stays the no-op it was meant to be.

- **Bitwarden, 1Password and Passbolt had the same keyring hole as KeePass (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — the report named KeePass, but the deferred keyring save reads whatever `collect_secret_settings` put in `SecretSettings`, and the `CredentialStorage::SystemKeyring` arm of all four backends returned nothing for the same reason. So a Bitwarden master password, a 1Password service-account token or a Passbolt GPG passphrase set to keyring storage was collected as `None` and never stored either. Each of those arms now carries the runtime-only `SecretString` as well, so the fix covers the backend the reporter used and the three that would have produced the next three reports.

- **1Password and Passbolt wrote a placeholder literal to disk instead of the encrypted secret (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — with encrypted-file storage selected, the collect step fell back to the strings `encrypted_token_placeholder` and `encrypted_passphrase_placeholder` when no ciphertext was at hand, and those literals were then persisted as if they were the secret. The next read decrypted a placeholder, so the backend behaved as though it had never been configured while the config file looked populated. Found while auditing the four `SystemKeyring` arms above.

- **Moving a secret from an encrypted file to the system keyring destroyed it (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — switching the storage of an already-configured backend without retyping the password left the new keyring entry unwritten and cleared the encrypted blob, because the collected settings had neither the ciphertext (storage changed) nor a freshly typed secret. `SecretSettings::carry_over_runtime_secrets` now brings the in-memory secret forward from the previous settings, so the switch re-homes the existing password instead of losing it.

- **Turning a backend off, or moving its secret elsewhere, left the old keyring entry behind (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — `delete_kdbx_password_from_keyring` and its three siblings existed in `rustconn-core::secret` but had no caller anywhere in the GUI, so a password stayed in GNOME Keyring or KWallet after the user disabled the backend or moved its secret to an encrypted file — the opposite of what changing that setting is for. `SecretSettings::keyring_revocations` compares the saved settings against the previous ones and reports which entries are now stale; `revoke_stale_keyring_credentials` deletes them as part of the same deferred save that writes the new ones.

- **A failed keyring write was a log line under a "Settings saved" toast (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — when the deferred save could not reach the keyring, it emitted `tracing::warn!` and nothing else, so the user saw a success toast for a credential that had not been stored. With keyring storage there is no encrypted blob on disk to fall back on, so that credential exists in memory only and is gone after a restart: losing the message loses data, which the GNOME HIG error-feedback rule puts in a modal dialog. The failure now raises `alert::show_error` explaining that the credential is session-only and what to check, and `KeyringGaps` tracks which backends are still missing an entry so the user can retry the save once the keyring is unlocked instead of having to guess which one failed.

### Changed

- **RUSTSEC-2026-0244 is fixed rather than accepted: `setlocale` moved into a new `rustconn-locale-sys` crate (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — the advisory reports `gettext-rs` `setlocale` as unsound in a multi-threaded program, and 0.19.19 answered it by removing the one call site that ran after threads existed and recording the analysis in `deny.toml`. That left the fixed `gettext-rs` 0.8.0 unusable, because its fix is to mark `setlocale` `unsafe` and every main crate sets `unsafe_code = "forbid"`. The call now lives in `rustconn-locale-sys`, the second sanctioned FFI crate alongside `rustconn-pty-sys`, so `gettext-rs` is on 0.8 and the advisory is off the ignore lists in both `deny.toml` and `.cargo/audit.toml` — `cargo deny check` passes without suppressing it.

  The new crate does more than relocate the `unsafe`: it turns the precondition into something checked. `init_locale` refuses to call `setlocale` if the process already has a second thread (counted from `/proc/self/task` on Linux), if an earlier call came from a different thread, or after `seal_locale()` has closed the startup window — and `i18n::apply_language_from_config()` seals it on every path that returns. So the invariant the previous release could only document and hand-verify by reading `main()` is now enforced: a `setlocale` call added to a running application panics with an explanation on the first run instead of corrupting locale state. The guard is a testable type, since the FFI call itself is deliberately unreachable from the multi-threaded test harness. User-visible behaviour is unchanged, including the Flatpak `LANGUAGE`/`LC_MESSAGES` handling from issue [#158](https://github.com/totoshko88/RustConn/issues/158).

- **A JSON file that is not an RDM export is a parse error, not an empty import (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — the visible behaviour change from the import rework, called out separately because it is the one that can turn a previously "successful" import into a failure. `{}`, `42`, `{"servers": …}` and `{"Name": …, "Host": …}` now return `ImportError::ParseError` naming what RDM export was expected, where before they were read as a document with no connections and reported as a successful import of nothing. An export with an empty list — `{"Connections": []}` — is still valid and still imports zero connections, because there the file really is an RDM export that happens to be empty.

### Removed

- **`locale_is_sealed()` (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — a public predicate on `rustconn-locale-sys` with no caller. Whether the startup window is still open is `init_locale`'s business, and it already refuses and says so; exposing the flag only invited a caller to check it and then race it.

- **`ImportResult::record_warning` (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — took an `impl Into<String>`, which is exactly what made untranslatable warnings easy to add. `ImportResult::add_warning` takes an `ImportWarning` and is the only way in, so a warning cannot be expressed as free text. (`ImportStatistics::record_warning`, a different type, is untouched.)

### Improved

- **One derivation of the erase-mode pair instead of one per call site (issue [#271](https://github.com/totoshko88/RustConn/issues/271))** — `ProtocolConfig::erase_modes()` returns the `(BackspaceSends, DeleteSends)` pair for SSH, Telnet and MOSH and `Automatic` for everything else, so the connect path, the in-place reconnect path and the Preferences re-apply path all ask the same question of the same function. Adding a fourth terminal protocol is a match arm in one place rather than three copies to keep in step.

- **One rule for what is written to disk, instead of a per-backend block in `AppState::update_settings` (issue [#272](https://github.com/totoshko88/RustConn/issues/272))** — the "do not persist an encrypted blob when the keyring holds it" logic had grown asymmetrically: KeePass had it, the other three did not, which is how the placeholder-literal bug above survived. `SecretSettings::apply_storage_persistence` states the rule once and applies it to all four backends, and `update_settings` calls it instead of reimplementing part of it.

- **`init_locale` is `#[must_use]`, and its platform limits are written down (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — a discarded `None` meant the locale had quietly not applied, so the return value now has to be handled or explicitly ignored. Two things the guard cannot do are stated in the SAFETY comment and the crate docs rather than left to be inferred: the thread count comes from `/proc/self/task` and no equivalent is implemented for macOS even though RustConn ships macOS builds, and the advisory's "no POSIX signal handlers" clause is not checkable at all — the Rust runtime installs handlers before `main`, so the condition is already false in every Rust process.

- **`crate-boundary-guard.sh` no longer depends on the checkout being called `RustConn` (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — the hook normalised paths by stripping `*/RustConn/`, so a clone named `rustconn/` or `RustConn-fork/`, or a cwd outside the repo, left the path absolute: the GUI-import invariant was skipped entirely and the unsafe invariant blocked legitimate edits to the sanctioned FFI crates. The greedy strip also cut the crate prefix off if `/RustConn/` appeared again below the root, disabling both. It now asks `git rev-parse --show-toplevel` from the nearest existing ancestor of the target — the file usually does not exist yet, since the hook runs before the write — and falls back to the cwd prefix, keeping the fail-open contract. The unsafe pattern is the `rustconn-*-sys` crate-name shape rather than a hardcoded crate list, and the keyword list covers edition-2024 `unsafe extern`.

- **`deny.toml` and `.cargo/audit.toml` explain the removal without naming an unreleased version (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — the notes left where RUSTSEC-2026-0244 used to be ignored said it was dropped in 0.19.20, a version that does not exist yet and that the entry would outlive anyway. They now say the ignore was removed because the advisory was resolved rather than at a release boundary, and that re-adding it means something regressed.

### Documentation

- **`docs/ARCHITECTURE.md` covers the five-crate workspace (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — the crate list, dependency graph and boundary table still described four crates and named `rustconn-pty-sys` as *the* place `unsafe` lives. They now include `rustconn-locale-sys` and describe the exception as the `rustconn-*-sys` shape, which is what the guard hook enforces.

- **`docs/AI_DEVELOPMENT.md` hook table matches the guard (issue [#267](https://github.com/totoshko88/RustConn/issues/267))** — the `crate-boundary-guard` row said `unsafe` outside `rustconn-pty-sys`; it now says outside a `rustconn-*-sys` crate.

- **`docs/USER_GUIDE.md` documents the Keyboard group, the Royal TS password limitation, RDM's tolerant parsing and keyring write failures (issues [#271](https://github.com/totoshko88/RustConn/issues/271), [#234](https://github.com/totoshko88/RustConn/issues/234), [#272](https://github.com/totoshko88/RustConn/issues/272))** — a new **Backspace and Delete Behavior** section with the three options and which host each suits, noting that MOSH has the same group and SFTP does not; MOSH and SSH rows in the protocol options table; why Royal TS passwords cannot be imported and the two ways around it; which scalar shapes the RDM importer accepts and that a file which is not an RDM export is now rejected outright; and, on the Secrets page, what happens when the keyring refuses a credential and when RustConn removes an entry it previously wrote.

### Dependencies

- **Updated**: gettext-rs 0.7.7→0.8.0 (see above; `gettext-sys` stays at 0.26.0, so no new system build dependency). The Flatpak and Flathub `cargo-sources.json` manifests were regenerated and also caught up with the resvg 0.47.0→0.48.1 bump that landed without them (adding base64 0.23.1, bytemuck_derive 1.12.0, font-types, harfrust, read-fonts, skrifa and dropping core_maths, rustybuzz, ttf-parser and the standalone unicode-* crates).

## [0.19.19] - 2026-08-09

### Fixed

- **Deleting a connection now removes its credential from the password vault (issue [#263](https://github.com/totoshko88/RustConn/issues/263))** — the reporter confirmed 0.19.18 fixed the rename half of this issue but found that deleting a connection left the entry behind in both KeePassXC and the system keyring. Two independent causes. First, deletion is a soft delete into a trash that acts as the undo buffer, and credential cleanup lived in `AppState::empty_trash` — which nothing in the GUI ever called, so no deletion was ever finalized and no credential was ever removed. The Undo toast now defines the undo window: when it dismisses without Undo having been used, the new `win.purge-deleted` action permanently removes that one trash entry and deletes its vault credential on a background thread. Pressing Undo restores the connection with its password intact, tracked by an explicit flag from the toast's `button-clicked` rather than inferred from trash state, so the purge can never race ahead of the restore. Bulk delete offers no Undo affordance and therefore purges immediately. Second, cleanup used the wrong lookup key for the system keyring: `delete_vault_credential` built the flat `"{name} ({protocol})"` key while 0.19.18 had moved storage to the group-scoped `"RustConn/{group}/{name} ({protocol})"`, and an attribute search that matches nothing reports success — so the delete silently did nothing even when it did run. Cleanup now covers every key format a credential may have been written under, current and legacy.

- **Renaming a connection in the configuration panel now updates the vault entry (issue [#263](https://github.com/totoshko88/RustConn/issues/263))** — 0.19.18 fixed the inline rename dialog but not the full connection editor, which is a separate save path. That handler never captured the pre-edit connection and never called any vault migration, so editing the Name field and saving left the credential under the old key. It is the more capable path of the two: it rebuilds the whole connection from the widgets, so one save can change the name, the group *and* the protocol, and all three are part of the lookup key — something neither existing helper could express, since both took a single protocol string and used it for the old and the new key alike. Key derivation for every rename entry point is now computed by one function that takes the old and new connection and derives each protocol from its own connection; `rename_vault_credential` and `rename_vault_credential_for_move` became thin wrappers over it, so a future gap cannot be fixed in one path and missed in the others. When the user also types a new password, that is written under the new key as before and the now-stale entry under the previous key is removed.

- **Credentials of connections outside any group were missed by vault cleanup and migration** — found while testing the above. `save_password_to_vault` stores an ungrouped connection under `RustConn/{name} ({protocol})`, but the new key derivation resolved an absent group to "no group path at all" rather than "an empty group path", producing the bare legacy `{name} ({protocol})`. Delete and migration therefore looked in the wrong place for every ungrouped connection. All three paths now agree with the resolver.

- **System Keyring entries are now visible in KDE Wallet (issue [#264](https://github.com/totoshko88/RustConn/issues/264))** — 0.19.18 added the group path to the keyring key but left the item label as `RustConn: oracle/admin (ssh)`. kwalletd cannot map a `:` onto its folder hierarchy and omits such items from its list entirely, so RustConn credentials only ever showed up in GNOME's Seahorse. The label is now a pure `/`-separated path, `RustConn/oracle/admin (ssh)`, which KDE Wallet renders as nested folders and GNOME Keyring accepts unchanged. The label is cosmetic — lookups match on the item's attribute map, never on the label — so nothing is orphaned; items written by an older release keep their old label until their credential is next saved.

- **Moving a connection between groups no longer orphans its keyring entry (issue [#264](https://github.com/totoshko88/RustConn/issues/264))** — `rename_vault_credential_for_move` and `migrate_vault_entries_on_group_change` returned early for every non-KeePass backend, on the assumption that only KeePass embeds the group path in its key. That stopped being true in 0.19.18 for libsecret and the macOS Keychain, so moving a connection to another group, or renaming a group, left the credential under the old path where the resolver no longer looks. Both migrations now handle the keyring backends as well, moving each entry by retrieve → store under the new key → delete the old one, since `SecretBackend` has no rename operation. Group credentials are keyed by group UUID and are correctly left alone.

- **Deleting a group with a vault password no longer leaves a mangled orphan entry in KeePass** — `delete_group_vault_credential` overwrote the entry with an empty username and password instead of deleting it, and passed an already-prefixed path to `save_password_to_kdbx`, which prepends `RustConn/` itself — writing to `RustConn/RustConn/Groups/{name}`. The entry is now removed with `delete_entry_from_kdbx` using the correct path.

- **macOS Keychain credentials were stored where the resolver never looked** — `generate_store_key_with_group` special-cased only `LibSecret`, so the Keychain fell through to the flat `rustconn/{name}` key while `CredentialResolver` resolved it through the hierarchical keyring path (`RustConn/{group}/{name} ({protocol})`), the same as libsecret. Saving a vault password on macOS therefore produced an entry that could not be read back. The store key now matches the resolver, and the old flat key is included in fallback and cleanup lookups so existing entries are still found.

### Improved

- **Keyring credential retrieval wipes its intermediate plaintext buffers** — `LibSecretBackend::retrieve_value` decoded the secret through a `String` that was left un-wiped on drop, and the malformed-UTF-8 error path dropped a buffer still holding the raw bytes; the error message also embedded the `FromUtf8Error`. Both now match `keyring::lookup`: the byte buffer is `Zeroizing`, the error buffer is wiped explicitly, and the message is a static string. `retrieve` also fetches the non-secret fields first and wraps each secret in `SecretString` as it is read, so no plain `String` holding secret material stays alive across a later fallible await.

- **One implementation for every vault key migration** — the rename, group-move and group-rename paths each carried their own copy of the key-derivation logic, which is how the group path reached the keys in 0.19.18 without reaching three of the four migrations. Key derivation now lives in a single function that takes the old and new connection; `rename_vault_credential` and `rename_vault_credential_for_move` are thin wrappers over it. Derivation is separated from the vault I/O so it is unit-testable without a live backend, and is covered by tests for a rename, a group change, a protocol change, all three at once, the KeePass variant and the flat-key backends.

### Changed

- **The saved interface language is now applied only during startup, before any thread exists** — RUSTSEC-2026-0244 reports `gettext-rs` `setlocale` as unsound when called from a multi-threaded program. RustConn called it from three places, and one of them — `apply_language()` from the GTK `activate` handler — ran after the GIO worker thread was already up. That call site is removed. `apply_language_from_config()` now applies the locale on every path it takes, from `main()`, before the tracing subscriber, GTK and tokio exist, which is the single-threaded precondition the advisory itself names as safe. Behaviour is preserved, including the Flatpak case the removed call existed for: `LANGUAGE` alone is not enough, because gettext ignores it when `LC_MESSAGES` resolves to `"C"` — which is what happens in a sandbox whose host locale is not installed (issue #158). Changing the language in Settings still takes effect on the next start, as before; the removed call could not retranslate already-rendered GTK labels either.

  The fixed `gettext-rs` 0.8.0 is deliberately **not** adopted: its fix is to mark `setlocale` `unsafe`, and this crate forbids `unsafe_code`, with a workspace guard confining unsafe to `rustconn-*-sys` crates. The advisory is instead recorded in `deny.toml` and `.cargo/audit.toml` together with the call-site analysis above; the upgrade path is to move the call into a dedicated `rustconn-locale-sys` crate.

### Dependencies

- **Updated**: openh264 + openh264-sys2 0.9.7→0.9.8, zbus + zbus_macros 5.18.0→5.19.0, zvariant + zvariant_derive 5.13.1→5.14.0, zvariant_utils 3.5.0→4.0.0 (pulls in syn 3.0.3 and zcheapstr 1.0.0 transitively).

## [0.19.18] - 2026-08-09

### Fixed

- **Renaming a connection now updates the credential entry in the password vault (issue [#263](https://github.com/totoshko88/RustConn/issues/263))** — when a connection was renamed, the rename logic for the secret backend already existed but failures were silently logged with `tracing::warn`, leaving the user unaware that KeePassXC (or any other vault) rejected the operation. The rename callback now shows an error toast so the user can take action (e.g. unlock the database or rename manually). Additionally, the Cloud Sync apply paths (both Simple Sync and Group Sync) did not trigger credential rename when a synced connection arrived with a new name — the local vault entry stayed under the old name-based key and subsequent password lookups failed. Both `apply_simple_sync_result` and `apply_group_merge_result` now detect name changes on incoming connection updates and migrate the vault entry before persisting the rename.

- **System Keyring: password collision for same-named connections in different groups (issue [#264](https://github.com/totoshko88/RustConn/issues/264))** — the libsecret/keyring backend stored credentials under a flat key `"{name} ({protocol})"`, which meant two connections named "admin" in different groups (e.g. `oracle/admin` and `pve/admin`) would overwrite each other's password. The keyring key format is now hierarchical: `"RustConn/{group_path}/{name} ({protocol})"` (e.g. `"RustConn/oracle/admin (ssh)"`). This also uses `/` as the path separator, which allows KDE Wallet to map entries into visual folder hierarchies. Existing credentials stored under the old flat key are transparently migrated on first retrieval.

- **KeePass database password in keyring no longer uses a generic entry name (issue [#265](https://github.com/totoshko88/RustConn/issues/265))** — the keyring entry for the KDBX unlock password was stored under the generic key `"kdbx-password"` with label `"KeePass Database Password"`, which could collide with other applications. The key is now `"rustconn/kdbx-password"` and the label is `"RustConn: KeePass Database Password"`. Existing entries are transparently migrated on first retrieval.

### Changed

- **System Keyring credential keys are migrated on first access** — credentials stored under the pre-0.19.18 flat key format (`"admin (ssh)"`) are automatically found via fallback lookup and re-stored under the new hierarchical key (`"RustConn/group/admin (ssh)"`). The migration is transparent and requires no user action. **Note:** downgrading to a version older than 0.19.18 after migration will require re-entering passwords, since older versions do not know the new key format.

## [0.19.17] - 2026-08-08

### Fixed

- **Opening Settings panel no longer wipes in-memory KeePassXC database password (issue [#259](https://github.com/totoshko88/RustConn/issues/259))** — closing the Settings dialog (even without making any changes) triggered a full settings save cycle. Because the password entry widget is intentionally left blank for security, `collect_secret_settings()` returned `kdbx_password: None`, and `update_settings()` unconditionally overwrote the in-memory settings struct — losing the runtime-only password that was loaded at startup from the encrypted file or system keyring. Subsequent credential lookups saw `has_password=false` and failed to open the KeePass database until restart. The fix preserves all six runtime-only `SecretString` fields (`kdbx_password`, `bitwarden_password`, `bitwarden_client_id`, `bitwarden_client_secret`, `onepassword_service_account_token`, `passbolt_passphrase`) from the previous settings when the incoming settings don't provide them — protecting all vault backends, not just KeePassXC.

- **OCI CLI version detection no longer shows Python tracebacks** — when the OCI CLI installation is broken (e.g. missing `oci_cli` Python module), `oci --version` emits a traceback to stderr. The version parser now detects traceback output and returns no version instead of displaying the raw error text in the Preferences panel.

- **Encrypted-file backend no longer fails under parallel test execution** — the machine-key derivation (`get_machine_key`) now uses a process-wide `OnceLock` cache. Previously, concurrent first-time callers could each generate a different random key file, with the last writer winning and earlier encryptions becoming undecryptable. This also protects production use when multiple async tasks trigger credential operations simultaneously at first startup.

### Improved

- **Settings dialog no longer saves to disk when nothing changed** — dirty-tracking compares collected settings against the original snapshot; closing the dialog without modifications skips the save entirely, eliminating unnecessary disk writes and reducing the surface for state-corruption bugs.

- **Bitwarden Unlock and KeePass Check buttons are now asynchronous** — previously both ran blocking `std::process::Command` on the GTK main thread, freezing the UI while the CLI responded (potentially seconds for vault unlock or argon2 key derivation). They now run via `gio::spawn_blocking` with intermediate status feedback ("Unlocking…", "Checking…").

- **Keyring credential saves moved to background thread** — writing passwords to the system keyring (D-Bus round-trip) previously happened synchronously inside the settings collect phase during dialog close. Saves now run asynchronously after `update_settings()` succeeds, and failures are reported via `tracing::warn` with a return value indicating success/failure.

- **SSH Agent add-key failures now show an error toast** — previously the error was only logged; now users see a toast with the failure reason when `ssh-add` rejects a key (e.g. wrong passphrase, unsupported format).

- **Clearer warning when system keyring is unavailable** — the status label now reads "System keyring unavailable — install libsecret (secret-tool)" instead of the previous terse message, giving users actionable guidance.

- **Import dialog no longer blocks the UI during file parsing** — SSH config, Remmina, Asbru, Ansible, and libvirt imports (including `virsh` subprocess calls) now run on a background thread via `spawn_blocking_with_callback`. The progress bar pulses while the operation runs, and results appear asynchronously when complete.

- **Import dialog clearly reports failure when no connections are imported** — previously a successful-looking "Successfully imported 0 connection(s)" appeared even when all entries failed. Now the result page explicitly states "Import failed with N error(s)" when the connections list is empty but errors were encountered.

- **Export "Open location" no longer blocks the main loop** — `open::that` was replaced with `open::that_in_background`, preventing UI freezes when `xdg-open` is slow to respond.

## [0.19.16] - 2026-08-08

### Fixed

- **Embedded RDP no longer reads the local clipboard just to announce it, which is what crashed the process on X11 (issue [#261](https://github.com/totoshko88/RustConn/issues/261))** — the reporter's three coredumps settle what 0.19.15 could only narrow down. All three are the same fault to the byte, and it is a NULL dereference in GTK rather than anything racy: `strcmp(NULL, "STRING")`, reached on a GIO worker thread through `_gdk_x11_display_text_property_to_utf8_list` ← `gdk_x11_text_list_converter_convert` ← a `GConverterInputStream` read ← `read_async_thread` ← `g_task_thread_pool_thread`.

  The GTK path is exact. `get_selection_property()` reports type `None` when a property fetch fails; `gdkselectioninputstream-x11.c` turns that into `priv->type` *before* it checks whether any bytes arrived, and returns the stream as successful anyway; `gdk_x11_get_xatom_name_for_display()` answers NULL for the `None` atom; and `gdkclipboard-x11.c` then hands that server-derived type straight to the text-list converter with no NULL guard, where `g_intern_string(NULL)` keeps it NULL until the converter dereferences it. Reaching it needs the X11 backend, a clipboard read, and a fallback to `COMPOUND_TEXT`/`TEXT`/`STRING` — GDK tries `UTF8_STRING` first and only falls back when that transfer fails, which is why it took time to show up rather than firing on the first copy.

  What made it ours is that RustConn was reading on a hair trigger. The embedded client watched the display-global clipboard and read the selection on *every* change anywhere on the desktop, purely to forward the text to the server — so every copy in any application was another roll of the dice, and 0.19.15's handler leak meant one roll per RDP session ever opened. MS-RDPECLIP never needed that: a client announces which formats it has and supplies the bytes only when the peer answers with a Format Data Request. `rustconn-core` already implemented that flow and documented it. The change handler now sends the announcement alone, and the read happens in the request handler that was already there — once per paste inside the session instead of once per copy on the desktop. The two places that still read on purpose, the Paste button and script paste, are single user-initiated actions and are unchanged.

  Announcing without data needed one guard: `on_format_data_request` answers from its parked payload before it asks the GUI, so text left over from an earlier explicit copy would have been served in place of the new clipboard contents exactly once. The new `AnnounceClipboardFormats` command drops the payload for precisely the formats it announces, which is narrower than the existing bulk clear on purpose — a pending file descriptor from a drag has nothing to do with a text announcement, and a test pins that distinction.

  This removes RustConn from the crashing path; it does not repair GTK. A paste inside the session still goes through `read_text_async`, so the upstream bug remains reachable, just no longer driven by ambient desktop activity. The GTK side has been reported and fixed upstream in [GTK merge request !10227](https://gitlab.gnome.org/GNOME/gtk/-/merge_requests/10227), against [GTK issue #6850](https://gitlab.gnome.org/GNOME/gtk/-/issues/6850).

- **Turning off Clipboard on an RDP connection now stops every clipboard read, not just the automatic ones (issue [#261](https://github.com/totoshko88/RustConn/issues/261))** — 0.19.15 made the setting gate the clipboard watcher, the server→local auto-sync and the reply to a server request. It did not gate the toolbar Copy and Paste buttons or Type Clipboard, all of which read the local selection on demand. That left the setting as a partial measure at exactly the moment users were reaching for it as a workaround: a reporter turned clipboard sharing off on every RDP profile and still saw the crash, which is consistent with those three paths staying live.

  All of them now check the same setting, so switching Clipboard off is a complete escape hatch until the GTK fix reaches distributions. Pressing the buttons with sharing off reports "Clipboard sharing is off" in the session status area rather than failing silently — the alternative, hiding the buttons, would have hidden them for connections whose configuration has not loaded yet.

  Two routes deliberately remain, because neither is gated by a clipboard setting and neither can be closed without removing working features: the string drop targets on the sidebar and split-view panels reach the same GTK converter through `gdkdrop-x11.c`, and a paste performed inside the remote session still asks the client for data. Both are user-initiated rather than ambient. The file drop target on the session view was checked and is not affected: it accepts `GdkFileList`, which negotiates `text/uri-list` and never enters the text-list converter.

### Dependencies

- **Updated**: thiserror 2.0.19→2.0.20, aws-lc-rs 1.17.3→1.18.0, wasm-bindgen 0.2.126→0.2.127, js-sys/web-sys 0.3.103→0.3.104, wide 1.5.0→1.6.1, yuv 0.8.16→0.8.17

## [0.19.15] - 2026-08-07

### Fixed

- **Embedded RDP sessions in Automatic graphics mode showed a frozen desktop (issue [#262](https://github.com/totoshko88/RustConn/issues/262))** — against Windows 11 25H2 the embedded client connected, negotiated the GFX pipeline and then painted nothing at all: server-side `RemoteFX Graphics` counters read 0.00 output frames per second while the session reported itself as active. The cause was on our side, and it was an honesty problem rather than a decoding one. `ironrdp-egfx` 0.3.0 has no AVC444 decoder — it matches `Avc444`/`Avc444v2` and hands those surface updates to a catch-all callback with "AVC444 codec not yet implemented" — yet the capability set RustConn advertised was the crate's default, which includes `V10_7` and therefore tells the server AVC444 is available. Windows takes the client's best offer, so every frame of the desktop arrived in a codec that was then dropped on the floor. Nothing about 25H2 is special here; any host that prefers AVC444 produced the same frozen picture, and the reporter's own FreeRDP baseline confirmed the server side was healthy (5.25 ms per frame through the same pipeline).

  RustConn now advertises only what it can decode: `V8.1` with AVC420 enabled, and `V8` for servers with no H.264 encoder at all. AVC420 *is* decoded, so Automatic mode paints again without giving up H.264. A test pins the requirement so re-adding `V10_7` after an `ironrdp-egfx` upgrade is a deliberate act rather than an accident, and there is a matching note beside the dependency.

  Two safety nets should have caught this and did not, so both were repaired. The existing degraded-quality detector counts *empty* bitmap updates, and AVC444 content never reached that callback — so the counter sat at zero while the session was completely blank. Undecodable surface content is now reported in its own right: the codec is named in the log the first time it appears, and once updates keep being dropped the session retries without GFX, then falls over to the external client, on the same path the pipeline's other failures already used. Separately, the fallback was gated on never having received a first frame, and a single small uncompressed region or legacy bitmap was enough to latch that flag and disable every recovery path for the rest of the session — exactly the half-painted state this bug produced. Recovery no longer depends on that flag; the report fires once per failure run, so it cannot flap.

- **Scrolling, window drags and solid fills were silently discarded in GFX sessions (issue [#262](https://github.com/totoshko88/RustConn/issues/262))** — this is what produced the horizontal bands and unfilled rectangle outlines across an otherwise-working GFX desktop. `ironrdp-egfx` decodes wire codecs but does no compositing: `SolidFill`, `SurfaceToSurface`, `SurfaceToCache` and `CacheToSurface` carry no pixels, only references to content the client is expected to already hold, and they are forwarded to handler callbacks and nothing else. RustConn left all four unimplemented, so the only EGFX content it could paint was AVC420 and uncompressed regions and everything else vanished. The handler now keeps an RGBA copy of every surface, including offscreen ones, plus the bitmap cache, and synthesises a frame update for each operation. Offscreen surfaces are stored but never pushed to the screen, which also fixes updates to unmapped surfaces being dropped with a warning instead of being remembered for the copy that follows. Both stores are bounded — the cache to the 16 MB our `SMALL_CACHE` capability declares — and past the ceiling the handler degrades to forwarding decoded bitmaps rather than growing without limit.

- **A session with no OpenH264 opened a GFX channel it could not paint through (issue [#262](https://github.com/totoshko88/RustConn/issues/262))** — the same freeze by a different route, found while fixing the one above. When OpenH264 failed to load, the pipeline was still registered, just without a decoder, on the documented assumption that it would "fall back to uncompressed/RFX-progressive within the GFX channel". It does not: with no decoder `ironrdp-egfx` advertises `V8` only, Windows answers with RFX Progressive, and there is no progressive decoder either — those PDUs are forwarded to a handler callback and the pixels are lost. The channel is now simply not opened when OpenH264 is missing, so the session stays on the RemoteFX path, which works. RFX Progressive is additionally reported through the same undecodable-content route as any other missing codec, so if a server sends it anyway the session recovers instead of freezing.

- **The session status bar reported a graphics pipeline the session was not using (issue [#262](https://github.com/totoshko88/RustConn/issues/262))** — the mode shown next to the RTT reading came from a compile-time constant: any build with the `gfx-h264` feature claimed "GFX + H.264", including sessions in Legacy or RemoteFX mode that never opened the GFX channel. It now reflects the capability set the server actually confirmed, and sessions that stay on the RemoteFX/bitmap path show the mode their advertised bitmap codecs imply instead.

- **Pressing Backspace in a session could kill the whole window** — the crash was an `abort()` inside libvte rather than a Rust panic: `map_erase_binding(): Assertion 'auto_mode != eTTY' failed`. VTE's default erase binding is `Auto`, and it resolves that by reading `VERASE` out of the termios of the pseudo-terminal it owns — which, since RustConn began creating the PTY itself ([#247](https://github.com/totoshko88/RustConn/issues/247)), is no terminal at all. Upstream added a fallback for the no-descriptor case in November 2025, but the vte 0.84 that distributions ship still carries a second Backspace path without it: in the compiled library one call site tests `m_pty` and falls back to `^H`, while the other passes `eTTY` unconditionally and walks into the assertion. An assertion in a library aborts the process that loaded it, so one keystroke ended every open session at once.

  RustConn now names both bindings on every terminal it creates instead of leaving them at `Auto`, which puts that branch out of reach — the mapper answers from the constant and never goes looking for a PTY. The values are the ones VTE would have arrived at on its own: Backspace sends DEL (`0x7f`), the `VERASE` our `openpty` PTY actually carries and therefore what the remote side's `stty erase` agrees with, and Delete sends the VT220 sequence `\e[3~`. Read-only terminals are covered too — the recording player disables input, and disabled input does not stop VTE from mapping the key before it checks. Telnet's **Backspace sends** and **Delete sends** set to *Automatic* selected exactly the crashing `Auto` and now select the same explicit pair; the explicit Backspace/Delete choices were never affected. A contract test next to the other #247 ones pins the requirement, because this failure mode arrives as a process abort rather than as a failing assertion.

- **Embedded RDP sessions leaked a clipboard watcher that ran a selection read for every copy on the desktop (issue [#261](https://github.com/totoshko88/RustConn/issues/261))** — symbolising the reported coredumps against the same GNOME 50 runtime libraries the reporter ran places the fault on a GLib thread-pool worker: `g_thread_proxy` → `g_thread_pool_thread_proxy` → `g_task_thread_pool_thread` → `read_async_thread` → a `GConverterInputStream` read → GTK's X11 text-list converter, which calls Xlib's `XmbTextPropertyToTextList` directly. In GTK 4.22 that combination is reachable from exactly two places, both of them clipboard and drag reads, and only when the selection owner offers `COMPOUND_TEXT`/`TEXT`/`STRING` instead of `UTF8_STRING`. So the immediate fault is upstream and X11-only, which is why Wayland sessions and the External FreeRDP mode were never affected — but what put several of those conversions on worker threads at once was ours.

  On connect, the embedded IronRDP path subscribes to `changed` on the clipboard belonging to the `GdkDisplay`, not to the widget. `disconnect()` — the method `Drop` and the tab-close path both call — cleared the resize handler but never that subscription; the only code that did was a helper reached solely from connection *failure* paths. Every session that connected successfully therefore left a live watcher behind for the rest of the process, each one starting a `read_text_async` for every copy anywhere on the desktop, so the conversions multiplied with the number of RDP sessions ever opened. Teardown now clears it on every path. A second defect compounded it: the handler id lived in a single slot shared by all connection generations, so a reconnect overwrote it without disconnecting the old handler, and the superseded polling loop then disconnected the id it found — the *live* one. The slot now records which generation installed the monitor, stale loops only remove their own, and a new monitor replaces any predecessor explicitly.

  Clipboard sharing also genuinely honours the profile setting now. `Clipboard` on an RDP connection reached only the CLIPRDR channel in `rustconn-core` and the external FreeRDP argument; the GTK-side watcher, the server→local auto-sync that takes ownership of the local clipboard, and the reply to a server clipboard request all ran regardless. That is why the reporter found that disabling clipboard sharing on every profile changed nothing, and it is the setting to reach for if a session still misbehaves.

  This removes the amplifier rather than the upstream fault, and it is being shipped on that basis: the reconstruction identifies the crashing code and explains why embedded mode alone reached it, but pinning the exact memory error needs the retained coredumps read against debug symbols. That is the next step on the issue.

## [0.19.14] - 2026-08-06

### Added

- **An empty split panel can start a local shell of its own** — until now an empty panel offered exactly one way forward: Select Tab, which moves a session that already exists somewhere else. A **Local Shell** button now sits beneath it for the case where nothing open is worth moving and what is wanted is a scratch shell beside the session being worked on. It is deliberately not the accented button — choosing an existing tab stays the panel's primary action — while sharing the same pill shape so the two read as one group.

  The new session never appears in the tab bar: it is created, placed in the panel and its own tab parked in one go. Two details make that work rather than merely look like it works. Creating a session selects its tab, and parking that tab afterwards leaves `AdwTabView` to pick whichever page happens to be next, so the tab hosting the split is re-selected explicitly — otherwise asking for a shell would throw the user out of the split they were working in. And focus is moved into the new pane, because someone who asked for a shell here means to type here. If the shell cannot be started at all, the session is left in a tab of its own instead of vanishing into a panel that failed to accept it.

### Fixed

- **Session transcripts lost command output and repeated whole screens (issue [#247](https://github.com/totoshko88/RustConn/issues/247))** — the transcript was reconstructed by reading text back out of the terminal widget and comparing it with the previous reading. That cannot be made to work, and the reason is worth recording: VTE rewraps its buffer whenever the window changes width, so a wrapped line stops occupying two rows and every row below it is renumbered *underneath the reader*. A session's terminal is rewrapped at least once on connect, when the widget receives its real allocation, and again on every window resize. Shifted one way the comparison re-reported lines already written, shifted the other it skipped lines that were never written — and with the earlier `skip(previous.lines().count())` variant, output stopped being recorded altogether once the screen filled, so `ls` or `df -h` left no trace at all.

  RustConn now owns the pseudo-terminal instead. The child is started on a PTY that RustConn creates, a reader thread publishes each chunk the moment it arrives, and the transcript is written from those bytes; VTE keeps rendering, key handling, selection and scrollback, and gives back the input it would have written through its `commit` signal, which it emits whether or not it owns a descriptor. The log is therefore a copy of the session rather than an inference about it: in order, complete, with nothing sampled and nothing deduplicated. Output is buffered to line boundaries because a log is read by lines, splitting on `\n` alone so a progress bar redrawing itself with `\r` stays one line rather than hundreds; a trailing line that stops growing — a shell prompt, a password question, neither of which ends in a newline — is written after half a second so it is visible while a connection is still being diagnosed; and the last line of a session, where a `logout` or an authentication failure lands, is written when the process exits. ANSI escape sequences are stripped in both writing paths, including the untimestamped one the default configuration actually uses, so a transcript opened in a text editor is readable, and because stripping runs before redaction an escape sequence in the middle of a prompt can no longer split the pattern redaction looks for.

  The behaviours this rests on are asserted against a real terminal rather than assumed, in `terminal::vte_contract_tests`: that `commit` carries input with no PTY attached, that no VTE signal announces a resize (which is why the window size is pushed down from a 250 ms poll), that cursor rows and text ranges share one coordinate space, and that widening the terminal renumbers rows. Three earlier attempts at this transcript reasoned about those instead of checking them.

- **Closing one pane took the whole split layout apart** — the close returned every remaining pane to its own tab, leaving the split empty. Layouts are looked up in a map keyed by session, and a layout is registered there under *every* session it displays, because several places need to find the layout a given session sits in. Only one of those sessions owns the layout, though: the one that asked for the split, on whose tab the split widget actually lives. Closing that tab does destroy the layout, so its guests have to be moved back to their own tabs first — and the tab-close handler ran that evacuation for whichever session it was handed, since a map lookup cannot tell owner from guest. Closing a guest therefore looked exactly like closing the owner. The layout now records which session hosts it and the evacuation only runs for that one. Removing a pane with **Remove from Split** was never affected: it moves a session without closing a tab.

- **Opening the Settings panel silently corrupted the KeePassXC database password configuration (issue [#259](https://github.com/totoshko88/RustConn/issues/259))** — the storage combo validation handler in the secrets tab treated a pending `secret-tool` detection result (`None`) as "unavailable", reverting the "System keyring" selection to "Don't save" before the dialog was even visible. On close the collected settings wrote `kdbx_save_to_keyring = false`, breaking database access until restart. The same mechanism also cleared the encrypted-file password blob for Scenario 1. The handler now only reverts when detection has **confirmed** absence (`Some(false)`), preserving previously-saved config values loaded before async detection completes. The mirror case is covered as well, and deliberately without touching the configuration: when detection finishes and reports that `secret-tool` really is missing, a keyring selection restored from config stays exactly as it was and the tab explains the problem instead — rewriting that selection behind the user's back is what destroyed the setting in the first place — while the guard still blocks *choosing* the keyring from that point on.

### Improved

- **One PTY path for every session on every platform (issues [#175](https://github.com/totoshko88/RustConn/issues/175), [#247](https://github.com/totoshko88/RustConn/issues/247))** — session commands used to be started two different ways: VTE's `spawn_async` on Linux, and a hand-written `openpty` path on macOS, where VTE's own spawn never connects the child's output to the PTY in the Homebrew build. Every environment or controlling-terminal fix therefore had to be made, and verified, twice. Both platforms now run the same code, and it is the macOS mechanism that survived: create the PTY, size it *before* `exec` so a program that reads its geometry once (`mc`, `vim`, `less`) starts at the real size instead of correcting itself on the first `SIGWINCH`, and spawn the child with the slave as its standard streams and a `pre_exec` hook that claims it as a controlling terminal — which is what lets `ssh` open `/dev/tty` to ask for a password.

  The environment is assembled in one place instead of two — extended `PATH` for CLI tools RustConn downloaded itself, SSH agent socket, the Flatpak and snap config-directory redirections, `TERM`, and the macOS `SSH_ASKPASS_REQUIRE` guard from [#161](https://github.com/totoshko88/RustConn/issues/161) — and it is now held in zeroizing buffers, because a jump host's password reaches `ssh` through it as an askpass variable and RustConn's own copy has no reason to outlive the spawn. Reading and writing the descriptor happen on their own threads, so neither a burst of output nor a large paste into a process that is not reading can block the window; output is bounded to half a megabyte in flight, after which the child is throttled by the kernel rather than the queue growing. Two smaller details: the spare descriptors handed to the child are close-on-exec, so a session's child no longer inherits three stray descriptors pointing at its own terminal, and `Ctrl+Space` still reaches the child, which it would not have — the signal that carries input truncates at the NUL byte that key sends, so the byte is restored from the length the signal also reports.

- **One copy of the split-panel placement sequence instead of two** — moving a session into a pane is nine steps (refuse a detached session, clear it from any other layout, resolve its widget, reparent it, register the bridge, set the tab colour, park the standalone tab, suspend monitoring, refresh and re-wire broadcast), and every one of them matters: skip the park and a dead placeholder tab is left behind, skip the registration and the pane is invisible to the clipboard and broadcast lookups. That sequence existed twice, once per split orientation, which is how [#252](https://github.com/totoshko88/RustConn/issues/252) happened — a fix applied to one copy and not the other. Both Select Tab callbacks and the new Local Shell button now share a single implementation, and the file lost 87 lines in the process.

### Dependencies

- **`nix` is now a dependency of `rustconn-pty-sys`** — only the `term` feature, for `openpty(2)`; in exchange the `rustconn` GUI crate dropped the direct `libc` dependency it no longer needs. No third-party crate version changed in this release.

### Translations

- **Ukrainian is complete again** — regenerating the translation template surfaced four strings that had been added since it was last refreshed and were therefore missing from every catalogue: the US Dvorak RDP layout name, the session-toolbar and panel-actions reveal tooltips, and the jump-host SFTP fallback notice. All four are translated in Ukrainian along with the new Local Shell tooltip. The other fifteen catalogues are unchanged and still complete — those five strings remain absent from them and read as English at runtime, exactly as they did before the template was refreshed. They are now recorded in `po/rustconn.pot`, so a `msgmerge` picks them up whenever a translator takes them on; they were deliberately not machine-filled, since a guessed translation is indistinguishable from a reviewed one once it lands in a catalogue.

## [0.19.13] - 2026-08-06

### Added

- **United States - Dvorak RDP keyboard layout (PR [#258](https://github.com/totoshko88/RustConn/pull/258))** — adds KLID `0x00010409` (stock Windows Dvorak layout) to the RDP keyboard layout dropdown, contributed by @cocide.

### Fixed

- **RDM JSON import rejected real-world Devolutions exports containing numeric fields (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — Devolutions Remote Desktop Manager serializes many fields inconsistently between data-source types: `ConnectionType` as an enum integer rather than a token string, port numbers as bare JSON integers, GUIDs that may be absent or null, and even PIN-style passwords as numbers. Prior versions added a custom deserializer for `ConnectionType` (post-0.19.2) and `Port`, but any *other* field arriving as an integer still triggered a fatal `"invalid type: integer N, expected a string"` and aborted the entire import. Every string field in the RDM parser — including `Folders[].ID`, folder/connection `ParentID`, `Host`, `Username`, `Domain`, `Description`, `Group`, `CredentialConnectionID`, `PrivateKeyPath` and both `Password` fields — now uses tolerant deserialization that coerces scalars (numbers, booleans) to strings and maps nulls to `None`. Missing folder IDs no longer collide in the folder map. Three additional `ConnectionType` numbers are recognized (28 = FTP, 38 = SCP, 100 = SFTP) so those entries are reported as "unsupported" with their name rather than crashing the parse.

- **Embedded RDP drive redirection paths could escape the configured share** — RDPDR requests supplied by the remote server were converted by replacing backslashes and joining the result to the share root, so `..`, an absolute path, or an existing symlink component could reach files outside the exported directory during create, write, rename, or delete operations. The resolver now validates every path component against a canonical share root, rejects traversal, roots, prefixes, and symlink components with `ACCESS_DENIED`, and applies the same containment check to rename destinations.

- **Embedded RDP directory queries ignored the requested path and information class** — directory enumeration always returned the same directory and `FileBothDirectoryInformation`, which broke exact lookups and clients requesting another supported result layout. Queries now honor the requested directory and case-insensitive `*`/`?` pattern, return `.` and `..` without allowing escape, skip symlinks, and encode the exact requested `Directory`, `FullDirectory`, `BothDirectory`, or `Names` information class.

- **Expect rules could send an unresolved placeholder and retain secrets after use** — a response containing an unresolved variable is now dropped instead of typing literal `${name}` into the remote session. Substituted responses and terminal snapshots use zeroizing buffers; rules are explicitly scrubbed on match, replacement, expiry, removal, and clear. Application tracing records only rule metadata and matched-line length, never terminal content or a response value.

### Improved

- **Session transcript captures initial connection output within 500 ms (issue [#247](https://github.com/totoshko88/RustConn/issues/247))** — the output logger previously sampled the VTE buffer every 5 seconds, so SSH `-v` debug lines, login banners and MOTD emitted in the first second carried timestamps 5 seconds late and lacked temporal precision for diagnosing connection failures. A GLib one-shot timer now guarantees capture after a 500 ms grace period even when VTE emits no later `contents-changed` signal, then change-driven captures return to the 5-second interval. Process exit cancels any pending timer, records a final snapshot before flushing, and resets the timer state so an in-place reconnect receives its own 500 ms capture. Transcript snapshots are zeroized when replaced or dropped; sessions without output logging remain unchanged.

- **Embedded RDP and VNC toolbar no longer consumes vertical space and does not block interaction** — the session toolbar (Copy, Paste, Autotype, Ctrl+Alt+Del) is a floating overlay over the remote desktop, preserving the full DrawingArea height and negotiated resolution. The toolbar is revealed exclusively by hovering or clicking a narrow arrow indicator at the top center — the rest of the top edge remains free for window controls and split-view actions. The revealer is fully transparent and pass-through (`can_target = false`) when hidden, so the remote desktop receives all input directly. The crossfade transition (150 ms) avoids the geometry-changing animation that previously forced the overlay to re-clip on every frame and invalidate the DrawingArea. A persistent 44×44 arrow control exposes the toolbar to touch and keyboard users, all toolbar actions meet the same minimum target, and auto-hide pauses while the pointer or keyboard focus is inside the toolbar or a popover is open.

- **Split view panel buttons auto-hide to avoid toolbar conflict** — the close and detach buttons on split panes are now hidden behind a subtle arrow indicator at the top-right corner. Hovering or clicking the arrow reveals the buttons with a crossfade; they hide automatically after 800 ms of inactivity. This eliminates the overlap between split panel controls and the RDP/VNC floating toolbar.

- **Embedded RDP rendering performance improved** — three per-frame optimizations in the draw function: (1) `set_device_scale` is cached and only called when the effective scale actually changes, avoiding Cairo's internal pattern-cache invalidation on every frame; (2) the dark background paint is skipped when the framebuffer covers the full widget at 1:1 scale; (3) nearest-neighbor filtering is used universally instead of bilinear/good — imperceptible at 60 fps interactive rates but significantly faster. Combined with the removal of a full-overlay `EventControllerMotion` that previously fired on every mouse movement, the overall redraw cost is measurably reduced.

### Dependencies

- **Updated**: aho-corasick 1.1.4 → 1.1.5, android_system_properties 0.1.5 → 0.1.6, clap_mangen 0.3.0 → 0.3.1, data-encoding 2.11.0 → 2.11.1, keccak 0.2.0 → 0.2.1, kqueue 1.2.0 → 1.2.1, open 5.4.0 → 5.4.1, regex-automata 0.4.16 → 0.4.18, zlib-rs 0.6.6 → 0.6.7. These are transitive patch releases picked up when the lockfile was regenerated.

## [0.19.12] - 2026-08-04

### Added

- **Terminal history now survives a reconnect (issue [#253](https://github.com/totoshko88/RustConn/issues/253))** — an in-place reconnect (SSH, Telnet, Serial, Kubernetes, Mosh and custom-command sessions) now keeps the previous session's scrollback, with a dim `── Reconnected at … ──` separator opening a fresh line so the preserved output and the new session stay apart.

  Previously, reconnect called `reset(true, true)`, whose second argument empties the scrollback — so a session dropped by a server idle timeout came back to an empty terminal. Two details had to come with the fix: `reset` only switches back to the normal screen in its `clear_history` branch, so a session that died inside a full-screen app (vim, htop, less) would have kept showing that app's frozen screen — `DECRST 1049` is now fed after every history-preserving reset, the one at disconnect included, which also makes the scrollback readable while the tab sits disconnected. And the viewport returns to the bottom, because the user may have scrolled up to read the dead session. "Keep on reconnect" in Settings → Terminal → Scrolling turns the whole thing off; it is on by default.

- **Automatic login for Telnet and serial sessions, with configurable expected prompt text (issue [#254](https://github.com/totoshko88/RustConn/issues/254))** — Telnet and Serial sessions now log in by typing the account name and password at the device's own prompts. The credentials come from the connection's Username and Password Source; a connection with neither set is untouched. Each step fires exactly once — a device that re-prompts after a rejection is handed back to the user, because automatic retries are how an account gets locked out.

  Because vendors word prompts inconsistently (`>>User name:` on a Huawei OLT MA5800, `Username:` on an S6700, `login:` on a Datacom), the Automation tab gained an **Automatic Login** group with **Username Prompt** and **Password Prompt** fields, matched as a case-insensitive substring rather than a regex. All common forms (`login:`, `login as:`, `user:`) are recognized with both fields left empty, so the fields are the exception rather than the setup step. The same two fields exist on a group (Edit Group → **Automation**) and are inherited field by field down the group chain; on the CLI: `rustconn-cli group edit --username-prompt/--password-prompt`. `Last login:` in an MOTD is explicitly not a username prompt.

- **A session can leave a split view without being closed (issue [#252](https://github.com/totoshko88/RustConn/issues/252))** — a session shown in a split pane has no standalone tab of its own (it is parked when it enters the layout), so the only way out of a pane was the × button, which terminates the session. There was no way back to a single tab short of closing everything the split held. Two actions now do it without touching a single connection. **Remove from Split** (`Ctrl+Shift+R`, a button beside × on every occupied pane, and the pane's context menu) hands the focused pane's session back to its own tab; **Remove Split** (`Ctrl+Shift+J`, the pane context menu, and the context menu of the tab that hosts the split) dismantles the whole layout and returns every session in it to a tab. The live widget is reparented rather than rebuilt — the same `reparent_terminal_to_tab` path a split already used when it collapsed on its own — so the PTY, the child process, the scrollback and an embedded RDP/VNC viewer's connection all survive the move, and monitoring, suspended when the session entered the split, resumes against the new container. Asking for the pane that owns the layout collapses the split instead, because the split widget lives in that tab and would be left without a host.

- **Configurable auto-login timeout** — the 10-second deadline the auto-fill watcher uses before giving up is now a per-connection (or per-group) field: **Login Timeout** on the Automation tab, inherited down the group chain. Network equipment with slow POST/boot (Cisco ASR, Huawei MA5800 via serial) can set 30–60 s; the default stays 10 s. On the CLI: `rustconn-cli connection edit --login-timeout-secs 30`.

- **Maximum scrollback on reconnect** — `max_scrollback_on_reconnect` in `config.toml` (under `[terminal]`) limits how many lines of previous-session output survive a reconnect. Without it, a connection that idle-timeouts every few minutes and auto-reconnects would grow the buffer without bound. When set, VTE's scrollback cap is temporarily lowered before the reconnect separator is inserted, trimming the oldest lines. Not exposed in Settings UI yet — edit `config.toml` directly.

### Fixed

- **`${password}` in an expect-rule response resolved to nothing (issue [#257](https://github.com/totoshko88/RustConn/issues/257))** — the stock "Sudo Password" template answers `[sudo] password for …:` with `${password}\n`, the Automation tab offers `${password}`, `${username}`, `${host}` and `${port}` under a "Built-in" heading, and the user guide called the first one "the connection's password from the configured secret backend". None of the four existed. `prepare_rules_from_config` substituted against a `VariableManager` seeded exclusively from `settings.global_variables`, and `substitute_for_command` replaces an undefined reference with an empty string, so the template sent a bare newline — sudo answered "Sorry, try again", the rule was spent (`one_shot`), and typing the same password by hand a second later worked. Reaching for a global variable instead was the documented workaround and failed too, for a second reason: `substitute_for_command` exists to build shell arguments and rejects any value containing `; | & \` $ ( ) < > !`, whereupon the caller fell back to the raw template and typed the literal `${pw}` into the session. All four placeholders are now supplied from the connection at connection time by `window::protocols::automation_variables` — the password from the same credential cache the SSH, Telnet and Serial auto-fill reads, so nothing lands in `config.toml` — and substitution goes through a new `VariableManager::substitute_for_terminal_input`, which validates for a PTY rather than for a shell: metacharacters pass untouched, while a NUL, a line break or another control character is rejected, because a newline inside the value would submit the answer before the rest of it was typed. A password is only copied into a `Variable` when an enabled rule actually references `${password}`, and that variable is marked secret so it is zeroized on drop. Wired into SSH, Telnet, Serial, Kubernetes, Mosh and Zero Trust; a built-in shadows a global of the same name for that connection only, which keeps the workaround working for anyone who set one up.

- **A backslash in a resolved value was reinterpreted as an escape sequence** — `\n`, `\t` and `\\` in an expect response were expanded at match time, i.e. *after* substitution, so the expansion also ran over whatever a variable had resolved to. A password containing `\n` was sent as a line break, splitting it in two and submitting the first half; one containing `\s` lost the backslash. Escapes are now expanded on the template first and substitution happens second, leaving resolved values alone.

- **An expect rule whose response could not be substituted typed the placeholder into the session** — the fallback for a failed substitution was the unsubstituted template, so a rule meant to answer a password prompt sent the characters `${pw}` to the remote host. Such a rule is now skipped with a warning naming the variable, which leaves the prompt for the user to answer. Unresolved references in an otherwise valid response are logged by name too — never by value, since `tracing` output is not redacted.

- **Connect detection and prompt detection read the oldest scrollback instead of the screen (issue [#253](https://github.com/totoshko88/RustConn/issues/253))** — `vte_terminal_get_cursor_position` documents its row as absolute, counted from the start of the scrollback rather than the top of the screen, and both `get_terminal_text` and the fallback in `cursor_line_text` addressed rows `0..row_count` as if they were the visible grid — the same trap the highlight overlay had to fix in [#154](https://github.com/totoshko88/RustConn/issues/154). Latent while a reconnect always started from an empty buffer, each one breaks the moment it does not: the "cursor advanced past the connect banner" check that flips a session to connected (every VTE protocol plus Local Shell) would have fired on the preserved scrollback alone, painting a failed reconnect green, and the jump-host failure scan and password-prompt fallback would have matched against the oldest lines in the buffer rather than what is on screen. The cursor row is now reported relative to the row the current connection started on — zero for a fresh session — and both text helpers are anchored to the visible window.

- **An expect-rule response was written to the application log in clear text** — `AutomationSession` logged every rule's response at INFO on session start and again on each match (`rule.response.escape_debug()`). The redaction that covers session transcripts (`sanitize_output`, which masks credential prompts, tokens and keys) is applied by `SessionLogger` only and has never applied to `tracing`, so anyone following the documented advice to answer a password prompt from an expect rule had that password in `~/.local/share/rustconn/logs` and in the terminal RustConn was started from. Both sites now log the response length instead of its contents.

- **"Move to New Tab" in a split pane's context menu did nothing (issue [#252](https://github.com/totoshko88/RustConn/issues/252))** — the handler mutated `SplitLayoutModel` directly instead of going through `SplitViewAdapter::remove_panel`, the only caller of `rebuild_widgets`, then recorded the result in `last_drop_outcome` and set `needs_rebuild` for the UI layer to act on. Nothing ever did: `take_last_drop_outcome` and `check_and_rebuild` had no callers outside `adapter.rs`. The model therefore drifted out of step with both the widget tree and the bridge's `panel_uuid_map`/`panes` maps, no tab was created and no widget moved — the item was inert for as long as it existed. "Close Connection" in the same menu had the identical defect. Both now activate the window actions that own the full teardown, and the menu is ordered with the destructive item last per the HIG.

- **A collapsed split left its layout behind, and the next split reused it** — when a split came down to a single session, `win.close-pane` hid the bridge widget but left the bridge in `session_split_bridges`. `get_or_create_session_bridge` reuses whatever it finds there, so the next split on any of those sessions picked up the hidden, half-wired layout instead of building a fresh one. Every teardown path now drops the entry for each session that took part.

- **Broadcast kept mirroring keystrokes out of a session that had left the split** — the per-terminal `commit` handler is connected once for the life of a session and never disconnected, and it was gated on `bridge.broadcast_active` alone. A session whose pane was closed, or that moved into a different split, therefore went on feeding its input into the layout it had left. The handler now also checks that the session is still displayed in that bridge.

- **SFTP could not reach a host behind a jump host (issue [#255](https://github.com/totoshko88/RustConn/issues/255))** — copying an RDP connection, switching it to SFTP and configuring the same jump host produced an `mc` window showing local files in both panels. Midnight Commander's `sh://` filesystem shells out to `ssh` with a fixed argument list (`ssh -p <port> -l <user> <host> "echo SHELL:; /bin/sh"`), and its URI syntax carries nothing but user, host and port — the documented options are compression, `rsh` and a port number. *Every* SSH setting the connection held was therefore dropped on that path: the bastion most visibly, but also the identity file, `HostKeyAlias` and custom options. The connection then failed and mc silently left the panel on the local directory, which is what "local files on both sides" was. The one injection point mc leaves open is `$PATH`, since it invokes `ssh` by name: RustConn now writes a per-session `ssh` wrapper plus a generated `ssh_config` under `$XDG_RUNTIME_DIR` and prepends that directory to mc's `PATH`, so the connection's real settings reach the `ssh` mc spawns. The generated file ends with `Match all` followed by `Include ~/.ssh/config` — without the `Match all` the include would be scoped to the preceding `Host` block and silently drop the user's own aliases — and each jump hop gets its own block, because `ProxyJump` does not pass `-i` down to a bastion (issue [#241](https://github.com/totoshko88/RustConn/issues/241)). This generalises the Flatpak-only wrapper that used to inject a writable `known_hosts` and nothing else.

- **A jump host picked from the connection list was invisible to every SFTP path** — a bastion can be configured two ways: as free text in `proxy_jump`, or as a reference to another connection (`jump_host_id`), which is what the jump-host dropdown in the connection editor writes. Resolving the reference form needs the whole connection list, so it only ever happened inside the GUI crate, and `rustconn-core` — where the SFTP builders live — saw the string form alone. The `sftp` CLI consequently built `-J` from the string only, and the `ssh … pwd` probe that finds the login home directory (issue [#212](https://github.com/totoshko88/RustConn/issues/212)) could not reach a host behind a picker-selected bastion, so the file browser fell back to the server root. Chain resolution now lives in `rustconn-core::connection::jump_chain` and is shared by all of them, with the hop ordering, cycle guard and 10-hop cap the SSH terminal path already used.

- **`rustconn-cli sftp` refused connections whose protocol is SFTP** — the guard accepted `ProtocolType::Ssh` only, so the exact connection shape issue #255 describes was rejected with "SFTP is only available for SSH connections". SFTP carries the same `SshConfig`; both are now accepted.

- **A pre-connect port check ran against hosts reachable only through a hand-typed bastion** — `bypasses_direct_probe()` recognised `jump_host_id` and `proxy_command` but not `proxy_jump`, so a connection whose bastion was entered as free text still got a direct TCP probe that could only time out before the connection was attempted.

- **Auto-login answered password-change prompts with the stored password** — `looks_like_password_prompt` accepted any line ending in `password:`, so `Old Password:`, `(current) UNIX password:`, `New password:` and `Retype new password:` all triggered auto-fill. The stored login password is never the right answer to any of these — typing it at `New password:` would set the new password to the old one, and repeating it at `(current)` after a forced change would loop. Lines containing `old password`, `(current)`, `new password`, `retype password`, `confirm password`, `verify password` or `repeat password` (plus Ukrainian and Russian equivalents) are now rejected alongside passphrase prompts.

- **A resolved expect-rule password lingered in memory after the rule fired** — `prepare_rules_from_config` substituted `${password}` into the rule's `response` field as a plain `String`, which was then stored in the `ExpectEngine` for the life of the session and freed without scrubbing. The response is now copied into a `Zeroizing<String>` before being fed to VTE, and the stored copy is explicitly zeroized before removal. `AutomationState::Drop` scrubs any rules still holding credentials when the session closes.

- **The mc SFTP wrapper accepted a non-executable `ssh` from PATH** — `find_real_ssh` checked `.is_file()` but not the executable permission, so a broken symlink or a data file named `ssh` earlier in PATH would be selected and fail. It now also verifies `mode & 0o111 != 0`.

- **Stale SSH agent key files accumulated after a crash** — `materialize_agent_identity` writes a `.pub` file under `$XDG_RUNTIME_DIR/rustconn/agent-keys/` for each agent-sourced connection and removes it on close, but a crash or kill left them behind. A startup prune now removes files older than 12 hours from that directory.

- **Broadcast kept a session wired after it left a split, preventing re-wiring in a new one** — the per-terminal `commit` handler checked `is_session_displayed` (which returns false once the session departs), but `broadcast_wired_sessions` was never cleared, so `wire_broadcast_for_session` refused to re-wire the session when it entered a different split. The set is now cleared on collapse and on individual pane removal.

### Improved

- **One prompt-watching implementation instead of three** — the detect-prompt-and-inject block (a one-shot guard, subscriptions to both `contents-changed` and `cursor-moved`, a 150 ms polling timer scheduled at most once, and a separate 10 s deadline timer) was duplicated verbatim in `start_ssh_connection_internal` and `reconnect_ssh_in_place`, about 136 lines each. Both now call `window::prompt_autofill::install_login_autofill`, which owns the username→password state machine, keeps the secret in a `SecretString` until the moment it is handed to VTE (then `Zeroizing`), and replaces the two-timer arrangement with one repeating timer that checks its own deadline. The issue [#191](https://github.com/totoshko88/RustConn/issues/191) bastion guard stays at both call sites: the target password is still only injected when there is no jump host, the bastion was authenticated out-of-band via `SSH_ASKPASS`, or it uses key/agent auth and never prompts in the VTE.
- **Prompt matching moved into `rustconn-core`** — the new `connection::login_prompt` module holds `looks_like_username_prompt`, `LoginPromptMatcher` and the default prompt list next to the existing `looks_like_password_prompt`, so the matching rules are unit-testable without gtk/vte. Password wins when a line matches both, since answering a password prompt with an account name would send the wrong secret.
- **The file-manager SFTP path falls back to mc when a jump host is configured** — the GVFS sftp backend spawns its own `ssh` with a hardcoded argument list (`-oControlMaster`, `-oControlPath`, `-oForwardX11 no`, `-oNoHostAuthenticationForLocalhost`) and accepts no `-F`, and it is D-Bus-activated so its `PATH` is not ours to set; a jump host cannot be applied there at all. Rather than showing a warning and opening a file browser that cannot reach the target, both SFTP paths now detect the jump host and automatically launch mc with the generated `ssh_config` wrapper instead — which does support bastions (issue #255). A toast informs the user of the fallback. The same warning is logged by `rustconn-cli sftp` (the CLI always used mc).

### Documentation

- **"Variable Substitution in Responses" rewritten around what the placeholders actually do** — the four built-ins are listed with where each value comes from, next to the precedence rule against a same-named global, the fact that the response is typed into the session rather than passed to a shell (so metacharacters survive), the line-break rejection, the escape-before-substitute order, and why a repeated prompt is handed back to the user instead of answered twice. The info label in the Automation tab and the group editor is left as it was: it promised these placeholders "resolve at connection time", which the fix above makes true, and rewording it would have retired a msgid that all fourteen catalogues already translate.

- **New "Automatic Login (Telnet & Serial)" section in the user guide** — what is typed and where it comes from, substring-not-regex matching, the table of prompts recognized without configuration, the one-shot and 10-second limits, and a reminder that Telnet still sends the password in clear text no matter who types it.

### Dependencies

- **Updated**: ipnet 2.12.0 → 2.12.1, libredox 0.1.18 → 0.1.19. Both are transitive patch releases picked up when the lockfile was regenerated; `cargo deny check advisories` is clean. Every bundled Flatpak module is already at its latest upstream release (GNOME runtime 50, FreeRDP 3.30.0, VTE 0.80.5, mc 4.8.33, cJSON 1.7.19, openh264 2.6.0, waypipe 0.11.0), and TigerVNC — the only pinned CLI download — is current at 1.16.2.

### Known Issues

- **Windows Explorer does not auto-refresh after file operations in an RDP drive share** — rename, delete and copy operations now work correctly in the embedded RDP drive redirection (issue [#256](https://github.com/totoshko88/RustConn/issues/256)), but Explorer's directory listing does not update until the user presses F5. This happens because `ironrdp-connector` does not yet expose the `ClientDriveNotifyChangeDirectoryResponse` PDU, so change-notification IRPs are never completed and Explorer keeps its stale view. The underlying data is correct; only the display is stale. A fix requires upstream `ironrdp` work and is tracked separately.

## [0.19.11] - 2026-08-02

### Fixed

- **Homebrew release automation updates comments but leaves formula pinned (issue [#251](https://github.com/totoshko88/RustConn/issues/251))** — the `update-homebrew` CI job copied `packaging/macos/rustconn.rb` into the tap and ran sed patterns that expected an archive tarball URL, but the formula used `url "...RustConn.git", tag: "v0.19.6"`. The sed matched only commented-out examples, so every automated release commit since v0.19.6 patched comments while the active source stayed pinned. The formula now uses `url "https://...archive/refs/tags/vX.Y.Z.tar.gz"` with a sha256 placeholder; the workflow sed is anchored to active directives (`^  url`, `^  sha256`) and a verification gate fails the job unless exactly one URL and one checksum were patched. `scripts/release.sh` validates the template format pre-release so drift cannot reach CI silently.
- **KeePassXC: group passwords failed to retrieve when database is password-protected (issue [#250](https://github.com/totoshko88/RustConn/issues/250))** — the "Load from Vault" button in the group editor and group creation dialogs passed `None` for the database master password when calling `keepassxc-cli`, causing it to exit with code 1 on password-protected databases. Individual connection passwords worked because their code path correctly forwarded the stored master password. Key-file-only databases were unaffected since the `--no-password` flag is valid there. Both call sites now pass `settings.secrets.kdbx_password`.
- **Snap: keyring error message now shows the `snap connect` command (issue [#249](https://github.com/totoshko88/RustConn/issues/249))** — when running in a snap with `password-manager-service` not connected, the startup banner and vault-save dialog now tell the user exactly which command to run (`sudo snap connect rustconn:password-manager-service`) instead of the generic "no system keyring is responding" that implies the keyring is broken. The interface is not auto-connected by snapd policy, so most snap users hit this on first launch.

### Improved

- **Embedded RDP background-tab throttling** — the IronRDP polling loop now detects when the drawing area is not mapped (tab in background) and skips 15 out of every 16 ticks, reducing CPU usage from ~60 Hz to ~4 Hz for invisible sessions while still handling lifecycle events (disconnect, error, watchdog).
- **Eliminated redundant full-collection cloning** — added `list_connections_owned()` and `list_groups_owned()` to `ConnectionManager`, which fill a pre-allocated vector in a single pass. Every `list_groups().into_iter().cloned().collect()` site is gone: 39 of them across 13 window files, plus the state layer and both `ConnectionManager` persist paths. Each one used to build an intermediate `Vec<&T>` and then a second, unreserved `Vec<T>`.
- **Decomposed `setup_ironrdp_polling` into handler functions** — extracted ~300 lines of event handling (frame updates, clipboard sync, file transfer, RTT, display control) into `polling_handlers.rs`. The handlers take one of two borrowed context structs, `FrameContext` and `FileTransferContext`, that the polling closure builds once per tick from variables it already owns; passing each captured widget and cell individually would have traded a 1000-line closure for an 11-argument function signature.
- **Pre-allocated collections in hot paths** — added `with_capacity` hints for trash HashMap construction, group hierarchy index building, and sidebar connection filtering to reduce allocation overhead during startup and search.
- **Moved filesystem cleanup off the main thread** — `empty_trash()` now performs webkit session directory removal (`remove_dir_all`) on a background thread, preventing potential UI freezes on slow storage.
- **Trimmed unused public API surface** — removed `PackageManager`, `detect_package_manager`, and `get_system_install_command` from `rustconn-core`'s crate-level re-exports (only used internally in tests).

### Documentation

- **Snap: `password-manager-service` moved to Manual Interfaces** — `docs/SNAP.md` incorrectly listed it under "Automatic Interfaces"; it has never been auto-connected by snapd and requires an explicit `snap connect`.

## [0.19.10] - 2026-08-01

### Added

- **A way to open the session logs (issue [#247](https://github.com/totoshko88/RustConn/issues/247))** — "Session Logs..." in the primary menu under Sessions, and "Session Log..." in a connection's context menu, which opens the directory *that* connection writes to rather than the shared one. A log viewer had existed since the feature was written, complete with file list, sizes, timestamps and a content pane, but nothing ever instantiated it and it was reachable from nowhere in the UI; its built-in default directory (`$XDG_DATA_HOME/rustconn/logs`) did not even match where logs are actually written. Both entry points now pass the real directory in and show it in the dialog header, since "which directory" was half the question being asked.
- **A leading `~` in a log path template** — the Log Path field is free text and nothing in the chain expanded `~`, so `~/logs/x.log` would have created a directory literally named `~`. A relative template is now anchored to the log directory from Settings instead of the process working directory, which is also what the placeholder suggests.

### Fixed

- **Session logging configured on a connection never wrote anything (issue [#247](https://github.com/totoshko88/RustConn/issues/247))** — the connection editor's Logs tab collected a complete configuration (enable switch, path template, timestamp format, size limit, retention, content toggles), saved it to `connections.toml` and read it back into the dialog, and *nothing else in the program ever looked at it*. `Connection.log_config` had no reader outside the dialog. The only code that actually wrote terminal output was an ad-hoc writer in `MainWindow::setup_session_logging`, which read the global `settings.logging` alone and built a hardcoded `<name>_<timestamp>.log`; the switch in the Logs tab therefore armed nothing, and the path template was never expanded because the only implementation of `${connection_name}`/`${date}`/`${time}`/`${HOME}` lived in `SessionLogger`, in the other of the two parallel logging implementations. That one — `SessionManager`'s `session_loggers`, with rotation, retention and template expansion — was unreachable in the GUI for a different reason: `SessionManager::start_session` is never called at all, so it only ever produced an empty file next to the real one. Logging now resolves one effective configuration per session (`LogConfig::resolve`), the connection's own winning over the global switch, and every write goes through `SessionLogger`. The ad-hoc writer is gone, so timestamp format, size rotation and retention apply on both paths.
- **A session log recorded typed passwords in clear text (issue [#247](https://github.com/totoshko88/RustConn/issues/247))** — with "User Input" on, whatever was typed at a password prompt was written verbatim to a plain file. The redaction used by session *recording* (`sanitize_output`, which masks credential prompts, API keys, tokens, private keys, AWS keys and JWTs) was never wired into logging. `SessionLogger` now redacts everything it writes, so both features are covered by one guard rather than one each.
- **"Retention (days)" never deleted a single file** — the cleanup only ran from `rotate()`, and rotation only happens on a size limit, which the global logging settings do not expose; logs accumulated forever. Age-based pruning now also runs when a session log is opened, and deliberately only inside the log directory RustConn manages: a user-supplied absolute template may point at a shared folder, and deleting unrelated `*.log` files there is not something a connection manager should do on its own.
- **The "Timestamps" switch did nothing** — `settings.terminal.log_timestamps` had no reader outside the settings dialog, and neither did `LogConfig.log_timestamps`, despite the subtitle promising "Prepend `[HH:MM:SS]` to each line in session logs". It now controls the transcript: on, every captured line carries its own timestamp in the configured format; off, the previous layout (one stamped `OUTPUT:` header, lines indented beneath) is kept unchanged. Activity and input records stay timestamped either way — the time is part of the record.
- **A log file that could not be created failed silently** — the only report was `tracing::error!`, which is exactly why the original issue was so hard to diagnose: the file simply never appeared and there was nothing to go on. A failure now raises an error toast naming the reason, and the rejected template is included in the log line.
- **The KDBX "Use password" and "Use key file" switches only hid rows** — both were saved and reloaded correctly but never affected which credentials the database was opened with, so a key file left in the entry kept being used after its switch had been turned off. They now gate the collected key file, the encrypted-blob write and the keyring save.
- **Eight RDP and VNC dropdown labels were untranslated in every language** — "External RDP client", "External VNC client", the three RDP performance modes and two of the VNC ones come from `display_name()` in `rustconn-core`, which `po/update-pot.sh` does not scan, so the strings never reached the catalogue and the `i18n()` at the call sites had nothing to look up. They are now declared in `i18n_markers.rs`, the file that exists for exactly this case.

- **An RDP or VNC session froze instead of reconnecting after the computer woke from sleep (issue [#248](https://github.com/totoshko88/RustConn/issues/248))** — the tab kept showing the last frame, accepted clicks that went nowhere, reported no error and never reconnected. A suspend leaves the TCP connection half-open, and nothing in the embedded client could notice: the active session loop is a `tokio::select!` over `read_pdu()` with no timeout, no tick and no heartbeat, and the socket had only `TCP_NODELAY` set — no `SO_KEEPALIVE` anywhere in the codebase — so the kernel never probed the peer either. The read simply parked forever, no `Disconnected` or `Error` event ever reached the GUI, the widget stayed in `Connected`, and the reconnect banner (which appears only on those two states) was never shown. Typed input made it worse rather than better: with no reset received, the kernel retransmits under `tcp_retries2` for 13 to 30 minutes before reporting a write error. The auto-reconnect that reacts to network changes could not help either, because it only acts on sessions already known to be disconnected. Both embedded clients now enable keepalive on the session socket (15 s idle, 5 s between probes, 3 probes — a dead connection is reported within 30 s) plus `TCP_USER_TIMEOUT` at 30 s, which bounds the retransmission path as well. That turns the silent freeze into a real disconnect, which the existing per-connection auto-reconnect then handles.

- **A frozen remote desktop was indistinguishable from a live one (issue [#248](https://github.com/totoshko88/RustConn/issues/248))** — waiting up to 30 s for the keepalive verdict while looking at a picture that may be minutes old is the actual complaint in the report, so RustConn now detects the resume itself and says so immediately. A one-second timer watches for a wall-clock gap it cannot otherwise explain (`SystemTime`, not `Instant`: on Linux `Instant` is `CLOCK_MONOTONIC`, which does not advance during suspend, so a monotonic timer sees an ordinary tick across a two-hour sleep and cannot detect it at all). On resume, every embedded RDP session that still claims to be connected is dimmed and offered its reconnect banner, with the reason in words. Nothing is torn down on suspicion: a session that outlived a short sleep clears the mark itself the moment its next frame arrives, and the reconnect sweep that follows five seconds later only touches sessions actually confirmed dead, and only where auto-reconnect is enabled. Reconnecting starts a fresh logon rather than resuming the same Windows session — `ironrdp-connector` can read the server's auto-reconnect cookie but has no way to send one back.

- **Picking a key from the SSH agent did not restrict which key was offered** — the connection editor's agent-key dropdown saved the choice (`SshKeySource::Agent { fingerprint }`, mirrored in `agent_key_fingerprint`) and restored it when the dialog reopened, but nothing consumed it at connect time: `resolve_ssh_key_path()` returns `None` for agent sources and `build_command_args()` deliberately emits no `-i` for them, so `ssh` ran with no identity at all and the agent offered every key it held. On an agent holding several keys the wrong one is tried first, and a server that hits `MaxAuthTries` can refuse the connection before the selected key is ever presented. The chosen key's *public* half is now written to a file under `$XDG_RUNTIME_DIR/rustconn/agent-keys/` (mode 0600 inside a 0700 directory) and passed as `-i` together with `-o IdentitiesOnly=yes`, which restricts the attempt to that one key. The public half rather than the private key path on purpose: pointing `-i` at the private key is what caused the double agent confirmation in issue [#125](https://github.com/totoshko88/RustConn/issues/125), whereas a `.pub` file makes OpenSSH ask the agent to sign and prompts once. `IdentitiesOnly` is added only when the identity file was produced — for an agent source without one it would hide every agent key and break authentication outright. If no agent is reachable, or it no longer holds the selected key, the connection proceeds as before with a warning rather than failing.

### Changed

- **An empty per-connection Log Path now means "the log directory from Settings"** — the placeholder and the empty-field default used to be an absolute `${HOME}/.local/share/rustconn/logs/…`, a third location that nothing ever wrote to. Both are now the relative `${connection_name}_${datetime}.log`, which resolves into the configured log directory, and the field's subtitle says so.

### Removed

- **The Encrypted Documents feature, which was never reachable** — a full model layer (`rustconn-core/src/document`, AES-256-GCM with an `EncryptionStrength` enum and per-level Argon2 parameters), a dialog, a set of window actions, an `AppState` API (`create_document`, `open_document`, `save_document`, `close_document`, `is_document_dirty`, …), a sidebar item kind with a dirty indicator and its CSS class, a document tier in `VariableManager` (`VariableScope::Document`, `set_document`, `set_connection_document`, …) and an `ItemType::Document` drag-and-drop case — none of it was ever presented. `ConnectionItem::new_document()` had no caller, the create path discarded the id it produced (`let _doc_id`), no root node was ever added to the sidebar and no menu item or accelerator existed, so a user could not create, open or save a document by any route. The documentation described a notes editor that the model could not represent (it holds connections, groups, variables and templates, with no text field), claimed the files live in `~/.config/rustconn/documents/` — a path that appears nowhere in the code — and stated that unprotected documents are "encrypted with the application master key" when they were written as plain JSON with mode 0600. What it promised is already covered by `.rcn` export, Cloud Sync and Workspaces, so it was removed rather than finished. Variable resolution is now the two tiers it always was in practice, connection then global.

- **`ProgressDialog`** — `rustconn/src/dialogs/progress.rs` was constructed by nothing; long operations report through toasts and banners.

- **A second, unused retry model** — `rdp_client/reconnect.rs` held `ReconnectPolicy`, `ReconnectState`, `DisconnectReason` and `ConnectionQuality`, with `ReconnectPolicy` stored in `RdpClientConfig` and consulted by nothing. Retry behaviour comes from the per-connection `RetryConfig` that the connection editor actually exposes, so keeping a parallel policy next to it would have meant two answers to the same question. Old profiles are unaffected: the field was `#[serde(default)]` and is now simply ignored.

- **Three menu actions that were registered but unreachable** — `unsplit-session` (no menu item, no accelerator, and the split view is dismantled through the tab's own close path), plus `new-snippet` and `new-cluster`, whose dialogs are opened from the Snippets and Clusters managers instead. The two helpers only those actions called (`show_new_snippet_dialog`, `show_new_cluster_dialog`) are gone with them.

- **Two configuration fields nothing read** — `SshConfig::sftp_enabled` was written by five importers, the connection dialog and the template editor, and never consulted: SFTP is offered per protocol, not per flag. `ZeroTrustConfig::detected_provider` cached a provider icon name that no reader ever asked for; `detect_provider()` and `CloudProvider::icon_name()` remain, and the dialog calls them when it needs the icon. Both fields are dropped from `connections.toml` on the next save and ignored when reading older files.

### Documentation

- **The Session Logging chapter says where the files go (issue [#247](https://github.com/totoshko88/RustConn/issues/247))** — `docs/USER_GUIDE.md` now states that either switch arms logging and which one wins, gives the default directory for both native and Flatpak installs, lists the path-template variables, explains that `${HOME}` is the *sandbox* home under Flatpak and which absolute paths a sandboxed session can write to, and documents rotation, retention and redaction.

- **SSH Agent authentication is described** — `docs/USER_GUIDE.md` now has its own paragraph next to the FIDO2 and PKCS#11 ones, stating that the picked key is the only one offered, which flags that produces, where the public key file is written, and what happens when the agent is unavailable.

- **The Encrypted Documents chapter is gone** — the section in `docs/USER_GUIDE.md`, its table-of-contents entry and its row in the Backup & Restore table are removed with the feature, as are the `DocumentManager` lines in `docs/ARCHITECTURE.md`.

### Dependencies

- **Updated**: time 0.3.54 → 0.3.55

## [0.19.9] - 2026-08-01

### Added

- **RDP audio can be left on the remote computer (issue [#245](https://github.com/totoshko88/RustConn/issues/245))** — the connection editor's Features group now has an "Audio" choice with three options instead of the old "Audio Redirection" switch: "Do not play", "Play on this computer" and "Play on the remote computer". RDP has always had three audio modes on the wire — `INFO_NOAUDIOPLAYBACK` and `INFO_REMOTECONSOLEAUDIO` are independent flags — and a single boolean could only ever express one of them, which is why leaving the sound on the remote machine was not expressible at all. "Play on the remote computer" runs the session through external FreeRDP, since `ironrdp-connector` exposes only `enable_audio_playback` and never sets `INFO_REMOTECONSOLEAUDIO`; the fallback is reported like the existing ones for RemoteApp and legacy TLS. `rustconn-cli` gained `--audio-mode <local|remote|none>` on `add` and `update`, with `--audio-redirect` kept as the shorthand for `local`, and `show` now prints the mode instead of an "enabled" line. Existing profiles are read through the old `audio_redirect` boolean (`true` → play locally, `false` → do not play), and both fields are written on save so downgrading and the Remmina/MobaXterm exporters keep working.

### Fixed

- **The RDP audio setting did nothing at all (issue [#245](https://github.com/totoshko88/RustConn/issues/245))** — the "Audio Redirection" switch was saved and reloaded correctly but never read at connect time, so every session ran with audio disabled no matter how it was set. On the embedded client `RdpClientConfig::with_audio()` had no caller, leaving `audio_enabled` at its `false` default, which sets `INFO_NOAUDIOPLAYBACK` in the Client Info PDU; on the external client none of the three FreeRDP argument builders emitted an audio flag, and FreeRDP's own default leaves both `AudioPlayback` and `RemoteConsoleAudio` off. Both paths therefore told the server "do not play audio" — the equivalent of mstsc's "Do not play" — which is why Windows reported no audio device inside the session and the sound could not be heard locally or left on the remote machine. The mode is now passed to the embedded client and emitted explicitly as `/sound`, `/audio-mode:1` or `/audio-mode:2` by every FreeRDP path, before any custom arguments so a hand-written override still wins. Numeric `/audio-mode:` values are used rather than the `redirect`/`server`/`none` aliases, which only exist in recent FreeRDP 3.x.
- **Custom FreeRDP arguments were dropped silently in embedded mode (issue [#245](https://github.com/totoshko88/RustConn/issues/245))** — `custom_args` are FreeRDP command-line options and the embedded IronRDP client has no command line, so they were discarded without a word; this is what made the audio bug so hard to work around, since a hand-written `/sound` appeared to be ignored for no reason. Starting an embedded session with custom arguments set now logs a warning and shows a toast naming External mode as the way to apply them.
- **The disabled audio backend still advertised volume control** — `RustConnAudioBackend::disabled()` returned `AudioFormatFlags::VOLUME` next to an empty format list, and `set_volume()` and `close()` forwarded events to the GUI with no `enabled` check. All three now respect the flag.
- **Bundled FreeRDP could have shipped without a sound backend** — the Flatpak manifests left `WITH_PULSE` and `WITH_ALSA` to CMake autodetection, which defaults them on but turns them off silently when the SDK headers are absent, producing a FreeRDP that cannot play session audio however it is configured. Both are now pinned on in the flatpak, flathub and local manifests, so a missing backend fails the build instead.
- **The snap package did not start at all (issue [#244](https://github.com/totoshko88/RustConn/issues/244))** — every 0.19.x snap died before `main()` with `error while loading shared libraries: libwebkitgtk-6.0.so.4`. The embedded WebKitGTK browser added in 0.19.0 is part of the `default` feature set, so the snap's `rustconn` binary linked `libwebkitgtk-6.0.so.4` and `libjavascriptcoregtk-6.0.so.1`, but only `libwebkitgtk-6.0-dev` was ever added — to `build-packages`, which satisfies the linker at build time and ships nothing. Nothing in the snap's runtime provides those libraries: the gnome-46-2404 platform carries only the GTK3-flavoured `libwebkit2gtk-4.1`, a different soname. The snap is therefore built without `web-embedded` until WebKit can be bundled properly (~120 MB of libraries, ICU is absent from both core24 and the platform, and WebKit's helper processes are looked up at a compiled-in absolute path that needs its own `layout:` bind under strict confinement). Web connections in the snap open in the system browser or a browser command of your choice, and a connection saved with the embedded mode falls back to the system browser with a warning. Not distribution-specific — the snap carries its own runtime, so it failed identically everywhere; Ubuntu 26.04 is simply where it was reported. Flatpak, DEB and RPM were never affected.
- **Nothing in CI noticed that the snap could not start** — `scripts/check-snap-runtime-libs.sh` now unpacks the packed snap and resolves the `DT_NEEDED` list of every binary in `usr/bin` against the libraries the snap, the base and the gnome platform actually provide. Both snap workflows run it right after `snapcraft pack`, and in the release workflow it gates the Snap Store publish, so a snap that cannot start can no longer reach a channel. Run against the shipped 0.19.8 snap it reports exactly the two missing WebKit libraries and nothing else.
- **RDP through an RD Gateway resolved the target host locally and failed (issue [#246](https://github.com/totoshko88/RustConn/issues/246))** — a server reachable only behind a gateway ended with `failed to connect to HOST:3389: failed to lookup address information`, because the gateway never reached the embedded client: `RdpClientConfig::with_gateway()` had no caller, so `uses_gateway()` was always false and the IronRDP path opened a plain TCP connection to the internal name. The MS-TSGU tunnel added in 0.18.7 was therefore unreachable code. The connect path now builds the client's gateway configuration from `RdpConfig.gateway`, with local-address bypass explicitly disabled — private addresses and `.internal`/`.local` names are exactly the targets a gateway exists for, and the default bypass would have skipped the tunnel for most of them.
- **RD Gateway tunnel sent an empty user name and a portless endpoint** — the gateway branch passed the gateway user only when a dedicated one was configured, so the usual "same account as the session" setup authenticated as an empty user against the gateway's HTTP Basic challenge; a bare name is now qualified as `DOMAIN\user` when a domain is set, and existing `DOMAIN\user` or `user@domain` forms are left alone. The tunnel endpoint also omitted the port on the default 443, which `ironrdp-mstsgu` hands straight to `TcpStream::connect` and rejects as an invalid socket address.
- **A failed gateway tunnel stranded the session instead of falling back** — `classify_rdp_failure` sorted every gateway error into `Other`, which does not warrant a FreeRDP fallback, so a gateway that `ironrdp-mstsgu` cannot satisfy (it only offers HTTP Basic) produced an error toast even though the external client handles the same gateway. Gateway failures are now their own `RdpFailureClass::GatewayFailure`, classified ahead of the transport markers that appear in the wrapped cause, and hand the session to external FreeRDP. Rejected credentials still classify as `Authentication` and stop, since the external client would be refused by the same account.
- **Gateway connections to a target port other than 3389 went to the wrong port** — `ironrdp-mstsgu` hard-codes 3389 in the MS-TSGU channel request, so such connections now go to the external FreeRDP client, which forwards the real port via `/gateway:`.
- **`.rdp` import read the gateway port from a non-standard field** — MSTSC writes the port inside `gatewayhostname` (`gw.example.com:444`), which the importer ignored while reading a `gatewayport` key that the documented format does not define; the embedded port is now used, with `gatewayport` kept as a fallback for third-party writers. `gatewayusagemethod:i:0` (never use the gateway) is honoured instead of being ignored, and `gatewaycredentialssource` — which only describes the credential prompt, not a separate account — no longer copies the session user into the gateway user field, leaving it empty to mean "same user as the session". An explicit `gatewayusername` is still imported.

### Dependencies

- **Updated**: clap 4.6.4→4.6.5, clap_builder 4.6.2→4.6.5
- Bundled FreeRDP stays at 3.30.0 and the GNOME runtime at 50 — both are the current upstream releases. `cargo deny check advisories` is clean, and `scripts/check-cli-versions.sh` reports every downloadable CLI current (TigerVNC pinned at 1.16.2, the rest auto-resolved).

## [0.19.8] - 2026-07-31

### Added

- **"Open a new session on every double-click" setting (issue [#242](https://github.com/totoshko88/RustConn/issues/242))** — Settings → Interface → Connections. The smart double-click added in 0.18.3 focuses a connection's running session instead of duplicating it, which removed the pre-0.18.3 way of opening several concurrent sessions on one host: double-clicking the same entry four times. Shift/Ctrl+double-click and right-click → "Open new session" already forced a new session, but neither is as immediate as the plain double-click that used to do it. With the new switch on, every double-click starts another session — the "already running in an external window" hint is skipped as well — and the modifier becomes redundant. Off by default, so the focusing behaviour stays the norm.

### Fixed

- **Remote Desktop Manager JSON import failed on the first entry (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — the importer expected `ConnectionType` to be a token name, while RDM serialises it as the numeric Devolutions `ConnectionType` enum, so a real export aborted with `invalid type: integer 25, expected a string` (25 is `Group`) and imported nothing. Both dialects are now accepted, together with the rest of the export's actual shape: the connection-type tokens RDM really writes (`RDPConfigured`, `SSHShell`, `Putty`, `Iterm`, `TerminalConsole`, `SecureCRT`, `VNC`, `NoVNC`, `Telnet`), the host in `Url`/`HostName` as well as `Host`, a port encoded as a string, and the folder tree, which RDM expresses as `Group` entries plus a backslash-separated `Group` path rather than the `Folders` array RustConn used to look for. Usernames, domains and passwords are read from the nested `Credentials` object and from a `Credential` entry referenced through `CredentialConnectionID` — previously the flat `Username` key never matched RDM's `UserName` and every credential was dropped. An imported password now goes to the secret backend (`PasswordSource::Vault`) instead of being parsed and discarded, and an entry of a type RustConn cannot map is reported in the skipped list with that type named.
- **Royal TS import skipped every RDP connection (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — the parser looked for a `RoyalRDPConnection` element, which does not exist in the format: Royal TS and Royal TSX store RDP as `RoyalRDSConnection` (older documents: `RoyalTerminalServicesConnection`) and keep its port in `RDPPort`, not `Port`. All three names are now recognised, and any other `Royal*Connection` object (web page, file transfer, TeamViewer, ...) is listed as skipped with its type instead of vanishing silently.
- **Royal TS connections imported without a username (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — only `CredentialId` was honoured, so the usual Royal TS arrangements produced no credentials at all: a credential assigned by name (`CredentialName`), typed into the connection (`CredentialMode` 2 with `CredentialUsername`), or — most commonly — inherited from the parent folder (`CredentialFromParent`, `CredentialMode` 1). Username and domain are now resolved through all of these, walking up the folder chain. Passwords stay a prompt: Royal TS keeps them encrypted inside the document, so they cannot be imported.
- **Compressed Royal TS documents could not be read (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — `.rtsz` is the compressed document format and was read as UTF-8 text, which failed with an opaque error. The ZIP container is now unpacked before parsing, `.rtsx` (uncompressed XML) is offered in the file chooser and recognised in batch import, and a document that is neither says so, naming encryption and lockdown as the likely cause.
- **XML entities corrupted imported Royal TS values (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — `quick-xml` reports `&amp;`, `&#39;` and friends as their own events, and each text fragment was written straight into the field, so only the fragment after the last entity survived: a connection named `Dev & Test` was imported as `Test`. Field text is now accumulated across text, CDATA and reference events and committed when the element closes.
- **Royal TS export wrote an element Royal TS cannot read** — the exporter emitted `RoyalRDPConnection` with a `Port` child for RDP and `VNCPort` for VNC; it now writes `RoyalRDSConnection`/`RDPPort` and the VNC `Port` that the format actually defines.

### Documentation

- **Import chapter matches the formats again (issue [#234](https://github.com/totoshko88/RustConn/issues/234))** — `docs/USER_GUIDE.md` now lists Royal TS as `.rtsz`/`.rtsx`, states which object types are imported and why passwords cannot be, and gained a "From Remote Desktop Manager" section covering the JSON export, the credentials option and how `Group` paths become folders.

### Dependencies

- **Updated**: hybrid-array 0.4.13→0.4.14, wide 1.5.0→1.6.0 — the only two compatible updates `cargo update` found; both transitive (RustCrypto and SIMD helpers).
- **Security clean**: `cargo deny check advisories` reports `advisories ok`. `RUSTSEC-2023-0071` (rsa, via `ironrdp` → `picky`/`sspi`) remains accepted in `deny.toml` and `.cargo/audit.toml`.
- **CLI downloads current**: `scripts/check-cli-versions.sh` exits 0 — kubectl 1.36.3, Tailscale 1.98.10, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.125.1, Bitwarden CLI 2026.7.0 and 1Password CLI 2.38.1 all auto-resolve to the latest release, and the one explicit pin, TigerVNC 1.16.2, is still current.
- **Packaging sources current**: GNOME runtime 50, FreeRDP 3.30.0 (newest on pub.freerdp.com), cJSON 1.7.19, openh264 2.6.0, fast_float 8.2.10, waypipe 0.11.0 and mc 4.8.33 are all at their latest upstream release, and the Flathub manifest matches the local one. Snap stays on core24 + gnome-46-2404: no `gnome-*-2604` platform snap exists yet (issue [#174](https://github.com/totoshko88/RustConn/issues/174)).

## [0.19.7] - 2026-07-30

### Added

- **`${password}` in a Custom Command template (issue [#151](https://github.com/totoshko88/RustConn/issues/151))** — a Custom Command can now pass the connection's password to its launcher, e.g. `rustdesk --connect ${id} --password ${password}`. The value comes from a local variable named `password` when one exists, otherwise from the connection's password source. It is never written into a command line: the placeholder is replaced by a reference to the `RUSTCONN_PASSWORD` environment variable, which keeps the secret out of `/proc/<pid>/cmdline` and `ps`. Quoting is supplied by RustConn, so `${password}` and `"${password}"` behave identically. In Flatpak, where the command runs on the host through `flatpak-spawn` (which does not forward the sandbox environment, and whose `--env=` would move the value back into an argv), the variable is handed over a file descriptor via `--env-fd`, backed by a mode-0600 file in `$XDG_RUNTIME_DIR` that the spawned shell unlinks before exec.
- **mDNS `.local` hosts resolve in Flatpak (issue [#241](https://github.com/totoshko88/RustConn/issues/241))** — the GNOME runtime carries no `nss-mdns` and exposes no Avahi socket, so a `.local` name that `ssh myhost.local` resolves on the host could not be resolved inside the sandbox. RustConn now asks the host to resolve such a name (`getent ahosts`, falling back to `avahi-resolve-host-name`) and connects by address, caching the outcome — negative results included — for a minute. SSH and SFTP keep the original name as `HostKeyAlias`, so `known_hosts` entries and host-key verification are unaffected. A no-op outside Flatpak and for every name that is not `*.local`.

### Fixed

- **Double-clicking a connection stopped opening it (issue [#242](https://github.com/totoshko88/RustConn/issues/242))** — the smart double-click added in 0.18.3 focuses an existing session instead of duplicating one, but it treated *any* session the notebook still knew about as live. A tab whose connection has ended keeps its transcript and Reconnect button (`close_on_clean_exit` is off by default), so after the first disconnect every double-click merely re-focused that dead tab. Session liveness is now tracked explicitly and only a connected session is focused. The same applies to external viewers: a *detaching* viewer (remmina, krdc, vinagre) is tracked without a process handle and therefore never clears on its own, so "Already running in an external window" used to block the connection for the rest of the session — only viewers whose liveness RustConn can actually verify count now.
- **An unresolvable hostname aborted the connection (issue [#241](https://github.com/totoshko88/RustConn/issues/241))** — the pre-connect TCP probe reported a DNS failure as a hard error, vetoing the launch. The probe is a latency optimisation, not an authority on reachability: a name it cannot see may still be resolvable by the client that actually connects, and a genuinely wrong name produces that client's own, more accurate error a moment later. Such a probe is now reported as "unresolved" and the connection proceeds.
- **Enter in the connection sidebar did nothing** — the key handler returned `Propagation::Stop`, swallowing the event the `ListView` needs to emit `activate`. This is the fix the 0.18.5 notes claimed; it had never actually been applied to the handler.
- **"Restore sessions on startup" did nothing (issue [#243](https://github.com/totoshko88/RustConn/issues/243))** — the setting and a complete, versioned, unit-tested persistence model (`rustconn_core::session::restore`) both existed, but nothing ever wrote or read them, so the switch had no effect at all. The open sessions are now snapshotted to `session_restore.json` when the window closes and reopened on the next start, honouring "Prompt before restoring" and the maximum-age limit. Sessions whose connection had already ended are not restored, and a connection deleted in the meantime is skipped with a warning instead of failing the whole restore. Split and detached layouts are out of scope — that is what Workspaces are for — so such a session comes back as an ordinary tab.
- **Host, Port, Username and password could not be edited for a Custom Command (issue [#151](https://github.com/totoshko88/RustConn/issues/151))** — these rows were hidden for the whole Zero Trust protocol group, yet 0.19.5 made the Custom Command template resolve `${host}`, `${port}`, `${username}` and `${name}` from exactly those fields, leaving the placeholders unfillable. Custom Command now shows them (and the password source); the other Zero Trust providers, which authenticate through their own CLI, still hide them. The inline password row is recomputed rather than only hidden, so an existing password no longer stays invisible depending on the order in which the dialog was populated.
- **A password-only connection never cached its resolved credential** — caching required a username to be present, so a connection that authenticates by password alone dropped the secret that had just been resolved.

### Removed

- **Dead `SessionRestoreSettings::saved_sessions` field and `SavedSession` struct** — never written and never read; the restore snapshot is a file, not a settings key.
- **`PortCheckError::ResolutionFailed`** — no longer produced now that a resolution failure is a `PortCheckResult::Unresolved` outcome instead of an error.

### Documentation

- **`${password}` documented (issue [#151](https://github.com/totoshko88/RustConn/issues/151))** — `docs/ZERO_TRUST.md` gained a "`${password}` and the command line" section covering the resolution order, the environment-variable indirection and its two limits (the launched program's own argv, and the Flatpak portal D-Bus call), plus a note that Host/Port/Username are editable for Custom Command.

### Dependencies

- **Audited, nothing to update**: `cargo update` locks 0 packages to newer compatible versions. The 16 transitive crates behind their latest release are held there by their own parents' requirements — the RustCrypto pre-releases (`aes-gcm`, `curve25519-dalek`, `ecdsa`, `ed25519-dalek`, `p256`/`p384`/`p521`, `primeorder`, `rfc6979`, `x25519-dalek`) and `picky 7.0.0-rc.25` come in through `ironrdp` → `sspi`/`picky`, and the `toml 0.8` / `toml_edit 0.20` chain is a second, transitive copy unrelated to the workspace's own `toml 1.1.4`. None can move without an upstream release.
- **Security clean**: `cargo deny check advisories` reports `advisories ok`; `cargo audit` finds no vulnerabilities across 730 dependencies, only the 12 already-accepted warnings (the unmaintained gtk-rs GTK3 bindings pulled in by the tray backend, `atomic-polyfill`, `proc-macro-error`, `rustybuzz`, `ttf-parser`, and the unsound `glib 0.18` iterator — all transitive). `RUSTSEC-2023-0071` (rsa, via `ironrdp` → `picky`/`sspi`) remains accepted in `deny.toml` and `.cargo/audit.toml`.
- **CLI downloads current**: auto-resolved kubectl 1.36.3, Tailscale 1.98.10, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.125.1, Bitwarden CLI 2026.7.0 and 1Password CLI 2.35.0 all resolve to the latest release; the one explicit pin, TigerVNC 1.16.2, is still current. All version endpoints reachable.
- **Packaging sources current**: GNOME runtime 50 is still the newest branch published on Flathub, and every bundled pinned source is at its latest upstream release — FreeRDP 3.30.0, cJSON 1.7.19, openh264 2.6.0, fast_float 8.2.10, waypipe 0.11.0, mc 4.8.33. The Flathub manifest matches the local one. VTE stays pinned below 0.81 by design.
- **Snap still on core24**: no `gnome-*-2604` platform snap exists in the store, so the core24 + gnome-46-2404 combination remains the only option for a strictly-confined build (issue [#174](https://github.com/totoshko88/RustConn/issues/174)).

## [0.19.6] - 2026-07-29

### Added

- **Local macOS release gate** — `scripts/macos-ci.sh` adds a local-only format, Clippy, test, supply-chain, bundle, signing and linkage audit workflow without changing GitHub Actions. Optional Developer ID signing, hardened runtime, notarization, stapling and validation are available through explicit build-script flags and a notarytool keychain profile.

### Fixed

- **Auxiliary secrets used a Linux-only helper on macOS** — auxiliary keyring operations now delegate to the native Security.framework-backed macOS Keychain implementation instead of invoking `secret-tool`. Blocking Keychain calls run outside the async executor with a 10-second timeout, and retrieved secret intermediates remain zeroizing.
- **Platform-specific Clippy regressions** — corrected the target-aware `statvfs` conversion expectation and removed an unused window observer so the canonical macOS feature profile passes with warnings denied.
- **H.264 decoding was unreachable on macOS** — the OpenH264 loader only probed Linux `.so` paths, so H.264 silently fell back to non-AVC codecs even when the library was present. macOS now probes the bundled `Contents/Frameworks/libopenh264.dylib` first, then Homebrew prefixes for development runs, and the canonical bundle fails to build if OpenH264 is missing instead of shipping a degraded artifact.
- **DMG signing could misrepresent an unsigned app** — `build-dmg.sh --skip-build` wrapped whatever bundle was on disk, so a Developer ID DMG could enclose an unsigned or ad-hoc application and only fail later at Apple's notary service. The packager now verifies the enclosed app strictly and rejects a mismatched identity, an ad-hoc signature, or a missing hardened runtime before signing or notarizing.
- **Malformed keyring values were not wiped** — invalid UTF-8 read from the macOS Keychain and from the Linux Secret Service is now zeroized before the error is returned, and the macOS decoding path is covered by unit tests.
- **Keychain timeouts could hide a late mutation** — a timed-out Keychain call is no longer abandoned silently; its outcome is observed and logged, the error states that the operation may still complete, and the per-key idempotence that makes this safe is documented.

### Improved

- **Self-contained macOS application and DMG** — `scripts/macos-build.sh` is now the canonical `.app` producer and recursively bundles and relocates all 58 non-system dynamic libraries into `Contents/Frameworks`, rewrites install names to `@rpath`, embeds GTK/libadwaita resources, schemas, icons, locales and OpenH264, and rejects unresolved absolute non-system dependencies. `build-dmg.sh` consumes that verified bundle instead of maintaining a divergent second builder.
- **Canonical macOS feature profile** — all macOS build, package and local-CI paths now use `tray-macos,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,adw-1-8`; removed Linux-only or deleted feature drift from macOS commands.
- **Explicit macOS signing policy** — unsigned output remains the default, ad-hoc signing requires `--adhoc`, and Developer ID signing proceeds inside-out (frameworks, CLI, main executable, app) with hardened runtime, entitlements and timestamps before optional notarization and stapling.
- **macOS supply-chain coverage** — `cargo-deny` now audits both `aarch64-apple-darwin` and `x86_64-apple-darwin`, matching the supported native and universal release targets.
- **Local gate covers every shipped crate** — `scripts/macos-ci.sh` now runs the GUI and full CLI test suites in addition to `rustconn-core`, so a failure there can no longer be reported as a passing gate.
- **Release source pinning made explicit** — the Flathub manifest intentionally carries only the tag, since the release commit cannot exist while the manifest is prepared; the immutable commit is added afterwards by hand or by the Flathub bot. The Homebrew formula documents its tag-only form as a temporary pre-tag state that must become an archive plus measured checksum before publication.

### Documentation

- **macOS build and release documentation refreshed** — documented the canonical producer and feature set, self-contained layout, local CI, ad-hoc and Developer ID workflows, notarization commands, wrapper resource path and current `0.19.6` examples.

### Dependencies

- **Updated**: displaydoc 0.2.6 → 0.2.7, toml 1.1.3+spec-1.1.0 → 1.1.4+spec-1.1.0.
- **CLI versions audited**: auto-resolved kubectl 1.36.3, Tailscale 1.98.10, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.122.2, Bitwarden CLI 2026.7.0 and 1Password CLI 2.35.0 are current; the explicit TigerVNC 1.16.2 pin remains current.
- **Security advisory formally accepted**: `RUSTSEC-2023-0071` (rsa 0.10.0-rc.18 Marvin Attack, medium 5.9) is reached only through `ironrdp 0.17.0` → `ironrdp-connector 0.10.0` → `picky 7.0.0-rc.25`/`sspi 0.21.3` → `rsa`, has no published fix, and is not exploitable in a local desktop client that exposes no RSA decryption oracle. The acceptance is now recorded in both `deny.toml` and a new `.cargo/audit.toml` so `cargo audit` and `cargo deny` agree, and it is re-reviewed when IronRDP, sspi or picky publish a new major version. No new Rust dependencies were added for the macOS work.
- **Audit tooling wired into local CI**: `cargo audit` and `cargo outdated` are installed and now run inside `scripts/macos-ci.sh`; both pass, with all root workspace dependencies reported up to date.
- **Build requirement**: producing the macOS bundle now requires Homebrew `openh264`, because the canonical feature set enables `gfx-h264`.
- **Future-incompatibility tracked**: `block 0.1.6` still triggers Rust's uninhabited-static compatibility warning through `gettext-rs 0.7.7` → `locale_config 0.3.0` → `objc-foundation 0.1.1`; all four crates are already at their latest published compatible versions, so resolving it requires an upstream fix or maintained fork.

## [0.19.5] - 2026-07-28

### Fixed

#### SSH

- **`unix_listener: path … too long for Unix domain socket` with long hostnames (issue #239)** — `ssh_control_path()` reserved only ~20 bytes for the `%r` (remote username) expansion and ignored the `.<16 random chars>` suffix OpenSSH binds while setting up the multiplex master, so a UUID subdomain combined with a UUID username overflowed `sun_path` and the session died before the ControlMaster socket appeared. The path is now sized against the full worst case (username budget plus the temporary master suffix), and hosts that still do not fit are identified by a 12-char SHA-256 digest of host+port instead of a truncated hostname — two hosts sharing a long prefix can no longer collapse onto the same master connection.

#### Custom Command (issue #151)

- **`${variable}` placeholders were never substituted** — a template such as `rustdesk --connect ${id}` went to `sh -c` verbatim, so the shell expanded `${id}` to an empty string even when a matching local variable existed. Placeholders are now resolved from the connection's local variables (Data tab), the synthetic connection fields (`host`, `port`, `username`, `name`) and the global variables, with local values taking precedence. Unknown references are left untouched so genuine shell expansion (`${HOME}`) keeps working, resolved values are rejected when they contain shell metacharacters, and secret values are masked in the echoed command line and the session log.
- **A one-shot command left a dead terminal behind** — launchers like RustDesk or WinBox return as soon as their own window is up, leaving a tab with a "session disconnected" notice and a pointless reconnect button. A Custom Command tab now closes itself on a clean exit regardless of the global close-on-exit setting; a failing command still keeps its tab so the output stays readable.
- **Tags could not be edited for a Custom Command** — the Tags field was hidden for the whole Zero Trust protocol group even though tags are protocol-independent metadata used by search and smart folders. It is visible for every protocol again.
- **In-place reconnect broke the command line** — the reconnect path wrapped the already complete `sh -c <template>` invocation in another shell, which turned the first template argument into `$0`. Both the initial launch and the reconnect now go through one shared builder.
- **The Command Template field did not say where placeholders come from** — the connection editor and the wizard now explain that `${…}` resolves from local variables and connection fields.

#### Sidebar (issue #237)

- **Folders could not be nested by drag and drop** — the drop handler recognised a dragged folder but only ever reordered it among its existing siblings: `reorder_group` shuffles `sort_order` and rejects folders that do not already share a parent, so a drop onto another folder either changed nothing or failed with a log-only error. Dropping a folder on the middle band of another folder now nests it, dropping it on a folder's top or bottom edge makes it a sibling at that level, and dropping it on a connection moves it into that connection's folder. Reparenting goes through the same `move_group_to_parent` path as "Move to Group…", so `KeePass` entry paths migrate with the subtree. Nesting a folder inside its own subtree is refused with an error toast instead of failing silently. Import folders stay off limits in both directions: neither an Import folder (or anything inside it) can be dragged elsewhere, nor can a folder be nested anywhere in an Import subtree — the next sync run would recreate the old layout, so the drop is refused with a toast that says why.
- **Emoji longer than two codepoints were saved but never drawn** — `validate_icon` accepts sequences of up to ten codepoints, while every render site decided "emoji or icon name" with a `chars().count() <= 2` check. ZWJ sequences and tag flags therefore passed validation, were stored, and then went to the icon theme as a name, which drew nothing. That decision now lives in one place, `dialog_utils::is_glyph_icon()`, shared by the sidebar, smart folders, the template list and the wizard's template buttons. Keycap sequences are accepted by validation as well.

### Improved

- **Unresolvable icon names fall back to a visible icon (issue #237)** — a stored GTK icon name that the active theme does not carry used to render as blank space. In Flatpak that is the normal case: the GNOME runtime ships only the Adwaita theme, so names found in a host icon browser cannot be resolved inside the sandbox. Connection and folder rows now fall back to the protocol or folder icon and log the unresolved name at debug level.
- **New strings localised in all 16 languages** — both folder-drop rejection messages (own subtree, sync-managed folder) are translated in be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn.

- **Release gate covers `rustconn-cli/Cargo.toml` and `po/rustconn.pot`** — both carry the release version but were absent from `PKG_FILES` in `scripts/release.sh`, so a stale value there could not fail the release. The pot header is now bumped by the release-version checklist and the CLI manifest is checked by the gate (17 packaging files instead of 16).

### Documentation

- **Custom Command placeholders documented (issue #151)** — `docs/ZERO_TRUST.md` gained a "Variable Placeholders" section listing the resolution order (local variables → connection fields → global variables), the metacharacter rejection, secret masking, and the one-shot tab-close behaviour. The old note claiming that `${…}` is never substituted was removed.
- **Sidebar drag and drop and icon fallback documented (issue #237)** — `docs/USER_GUIDE.md` describes the three drop zones (into a folder, sibling before/after, onto a connection), the self-nesting and Import-folder restrictions, multi-codepoint emoji support, and the fallback for icon names the active theme does not carry.

### Dependencies

- **Updated**: socket2 0.5.10 → 0.6.5 ([#238](https://github.com/totoshko88/RustConn/pull/238), the MPTCP socket helper needed no code change), aes 0.9.1 → 0.9.2, clap_complete 4.6.7 → 4.6.8, event-listener 5.4.1 → 5.4.2, toml_parser 1.1.2 → 1.1.3, tray-icon 0.24.1 → 0.24.2
- **Audited, no change needed**: `cargo deny check advisories` clean; pinned CLI downloads (kubectl 1.36.3, Tailscale 1.98.9, Teleport 18.10.0, Boundary 0.21.3, Hoop.dev 1.121.1, Bitwarden 2026.7.0, 1Password 2.35.0, TigerVNC 1.16.2) all current; Flatpak/Flathub bundled sources already at their latest releases (FreeRDP 3.30.0, cJSON 1.7.19, openh264 2.6.0, waypipe 0.11.0, mc 4.8.33, fast_float 8.2.10, GNOME runtime 50), VTE stays pinned below 0.81 by design; Snap still on core24 + gnome-46-2404 because no core26 GNOME extension exists yet (issue #174).

## [0.19.4] - 2026-07-27

### Added

- **Detachable session windows (issue #236)** — move any in-process session (VTE, embedded RDP/VNC, Web) into its own top-level window via tab context menu, `Ctrl+Shift+M`, or a per-monitor submenu. The live widget is reparented without reconnecting — scrollback, monitoring, recording, and tunnels survive the move. A header-bar button returns the session to a tab. Close/quit confirmation counts detached sessions; re-activation always presents the main window. Not offered for external-viewer sessions or unsplit tabs.

### Fixed

#### Detachable windows (issue #236)

- **Split picker could steal a detached session** — detached sessions are now filtered out of both split pickers; the placement rule is checked before touching the widget.
- **Detaching the last tab left the main window blank** — parking the last tab now recreates the Welcome tab.
- **Disconnect/auto-reconnect ignored detached sessions** — session feedback and auto-reconnect now follow the session into whichever window holds it.
- **Detached sessions missing from session manager and workspaces** — they are now listed, counted, and saved/restored like tabbed ones.
- **Re-attaching a session lost tooltip and group label** — tooltip, host line, and group metadata are now restored with the tab.
- **Disconnected embedded RDP/VNC in a detached window showed no feedback** — a "Session disconnected" banner is now raised in the detached window.
- **Reconnect-by-restart came back as a tab** — placement is remembered across the restart step.
- **Rename did not propagate to open sessions** — a rename now updates titles in tabs and detached windows.
- **Failed attach left one session in two places** — the path now rolls back completely on failure.
- **Activity/silence toasts landed on the main window** — notifications now target the window that holds the session.
- **Quit confirmation appeared on the wrong window** — the dialog is now parented to the focused session window.
- **Detached reconnect could target the wrong session** — reconnect now registers a one-shot observer and verifies the result before declaring success; the window is restored to the same monitor by stable identity.

#### RDP

- **"Server only supports Standard RDP Security" killed the session instead of falling back (issue #235)** — `negotiation failure` wording was not matched by the fallback detector. Such failures are now classified as `SecurityUnsupported` and handed to FreeRDP.
- **FreeRDP fallback broken on 3.24/3.25 (regression from 0.19.3 fix for issue #234)** — the `file:` prefix only exists from FreeRDP 3.26; the launcher now probes `--version` and picks bare path or `file:` accordingly.
- **CredSSP logon failures triggered a pointless FreeRDP fallback** — the NTSTATUS was never propagated to the GUI. Wrong credentials are now mapped to `AuthenticationFailed` at source and reported without a fallback attempt.
- **Legacy RDP fallback silently weakened transport security** — downgrading to Standard RDP Security now requires explicit user consent; auth failures never trigger the downgrade.
- **FreeRDP fallback could freeze GTK and leave orphan clients** — version probing and spawning now run in a bounded background task; stale child processes are terminated.
- **Secrets could enter the args file through unvalidated extra arguments** — password-bearing options (`/p:`, `/password:`, `/gp:`, `/gateway-password:`, `/pth:`) are now rejected before the file is opened.

### Removed

- **Dead `external_window` module (`ExternalWindowManager`)** — 288-line stub never wired to any UI; replaced by the new `detached_window.rs` implementation.

### Improved

- **RDP failure classification moved to `rustconn-core`** — `classify_rdp_failure()` returns a typed `RdpFailureClass` covered by unit tests, replacing fragile `msg.contains(..)` checks in the GTK layer (issues #199, #234, #235).
- **Single code path for session tab content** — `build_session_content` + `park_tab_page` replace the monolithic `reparent_terminal_to_tab`, eliminating duplication for the detach/attach path.
- **HIG-compliant menu wording and window titles (issue #236)** — multi-monitor entry drops the ellipsis (submenu, not dialog); untitled connections use protocol as window title.
- **Placement tests assert contracts, not code copies (issue #236)** — detachability and split-eligibility predicates live in `rustconn-core` with property tests pinning precedence order; display-dependent notebook checks run in the quality gate.
- **New strings localised in all 16 languages** — menu items, per-monitor labels, detached-window title, failure toasts, shortcut description, and session-manager row label translated in be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn.

### Dependencies

- **Updated**: cc 1.3.0 → 1.4.0, either 1.16.0 → 1.17.0

## [0.19.3] - 2026-07-23

### Added

- **Option to hide Welcome tab at startup (issue #232)** — the Welcome tab is no longer shown when a startup action (Local Shell or a saved connection) is configured, eliminating the brief flash of the Welcome page before it was replaced. A new "Show Welcome tab" switch in Settings → Startup gives explicit control. The Welcome tab itself now includes a "Don't show this page at startup" checkbox for quick in-place opt-out. The preference is also respected when all sessions are closed (the Welcome tab won't reappear if disabled).

### Fixed

- **FreeRDP fallback fails on FreeRDP 3.26+ due to `/args-from:file:` exclusivity** — FreeRDP 3.26 ([PR #12697](https://github.com/FreeRDP/FreeRDP/pull/12697)) enforces that `/args-from:file:` must be the sole CLI argument and cannot be combined with other arguments on the command line. The previous approach wrote only the password to the args file while passing all other connection parameters (`/v:`, `/u:`, `/w:`, `/h:`, etc.) directly on argv. Users who updated to FreeRDP 3.26+ (shipped in recent distro updates) got "can not be used in combination with other arguments" errors, breaking both the IronRDP→FreeRDP fallback path and direct FreeRDP launches. Now all connection arguments are written into the ephemeral args file in `$XDG_RUNTIME_DIR`, with only `/args-from:file:<path>` on the command line. This also improves security: no connection parameters (including hostname and username) are visible via `/proc/<pid>/cmdline`.
- **RDP clipboard syncing even when disabled in connection settings (issue #233)** — the embedded RDP session builder hardcoded `.with_clipboard(true)` when constructing the `EmbeddedRdpConfig`, completely ignoring the saved `clipboard_enabled` setting from the connection profile. Users who disabled clipboard sharing in connection properties still had full clipboard sync between client and server. Now correctly reads `rdp_config.clipboard_enabled` from the persisted connection.
- **SSH MPTCP used non-existent `-o TCPMultipath=yes` option (issue #231)** — OpenSSH has no `TCPMultipath` option; the previous implementation was based on a hallucinated SSH directive. SSH MPTCP now correctly wraps the command with `mptcpize run` (from the mptcpd package), which forces TCP sockets to use the MPTCP protocol. SSH config import/export no longer reads or writes the invalid `TCPMultipath` directive. Embedded RDP/VNC MPTCP (via `socket2` with `IPPROTO_MPTCP`) remains unchanged and valid.

### Dependencies

- **Updated**: rustls-pki-types 1.15.0 → 1.15.1

## [0.19.2] - 2026-07-23

### Added

- **Multipath TCP (MPTCP) support (issue #231)** — enables using multiple network paths simultaneously for seamless mobility (switch between Wi-Fi and Ethernet without dropping connections) and bandwidth aggregation. MPTCP is available as a per-connection toggle for SSH, embedded RDP, and embedded VNC protocols. SSH connections use `mptcpize run` wrapper (requires mptcpd package). Embedded RDP and VNC clients use MPTCP sockets via the new `socket2`-based helper in `rustconn-core/src/connection/mptcp.rs`. Falls back to regular TCP transparently when the kernel does not support MPTCP (requires Linux 5.6+ with `CONFIG_MPTCP=y`). Runtime MPTCP availability is detected via `/proc/sys/net/mptcp/enabled`.

### Fixed

- **VPN connect/disconnect no longer kills healthy SSH sessions (issue #230)** — the network-change handler previously ran `ssh -O exit` on all ControlMaster sockets unconditionally, terminating active SSH sessions even when the VPN only added/removed specific routes without affecting the default gateway. Now uses `ssh -O check` to probe each socket first: healthy masters are left untouched, and only truly dead sockets are removed. Unconditional cleanup (`ssh -O exit`) is reserved for the network-down path where all TCP connections are assumed dead.

### Dependencies

- **Updated**: clap 4.6.3 → 4.6.4, libc 0.2.188 → 0.2.189, syn 3.0.2 → 3.0.3, tokio-stream 0.1.18 → 0.1.19
- **Added**: socket2 0.5 (MPTCP socket creation)

### Improved

- **MPTCP status indicator in embedded sessions** — the RDP toolbar status label now shows "| MPTCP" alongside RTT and graphics mode (e.g., "RTT: 12 ms | GFX + H.264 | MPTCP") when MPTCP is enabled for the connection; the VNC toolbar shows "MPTCP" on connect. Helps users confirm the feature is active for the current session
- **CLI `show` displays MPTCP state** — `rustconn-cli show` now prints "MPTCP: enabled" in table output and `"mptcp": true` in JSON output for SSH, RDP, and VNC connections that have MPTCP enabled
- **CLI `update --mptcp` accepts true/false** — bare `--mptcp` enables (unchanged); `--mptcp false` disables MPTCP on an existing connection without opening the GUI. Matches the `--javascript` pattern used by Web protocol flags
- **Network monitor thread spawn failure logged** — if the background thread for SSH socket health-checking cannot be spawned (e.g., ulimit exhaustion), a `tracing::warn!` is now emitted instead of silently discarding the error via `.ok()`
- **MPTCP property tests** — protocol test generators now randomize the `mptcp` field; 4 new property tests verify JSON serialization round-trip preservation for SSH/RDP/VNC configs and that `TCPMultipath=yes` is never emitted in SSH command args

### Documentation

- Updated USER_GUIDE.md MPTCP section with `--mptcp false` CLI usage and toolbar status indicator description

## [0.19.1] - 2026-07-21

### Fixed

- **RDP certificate mismatch causes silent connection failure (exit 255)** — when a server's TLS certificate changed since the last connection, FreeRDP's TOFU check required interactive confirmation (`Y/T/N`) that the embedded process could not provide, resulting in an immediate exit with no useful feedback. Now RustConn detects the certificate mismatch from FreeRDP stderr and shows an `adw::AlertDialog` asking the user whether to accept the new certificate. On acceptance, the old fingerprint is removed from `~/.config/freerdp3/known_hosts2` and the connection is retried with TOFU — saving the new certificate as trusted permanently.
- **SSH password auto-fill intermittently stuck on prompt** — VTE in no-echo mode may not emit `contents-changed` or `cursor-moved` signals after the SSH password prompt lands in the terminal grid. The previous single 120 ms deferred re-check was insufficient when the SSH handshake took longer than 120 ms to display the prompt. Replaced with a 150 ms polling timer (up to 10 s) that continuously checks the VTE grid for the password prompt and self-cancels once credentials are injected or the deadline expires. Affects both initial connect and in-place reconnect paths.

### Improved

- **RDP certificate store: exact hostname matching** — `remove_from_known_hosts2` now compares the first two whitespace-separated fields (host and port) exactly instead of using substring `contains()`, preventing false-positive removal of entries where one hostname is a substring of another (e.g. `db.example.com` would no longer accidentally remove `my-db.example.com`)
- **SSH password polling stops on closed session** — the 150 ms polling timer now checks whether the terminal still exists before each tick; if the user closes the tab while the prompt is stuck, the timer stops immediately instead of running for up to 10 s

## [0.19.0] - 2026-07-20

### Added

#### Embedded Web Browser (WebKitGTK 6.0)

- **In-tab WebKitGTK 6.0 browser for Web connections (issue #151)** — Web protocol connections now render pages inside RustConn tabs instead of launching an external browser, providing an integrated browsing experience for internal tools, dashboards, and admin panels (noVNC, Guacamole, KasmVNC, Grafana, Proxmox, etc.)
- **Browser mode selection** — each Web connection chooses how URLs open: **Embedded** (in-tab via WebKitGTK), **System** (xdg-open / UriLauncher), or **Custom** (user-specified command); configured in the connection dialog protocol tab
- **Per-connection persistent sessions** — isolated WebKitGTK `NetworkSession` per connection backed by `~/.local/share/rustconn/webkit/<uuid>/`; login sessions survive restarts without cross-connection data leakage
- **Navigation toolbar** — Back/Forward/Reload/Home buttons, editable URL bar (Enter to navigate, auto-prepends `https://` for bare hostnames), Autofill button, Zoom In/Out, and a "⋯" overflow menu (Copy URL, Open in System Browser, Zoom Reset, Clear Session Data)
- **Credential autofill** — injects stored credentials into login forms via JavaScript (targets username/email + password fields, dispatches `input`/`change` events for SPA compatibility); HTTP Basic/Digest 401 challenges answered automatically from the configured secret backend
- **Per-connection JavaScript toggle** — disable JS execution for security-sensitive bookmarks
- **Accept invalid TLS certificates** — `accept_invalid_certs` for self-signed certs on local services (Cockpit, Proxmox dev, etc.)
- **Custom user agent** — per-connection override string (max 512 chars, validated at deserialization)
- **Split view support** — embedded Web sessions participate in the split-view system alongside RDP/VNC/terminals
- **Loading progress bar** — thin bar under the toolbar shows real-time page load progress
- **Auto-fit zoom** — when the WebView is narrower than 1024 px, zoom is reduced proportionally to prevent horizontal overflow
- **Zoom persistence** — zoom level saved to connection config (debounced 2 s) and restored on reopen; range 30–300 %
- **Download notifications** — file downloads show a toast on completion (saved to `~/Downloads/`)
- **`file://` URL support** — Web connections now accept `file://` URLs in addition to `http://` and `https://`
- **`web-embedded` feature flag** — WebKitGTK dependency gated behind a Cargo feature (default on Linux, excluded on macOS); all WebKitGTK code conditionally compiled with `#[cfg(feature = "web-embedded")]`; feature declared in both `rustconn` and `rustconn-core`

#### CLI

- **`add`/`update --protocol web` flags: `--browser-mode`, `--javascript`, `--user-agent`, `--accept-invalid-certs`, `--private-mode`, `--zoom-level`** — Web connections can be fully configured from the CLI; `--javascript` and `--accept-invalid-certs` accept `true`/`false` (bidirectional — can both enable and disable); bare `--javascript` = `false`, bare `--accept-invalid-certs` = `true`; `--user-agent` validates the 512-char limit; `--zoom-level` validates 0.3–3.0 range; all flags available on both `add` and `update` subcommands
- **`show` displays Web connection settings** — browser mode, JavaScript state, user agent, accept_invalid_certs, browser command, private mode, and zoom level are shown in table, JSON, and CSV output
- **CLI binary lint overrides** — `unreachable_pub`, `clippy::print_stdout`, and `clippy::print_stderr` allowed crate-wide with documented reasons, reflecting that a CLI binary communicates via stdout/stderr by design

#### Developer Tooling

- **`typos.toml` configuration** — project-wide typo checking via `typos-cli`; excludes generated/binary files (`.mo`, `.po`, `Cargo.lock`, `.flatpak-builder/`) and project-specific terminology (`rustconn`, `gio`, `glib`, `adw`, `uk`)
- **`profile.test` optimizations** — `proptest` and `rand_chacha` compile with `opt-level = 3` in test builds, reducing property test runtime from ~120 s to ~30 s
- **WebConfig property tests** — `web_config_tests.rs` with proptest coverage for serialization round-trip, user agent validation, zoom clamping, and compile-time default behavior

### Fixed

- **Embedded RDP fallback fails on servers behind RD Connection Broker (issue #218)** — when IronRDP falls back to an external FreeRDP process, passwords were previously piped via `/from-stdin`. This broke on servers using a Connection Broker: the broker issues a Server Redirection PDU, FreeRDP reconnects to a different host, but stdin was already consumed. All sessions (not just RemoteApp) now pass passwords via a single-use ephemeral args file (`/args-from:file:<path>`) that FreeRDP reads once into memory, surviving redirects without exposing the secret in `/proc/PID/cmdline`
- **Flatpak RDP: removed unnecessary `--forward-fd=0`** — `flatpak-spawn` no longer passes `--forward-fd=0` since stdin piping is replaced by the args file approach
- **Zero Trust Generic provider double shell wrapping** — `build_command()` for the Generic provider already returns `("sh", ["-c", "template"])`; wrapping it in another `bash -c '...'` broke argument parsing. Now spawned directly
- **Potential panic on non-ASCII error messages** — the embedded browser's `load-failed` error truncation used byte-position slicing (`&description[..197]`) which panics on multi-byte UTF-8 (Cyrillic, CJK, emoji); now uses `floor_char_boundary` for safe truncation

### Improved

- **Group dialog UI** — replaced adaptive ViewSwitcher/breakpoint pattern with always-visible bottom `ViewSwitcherBar`; fixed minimum dialog height (500 px); pages registered in consistent order (Identity → Connections → Cloud Sync → Dynamic → Automation)
- **Responsive embedded browser toolbar** — secondary buttons (Home, Autofill, Zoom In/Out) auto-hide at < 500 px width; all actions remain reachable via the "⋯" menu
- **Reconnect banner on load failure** — network errors display a banner with error description + "Reload" button (matching the RDP reconnect pattern)
- **Zoom button tooltips** — dynamically show current percentage (e.g. "Zoom in (120%)")
- **Stricter workspace lints** — added `unreachable_pub`, `redundant_imports` (rustc); `unwrap_used`, `dbg_macro`, `todo`, `print_stdout`, `print_stderr`, `wildcard_imports` (clippy); `allow-unwrap-in-tests = true` in `.clippy.toml` to keep test code ergonomic
- **Import style enforced** — `rustfmt.toml` sets `imports_granularity = "Module"` and `group_imports = "StdExternalCrate"` (nightly `rustfmt`); all imports across the codebase reformatted: std → external → crate
- **CLI module visibility tightened** — internal helpers in `rustconn-cli` changed from `pub` to `pub(super)` / `pub(crate)` to prevent accidental API surface leakage
- **`rustconn-core` re-exports alphabetized** — `lib.rs` pub use statements sorted and grouped consistently
- **Trash cleanup includes webkit sessions** — `empty_trash()` removes WebKit data/cache directories for permanently deleted Web connections
- **Cloud Sync strips device-local Web fields** — `zoom_level` is reset to 1.0 during sync export so per-device display preferences do not propagate between machines

### Documentation

- Updated USER_GUIDE.md to version 0.19.0 with comprehensive embedded browser section (toolbar, zoom, autofill, keyboard shortcuts, known limitations, CLI examples)
- Updated README.md Web protocol description and demo video link in feature table
- Updated CLI_REFERENCE.md with Web protocol flags and examples
- Updated metainfo XML with 0.19.0 release entry
- Updated debian/changelog and OBS packaging changelogs

### Dependencies

- **Added**: webkit6 0.6 (WebKitGTK 6.0 Rust bindings), javascriptcore6 0.6, soup3 0.9
- **Updated**: anyhow 1.0.106→1.0.107, proc-macro2 1.0.106→1.0.107, serde 1.0.228→1.0.229, syn 2.0.18→2.0.19, thiserror 1.2.67→1.3.0, tracing-subscriber 0.1.20→0.1.21
- **CI**: added `libwebkitgtk-6.0-dev` to install-deps action

### Known Limitations

- **WebKitGTK does not support WebCodecs API** — hardware-accelerated H.264/VP8 decoding via WebCodecs (used by Selkies WebRTC streaming) is not available; low-latency streaming tools like Selkies require a Chromium-based engine
- **No DRM/EME content playback** — encrypted media (Netflix, Spotify) is not supported in the embedded view (Widevine not available in WebKitGTK)
- **Embedded mode is Linux-only** — requires WebKitGTK 6.0; other platforms fall back to System mode

## [0.18.12] - 2026-07-18

### Added

- **Graphics Pipeline selector for embedded RDP connections (issue #218)** — adds a "Graphics Pipeline" dropdown (Automatic / Legacy / RemoteFX) to the RDP connection properties dialog. Users experiencing incompatibility with the GFX/H.264 pipeline on certain Windows Server 2019 hosts can now set "Legacy (Compatible)" per-connection to skip the EGFX channel entirely. This eliminates the 15-second watchdog timeout, the retry cascade, and the fallback to external FreeRDP. The setting is persisted and only visible in Embedded client mode

### Fixed

- **Missing mouse cursor on remote Wayland VNC sessions (issue #220)** — the embedded VNC client now advertises `CursorPseudo` (RFB encoding -239) and `DesktopSizePseudo` (-223) during connection negotiation. Without `CursorPseudo`, Wayland VNC servers never send cursor shape updates because the compositor does not composite the hardware cursor into the framebuffer capture (unlike X11). The cursor rendering handler in `embedded_vnc` was already implemented but never received events — this fix completes the pipeline so the remote pointer is visible on Fedora 42/44 Wayland sessions

## [0.18.11] - 2026-07-16

### Fixed

- **Missing mouse cursor on remote Wayland sessions over VNC (issue #220)** — the embedded VNC client now processes `CursorUpdate` pseudo-encoding events from the server, building a proper GDK cursor texture from the received bitmap. Previously the event was silently ignored, leaving the pointer invisible when the remote host runs a Wayland compositor (which cannot bake the hardware cursor into the framebuffer capture, unlike X11)
- **Embedded RDP: retry without GFX pipeline before falling back to external FreeRDP (issue #218)** — when the IronRDP GFX/H.264 pipeline fails (decode errors or no first frame within 15s), RustConn now disconnects and retries the same connection with the EGFX channel disabled (Legacy/RemoteFX bitmap path). Only if the retry also fails does it fall back to an external FreeRDP process. This avoids the "Authentication failed" race condition on Windows Servers with single-session policy, where the FreeRDP reconnection was rejected because IronRDP's previous session hadn't fully torn down. The `graphics_mode` field in `RdpClientConfig` is now respected — `Legacy` and `RemoteFx` modes skip EGFX DVC registration entirely
- **Nix flake: tests no longer run during `nix build`** — added `doCheck = false` to `flake.nix`; the full test suite (argon2 property tests, ~120s) was running unnecessarily during end-user installation. Tests are validated in CI, not at install time

## [0.18.10] - 2026-07-16

### Added

- **Nix flake for NixOS / Nix users** — `flake.nix` in the repository root allows `nix run github:totoshko88/RustConn` or `nix profile install` without waiting for upstream nixpkgs packaging. Both `rustconn` (GUI) and `rustconn-cli` are included. Builds against nixpkgs unstable (GTK4 4.14+, libadwaita 1.5+)

### Fixed

- **Embedded RDP sessions to Windows Server 2019 with AD auth falsely triggered "Server Incompatible" fallback (issue #218)** — the first-frame watchdog timeout (8 seconds) was too short for servers with Active Directory login scripts and Group Policy refresh, which can take 10+ seconds to render the first desktop frame through the GFX/H.264 pipeline introduced in v0.18.5. The timeout is now 15 seconds. Additionally, when the GFX pipeline reports a persistent decode failure (10+ consecutive empty frames), fallback to FreeRDP is triggered immediately instead of waiting for the full timeout
- **FreeRDP fallback after IronRDP disconnect could fail with "Authentication failed" on single-session servers** — Windows Server configured to restrict users to a single RDP session would reject the FreeRDP reconnection because IronRDP's session was still tearing down. The increased watchdog timeout (15s) reduces false fallback triggers, avoiding the double-connection race entirely for servers that simply need more time to deliver the first frame

### Documentation

- Added NixOS / Nix installation section to `docs/INSTALL.md` with flake usage, home-manager integration example, and local build instructions

### Dependencies

- **Updated**: bitflags 2.13.0→2.13.1, clap 4.6.1→4.6.2, ksni 0.3.5→0.3.6, regex 1.13.0→1.13.1, simd-adler32 0.3.9→0.3.10, sspi 0.21.1→0.21.2, syn 2.0.118→2.0.119, uuid 1.23.5→1.24.0

## [0.18.9] - 2026-07-15

### Fixed

- **Secret values from vault backends now properly zeroize intermediates** — `load_variable_from_vault_with_path` returns `Zeroizing<String>` instead of plain `String`, ensuring that password intermediates are wiped from memory when dropped. Previously, a `Zeroizing` wrapper was created but then a new unprotected `String` copy escaped via `String::from(z.as_str())`, defeating the zeroization (9 call sites across Bitwarden, 1Password, Pass, Passbolt, KeePass, LibSecret, EncryptedFile, MacOS Keychain backends)
- **Clipboard password now wrapped in `Zeroizing<String>`** — the "Copy Password" action previously held the exposed password as a plain `String` in a 30-second GLib timeout closure for clipboard auto-clear comparison. The password is now wrapped in `Zeroizing<String>`, ensuring it is wiped from heap memory when the closure drops
- **Pre/post-connect tasks no longer block the UI indefinitely** — when a user-defined connection task has no explicit `timeout_ms` configured, the GUI now enforces a 60-second safety ceiling via `tokio::time::timeout`. Previously, a script without a timeout could hang the GTK main thread forever, freezing the entire application
- **Keyring save operations now have a 5-second timeout** — saving credentials to the system keyring (Bitwarden, 1Password, Passbolt, KeePass passwords) previously called `block_on()` without a timeout. If the Secret Service daemon was unresponsive (locked session, D-Bus issues), the GTK UI would freeze. All 4 save functions now wrap in `tokio::time::timeout(5s)`

### Improved

- **`deny.toml` cleaned up** — removed 5 stale advisory ignores (RUSTSEC-2023-0089, RUSTSEC-2024-0384, RUSTSEC-2024-0436, RUSTSEC-2026-0118, RUSTSEC-2026-0119) that no longer match any crate in Cargo.lock, and 3 unmatched license allowances (GPL-3.0, OpenSSL, Unicode-DFS-2016). Added review date.

### Dependencies

- **FreeRDP 3.28.0 → 3.29.0** (Flatpak/Flathub manifests) — security update resolving 22 advisories including heap buffer overflows in H.264/AV1 decoders, camera channel, X.509 null bytes, and key negotiation

## [0.18.8] - 2026-07-14

### Fixed

- **Connections stop working when switching network interfaces (issue #217)** — RustConn now monitors network changes via `gio::NetworkMonitor` and reacts immediately: stale SSH `ControlMaster` sockets are closed, a toast notifies the user, and disconnected sessions with auto-reconnect enabled are reconnected without waiting for the backoff timer
- **SSH connections hang for minutes after VPN/network change** — default `ServerAliveInterval=15` + `ServerAliveCountMax=3` is now applied to all SSH sessions (unless the user configures a custom value). Dead connections are detected within ~45 seconds instead of relying on TCP timeout (15+ minutes)
- **New SSH connections fail after interface switch** — `ControlPersist` reduced from 10 minutes to 60 seconds. Combined with proactive socket cleanup on network change, new connections no longer attempt to multiplex through dead master sockets
- **Terminal keyboard shortcuts work after remapping (issue #216)** — the focus-based accelerator suspend (`terminal_passthrough_ctrl`) now removes only single-modifier accelerators (e.g. `Ctrl+W`) while keeping multi-modifier variants (e.g. `Ctrl+Shift+W`) active. Previously, all accelerators for conflicting actions were stripped when the terminal had focus, making shortcuts like "Close Tab" unreachable regardless of keybinding overrides

### Improved

- **Network monitor skips reconnect behind captive portals** — when `gio::NetworkMonitor` reports connectivity below `Full` (e.g. `Portal` or `Limited` after a WiFi switch to a captive-portal network), the reconnect attempt is skipped and the user sees "Network limited — full connectivity not yet available" instead of flooding failed connections
- **Network monitor rate-limits reactions during VPN reconnect loops** — if more than 3 network-change events fire within 60 seconds the monitor enters quiet mode (socket cleanup only, no toast or reconnect), preventing toast spam and wasted reconnection attempts when a VPN flaps
- **Network-change reconnect is delayed 3 s after socket cleanup** — reconnects are now scheduled via `glib::timeout_add_local_once(3s)` instead of `idle_add_local_once`, giving the background `ssh -O exit` thread time to close stale `ControlMaster` sockets before new connections try to multiplex
- **Embedded RDP/VNC sessions reconnect on network change** — the network monitor now detects embedded (non-VTE) sessions in disconnected/error state and triggers their `reconnect()` method directly, instead of relying solely on the VTE reconnect overlay which embedded viewers do not use
- **Socket cleanup dummy destination changed from `"none"` to `"_"`** — `close_all_control_sockets` now passes `"_"` as the dummy SSH destination for `ssh -O exit`, avoiding a potential collision with a real `Host none` entry in `~/.ssh/config`

### Dependencies

- Updated lockfile: simd_cesu8 1.1.1→1.2.0, socket2 0.6.4→0.6.5, spin 0.9.8→0.9.9, toml 1.1.2→1.1.3, toml_edit 0.25.12→0.25.13, toml_writer 1.1.1→1.1.2

## [0.18.7] - 2026-07-13

### Changed

- **`rustconn-core` default features are now empty** — the core library is a headless domain kernel by default. Embedded client runtimes (`vnc-embedded`, `rdp-embedded`, `gfx-h264`, `rd-gateway`) and host keyring integration (`system-keyring`) are opt-in features enabled by consumer crates. This reduces compile time and binary size for headless/CI builds
- **`rustconn-cli` is minimal by default** — the CLI binary now ships only headless management commands (list/add/update/delete, import/export, groups, tags, templates, clusters, stats, completions). Desktop client-launch (`connect`, SFTP file-manager) and secret-management commands are gated behind `client-launch` and `secret-management` features respectively; `--features full` enables everything
- **System keyring dependencies are optional** — `oo7` (Linux/BSD Secret Service) and `security-framework` (macOS Keychain) are now behind the `system-keyring` feature. Headless builds no longer pull DBus/keyring dependencies
- **`native-tls` is optional in core** — compiled only with `rdp-embedded` or `rd-gateway` features
- **`AudioFormatInfo` moved from `rdp_client::audio` to `rdp_client::event`** — the type is now available without the `rdp-embedded` feature, allowing headless consumers to inspect audio format metadata. The constructor is now `const fn`. The top-level re-export `rustconn_core::AudioFormatInfo` remains stable (no longer gated behind `#[cfg(feature = "rdp-embedded")]`)
- **`NO_COLOR` env var handling follows the spec** — an empty `NO_COLOR=""` no longer incorrectly suppresses colors; only a non-empty value is respected per the [no-color.org](https://no-color.org) specification
- **Disabled keyring backends log at `warn` level** — when a user's config selects a system keyring backend but `system-keyring` is not compiled in, the message is now a visible `tracing::warn!` (previously silent `debug!`)

### Added

- **Feature-gated integration tests for CLI commands** — tests for `connect` and `secret` commands are now properly gated with `#[cfg(feature = "...")]`, verifying both presence and absence in help output depending on the feature set
- **Stub keyring functions for headless builds** — `keyring::store`, `keyring::lookup`, `keyring::clear`, and `keyring::is_secret_tool_available` compile as unavailable stubs when `system-keyring` is disabled, returning `BackendUnavailable` errors. API surface stays identical regardless of features
- **CLI feature hierarchy** — `client-launch`, `secret-management`, `desktop-integration` (= both), `full` (= all)
- **`system-keyring` feature forwarding in GUI crate** — `rustconn/Cargo.toml` explicitly enables `rustconn-core/system-keyring` in its default feature set
- **`gfx-h264` feature forwarding in GUI crate** — `rustconn/Cargo.toml` explicitly enables `rustconn-core/gfx-h264` in its default feature set, ensuring the H.264/EGFX pipeline is compiled into desktop builds

### Fixed

- **Release and OBS DEB builds shipped CLI without full features** — `release.yml` (GitHub Actions .deb/.rpm/.AppImage) and `packaging/obs/debian.rules` built `rustconn-cli` with default (empty) features, stripping `connect`, `secret`, and SFTP file-browser commands from distributed packages. All build paths now pass `--features full`, matching Snap/Flatpak/OBS RPM
- **CI did not test CLI with full feature set** — added `cargo test -p rustconn-cli --features full` to the CI test job so feature-gated commands and their integration tests are exercised on every push

### Documentation

- Updated `docs/ARCHITECTURE.md` with headless boundary section, feature table, and default edit targets
- Updated `docs/BUILD.md` with feature tables for all three crates and build examples
- Updated `docs/CLI_REFERENCE.md` with feature set table and per-command availability notes
- Updated `docs/AI_DEVELOPMENT.md` with Codex target boundaries
- Updated `.kiro/steering/project-rules.md` and `protocol-guide.md` with refined crate descriptions

### Dependencies

- Updated lockfile: cc 1.2.66→1.2.67, http-body 1.0.1→1.1.0, http-body-util 0.1.3→0.1.4, mio 1.2.1→1.2.2, open 5.3.6→5.4.0, polyval 0.7.1→0.7.2, rand 0.9.4→0.9.5, rustls 0.23.41→0.23.42, uuid 1.23.4→1.23.5, winnow 1.0.3→1.0.4, zmij 1.0.21→1.0.23

## [0.18.6] - 2026-07-11

### Added

- **Native RD Gateway (MS-TSGU) support for the embedded RDP client** — connections routed through an RD Gateway now use IronRDP's native `ironrdp-mstsgu` tunneling instead of falling back to an external xfreerdp process. This means GFX pipeline + H.264 decoding, clipboard, shared folders, printer redirection, and audio all work through gateway connections identically to direct TCP. The gateway authenticates with Basic auth (username + password shared with the session credentials by default). Gated behind the `rd-gateway` feature flag (enabled by default); disabling it restores the previous fallback-to-xfreerdp behaviour
- **Mouse X1/X2 (browser back/forward) button support for embedded RDP** — the embedded client now maps GTK4 buttons 8 and 9 (back/forward side-buttons) through to RDP Extended Mouse buttons X1 and X2, forwarding browser back/forward side-buttons to the remote desktop

### Improved

- **Input handling migrated to `ironrdp-input` state machine** — the embedded RDP client now uses the upstream `ironrdp_input::Database` for keyboard and mouse event generation instead of hand-rolled `FastPathInputEvent` construction. Benefits: deduplicates redundant mouse-move events when the cursor hasn't moved, prevents sending key-release for a key that was never pressed (avoids confusing remote apps), and correctly tracks the full set of 5 mouse buttons. ~80 lines of bespoke input code replaced by 6 `Operation` dispatches through the canonical library
- **Zero-latency input delivery via `tokio::select!`** — the session loop now uses `tokio::select!` between the server PDU stream and the command channel (migrated from `std::sync::mpsc` to `tokio::sync::mpsc::unbounded`), eliminating the previous 0–50ms polling delay for keyboard and mouse events. Input is now delivered to the server in the same event loop tick as it arrives from the GUI, which is particularly noticeable on high-latency gateway connections
- **Optimized RGBA→BGRA pixel conversion for GFX pipeline** — the H.264 frame conversion now uses `chunks_exact(4)` with pre-allocated output buffer instead of byte-by-byte `push()`, enabling LLVM to auto-vectorize into SSSE3 `pshufb` on x86_64. Measured ~3× faster on 4K (3840×2160) frames
- **Gateway password intermediate is zeroized** — the intermediate `String` bridging `SecretString::expose_secret()` to `ironrdp-mstsgu`'s owned-String API is wrapped in `zeroize::Zeroizing` and overwritten on drop, matching the discipline used for the session password. The final owned copy passed into the library is controlled by `ironrdp-mstsgu` (not yet zeroize-aware); upgrade tracked upstream

### Fixed

- **RDP authentication failures showed a misleading "external client closed unexpectedly" error** — when IronRDP received a CredSSP `STATUS_LOGON_FAILURE` (wrong username/password, `0xc000006d`) the error message contained "Connection finalize failed", which matched the protocol-incompatibility heuristic and triggered a pointless FreeRDP fallback with the same credentials. FreeRDP also failed, and the user saw only a generic "External RDP client closed unexpectedly (exit status: 255)" toast. Auth failures (wrong password, locked/disabled account, expired password) are now excluded from the fallback heuristic and reported immediately as "Authentication failed: invalid username or password." When FreeRDP fallback *is* triggered for genuine protocol incompatibilities, the watchdog now classifies the external client's stderr output (LOGON_FAILURE, ACCOUNT_LOCKED, transport failure, certificate rejection) into specific user-facing messages instead of the generic one
- **FreeRDP error parser merged duplicate auth branches** — the `ERRCONNECT_AUTHENTICATION_FAILED` branch returned the same message as `ERRCONNECT_LOGON_FAILURE` and was unreachable; merged into a single condition (clippy `if_same_then_else`)

### Internal

- **Session command channel migrated to `tokio::sync::mpsc::unbounded`** — `RdpCommandSender` is now `tokio::sync::mpsc::UnboundedSender`, enabling `tokio::select!` in the session loop. The sender is `Send + Sync` and supports non-async `send()` from the GTK thread without blocking. Public API is unchanged (callers use `send_command()`)
- **Extracted `send_file_contents_error` helper** — the 5-line "get cliprdr → submit error → process → write" pattern was duplicated 3× in `handle_file_contents_request`; now a single async helper called from all error paths

### Dependencies

- **ironrdp-input 0.7.0** (new) — canonical input state machine for keyboard/mouse events
- **ironrdp-mstsgu 0.0.1** (new) — RD Gateway (MS-TSGU) WebSocket tunnel client
- **tokio-tungstenite 0.29.0, tungstenite 0.29.0** (new, transitive) — WebSocket transport for `ironrdp-mstsgu` gateway tunneling
- Updated lockfile: aead 0.6.0-rc.10→0.6.1, bytemuck 1.25.0→1.25.1, ironrdp-core 0.2.0→0.2.1, sha1 0.10.6→0.10.7, thread_local 1.1.9→1.1.10, tinyvec 1.11.0→1.12.0, zip 8.5.1→8.6.0

## [0.18.5] - 2026-07-11

### Added

- **GFX pipeline with H.264/AVC decoding for embedded RDP** — the embedded RDP client now supports the RDPGFX (MS-RDPEGFX) graphics pipeline via `ironrdp-egfx` 0.3, dramatically improving image quality and bandwidth usage over WAN connections:
  - H.264 decoding uses OpenH264 loaded at runtime via `dlopen` — no build-time linking, Flatpak-compatible
  - Auto-selects the best mode: GfxAvc444 > GfxH264 > Gfx > RemoteFX > Legacy
  - Falls back gracefully when OpenH264 is missing (non-AVC codecs within GFX) or when the server doesn't support EGFX (RemoteFX/Legacy)
  - Gated behind `gfx-h264` feature flag (enabled by default); disabling it compiles the feature out with no runtime cost
- **Performance mode mapping for graphics selection** — Quality→GfxAvc444, Balanced→GfxH264, Speed→RemoteFX/Legacy
- **GFX decode failure signalling** — emits `GfxDecodeFailure` event after 10+ consecutive empty frames, enabling a future degraded-quality banner
- **Active graphics mode in session statistics** — the RTT status label shows the active pipeline (e.g. "RTT: 12 ms | GFX + H.264"); `FrameStatistics` tracks blit time via EMA and warns at >5% frame drop rate
- **OpenH264 in Flatpak manifest** — built from source as a meson module (`org.freedesktop.Platform.openh264` was removed from SDK 23.08+), installed to `/app/lib/`

### Removed

- **Dead `get_export_options` method on the export dialog** — a `get_export_options` accessor built an `ExportOptions` from only the format, output path and "include groups" flag, silently ignoring the CSV field selection and the group-scope filter. It was never called (the export button has its own complete collection logic), so it was removed to prevent a future caller from reintroducing the field-dropping bug
- **Dead `sanitize` parameter on session recording** — `start_recording` accepted a `SanitizeConfig` that every caller filled with the (enabled) default, implying recordings had their secrets redacted. Nothing consumed it: the recording is captured verbatim by `script`, and the string-oriented sanitizer is only meaningful for the separate session-logging path, not a binary `script` capture. The misleading parameter has been dropped from the recording API and its six call sites rather than shipping ineffective, false-sense-of-security redaction. Sensitive-data redaction remains available where it works — session logging (Settings → Logging)

### Documentation

- **Corrected the primary menu doc comment** — `create_app_menu`'s doc listed a separate "Security" submenu for Password Generator / Wake On LAN / SSH Tunnels, but those live in a section of the Tools submenu. The comment now matches the built menu (Tools also lists Workspaces)

### Fixed

- **Closing the terminal search via the window × left match highlights behind** — the search dialog cleared its terminal highlights only from an invisible close button (never added to the UI, so its handler never ran) and from the Escape key. Closing through the header-bar × left the highlighted matches in the terminal. Cleanup now runs from the dialog's `connect_closed`, so highlights are removed however the dialog is dismissed (×, Escape, or programmatically); the dead close button was removed. The log viewer had the same never-shown close button (harmless there, since × already closes) and it was removed too
- **Opening and closing Settings wiped a CLI file-launch startup action** — a startup action set from the command line to open an RDP or `.vv` file (`StartupAction::RdpFile`/`VvFile`) has no entry in the "On startup" dropdown and shares index 0 with "Do nothing", so collecting the settings on save collapsed it to `None`, silently discarding it. The save path now preserves a file-based startup action when the dropdown was left on that slot, alongside the other runtime-managed state it already keeps
- **Logging "Timestamps" switch stayed active when session logging was off** — the enable/disable gating for the Session Logging expander toggled the directory, retention, activity, input and output rows with the master switch, but omitted the Timestamps row, so it remained interactive (and misleadingly editable) while logging was disabled. It is now gated with the rest, both on load and when the master switch changes
- **Snippet editor's "Add Variable" button produced unusable variables** — snippet variables are auto-detected live from `${name}` placeholders in the command (the name field is intentionally read-only so it mirrors the command). The separate "Add Variable" button bypassed that: it created a row with a counter name (`var1`, `var2`, …) whose read-only name could never be edited to match a real placeholder, so the variable never bound to anything. The redundant button was removed — typing `${name}` in the command adds the row automatically — and the group description now spells out that behaviour
- **Tunnel Builder "connection not found" message passed an argument to a format string with no placeholder** — `show_missing_connection_error` called `i18n_f` with the connection id but the message had no `{}`, so the id was silently discarded. Since a raw UUID is not useful to the user anyway, the call is now a plain `i18n()` with the self-explanatory message
- **The delete "Undo" toast button was not translatable** — `show_undo_toast_on_window` set the button label to a hard-coded English "Undo", so it stayed English in all 16 locales. It is now wrapped in `i18n()`
- **Embedded VNC never delivered the remote clipboard locally** — the client received the remote clipboard (RFB `ServerCutText` → `ClipboardText` event) but the handler did nothing (`// Could sync with system clipboard`), so text copied on the remote could not be pasted locally, and the toolbar "Copy" button (which cannot pull the push-only RFB clipboard on demand) only logged a hint. Remote clipboard changes are now mirrored into the local system clipboard automatically as they arrive, which is the direction RFB actually supports; local→remote paste already worked
- **Embedded VNC ignored the connection's encoding preference** — the embedded VNC client was built from host/port/shared/view-only/password only, so the per-connection encoding choice was dropped (it applied only to the external viewer). The embedded client now moves a recognised preferred encoding (Tight, ZRLE, CopyRect, Raw) to the front of the offered list. Quality and compression are Tight/zlib negotiation parameters the minimal embedded client does not implement — they remain external-viewer-only and are now logged when set so the difference is traceable, rather than appearing to apply
- **8-bit RDP audio played back distorted** — the audio buffer always parsed incoming PCM as 16-bit signed (two bytes per sample), so an 8-bit stream was mis-read (two unsigned bytes fused into one wrong 16-bit sample) and played as noise. The buffer is now told the sample width when the stream is configured and converts 8-bit unsigned samples (centred on 128) up to the full i16 range on push. The redundant separate 8-bit stream builder — which was an exact copy of the i16 one and did no conversion — was removed; both formats now share a single i16 output stream
- **The terminal tab context menu had no "Close Tab" entry** — the `tab.close` action (which closes the right-clicked tab) was registered but never added to the menu, so it was unreachable from the context menu. A "Close Tab" item now heads the close section, alongside the existing Close Others / Left / Right / All entries
- **Enter in the connection sidebar did not open the selected connection** — the sidebar key handler caught Return / keypad-Enter and returned `Stop` while relying on the ListView's built-in activation to open the row, but stopping the event pre-empted that activation. It now lets the event proceed so the ListView emits `activate` (wired to open/connect the selected row), matching a double-click
- **The sidebar's unsaved-changes indicator for documents never showed** — the dirty state (a CSS dot plus an "unsaved changes" tooltip and accessible label) was applied to whatever `content_box.last_child()` returned, but the last child is the split-view marker, not the name label, so the downcast to `Label` always failed and the indicator was silently skipped. It now updates the actual name label already bound earlier in the row, so a modified document is marked as expected
- **Zero Trust connections could be saved with no provider target** — the connection editor's validation covered SSH, Serial, Kubernetes, Wake-on-LAN and the icon, but had no branch for Zero Trust, so a connection could be saved with every provider field blank, producing an unusable command. Saving a Zero Trust connection now validates the selected provider's identifying field(s) — AWS SSM target, GCP instance + zone, Azure Bastion resource ID/group/name, Azure VM + group, OCI bastion/target/IP, Cloudflare hostname, Teleport/Tailscale host, Boundary target, Hoop.dev connection name, or the Generic command template — and reports which are missing before saving. Optional fields (AWS profile/region, Teleport cluster, Hoop.dev URLs) remain optional
- **Deleting a KeePass-backed connection left its entry in the database** — permanent deletion (empty trash) of a connection whose credentials live in a KDBX/KeePassXC vault only overwrote the entry with an empty password as a best-effort, because the code predated a real delete path. It now calls `KeePassStatus::delete_entry_from_kdbx` (the same `keepassxc-cli` removal the CLI uses), passing the full `RustConn/…(protocol)` entry path, so the stale entry is actually removed instead of lingering with a blank secret
- **Deleting a workspace profile no longer skips confirmation** — the Workspaces manager's Delete button removed the selected profile immediately, unlike the tunnel and cluster managers which confirm first. It now shows a destructive `adw::AlertDialog` naming the workspace before removing it (GNOME HIG), so an accidental click can be cancelled. The list rows were also marked activatable but ignored activation; double-clicking or pressing Enter on a row now opens that workspace, matching the primary action
- **Document and Variables dialogs fired their result callback twice (Some then None)** — the New Document, Open-encrypted-document password, Document Protection, and Global Variables dialogs each delivered their result from the action button and then called `dialog.close()`, which fires `connect_closed` — whose handler unconditionally reported a `None` (cancel) result. On the success path the caller therefore received the real result immediately followed by a spurious cancel, which a handler treating `None` as "reset/discard" could act on and undo the change. Each dialog now sets a `completed` flag when the button delivers a result, and `connect_closed` skips the cancel notification when it is set, so the callback fires exactly once
- **SSH "Agent" and "Security Key (FIDO2)" auth silently discarded the selected key** — the connection editor derived the SSH key material solely from the "Key Source" selector, but for the Agent and Security Key auth methods that selector is hidden and replaced by its own row (the agent-key dropdown / the sk key-file entry). Because the hidden selector stayed at its default value, `build_ssh_config` read no key: an Agent connection saved with no chosen fingerprint, and a FIDO2 connection saved with no key path, so both silently lost the user's selection on save. Key derivation now branches on the auth method first — Agent auth reads the agent-key dropdown, Security Key auth reads the key-file entry — and only falls back to the "Key Source" selector for the methods that actually show it
- **"New SSH Connection" buttons in the Tunnel Builder did nothing** — the tunnel wizard's Step 1 exposes two "New SSH Connection" buttons (an inline one and the empty-state call-to-action shown when no SSH connections exist), both routed through a `connect_new_connection` callback that was never registered. Clicking either did nothing, and with zero SSH connections the wizard became a dead end: the empty state hid the Next button, leaving no working action. The buttons now open the standard new-connection editor (reusing the canonical dialog, so the connection is persisted and the sidebar reloads) and refresh the wizard's connection list on success, so a freshly created connection is immediately selectable
- **Editing a Telnet template opened the SSH options page** — the template editor derives the protocol dropdown index and the visible options page from two separate `match` blocks that had drifted apart: the stack switch had no arm for Telnet (index 5), so it fell through to the SSH page, and it referenced a non-existent `"serial"` page for an out-of-range index. Templates can only be created for six protocols (SSH, RDP, VNC, SPICE, Zero Trust, Telnet), but the index map also listed five protocols the dialog has no dropdown entry or page for (Serial, SFTP, Kubernetes, MOSH, Web), producing an out-of-range dropdown selection had such a template ever been loaded. Both maps are now a single source of truth: index and page are derived together, Telnet shows its own page, and unrepresentable protocols fall back cleanly to the SSH view
- **"Save & Connect" from the connection wizard created the connection but never connected** — both the new-connection wizard and "Duplicate via Wizard" paths, on the *Save & Connect* result, activated a `connect-by-id` window action that does not exist (the real action is `connect-to`, with the same string parameter). GTK silently logged "no action" and the freshly saved connection was left disconnected, so the combined "save and connect" affordance behaved like a plain "save". Both call sites now activate `connect-to`, so the wizard connects immediately after saving as intended

### Dependencies

- **ironrdp 0.16→0.17, ironrdp-tokio 0.9→0.10** — major upgrade of the embedded RDP stack; brings the EGFX-capable session layer, updated PDU parsing, and new rdpdr/cliprdr/displaycontrol APIs. Transitive sub-crate bumps: ironrdp-async 0.9→0.10, ironrdp-cliprdr 0.6→0.7, ironrdp-connector 0.9→0.10, ironrdp-displaycontrol 0.7→0.8, ironrdp-dvc 0.7→0.8, ironrdp-graphics 0.8→0.9, ironrdp-pdu 0.8→0.9, ironrdp-rdpdr 0.6→0.7, ironrdp-rdpsnd 0.8→0.9, ironrdp-session 0.10→0.11, ironrdp-svc 0.7→0.8, ironrdp-tls 0.2.1→0.2.2
- **ironrdp-egfx 0.3.0** (new) — EGFX DVC processor with H.264 decoding
- **openh264 0.9.7, openh264-sys2 0.9.7** (new, transitive) — OpenH264 FFI for runtime `dlopen`
- **safe_arch 1.0.0, wide 1.5.0** (new, transitive) — SIMD abstractions for pixel conversion
- **sponge-cursor 0.1.0** (new, transitive) — cursor helper for sha3 0.12
- **Updated (transitive/crypto patch bumps)**: aes-gcm 0.11.0-rc.3→0.11.0-rc.4, curve25519-dalek 5.0.0-pre.6→5.0.0-rc.1, ecdsa 0.17.0-rc.17→0.17.0-rc.22, ed25519 3.0.0-rc.4→3.0.0, ed25519-dalek 3.0.0-pre.6→3.0.0-rc.1, elliptic-curve 0.14.0-rc.31→0.14.1, ff 0.14.0-pre.0→0.14.0, group 0.14.0-pre.0→0.14.0, p256 0.14.0-rc.9→0.14.0-rc.14, p384 0.14.0-rc.9→0.14.0-rc.14, p521 0.14.0-rc.9→0.14.0-rc.14, pbkdf2 0.13.0-rc.10→0.13.0, picky 7.0.0-rc.23→7.0.0-rc.25, picky-krb 0.12.3→0.12.4, pkcs8 0.11.0-rc.11→0.11.0, primefield 0.14.0-rc.9→0.14.0, primeorder 0.14.0-rc.9→0.14.0-rc.14, rfc6979 0.5.0-rc.5→0.6.0-pre.0, rsa 0.10.0-rc.17→0.10.0-rc.18, sha3 0.11.0→0.12.0, signature 3.0.0-rc.10→3.0.0, sspi 0.21.0→0.21.1, winscard 0.3.2→0.3.3, x25519-dalek 3.0.0-pre.6→3.0.0-rc.1 — no API impact; pulled in by the ironrdp 0.17 dependency tree

## [0.18.4] - 2026-07-09

### Added

- **SFTP file browser opens in the home directory instead of the server root** ([#212](https://github.com/totoshko88/RustConn/issues/212)) — opening an SSH connection over SFTP in a GNOME/KDE file manager used to land on `/`, which fails with "this location cannot be displayed" on shared hosting where the account has no access to the root. The GVFS sftp backend mounts at the server root and ignores the mount's home `default_location`, unlike `ssh`, whose login shell starts in `$HOME`. RustConn now resolves the login home directory once per session (a best-effort `ssh … pwd` with `BatchMode=yes` and a short `ConnectTimeout`, reusing the connection's proxy-jump, port and key, run off the UI thread and cached) and opens the browser there, falling back to the server root if resolution fails. A new optional **SFTP Remote Directory** field (Connection editor → SSH → Session) lets you pin an explicit path for chroot or non-standard layouts; leaving it empty keeps the automatic home detection. The `ssh`/`sftp` CLI and `mc` paths were already correct and are unchanged

### Fixed

- **External-viewer children were left as zombies when `shutdown()` ran while the app kept living** — application-quit cleanup SIGKILLed each RustConn-owned external viewer (TigerVNC, FreeRDP, remote-viewer) but never reaped it. At real process exit `init` adopts and reaps the zombie, so production was unaffected, but any path where `shutdown()` runs without the process actually exiting (an idempotent second `shutdown`, or a `close_request` that does not quit) leaked a zombie, and the killed child stayed visible in the process table. `shutdown()` now reaps each killed child (`kill` + `wait`, the same kill-then-reap pattern already used by `disconnect`); `wait` returns almost immediately because SIGKILL is delivered promptly

### Improved

- **Sidebar bottom toolbar now matches the header's icon size and spacing** — the six action icons at the bottom of the sidebar were custom-shrunk (24px width, 44px height, tight padding) and so read at a visibly different density than the standard Adwaita icon buttons in the header bar. The custom sizing is dropped: the sidebar buttons now inherit the same standard flat icon-button metrics as the header (with a matching 6px inter-icon gap), so both toolbars read uniformly in normal and compact mode, as GNOME HIG intends. Trade-off: six standard-width buttons raise the practical minimum sidebar width from ~180px (relaxed in 0.18.2) to ~240px; the width setting still accepts lower values but the button row no longer forces itself narrower
- **SFTP home-directory resolution no longer repeats a failed probe every open** — the `ssh … pwd` probe that finds the login home directory (issue #212) only cached successes, so a host where the non-interactive `BatchMode` probe cannot authenticate — most commonly a password-only account, exactly the shared-hosting case #212 targets — paid the full connect round-trip on *every* SFTP open before falling back to the server root. The probe outcome is now memoised per host including failure, so a second open in the same session returns instantly. The probe also honours a per-connection or group **SSH agent socket** override (it previously used only the app-started agent, so a connection pointing at a custom agent could fail the probe and land on the server root, reintroducing #212), and adds `ServerAliveInterval`/`ServerAliveCountMax` keepalives so a link that stalls *after* connecting cannot pin the blocking worker thread beyond `ConnectTimeout`

### Internal

- **Accessible-label relations no longer block libadwaita updates** — the `LabelledBy` relations that name form entries for screen readers upcast their labelling `adw::ActionRow` directly to `gtk4::Accessible`, which depended on the `IsA<Accessible>` bound that libadwaita 0.9.2 dropped from `ActionRow` (the reason the crate was pinned at 0.9.1 since 0.18.2). The relations are now set through a single `utils::set_labelled_by` helper that upcasts via `gtk4::Widget` — every widget is an `Accessible` through `Widget` regardless of the concrete row type — so the same a11y wiring holds across `ActionRow` binding changes. The 17 call sites in the connection editor (`general_tab`) and the group editor (`edit_group`) were migrated; accessibility behaviour is unchanged

### Dependencies

- **libadwaita 0.9.1 → 0.9.2** — the accessible-label refactor above removes the last compile blocker, so the pin is lifted and the workspace builds clean on 0.9.2 (clippy 0 warnings across all four crates). This closes the "revisit with a source fix" note carried since 0.18.2
- **Updated (transitive/utility patch bumps)**: der 0.8.0→0.8.1, inotify 0.11.3→0.11.4, regex 1.12.4→1.13.0, regex-automata 0.4.14→0.4.15, zerocopy 0.8.53→0.8.54, zerocopy-derive 0.8.53→0.8.54, zlib-rs 0.6.5→0.6.6 — no API impact

## [0.18.3] - 2026-07-09

### Added

- **External-session tracking for VNC/RDP/SPICE external-viewer sessions** — connections opened in External Window mode (or with an external client mode) are now tracked in a lightweight process registry watched by a single shared poll timer, instead of being given a placeholder tab. The tracked session surfaces in the sidebar with an external-viewer emblem (a window icon) next to the connected status, and a right-click context menu adds **Disconnect** (terminates a RustConn-owned viewer without blocking the UI) and **Stop tracking** (deregisters without killing, for detaching viewers like remmina/krdc). A split-membership marker distinguishes sessions placed in a split, and a smart double-click focuses an existing active session — selecting its owner tab and pane — instead of opening a duplicate (hold Shift/Ctrl, or use "Open new session", to force a new one)
- **Optional connection name in the window title** ([#211](https://github.com/totoshko88/RustConn/issues/211)) — a new "Show connection in window title" switch (Settings → Interface → Window, **off by default**) makes the window title follow the active tab, e.g. `RustConn - Project A`. Time-tracking tools such as ManicTime read the window title, so this lets them attribute usage to the connection you are working on. Off by default for privacy, since the connection name would otherwise be visible in the taskbar, window list, and screen shares. The title updates as you switch, open, or close tabs, and reverts to plain `RustConn` when no session is active; toggling the setting applies immediately, no restart needed

### Fixed

- **VNC/RDP/SPICE connections in External Window mode no longer leave a dead notebook tab** ([#209](https://github.com/totoshko88/RustConn/issues/209)) — when a graphical connection delegates its display to a separate external viewer (TigerVNC, xfreerdp, remote-viewer), RustConn launched the viewer correctly but also opened an unusable placeholder tab in the main window. Tab suppression is now decided by a single shared predicate in `rustconn-core` (protocol + window mode + client mode) used by both VNC launch paths; the session is surfaced in the sidebar with a connected status and an external-viewer emblem instead of a tab. Embedded RDP tabs are unchanged, and a viewer-launch failure now shows an error without creating a tab
- **Telnet connections were stuck on the Vault password source** ([#210](https://github.com/totoshko88/RustConn/issues/210)) — the connection editor hid the "Password Source" selector for Telnet (it was grouped with Serial/MOSH/Kubernetes/Zero Trust as protocols that "don't use stored passwords"), so a Telnet connection whose source was still `Vault` — e.g. one created before the selector was hidden — could never be switched to `None`. On every connect its credential resolution ran a vault lookup that found nothing and surfaced the "Vault entry not found for '<name>'. You will be prompted for a password." toast. Telnet is an interactive login protocol, so its Password Source selector is now shown again alongside SSH/RDP/VNC/SPICE, letting users pick `None` (or `Prompt`) and clear the spurious warning

### Improved

- **Orthogonal (color-independent) sidebar state indicators** — connection state in the sidebar is now conveyed by shape plus icon, not color alone, so the connected, in-split, and external-viewer states stay mutually distinguishable in grayscale and for color-vision-deficient users. When a single connection matches several states at once, each is shown as a separate indicator with a distinct shape, and every icon and emblem carries a screen-reader-reachable accessible label
- **"Disconnect" for an external viewer no longer blocks the interface** — ending a RustConn-owned viewer from the sidebar previously drained the killed process on the GTK main thread with a busy wait of up to 5 seconds, which could freeze the window (and multiplied across a connection with several sessions). Disconnect now signals the kill and hands the reap to the shared poll timer: the sidebar clears immediately when the viewer exits at once (the common case) or within one poll cycle (≤2 s) otherwise, and the UI stays responsive throughout
- **Split-view guest sessions no longer leave a placeholder tab** — when a session is moved into another tab's split layout, its own standalone tab (which previously showed a "Displayed in Split View" placeholder) is now removed entirely, so split guests no longer clutter the tab bar or the Tab Overview. The session stays fully live in the split, keeps its sidebar "connected" status and split-color indicator, and its tab is recreated automatically when it leaves the split (unsplit, close pane, or the owner tab closing)
- **External-viewer sessions are cleaned up on quit and counted in the close prompt** — a RustConn-owned external viewer (TigerVNC, FreeRDP, remote-viewer) is now terminated when you quit the app, instead of being orphaned as a stray process, and its open history entry is closed at the same time (detaching viewers like remmina/krdc keep running, as before). Because these sessions have no notebook tab, they were also invisible to the "close with active connections?" confirmation; the prompt now counts tracked external sessions too, so a window holding only external viewers still warns before quitting. Both the window close button and Ctrl+Q are covered
- **Split panes can be closed directly, and the split tab keeps its color indicator** — each session shown in a split pane now has a close (×) button in its top-right corner that terminates that session (previously a split guest, having no standalone tab, could not be closed without first ejecting it). The split owner's tab also keeps its colored split indicator reliably: it is no longer wiped by connection-state changes (an embedded RDP session, for instance, re-reports "connected" on every resolution change), so you can always tell at a glance which tab hosts a split

### Internal

- **Removed the dead embedded-SPICE widget** — since SPICE always renders in an external viewer (the native embedded client was removed in 0.18.0) and external-session tracking (this release) suppresses the SPICE tab up front, the `EmbeddedSpiceWidget` and its tab-creation, split, and redraw plumbing could never run. The whole embedded-SPICE path is removed: the `embedded_spice` module and its `SessionWidgetStorage::EmbeddedSpice` variant, the `create_spice_session_tab`/`get_spice_widget` helpers, the dead SPICE branch in the launch path, and the now-orphaned `launch_spice_viewer`/`SpiceViewerLaunchResult` from `rustconn-core`. SPICE launching is unchanged (`detect_spice_viewer` + `build_spice_viewer_args` + the shared external-session registry). The detaching-viewer check now matches on the viewer's file name, and the FreeRDP `/p:` password argument carries an explicit security note explaining why argv (not stdin) is used with sdl-freerdp3

### Dependencies

- **Updated**: bytes 1.12.0→1.12.1, memchr 2.8.2→2.8.3 — transitive patch bumps only, no API impact; the workspace still builds clean in release. `cargo deny check` passes (advisories, bans, licenses, sources all ok — no new advisories). libadwaita held at 0.9.1 — 0.9.2 still drops the `IsA<Accessible>` bound on `ActionRow`, which would break the accessible-label relations (same reason as 0.18.2). CLI download versions audited via `scripts/check-cli-versions.sh`: all auto-resolve endpoints reachable and TigerVNC (the only pinned component) is at 1.16.2 (latest) — no CLI download updates required

## [0.18.2] - 2026-07-08

### Added

- **SPICE unix socket connections** — connect to a SPICE server via a local unix socket (`spice+unix:///path/to/socket`) instead of host:port. Useful for libvirt/QEMU VMs with a local socket. Available in the Advanced connection editor (SPICE tab → Connection → Unix Socket toggle + socket path field with a Browse button that defaults to `/run/libvirt/qemu`). Enabling the toggle disables the Jump Host row and clears any stored jump host, since a socket connects locally. The wizard creates the connection and the user sets the socket path in the Advanced editor. Closes #208

### Changed

- **Compact interface mode extended to more elements** — compact rules now also cover the monitoring bar, split panel margins, and playback toolbar, so density is consistent across the whole window instead of only the header and tab bars. Monitoring bar height is now CSS-driven (no more Rust hardcode), letting compact mode reduce it from 28px to 22px. The subtitle in Settings now mentions macOS alongside KDE and small screens
- **Compact interface is now reachable from the primary menu and a keyboard shortcut** — besides Settings, a "Compact Interface" toggle (with a checkmark reflecting the current state) sits in the primary menu next to Fullscreen, and `Ctrl+Shift+D` toggles it from anywhere. The shortcut is listed in the keyboard-shortcuts editor and can be remapped. Toggling from the menu or shortcut persists the setting, same as the Settings switch
- **Automatic compact mode on small windows** — a new "Compact automatically on small windows" option (Settings → Interface, off by default) engages the compact chrome on its own when the window becomes short (≤800px) or narrow (≤900px) and relaxes it again when the window grows. It is driven by watching the window size and toggling the `.compact` CSS class, deliberately independent of the adaptive-layout `AdwBreakpoint`s (which drive property setters, not classes) so the two never conflict. The manual switch still forces compact on always. The preference is stored locally per device — compact settings are not part of Cloud Sync, so a small laptop and a large desktop keep their own density
- **Compact mode: larger minimum tap targets and consistent scale** — interactive controls (sidebar toolbar, protocol-filter, embedded-viewer and playback buttons) no longer shrink below 24px in compact mode, and the smallest font reductions were eased (0.85em → 0.9em), keeping compact chrome comfortably clickable and legible on a pointer/small-screen setup while still denser than the default

### Fixed

- **Sidebar width setting could not narrow the sidebar below ~360px** — lowering the "Sidebar width" preference had little visible effect because three separate floors held the sidebar wide. First, `260` was hardcoded as the minimum in the setting's SpinRow, both clamps, the `AdwOverlaySplitView` `min-sidebar-width`, the migration range, and the sidebar container's `width-request`; all are now `180`. Second, the six bottom-toolbar icon buttons were pinned to a `min-width: 44px` HIG tap target (~360px for the row), so the row alone could not shrink; their width is relaxed to `24px` with tighter padding and margins (height keeps the 44px target, matching what compact mode already used). Third — the real culprit — the collapsed protocol-filter and bulk-action `GtkRevealer`s still reserved their child's *width* even while hidden: a `SlideDown` revealer animates only height, and its horizontal button rows do not wrap without libadwaita 1.7, so ~300px was reserved permanently. Collapsed revealers are now dropped from layout entirely (made invisible once fully closed, restored the instant they start revealing, animation preserved). The sidebar can now be dragged down to its configured minimum
- **Compact mode inflated the chrome below the header instead of shrinking it** — two issues combined to make the top of the window look ~3x taller in compact mode. First, the compact CSS set a `min-height` floor on the header's inner `windowhandle`/`box` nodes via a `> box > *` universal rule, which GTK stacked vertically. Second, a `min-height` on the `banner` node forced the two normally-collapsed AdwBanners (cloud-sync and secret-backend warnings) to each reserve 28px, adding a ~56px phantom band below the header. Compact now only reduces the `headerbar` element's own min-height (with shrunk, zero-margin buttons) and leaves banners to size to their content
- **Homebrew formula: merged two `cargo install` calls into one `cargo build --release`** — the previous formula ran `cargo install` separately for the GUI and CLI binaries, duplicating dependency resolution and target scanning. Now a single `cargo build --release -p rustconn -p rustconn-cli` builds both at once, reducing macOS Homebrew install time
- **SPICE CA-certificate Browse button now opens a file chooser** — a pre-existing defect: the CA-certificate Browse button in the SPICE tab was created but never wired to a handler, so clicking it did nothing. The file-chooser is now attached directly at button creation (resolving the parent window from the widget tree at click time, matching the unix-socket Browse button), and the unused `setup_ca_cert_file_chooser` helper was removed. The SSH key button was verified to be correctly wired in every dialog path
- **CLI and GUI SPICE viewer commands no longer diverge on USB redirection** — `SpiceProtocol::build_command` (used by `rustconn-cli connect`) emitted `--spice-usbredir-redirect-on-connect=auto` while the GUI viewer path emitted `--spice-usbredir-auto-redirect-filter`, so the two produced different `remote-viewer` arguments. Both now share one URI builder (`spice://` / `spice+tls://` / `spice+unix://`) and the same USB auto-redirect filter, and the duplicated unix-socket branch in `build_command` was collapsed

### Dependencies

- **FreeRDP (Flatpak) updated 3.27.1 → 3.28.0** — the bundled FreeRDP used by the Flatpak/Flathub builds for embedded and external RDP now tracks the latest stable 3.28.0 (revived iOS client, Android build updates, smartcard API; carries the 3.27 security hardening). Manifest URL and sha256 updated across the Flatpak, local, and Flathub manifests
- **Updated**: zbus 5.16→5.17, zvariant 5.12→5.13, inotify-sys 0.1.7→0.1.8, num-iter 0.1.45→0.1.46, rustversion 1.0.22→1.0.23. (libadwaita held at 0.9.1 — 0.9.2 drops the `IsA<Accessible>` bound on `ActionRow`, breaking the accessible-label relations; revisit with a source fix in a later release)

## [0.18.1] - 2026-07-07

RustConn 0.18.1 generalizes split view from VTE terminals to **any in-process (embedded) tab**. Headline change: split eligibility is now decided by the session's *widget kind*, not by whether it runs in a VTE terminal — so RDP, VNC, and SPICE sessions rendered by the in-process embedded viewer can now be placed in split panels alongside or mixed with terminal sessions, while only external-viewer sessions are declined. Embedded viewers also adapt their toolbar and resolution to narrow panels and small windows, and a batch of embedded-RDP rendering/scaling fixes lands alongside.

Changes are grouped by type in the sections below (Added, Changed, Fixed, Dependencies).

### Added

- **Split view now works for any embedded tab, not just VTE terminals** — previously only terminal-based tabs (SSH, Telnet, Serial, Kubernetes, Local Shell, and SFTP in mc mode) could be split; eligibility is now keyed on the session's widget kind, so any in-process embedded viewer qualifies. In practice this adds embedded RDP, VNC, and SPICE remote desktops: they can now be split horizontally/vertically, dragged between panels, placed via Select Tab, focused, closed, and evicted to a new tab just like terminals, and each session keeps its live connection while it moves. Terminals and embedded desktops can be mixed in one split, and every panel shares the split container's color
- **Embedded viewers adapt to narrow panels and small windows** — when a split panel, or a small/narrow application window, gets tight, the embedded toolbar collapses its secondary actions (Copy, Paste, Autotype, Scripts, Quick actions, Save Files) into an overflow ("⋯") menu while the primary actions (Fit resolution and Ctrl+Alt+Del) stay directly visible, and the remote desktop rescales so a small or oddly-shaped area stays fully filled and legible instead of letterboxed. This also benefits a single embedded tab in a small window

### Changed

- **Splitting an external-viewer session is now declined with a clear message** — a session shown through an external viewer (xfreerdp, vncviewer, or an external SPICE viewer) has no in-process widget to place in a panel, so a split attempt now shows "Split view is not available for external-viewer sessions. Switch this connection to embedded mode to use split." and leaves the layout unchanged. The old "Split view is available for terminal-based sessions only" message is gone
- **Keystroke broadcast is restricted to terminal sessions** — the broadcast toggle appears only when a split holds at least two terminal sessions and a terminal panel is focused, and mirroring never targets an embedded remote desktop. In a split that mixes terminals and embedded desktops, keystrokes are mirrored only among the terminals
- **Disconnecting inside a split keeps the panel open** — when an embedded session in a panel loses its connection, the panel stays open showing the in-widget reconnect banner instead of collapsing the split; reconnecting restores the session in the same panel

### Fixed

#### Split view

- **Closing the tab that owns a split stranded its other sessions on the "Displayed in Split View" placeholder** — the tab-close cleanup only cleared the closing session from the split bridge; it never returned guest sessions to their home tabs. It now reparents every guest back to its own tab and resumes suspended monitoring before the owner tab is torn down, so both terminal- and RDP-owner splits recover correctly
- **Clicking on an embedded RDP/VNC panel in split view only changed focus but did not pass mouse events to the remote desktop** — the split panel's capture-phase `GestureClick` recognised only buttons and VTE terminals as interactive, consuming clicks meant for the `DrawingArea`. The handler now also recognises `DrawingArea` and `GLArea` as interactive, so mouse events propagate to the embedded viewer's input controllers

#### Embedded RDP rendering

- **Embedded RDP on a small window rendered a huge cursor/UI instead of a dense, fully-visible desktop** — on a scaled display the server rendered at the full display DPI, producing a tiny logical desktop with oversized UI. Resolution is now keyed on *logical* size via `desktop_request_for_area`: below 640×480 the client requests a ≥-minimum desktop at 100% DPI and downscales locally. All four request paths (initial, settle-snap, resize, Fit button) share one guard
- **Embedded RDP went blank after unsplitting** — re-wrapping the returning RDP widget in a fresh `adw::ToastOverlay` broke the `DrawingArea`'s draw function. The viewer is now reparented directly (like VNC/SPICE), rendering immediately after unsplit
- **Embedded RDP could enter a resize feedback loop on small windows** — the debounced handler's threshold comparison always triggered, causing endless resize ping-pong. It now compares the *computed request* to current resolution and adds a 48 px hysteresis, so minor allocation nudges are ignored

#### Workspace restore

- **Workspace restore skipped RDP/VNC/SPICE connections** — restore now uses `start_connection_with_credential_resolution` for all entries, so non-SSH sessions resolve vault credentials correctly
- **Workspace split restore failed for async connections (RDP/VNC/SPICE)** — the profile now persists guest entries and recreates multi-panel splits with a deferred placement callback that moves each guest session into its panel the moment its tab is created
- **Workspace restore dropped Local Shell tabs** — restore now detects Local Shell entries by `connection_id.is_nil() && protocol == "local"` and spawns them via the manual "Local Shell" path
- **Multi-panel split restore (3+ panels) applied splits to the wrong tab** — save now records `split_owner_entry_index`, and restore defers split actions until the owner's tab appears
- **Multi-panel restore lost guest sessions that connected before the owner** — early-arriving guests are now buffered and placed via a deferred idle callback after splits complete
- **Workspace split restore created all panels in one direction** — save now captures per-split directions from the bridge's tree (DFS) and fires the correct action for each split
- **Workspace split restore produced unbalanced layouts** — restore now uses a balanced splitting algorithm that picks the largest panel and divides along its longest side, yielding a uniform grid regardless of original action order
- **Workspace save recorded duplicate guest indices for multiple Local Shell sessions** — save now tracks used entry indices and finds the next unused match for each duplicate `Uuid::nil()` connection ID
- **Workspace restore with sync owner (Local Shell) left guest panels empty** — a dedicated idle-deferred placement path now scans the notebook after splits complete and places all sync guests in one pass

#### Networking

- **SSH connections via jump host (ProxyCommand) failed with "Permission denied (publickey)" when opened in parallel** — the resolved identity key is now passed directly to the ProxyCommand, and a `ControlPath` (slave-only) is set so that re-authentication is skipped when the master socket exists

### Dependencies

- **Updated**: `cc` 1.2.65→1.2.66, `crossbeam-channel` 0.5.15→0.5.16, `crossbeam-deque` 0.8.6→0.8.7, `crossbeam-epoch` 0.9.18→0.9.20, `crossbeam-utils` 0.8.21→0.8.22, `inotify` 0.11.2→0.11.3, `jobserver` 0.1.34→0.1.35, `lzma-rust2` 0.16.4→0.16.5, `num-bigint` 0.4.7→0.4.8, `zerocopy` 0.8.52→0.8.53, `zerocopy-derive` 0.8.52→0.8.53 (transitive build/utility crates; no user-facing change)

## [0.18.0] - 2026-07-05

RustConn 0.18.0 is a **HiDPI and cleanup** release. Headline changes: a new *Native (full HiDPI)* Display Scale option plus sharper RDP scaling and cursor rendering on 4K/retina screens ([#207](https://github.com/totoshko88/RustConn/pull/207)); embedded VNC now decodes Tight/JPEG and no longer leaves stale regions after a scroll or window move; and a large internal cleanup removes the abandoned native-SPICE experiment, an unused KeePassXC browser backend, dead render buffers, and a parallel tracing subsystem. Rounding it out are translation fixes (typographic strings now actually localise), fewer per-search allocations, and refreshed desktop-integration dependencies.

Changes are grouped by type in the sections below (Added, Performance, Removed, Changed, Internal, Fixed, Dependencies).

### Added

- **RDP/VNC Display Scale gained a "Native (full HiDPI)" option** ([#207](https://github.com/totoshko88/RustConn/pull/207)) — the embedded Display Scale dropdown offered `Auto` (logical resolution, bandwidth-saving) and fixed steps (125–400%), but to get a crisp "retina" remote desktop the user had to know their monitor's scale and pick the matching percentage by hand, which then broke if the window moved to a differently-scaled monitor. The new `Native` option follows the display's live scale factor, so a HiDPI screen gets a full-resolution image that adapts across monitors — the one-toggle "full retina" behaviour requested on #207, without displacing the bandwidth-saving `Auto` default. Implemented by resolving the scale multiplier against the widget's runtime `scale_factor()` (a new `ScaleOverride::resolved_scale`) instead of a compile-time constant

### Performance

- **Search result `matched_fields` no longer allocates for the built-in field labels** — each search hit recorded which fields matched (`name`, `host`, `tags`, `group`, `username`, `description`) by pushing freshly-allocated `String`s into `ConnectionSearchResult.matched_fields`. Those labels are compile-time constants, so the field is now `Vec<Cow<'static, str>>`: the fixed labels are borrowed `&'static str` (zero allocation) and only the per-connection `custom_property:<name>` entries allocate. A small win confined to matched results, not the hot scan path
- **Embedded VNC dropped its redundant per-frame `VncPixelBuffer` copy** — like the RDP widget, VNC wrote every frame update and `CopyRect` into both the authoritative Cairo-backed buffer and a legacy `VncPixelBuffer` that was only read by a fallback draw path that never triggered (the Cairo buffer always has the data). The legacy buffer, its writes, and the dead fallback are removed, saving a full-frame copy per update. As part of this, the surface-dimension OOM guard now lives in the shared `CairoBackedBuffer` (clamped to 16384 px/axis), so a server-requested resolution is bounded on both the RDP and VNC render paths instead of only inside the removed VNC buffer
- **Removed the dead legacy `PixelBuffer` from the embedded RDP widget** — investigation confirmed the FreeRDP fallback renderer that was meant to feed this buffer is not wired up (`on_end_paint` had zero callers, the FreeRDP worker thread ignored its `frame_buffer`, and FreeRDP sessions run in an external `xfreerdp3` window instead). The IronRDP path renders exclusively through the Cairo-backed buffer, so the `PixelBuffer` was allocated, resized and cleared every connection/resize but never displayed. Removing it (the struct, the widget field, the never-taken fallback draw branch, `on_end_paint`, and the FreeRDP thread's unused frame buffer) deletes dead allocations and a whole render path that could never fire
- **Sidebar search no longer deep-clones every connection on each keystroke** — `SearchEngine::search` (and the debounced/benchmark variants) took an owned `&[Connection]`, so the sidebar filter had to clone the entire connection list (all fields, tags, per-protocol config) on every search, only softened by the 100 ms debounce. The API now takes `&[&Connection]` and the sidebar passes the borrowed list `list_connections()` returns directly — zero per-keystroke copies. Group scoring/filtering also switched from a linear `groups.iter().find()` per connection (O(connections × groups)) to a single `HashMap<Uuid, &ConnectionGroup>` lookup built once per search, and a redundant `String` allocation in the tag-field dedup check was removed

### Removed

- **Dead tracing initialisation/config subsystem** — `rustconn-core`'s `tracing` module carried a full parallel logging-setup API (`TracingConfig` + builders, `TracingLevel`, `TracingOutput`, `TracingError`/`TracingResult`, `init_tracing`, `get_tracing_config`, `is_tracing_initialized`, the `field_names` constants, and the `trace_operation!`/`trace_operation_debug!` macros) that nothing ever called: the application initialises `tracing` directly via `tracing_subscriber::fmt().init()` in `main.rs`, and the only references to this API were its own unit/property tests. Removed the subsystem and the tests that exclusively covered it, keeping the live `span_names` constants (used across the export/import/search/session paths) and their tests
- **Unused `SplitSessionId` re-export alias** — `split::SessionId` was re-exported at the crate root as `SplitSessionId`, but nothing imported that alias; the alias is dropped (the underlying `split::SessionId` type is unchanged)
- **Unreachable "public API for future use" helper methods** — six methods carried `#[expect(dead_code, reason = "Public API for …")]` with no callers and no tracking issue: `ConnectionSidebar::{is_connection_recording, recording_checker_rc}`, `TerminalNotebook::{remove_tab_group, known_group_names, has_active_cluster_sessions}`, and `Playback::state`. Per the project rule that a `dead_code` allowance must point to a concrete plan, these speculative methods are removed (the underlying fields stay live via their real callers — `set_recording_checker`, `set_tab_group`, the cluster register/unregister/get helpers). Removing `remove_tab_group` also retired its now-orphaned private `clear_group_color` helper
- **Dead `KeePassXC` browser-integration backend** — `KeePassXcBackend` implemented the `KeePassXC` browser protocol (association handshake + credential store/retrieve/delete over a Unix socket), but it was never constructed anywhere in the application: every code path for the `KeePassXc`/`KdbxFile` backend types resolves credentials through direct `.kdbx` file access or falls back to libsecret/Keychain, and the only place the struct was instantiated was its own unit test. Its `delete()` returned an "unsupported" error that, being unreachable, never fired. The struct, its `SecretBackend` impl, the browser-protocol request/response types and association/socket code, and the `KeePassXcBackend` re-export are removed. The still-used KDBX-database keyring helpers (`store`/`get`/`delete_kdbx_password_from_keyring`) are kept and moved to a focused `kdbx_keyring` module. No user-facing change — `KeePassXC` databases are, and were, opened via the direct-file backend
- **Dead Wayland-subsurface placeholders and an unused toolbar field** — the embedded RDP and VNC widgets each carried a `WaylandSurfaceHandle`/`VncWaylandSurface` skeleton whose `initialize`/`commit`/`damage`/`cleanup` methods were all no-ops (rendering actually goes through a GTK `DrawingArea` + Cairo), plus dead `on_frame_update`/`on_copy_rect` widget methods with no callers. These placeholders and their wiring are removed; native Wayland compositing can be added directly if/when it is actually implemented. Also dropped the unread `PlaybackToolbar::search_entry` field (the widget stays alive through the toolbar it is appended to)
- **SPICE widget render buffers** (`SpicePixelBuffer` + Cairo buffer) — with native embedding gone, the SPICE session always runs in an external viewer, so the widget's `DrawingArea` only shows a status line. The frame buffers and the unreachable embedded draw branch they fed are removed; the widget now draws status text only
- **Native embedded SPICE client** (the `spice-embedded` feature) — a spike against the bundled `spice-client` 0.2 confirmed that embedded SPICE cannot work without forking the crate: its public API exposes no inputs channel (keyboard/mouse could never be forwarded — the handlers only logged events) and no way to read raw display frames after the event loop starts (`start_event_loop` moves the display channels into background tasks, and the only frame accessor is a WASM-oriented base64 data URL, not BGRA). The feature was already disabled by default in 0.17.10; it is now removed entirely, along with the `spice-client` dependency and its transitive tree. SPICE sessions continue to open in an external viewer (virt-viewer/remote-viewer), which is unchanged and fully functional. This deletes the dead native `SpiceClient`, the `SpiceClientEvent`/`SpiceClientCommand`/`SpiceRect` types, `is_embedded_spice_available()`, and the never-reachable input/render code paths in the SPICE widget

### Changed

- **Operation-result feedback now uses toasts, and a data-loss error uses a dialog (GNOME HIG)** — successful Delete / Export / Import previously popped a blocking `adw::AlertDialog` the user had to dismiss; these are now non-blocking toasts, matching the rest of the app. Conversely, failing to save a secret variable to the vault was reported with a transient toast even though the plaintext value is cleared from settings right after (so a missed toast meant silent secret loss); it is now a blocking `adw::AlertDialog` naming the variable and telling the user to re-enter it and check the secret backend

### Internal

- **Command Palette dialog now has an accessible title** — the `adw::Dialog` was created with an empty title, leaving screen readers without a name for the window; it is now titled "Command Palette". Also wrapped the smart-folder example placeholders ("Prod SSH Servers", host pattern, example tags) in `i18n()` so they are translatable, and refreshed the translation template (16 languages)

### Fixed

- **Embedded VNC now decodes Tight/JPEG rectangles instead of showing noise** — the most bandwidth-efficient VNC encoding (Tight, which sends photographic regions as JPEG) was disabled in 0.17.10 because its JPEG sub-rectangles were forwarded to the renderer as if they were raw BGRA pixels, painting garbage. The client now decodes each JPEG rectangle to BGRA (via the pure-Rust `zune-jpeg`, already present transitively, so no new third-party crate enters the tree) and Tight is offered first again, falling back to ZRLE/CopyRect/Raw. Grayscale (Luma) and truecolor (RGB) JPEGs are both handled; a rectangle that fails to decode is skipped with a warning rather than tearing down the session
- **Embedded VNC left stale regions on screen after a server-side scroll or window move** — the `CopyRect` encoding (which tells the client to blit an already-received region to a new location instead of resending pixels) was applied only to the legacy `VncPixelBuffer`, but the widget's fast draw path reads the persistent Cairo-backed buffer. After any `CopyRect` (common when scrolling a terminal or dragging a window on the remote) the moved region was correct in the unused buffer but stale in the one actually painted, leaving ghost/torn areas until that region happened to be repainted by a later full update. `CopyRect` is now mirrored into the Cairo buffer as well, via a new `CairoBackedBuffer::copy_rect` that stages the source through a temporary buffer so overlapping copies stay correct in either direction. RDP is unaffected (IronRDP delivers moved regions as ordinary frame updates, which already write the Cairo buffer)
- **RDP display scale was lost on dynamic resize** ([#207](https://github.com/totoshko88/RustConn/pull/207)) — the MS-RDPEDISP `SetDesktopSize` path sent the new resolution without a desktop scale factor (`encode_resize(.., None, None)`), so after any dynamic resize (e.g. toggling the sidebar with F9) the server reverted to 100% DPI and an explicitly-scaled HiDPI session shrank to a tiny UI. The requested scale is now threaded through `SetDesktopSize → encode_resize` and re-sent on every dynamic resize and on the initial settle-snap, so an explicit Display Scale (e.g. 200%) stays crisp across resizes. With `Display Scale = Auto` the factor is 100% on the logical-sized desktop, as introduced in 0.17.10. Based on the contribution by @dwetscher
- **Embedded RDP HiDPI cursor was partly missing and mis-sized** ([#207](https://github.com/totoshko88/RustConn/pull/207)) — on a scaled session the pointer bitmap arrives at the session DPI (2× on 200%) and was downscaled to logical size with a nearest-neighbor sampler that dropped every other row/column, erasing the thin 1px strokes of HiDPI cursors (the "half-missing" pointer). Cursor downscaling is now an alpha-weighted area average (box filter) over every covered source pixel, preserving thin strokes, with correct premultiplied-alpha edge blending and R↔B swap for GDK. At `Display Scale = Auto` (100% session) it is an identity copy. Based on the contribution by @dwetscher
- **Several UI strings were never translated because their `\u{…}` escapes leaked into the message catalog** — user-facing strings that embedded typographic characters via Rust unicode escapes (e.g. `"Connection \u{201c}{}\u{201d} created"`, `"Advanced\u{2026}"`, the variable-setup prompt) were extracted by `xgettext --language=C`, which does not understand Rust's `\u{XXXX}` syntax and stored the literal backslash-escape as the `msgid`. At runtime the lookup key is the *real* character (`"Connection "{}" created"`), so it never matched the catalog and the string always fell back to English in every locale. The escapes are now written as the actual UTF-8 characters (`… ‘ ’ " "`) in the source, and the 16 translation catalogs were converted in place so their existing translations are preserved, then re-merged against the refreshed template

### Dependencies

- **Updated**: `cpal` 0.17 → 0.18 (embedded-RDP audio output — migrated to the new by-value `StreamConfig` argument of `build_output_stream`; streams are still started explicitly via `play()`), `muda` 0.16 → 0.19 and `tray-icon` 0.20 → 0.24 (macOS menu-bar tray stack — drop-in for the menu/icon/builder API used here). No user-facing change; the stale transitive `windows-*` 0.42 crates were dropped from `Cargo.lock`
- **CLI downloads** — no changes; all seven pinned Flatpak CLI tools verified current against upstream (`./scripts/check-cli-versions.sh` → all ✅)

## [0.17.10] - 2026-07-04

### Fixed

- **Embedded RDP/VNC requested a scale-inflated resolution on HiDPI displays** — the remote desktop was requested in *device* pixels (widget logical size × compositor scale factor), so on a 4K screen at 200% the client negotiated resolutions like 3868×2518 and sent a DPI hint the server often ignored, leaving a huge, tiny-UI desktop and pushing far more pixels over the network than needed. `Display Scale = Auto` now requests the widget's *logical* resolution (device ÷ scale) and the framebuffer is upscaled locally for HiDPI, so a session uses roughly a quarter of the bandwidth at 2× scale and the remote UI is comfortably sized again. The explicit Display Scale values (125–400%) still request a proportionally higher remote resolution for a sharper image when the extra bandwidth is acceptable
- **Embedded SPICE showed a black, unresponsive screen by default** — the bundled `spice-client` 0.2 does not forward frames or input in embedded mode (the client is moved into a background event-loop task while the command loop only emits connection events; key/pointer events were logged and dropped). Because the native path always returned `Ok`, the external-viewer fallback never fired, so the default SPICE experience was a blank window with no keyboard or mouse. The `spice-embedded` feature is no longer in the default set: SPICE now uses the external viewer (virt-viewer/remote-viewer), which works. Native embedded SPICE can still be opted into with `--features spice-embedded` and will return once native frame/input forwarding is implemented
- **Embedded VNC rendered garbage against TightVNC/TigerVNC** — the client advertised the Tight encoding, whose JPEG sub-rectangles were passed straight through as if they were raw BGRA pixels, so JPEG-coded regions showed as noise. Tight is removed from the default encoding list (ZRLE, CopyRect and Raw remain — comparable compression, decoded correctly). Tight/JPEG returns once a JPEG decoder is wired in (see `docs/PLAN-0.18.0.md`)
- **VNC input could momentarily stall the UI** — `send_command`/`disconnect` used `blocking_send` on the GTK main thread despite the method documenting itself as non-blocking; a full command channel would freeze the UI. They now use `try_send` (the channel has capacity 32; a rare input overflow is dropped rather than blocking the interface)

### Performance

- **Embedded RDP copied every frame twice** — on the IronRDP path each `FrameUpdate`/`FullFrameUpdate` was written into both the authoritative Cairo-backed buffer and a legacy `PixelBuffer` that is only ever read by the FreeRDP fallback renderer, costing an extra full-frame `memcpy` (~33 MB per frame at 4K). The redundant per-frame copy is removed; the FreeRDP fallback still populates its own buffer via `on_end_paint`
- **Sidebar search did an O(n²) result lookup** — after ranking, each result was matched back to its connection with a linear `Vec::find`; results are now indexed by id once (`HashMap`) for O(1) lookup

### Removed

- **Dead `detect_monitors()` placeholder** (`rdp_client/multimonitor.rs`) — an uncalled public helper that always returned a hard-coded 1920×1080 layout; removed (YAGNI) to avoid callers relying on fake data

### Internal

- Wrapped nine remaining user-facing strings in `i18n()` (SSH connection options, Wake-on-LAN / mc / ssh-agent warnings, vault-save error)
- Removed the unused `futures` dependency from the `rustconn` GUI crate

## [0.17.9] - 2026-07-03

### Fixed

- **SSH multi-hop password chain still crashes when a bastion's host key is unknown** ([#203](https://github.com/totoshko88/RustConn/issues/203)) — the 0.17.7 fix wired a password to every hop but multi-bastion password connections could still die instantly with `Connection closed by UNKNOWN port 65535`. Each password hop is reached through a nested `ProxyCommand` that has no controlling TTY, so its password is delivered via `SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=force`. When that hop's host key was not already in `known_hosts`, OpenSSH routed the interactive `Are you sure you want to continue connecting (yes/no/[fingerprint])?` host-key prompt to the same askpass helper — which answered with the *password*. SSH rejects the password as an invalid confirmation answer and re-prompts, looping until the bastion drops the connection (a local reproduction showed the helper invoked ~850 times in six seconds). Every hop that authenticates via forced askpass now sets `StrictHostKeyChecking=accept-new`, so a first-seen host key is accepted non-interactively (a *changed* key is still rejected, preserving MITM protection) and the prompt never reaches the password helper. Applies to both the first hop and every deeper nested hop; key/agent hops and single-bastion setups are unaffected
- **Embedded RDP: first frame is blurry and never sharpens** ([#206](https://github.com/totoshko88/RustConn/pull/206)) — the initial RDP resolution is measured before the permanent session toolbar is laid out, so the negotiated desktop is slightly taller than the drawing area it ends up in. That mismatch is smaller than the 50 px resize threshold, so the debounced resize handler never corrects it and every frame is sub-pixel-rescaled (blurry). Once the connection is established (layout has settled) the desktop is now re-requested at the drawing area's real size over the Display Control channel (MS-RDPEDISP), so the first real frame arrives at a 1:1 pixel map; it is a no-op when the size already matches (e.g. reconnect). The renderer also blits 1:1 when the framebuffer is within a few pixels of the drawing area instead of rescaling, for a sharp border. Based on the contribution by @dwetscher
- **Embedded RDP: initial snap caused a visible connect → reconnect flicker** ([#206](https://github.com/totoshko88/RustConn/pull/206)) — the "snap to settled size" above fired the moment the session connected, but the Display Control channel (MS-RDPEDISP) is not yet negotiated that early, so `encode_resize` returned `None` and the client treated it as *server does not support dynamic resize* — tearing the fresh session down and doing a full reconnect. The session visibly connected, dropped, then reconnected on its own (~2.5 s and a flash), even though the server supports MS-RDPEDISP and later manual resizes worked fine. The initial snap now waits for the new `DisplayControlReady` event (emitted when the server's Display Control capabilities arrive over DRDYNVC) and resizes smoothly over MS-RDPEDISP with no reconnect. It is never forced on a timer: some servers take much longer than a couple of seconds to negotiate the channel, and forcing the snap early made `encode_resize` fail over to a full reconnect (and occasional `Connection reset by peer` from the connect/disconnect churn). If the server never negotiates Display Control the desktop simply stays at the server size and is scaled to fit — a slightly softer frame, but no reconnect and no dropped session
- **Embedded RDP: stale seam/line left on screen after a resolution change** — after a resize (or the initial snap) the framebuffer is recreated blank and the server only resends the regions it considers changed, so an untouched strip kept its fill and showed as a persistent horizontal line that only cleared when its content later changed. On every resolution change the client now sends a full-desktop Refresh Rect PDU (MS-RDPBCGR `TS_REFRESH_RECT_PDU`), forcing the server to repaint the whole screen; the previously no-op `RefreshScreen` command is now implemented. No-op on servers that do not support refresh rects
- **Embedded RDP: over-conservative resolution rounding** — desktop dimensions were rounded down to a multiple of 4 "for codec compatibility", trimming up to 3 px of usable edge. Rounding is now the minimum the protocol actually requires — both dimensions forced even (MS-RDPEDISP mandates an even width; an even height keeps RemoteFX/H.264 tiling artifact-free) — and clamped to 7680×4320, staying under the MS-RDPEDISP 8192 px hard limit so a resize on a >4K display is no longer silently rejected. The rounding is consolidated into a single helper shared by the connect, resize and manual-fit paths

## [0.17.8] - 2026-07-03

### Fixed

- **Nested groups lose their parent when importing an Asbru-CM config** ([#205](https://github.com/totoshko88/RustConn/issues/205)) — importing an Asbru-CM configuration with three or more group levels intermittently placed the deepest groups directly under the import root instead of under their real parent (importing the same file repeatedly sometimes "fixed" it, since the outcome depended on hash-map ordering). The import routine topologically sorts groups so each parent is created before its children, but the readiness check consulted `group_uuid_map` — a map that is only populated later, in the group-creation loop, and was therefore empty during the sort. As a result only root-level groups sorted correctly; anything deeper was appended in arbitrary `HashMap`-iteration order, and whenever a grandchild landed before its (child) parent, the creation loop could not yet find that parent and re-parented the grandchild to the import root. The sort now tracks already-sorted group IDs in a dedicated set, so the ordering is deterministic and arbitrarily deep hierarchies are preserved on every import
- **Minimum window width too large in some locales — cannot tile or resize narrow** ([#204](https://github.com/totoshko88/RustConn/issues/204)) — the runtime minimum-width measurement added in 0.17.7 mirrored what the narrow breakpoint hides in the header bar, but forgot to also collapse the sidebar the way that tier does. Because `AdwOverlaySplitView` requests `sidebar-width + content-width` while it is not collapsed, the measurement captured the expanded sidebar — including its filter/search/bulk-action labels, which are locale-dependent and noticeably wider in German than in English — and pinned `width-request` to that inflated value. The result was a window that could not be tiled to half a screen or resized narrow when the UI language was German (English, Italian and Dutch were unaffected). The measurement now collapses the sidebar (so it overlays instead of taking side-by-side width) exactly as the narrow tier does before measuring, so only the content contributes and the resulting floor is locale-independent and small enough for tiling on every screen
- **Welcome-screen hint pinned the window wider in verbose locales** ([#204](https://github.com/totoshko88/RustConn/issues/204)) — with the sidebar removed from the measurement, the only remaining locale-dependent contributor was the "Double-click a connection…" hint at the bottom of the welcome screen: a plain `GtkLabel` does not wrap by default, so it reported its full translated width as the minimum, which is longer than the English original in several languages. The hint now wraps (and is centred), dropping its minimum width to the longest word, so no locale can hold the window open through the welcome content either

### Dependencies

- **Updated** (semver-compatible refresh): `arrayvec` 0.7.7→0.7.8, `num-bigint` 0.4.6→0.4.7

## [0.17.7] - 2026-07-02

### Fixed

- **Window controls disappear when the window is narrow** ([#204](https://github.com/totoshko88/RustConn/issues/204)) — on GNOME Wayland (and when half-tiling), narrowing the window made the minimize/maximize/close buttons vanish from the title bar until it was widened again. The cause was header-bar overflow, not a compositor bug: the header packs many icon buttons pinned to a 44×44 minimum tap target, so on narrow widths their combined minimum left no room for `GtkWindowControls` and GTK clipped the controls off the end. The old 600 sp breakpoint that was meant to shed buttons never fired because the window's minimum width was hard-coded to 800 px (so it could not reach 600 sp at normal text scale, and half-tiling forced a smaller allocation past the guard). The minimum width is now 360 px and a two-tier cascade of `AdwBreakpoint`s progressively hides non-essential buttons as the window narrows — split-view buttons below 820 sp, then Quick Connect / Delete / New Group / Settings / Shell below 560 sp — so the window controls always keep their space. Every hidden button remains reachable from the primary menu or the sidebar context menu
- **Window would not resize narrow smoothly; sidebar never auto-hid** ([#204](https://github.com/totoshko88/RustConn/issues/204)) — the real minimum-width driver turned out to be the header bar itself (all of its buttons plus the window controls ≈ 794 px), not the sidebar or welcome screen. The adaptive breakpoints are now designed so the window shrinks in one continuous drag and the sidebar auto-hides like F9: `AdwApplicationWindow` applies only one breakpoint at a time (the last-added whose condition holds) and computes the minimum width from the currently-active tier, so if a tier's resulting minimum is wider than the next tier's threshold the drag "sticks" at the boundary (the reported behaviour where shrinking stopped, then resumed to the sidebar width on a second drag). Two cumulative tiers were chosen so each tier's minimum falls below the next threshold — medium ≤ 820 sp collapses and hides the sidebar and drops the split, Delete and New Group buttons (≈ 578 px); narrow ≤ 600 sp additionally drops Quick Connect, Settings and the Shell pill (≈ 390 px), leaving Sidebar toggle, New Connection and the menu beside the window controls. When collapsed the sidebar is hidden (`show-sidebar` = false), not shown as an overlay, matching the F9 toggle; F9 or the edge gesture reveals it as an overlay, and growing the window restores everything. The centre title is hidden in both narrow tiers to free space for the window controls, and the window's minimum width is now measured at runtime rather than guessed: once the window is mapped (and the window controls are realized), the widgets the narrow tier hides are momentarily hidden, the content's real minimum width is measured, and `width-request` is pinned to it. This guarantees the minimum matches the actual narrowest header — Sidebar toggle, New Connection, the menu and the window controls — so the close button is never clipped, independent of theme, font, locale or the compositor's decoration layout. The sidebar's configured width is also honoured again: the width fraction was raised (0.27 → 0.7) so the "Sidebar width" preference sets the actual width instead of being capped below it by a small proportion
- **Welcome screen wraps shortcut labels character-by-character on narrow widths** ([#204](https://github.com/totoshko88/RustConn/issues/204)) — the three-column welcome layout squeezed the keyboard-shortcut column until combos like `Ctrl+Shift+Backspace` broke mid-word into an unreadable vertical stack, and the fixed three-column horizontal row also forced a large minimum width on the whole window. The responsive wide/narrow switch never actually triggered (it listened on `width-request` changes and on the whole-window surface size rather than the content width). The feature / shortcut / extras groups now live in a `GtkFlowBox` that shows three columns when wide and reflows to two then one as the window narrows, scrolling vertically inside `AdwStatusPage`'s own scrolled window. A flow box reports a one-column minimum width, so it no longer pins the window open the way the fixed three-column row did, and shortcut combos are pinned to a single line so they never wrap by character. The quick-action pills (New Connection / Quick Connect / Import) were moved into a flow box for the same reason, so the welcome no longer holds the window wider than the header bar needs. The duplicated narrow-mode column tree and the two dead size handlers were removed
- **SSH multi-hop password chain fails — only one bastion receives a password** ([#203](https://github.com/totoshko88/RustConn/issues/203)) — with a two (or more) reference jump host chain where both bastions authenticate by password (client → JUMP1 → JUMP2 → TARGET), only JUMP2 (the hop adjacent to the target) received its password via `SSH_ASKPASS`; JUMP1 (the entry bastion, built as a nested `ProxyCommand`) had no askpass wiring and no password, so it could not authenticate without a TTY and the session died instantly with exit 255. The askpass mechanism now generates per-hop scripts (`rustconn-jh-askpass.sh`, `rustconn-jh-askpass-1.sh`, …) each reading its own indexed env var (`_RC_JH_PW`, `_RC_JH_PW_1`, …), and `build_nested_proxy_command_with_askpass` wires the correct `SSH_ASKPASS` prefix into every nested level. All hop passwords are resolved from cache/vault before spawn and passed as environment variables to the VTE process. Single-bastion setups are unaffected (backward compatible)

- **Snap build fails on Launchpad with Snapcraft 9.0** — the `plugin: rust` environment validation runs before any part executes and expects `rustup` on PATH; on a clean Launchpad container it is absent. The previous no-op `rust-deps` part did not actually install a toolchain. Switched the main part to `plugin: nil` (bypassing the Rust plugin's validation entirely) and made `rust-deps` install Rust 1.95 via `rustup.rs` in `override-pull`. The `override-build` adds `$HOME/.cargo/bin` to PATH before invoking `cargo build`. Both amd64 and arm64 builds are fixed

### Security

- **Updated `quick-xml` 0.39.4→0.41.0** ([RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194), [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195)) — the pinned `0.39` line was flagged by two advisories: a crafted XML document with a huge number of namespace declarations (or duplicate attributes) on one element could drive `quick-xml` into unbounded allocation and OOM-kill the process. RustConn only parses XML for the RoyalTS and libvirt import paths (`rustconn-core/src/import/`), so exposure is limited to importing an untrusted file, but the bump closes it regardless. 0.41 caps per-element namespace declarations and keeps duplicate-attribute checking linear; no code changes were needed for the upgrade

### Dependencies

- **Updated** (semver-compatible refresh): `inotify-sys` 0.1.6→0.1.7, `rand` 0.10.1→0.10.2

## [0.17.6] - 2026-07-01

### Fixed

- **SSH jump host password not resolved with KeePassXC and inherited/script sources** ([#191](https://github.com/totoshko88/RustConn/issues/191)) — connecting to a target through a bastion could get stuck on the bastion's password prompt: the bastion's own password was never resolved, so the target password leaked toward the bastion (or auto-fill was suppressed) unless you first opened the bastion connection directly. The bastion resolver only reimplemented a subset of normal login: it went through the flat vault path (which routes KeePassXC/KDBX lookups to LibSecret with a `rustconn/{name}` key, while the credential actually lives in the KDBX file under a hierarchical `{group}/{name} ({protocol})` key) and did not handle the `Inherit` (password from a parent group) or `Script` (external command) password sources at all. The jump-host resolver now delegates to the exact same routine as normal connection login (`AppState::resolve_connection_password_blocking`), so every backend (KeePassXC/KDBX included) and every password source (Vault, Variable, Inherit, Script) resolves identically and out-of-band via `SSH_ASKPASS`, regardless of connection order. The duplicated bastion resolution logic was removed
- **RDP session still aborts on resize — bulk compression disabled** ([#200](https://github.com/totoshko88/RustConn/issues/200)) — the 0.17.5 fix rebuilt a fresh FastPath bulk decompressor after the Deactivation-Reactivation Sequence, but a fresh decompressor starts with an empty compression history while the server continues its own history across the resize, so real servers (including Windows) still desynchronised and dropped the session. RustConn no longer requests bulk (MPPC/NCRUSH/XCRUSH) compression at all — `compression_type` is now `None`, matching the upstream `ironrdp-client` default — so the server never sends compressed FastPath updates and the whole failure mode disappears. Graphics stay compressed via RemoteFX (Quality/Balanced) or RLE/RDP6 bitmap compression (Speed), so the bandwidth impact is negligible. The now-unused reactivation decompressor rebuild and the direct `ironrdp-bulk` dependency were removed

- **Saving Preferences no longer wipes persisted window/sidebar state** ([#202](https://github.com/totoshko88/RustConn/issues/202)) — the settings dialog rebuilt `UiSettings` from scratch and only carried a couple of runtime-managed fields back, so clicking Save reset the stored window size (and maximized state), expanded sidebar groups, and search history to their defaults until the next normal close happened to rewrite some of them. The collected settings now preserve `window_width`/`window_height`/`window_maximized`, `expanded_groups`, and `search_history` from the current configuration, matching how `show_smart_folders` was already handled
- **Credentials silently lost when the system keyring is unavailable** ([#201](https://github.com/totoshko88/RustConn/issues/201)) — on hosts with no responding Secret Service (headless servers, minimal desktops, some sandboxes) saving a connection password appeared to succeed while the credential was actually dropped, so the next connection prompted again with no explanation. RustConn now probes real backend availability before trusting it and distinguishes a missing keyring client from an installed-but-unresponsive Secret Service. When a save cannot reach the keyring it raises an actionable blocking dialog that points to Settings → Secrets instead of failing quietly, and it automatically falls back to the new application-managed encrypted-file store, surfacing a toast so you know where the credential landed

### Added

- **Window remembers its maximized state** ([#202](https://github.com/totoshko88/RustConn/issues/202)) — RustConn already restored the last window size, but not whether the window was maximized, so a maximized session always reopened un-maximized. The close handler now also records `is_maximized()` into `window_maximized`, and startup re-maximizes when it was set (guarded by the existing "Remember window geometry" toggle). The unmaximized size is still stored underneath, so un-maximizing restores the previous dimensions. Window position is intentionally not persisted — Wayland does not let a client set its own placement
- **Application-managed encrypted-file secret backend** — a new "Encrypted file (no system keyring)" backend stores each credential as its own AES-256-GCM blob with an Argon2id key derived from a machine-specific key, written atomically with `0600` permissions to `dirs::data_dir()/rustconn/credentials.enc`. It works everywhere the system keyring does not — headless boxes, Flatpak/Snap sandboxes, and minimal desktops — and can be selected explicitly in Settings → Secrets or used automatically as the terminal fallback when the keyring is unreachable. The threat model for this store is documented in `docs/ZERO_TRUST.md`
- **Proactive keyring availability surfacing** — startup and the Settings → Secrets page now show distinct warnings for a missing keyring client versus an installed Secret Service that does not respond, so the cause of a degraded secret backend is visible before you try to save a password rather than after

### Changed

- **System keyring path migrated to the in-process `oo7` client (Linux/BSD)** — the libsecret/keyring integration no longer shells out to the `secret-tool` binary; it now talks to the Secret Service directly via `oo7` under `#[cfg(not(target_os = "macos"))]`, which removes the bundled libsecret/secret-tool dependency from the Flatpak manifests. macOS continues to use the system Keychain and never compiles `oo7`. Existing keyring entries stay compatible because the same attributes and labels are used, so no re-entry of saved credentials is required

### Dependencies

- **Updated** (semver-compatible refresh): `aws-lc-rs` 1.17.0→1.17.1, `aws-lc-sys` 0.41.0→0.42.0, `clap_complete` 4.6.5→4.6.7, `inotify-sys` 0.1.5→0.1.6, `libredox` 0.1.17→0.1.18, `rustls-pki-types` 1.14.1→1.15.0, `time` 0.3.51→0.3.53, `time-macros` 0.2.30→0.2.31, `zlib-rs` 0.6.4→0.6.5

## [0.17.5] - 2026-06-30

### Fixed

- **RDP session aborts when resized** ([#200](https://github.com/totoshko88/RustConn/issues/200)) — resizing an embedded IronRDP session (e.g. toggling the sidebar with F9) triggers a Deactivation-Reactivation Sequence; the reactivation handler rebuilt the FastPath processor with `bulk_decompressor: None`, but the server keeps bulk compression enabled across the resize. The next compressed FastPath update then logged `Received compressed FastPath data but no decompressor is configured` and the undecodable payload aborted the session with `Protocol error: Session error: [Fast-Path …] custom error`, dropping the connection. The handler now rebuilds a fresh decompressor for the negotiated compression type (RDP4/5/6/6.1) — mirroring `ActiveStage::new` — so compressed updates keep decoding after a resize
- **RDP to GNOME Remote Desktop falls back to FreeRDP instead of dead-ending** ([#199](https://github.com/totoshko88/RustConn/issues/199)) — connecting to a GNOME Remote Desktop (grd) server completed NLA/CredSSP, then IronRDP's connector tripped its own state machine in the capabilities/finalization phase and returned `general_err!("invalid state (this is a bug)")`, surfaced as `Connection finalize failed: … general error`. The in-session error handler's protocol-incompatibility detector only matched the literal `connect_finalize` (which never appears in the message — core wraps it as "Connection finalize failed" and IronRDP renders "invalid state (this is a bug)"), so no FreeRDP fallback fired and the user saw a hard failure even though Remmina/FreeRDP connects fine. The detector now matches the actual wrapper prefix and the upstream bug signature, so these servers transparently fall back to the external FreeRDP client
- **arm64 snap build links against a single pango** ([#198](https://github.com/totoshko88/RustConn/issues/198)) — the native arm64 snap leg failed at the link step with `undefined reference to pango_font_description_{get,set}_features` because ld mixed noble's `libpango-1.0` (from `/usr/lib`) with the gnome-46-2404 SDK's newer `libpangoft2-1.0`. The `snap/snapcraft.yaml` build now prepends the SDK arch-triplet lib dir to the rustc link search path (`RUSTFLAGS -L native=…`), so both halves of pango resolve from the SDK — the same copy used at runtime. amd64 already aligned, so this is a no-op there
- **Variable password auto-login on network equipment** ([#194](https://github.com/totoshko88/RustConn/issues/194)) — the 0.17.4 fix subscribed to VTE `cursor-moved` but still read `.lines().last()` of the full grid, which is empty when the prompt sits above ~20 blank rows (no-echo prompts on OLT/router gear). Detection now reads the line under the cursor (`get_cursor_line_text`, cursor position + `text_range_format`, falling back to the last non-empty grid line) and delegates matching to a pure, testable `looks_like_password_prompt` in `rustconn-core`. An idle re-check (~120 ms, scheduled at most once) covers the race where the signal fires before the prompt glyphs land in the grid. The one-shot injection guard is preserved
- **Jump host authenticates with its own Variable/Vault password** ([#191](https://github.com/totoshko88/RustConn/issues/191)) — the 0.17.2 fix resolved the bastion password only from the vault store key, missing a bastion whose password comes from a Variable source, and skipped the first reference hop entirely when a string `proxy_jump` was also present. The bastion password now resolves via the same `PasswordSource`-aware path as the target (Variable/Vault/cache) and is delivered out-of-band via `SSH_ASKPASS` on the ProxyCommand. A guard ensures the target password is injected into the VTE prompt only when there is no jump host or the bastion was handled out-of-band, so the target password can never leak to the bastion prompt

### Added

- **Terminal control shortcuts reach the session** ([#197](https://github.com/totoshko88/RustConn/issues/197)) — readline chords (Ctrl+F/P/N/W/H/M/I) were intercepted by the application accelerators before reaching the focused terminal. A new setting "Send terminal control shortcuts to the session" (enabled by default) temporarily suspends those single-Ctrl accelerators while the VTE terminal or an embedded RDP/VNC/SPICE viewer has focus, and restores them when focus leaves. `<Control><Shift>` chords and function keys stay active throughout; disabling the setting restores the old always-active behavior
- **Native arm64 snap build** — the Snap package now builds for arm64 (aarch64) alongside amd64 via a parallel CI matrix on native runners (`ubuntu-24.04` + `ubuntu-24.04-arm`, no QEMU). `snap/snapcraft.yaml` declares both platforms and uses architecture-independent `prime:` exclusions so the GTK/GLib deduplication works on arm64. The snap build stays best-effort: a failure of either architecture does not gate the release

### Changed

- **Dependency refresh** — bumped the gtk4-rs stack to the latest compatible patch releases: `gtk4`/`gtk4-sys`/`gtk4-macros` 0.11.4, `gdk4`/`gdk4-sys` 0.11.4, `gdk4-wayland` 0.11.4, `gsk4`/`gsk4-sys` 0.11.4, `graphene-rs`/`graphene-sys` 0.22.8, `gio`/`gio-sys` 0.22.8, `glib`/`glib-sys` 0.22.8, and `pango` 0.22.8. Also updated `hybrid-array` 0.4.13 and `open` 5.3.6, and dropped the now-unused `pathdiff`

## [0.17.4] - 2026-06-27

### Fixed

- **RDP vault login sends the correct domain** ([#188](https://github.com/totoshko88/RustConn/issues/188)) — when credentials came from the secret vault (Tresor), the domain field was passed as an empty string instead of the configured value, causing NLA/CredSSP to reject `DOMAIN\user` logins with `STATUS_LOGON_FAILURE`. The vault path now falls back to the connection's saved domain, matching the manual-prompt flow
- **Variable password auto-login works on network equipment** ([#194](https://github.com/totoshko88/RustConn/issues/194)) — connections using a Variable password source resolved the secret from the vault correctly but never injected it into the SSH prompt. The password auto-fill relied solely on VTE's `contents-changed` signal, which does not fire reliably for SSH password prompts output in no-echo mode with cursor-positioning escape sequences (common on OLT/router SSH servers). The detection now also subscribes to VTE's `cursor-moved` signal, which fires for all cursor activity including prompts without a trailing newline

### Changed

- **CUPS printer redirection forwards all local queues** ([#192](https://github.com/totoshko88/RustConn/issues/192)) — the embedded IronRDP printer channel previously announced a single dummy "RustConn" printer. It now enumerates all local CUPS queues (or a configured subset via `RdpClientConfig::with_printers`) and registers each as its own redirected printer, routing print jobs back to the correct local queue. The CUPS default printer is announced last so it wins the IronRDP `DEFAULTPRINTER` flag

## [0.17.3] - 2026-06-26

### Fixed

- **Switching GNOME workspaces no longer breaks RDP keyboard input** ([#193](https://github.com/totoshko88/RustConn/issues/193)) — pressing `Super`+digit to switch workspace let the GNOME compositor grab the `Super` chord before its key-release reached the RDP widget, so the embedded session kept treating `Super` (and any modifier caught the same way, e.g. via Alt+Tab) as held down, mangling all further input until a full reconnect. The widget now releases every still-pressed key when it loses keyboard focus, so a compositor-grabbed modifier can no longer stick in the remote session
- **RD Gateway connections work again with FreeRDP 3.x** ([#187](https://github.com/totoshko88/RustConn/issues/187)) — the external FreeRDP launcher emitted the FreeRDP 2.x gateway aliases `/g:`, `/gu:` and `/gp:`, which FreeRDP 3.x removed in favour of the unified `/gateway:` option. The 3.x client rejected `/g:` as an "Unexpected keyword" and exited before connecting (exit status 23). The launcher now builds `/gateway:g:<host>:<port>` (matching the working manual `xfreerdp /gateway:g:HOST /u:NAME /d:DOMAIN` command) and lets FreeRDP reuse the session credentials for the gateway. An explicit gateway user is added (`,u:<user>`) only when it differs from the session user; the broken `/gp:` args-file path is removed (a distinct gateway password is not stored yet and remains future work)
- **Multi-hop (double) jump hosts work in Flatpak** — chaining through two or more bastions failed with `Connection closed by UNKNOWN port 65535` (issue #191 follow-up). The inner hops were reached with a plain `-J`, which does not inherit the identity key or the Flatpak-writable `known_hosts` from the outer command, so the second bastion had neither its key nor host-key verification. RustConn now nests a `ProxyCommand` per hop (terminal SSH, RDP/VNC/SPICE tunnels), passing the identity file and `known_hosts` to every hop. The remote-monitoring probe had the same chain bug in a sharper form — it passed the whole comma-joined chain as a single destination host — and is fixed the same way
- **Multi-hop jump host order corrected outside Flatpak** — the plain `-J` path (native builds, terminal SSH, RDP/VNC/SPICE tunnels, and the monitoring probe) joined the bastions in RustConn's internal target-first order, but OpenSSH `-J` visits hops client-first, so a chain of three or more bastions was traversed in reverse and failed to connect. The hop list is now reversed for every `-J` call to match the corrected nested-`ProxyCommand` direction. Single-bastion connections (the common case) are unaffected

### Added

- **RDP printer redirection** — a new "Printer Redirection" toggle (RDP connection editor → Features) maps your local printer into the remote session, so you can print to it from the Windows side (issue #192). For the embedded IronRDP client, RustConn announces a virtual PostScript printer over the RDPDR channel and forwards each print job to the local CUPS spooler (`lp`) on a detached thread, so a large job never stalls the session's framebuffer or input; for the external `xfreerdp3` client it passes `/printer`. The setting is available in the GUI, via the CLI (`rustconn-cli add/update --printer`), and is imported from Windows `.rdp` files (`redirectprinters:i:1`). The template editor does not expose the toggle yet — configure it per connection

### Changed

- **External RDP now prefers the maintained SDL3 FreeRDP client** — client detection put the deprecated `wlfreerdp` first on Wayland sessions, even though FreeRDP 3.x prints a deprecation warning for it and steers users to the SDL3 client (`sdl-freerdp3`). External launches (RD Gateway, RemoteApp fallback, IronRDP fallback) now prefer `sdl-freerdp3`, which is actively maintained and parses the unified `/gateway:` option correctly. Embedded mode still uses `wlfreerdp` directly where present, so the in-tab Wayland-subsurface experience is unchanged

## [0.17.2] - 2026-06-25

A hardening release following a deep, per-feature codebase audit (15 actionable findings).

### Security

- **Generated passwords now auto-clear from the clipboard** — the password generator's "Copy" left the password on the clipboard indefinitely, unlike the connection "Copy Password" action (30 s auto-clear). It now clears after 30 seconds, but only if the clipboard still holds that password (so it never clobbers something you copied since)
- **SSH password no longer lingers in memory after auto-login** — the injected password (initial connect and in-place reconnect) is now wrapped in `Zeroizing` so the plaintext is wiped immediately after it is handed to VTE, instead of remaining in a `String` until garbage collection
- **SSH tunnel askpass file race fixed** — the temporary `SSH_ASKPASS` helper script used a PID-only filename, so concurrent tunnels shared one path and a second `File::create` could truncate the script while the first `ssh` was still reading it. The filename now includes a per-tunnel UUID

### Fixed

- **SSH jump host uses its own password** — connecting through a jump host fed the *target's* password to the bastion prompt, so a bastion with a different password failed (issue #191). The bastion now authenticates with its own saved password, delivered out-of-band so the terminal still prompts for the target password as before. Covers a single jump host with a saved/vault password; chained hops and prompt-only bastions remain future work
- **RDP dynamic resize no longer requests degenerate resolutions** — the debounced resize path sent the new desktop size without the minimum-size guard that the manual "Fit resolution to window" path already had, so a widget caught mid-layout or collapsed could ask the server for a sub-640×480 desktop. The debounced path now applies the same 640×480 floor
- **RD Gateway password is now sent (same-account gateways)** — when a separate RD Gateway *username* is configured, FreeRDP received `/gu:` but no password, so it fell back to an interactive prompt that hangs in the spawned client. The session password is now passed as the gateway password (`/gp:`) through the same single-use, mode-0600 args file used for RemoteApp credentials, so it never appears on the command line. This covers gateways that authenticate against the same account as the session; a fully independent gateway credential remains future work. When no gateway username is set, FreeRDP keeps reusing the session credentials as before
- **RDP shared-folder names with commas no longer corrupt drive redirection** — FreeRDP's `/drive:<name>,<path>` switch is comma-delimited, so a comma in the share name split the argument and broke the mapped path. Share names are now sanitized (commas replaced with `_`) on both external-client paths
- **Multilingual SSH auto-login on reconnect** — in-place reconnect only matched a handful of English password prompts, so auto-login failed on non-English systems after a reconnect. Prompt detection (initial connect and reconnect) is now a single shared `detect_password_prompt()` covering all supported languages

### Added

- **Simple Sync — bidirectional multi-device sync** — the "Sync everything between your devices" toggle (Settings → Cloud Sync) is now fully wired. Enabling it publishes connections, groups, templates, snippets, and non-secret variables to `full-sync.rcn` in the configured sync directory; on startup and after local changes the app merges remote changes by UUID (`updated_at` wins) and applies creates, updates, and deletions. Groups carry their own modification timestamp, so renames, icon/inheritance edits, and re-parenting propagate (not just create/delete); device-specific group fields (SSH key path, jump-host, agent socket, UI/order state) and Group-Sync bookkeeping are stripped from the shared file and preserved per device on import. Deletions propagate via tombstones (default 30-day retention), and tombstones from other devices are carried forward so a deletion reaches a third device through the shared file. Notes: passwords are never written to the sync file (they stay in your per-device keyring); non-secret variables are merged additively by name (a new variable on one device appears on the others — secret variables, variable edits, and variable deletions do not propagate, since variables have no per-item timestamp); clusters are not synced yet (their model lacks the modification timestamp the merge needs); connections that start asynchronously after a port check are regrouped on the next sync rather than instantly
- **SSH config import follows `Include` directives** — the importer ignored OpenSSH `Include` lines, so hosts defined in included files (common in modern split configs) were missed. `Include` is now expanded — glob patterns supported, relative paths resolved against `~/.ssh`, recursion capped at OpenSSH's 16 levels — and each physical file is parsed only once (no duplicates when a `config.d` glob is both auto-enumerated and included)

### Changed

- **Tab groups persist in workspaces** — assigning a tab to a named group (e.g. "Production") was in-memory only and lost on restart. A workspace now stores each session's group and restores it when reopened. (Port-checked connections that start asynchronously are not regrouped on restore yet.)
- **Jump-host / SSH-args resolution deduplicated** (internal) — initial connect and in-place reconnect each carried ~150 near-identical lines building the identity file, jump-host `ProxyCommand`/`-J` chain (with Flatpak known_hosts and first-hop PKCS#11), and waypipe detection, so a fix to one path could silently miss the other. Both now call a single `build_ssh_command_args()`; reconnect also gains the waypipe-fallback logging it previously lacked
- **Dynamic-connection IDs are now stable across Rust versions** — IDs were derived from `std::collections::hash_map::DefaultHasher`, whose output is not guaranteed stable between toolchain releases, so a compiler upgrade could silently change every dynamic connection's UUID. They now use spec-defined UUID v5 (SHA-1, group as namespace, name+host+protocol as key). Upgrading to 0.17.2 regenerates dynamic-connection IDs once; they stay stable afterwards. Removed the now-unused `DynamicConnectionId` type

### Dependencies

- **Updated**: uuid 1.23.3→1.23.4, wasm-bindgen/js-sys/web-sys ecosystem (wasm-bindgen 0.2.125→0.2.126, js-sys 0.3.102→0.3.103, web-sys 0.3.102→0.3.103, wasm-bindgen-futures 0.4.75→0.4.76)

## [0.17.1] - 2026-06-24

### Added

- **WinBox connection preset** ([#190](https://github.com/totoshko88/RustConn/issues/190)) — added a ready-made template (Remote Desktop category) for MikroTik's WinBox GUI: `WinBox ${host} ${user} ${password}`
- **Native PKCS#11 / YubiKey SSH authentication** ([#189](https://github.com/totoshko88/RustConn/issues/189)) — a new "PKCS#11 Provider" field in the SSH connection editor (Session group) lets you point at a hardware-token library (e.g. `/usr/lib64/libykcs11.so.2`); it maps to `-o PKCS11Provider=<path>`, so YubiKey/PIV/smart-card keys are offered without the SSH-agent workaround. The directive is also imported from `~/.ssh/config` (`PKCS11Provider`).
  - **Works through SSH tunnels** — because `-o PKCS11Provider` is *not* inherited by `ProxyJump` child connections, the provider of a jump-host connection is now injected explicitly into the first hop's `ProxyCommand` (terminal SSH and RDP/VNC/SPICE tunnels). Enable PKCS#11 on the bastion connection to authenticate the jump itself with the token.
  - PKCS#11 does not force `IdentitiesOnly`, so the token's keys are always offered. The PIN/touch prompt appears in the session terminal. Note: with a jump host the token may prompt once per hop (separate SSH processes).

### Fixed

- **Flatpak: GUI / non-`script` Generic commands now launch** ([#190](https://github.com/totoshko88/RustConn/issues/190)) — a Generic Zero Trust command failed with `Portal call failed: Failed to start command: "script"` whenever the host had no reachable `script` (util-linux) binary — atomic distros, or `script` outside the sandbox PATH the `flatpak-spawn` portal resolves against. GUI tools such as WinBox also do not need the PTY that `script` allocates. The host command is now run through a login shell (`sh -lc`, so the host PATH resolves binaries) that probes for `script` and falls back to a plain `sh -c` when it is absent

### Changed

- **Removed dead VNC FFI stub** (internal) — deleted `rustconn-core/src/ffi/` (`VncDisplay`, `FfiDisplay`, `ConnectionState`, `FfiError`). Despite documenting itself as a safe wrapper around the `gtk-vnc` C library, it had no `gtk-vnc` dependency, opened no connection (`open_host` only mutated in-memory state), and its signal callbacks were never invoked — so the `VncSessionWidget` "native" path it backed could never connect. Embedded VNC already runs through the native `vnc-rs` client (`EmbeddedVncWidget`) with an external-viewer fallback; the stub and its now-unreachable `VncSessionWidget` methods (`connect`, `provide_credentials`, `set_scaling`, `display`, `connect_auth_required`) were removed (YAGNI). Synced `docs/ARCHITECTURE.md` (dropped the stale `broadcast.rs` entry) and bumped the doc version headers.
- **Removed archived-spec traceability comments** (internal) — stripped dangling `// Requirement X.Y` / `# Requirements Coverage` doc-comment references (~225 lines across 30 source files) that pointed at specs now under `.kiro/specs/_archive/`. Descriptive text was preserved (the requirement prefix removed), pure traceability sections deleted.
- **Removed unused `performance` scaffolding** (internal) — deleted dead utilities from `rustconn-core` that had no production callers: object pool, compact string, batch processor, lazy init, shrinkable vec, virtual scroller, the performance-metrics timer, and the `MemoryOptimizer`/`MemoryTracker` machinery (~1900 LOC). The only live functionality — the connection string interner and the search debouncer — is retained; the global interner is now exposed directly as `performance::interner()`. Also synced `docs/ARCHITECTURE.md` (connection-dialog `dialog/` split, `builders.rs`/`web.rs`, `WebProtocol`, `performance/`, `tracing/`).

### Dependencies

- **Updated**: chacha20 0.10.0→0.10.1

## [0.17.0] - 2026-06-23

A hardening release: targeted security, performance, and tech-debt fixes following a full codebase audit. No major new functionality.

### Security

- **kubectl / Zero Trust Generic command injection** — Kubernetes and Zero Trust Generic sessions now spawn their command argv directly instead of through `sh -c`, so shell metacharacters in (possibly imported, untrusted) configs can no longer be interpreted as commands
- **Legacy XOR credential format removed** — the obsolete XOR fallback for credentials without the `RCSC` header has been removed; it provided no real protection and its migration window (since v0.12) has long passed. Only AES-256-GCM credentials are read
- **Credential threat model documented** — `SECURITY.md` now explains the machine-key encryption model (obfuscation at rest, not protection against same-user read) and recommends keyring/vault backends for real secrets
- **Passbolt passphrase exposure documented** — added a Known Issue noting that `go-passbolt-cli` accepts the passphrase only as a command-line argument, with mitigations and upstream status
- **Transient secret hardening** — Bitwarden and KeePassXC serialized item buffers are now wrapped in `Zeroizing` so plaintext is wiped on drop

### Added

- **Workspace split layout restore** — opening a saved workspace now restores its split-pane layout, not just the connections

### Fixed

- **RDPDR dead notify computation** — removed an unused `FILE_NOTIFY_INFORMATION` computation on the directory-watch path (the builder is kept ready for when IronRDP exposes the response type)

### Improved

- **Terminal highlight rendering** — highlight colours are pre-parsed once at rule-compile time instead of on every repaint; match values are now `Copy` and allocation-free on the hot path, and column offsets are computed as a single-scan delta
- **Connection sorting** — group sort caches lowercase keys (`sort_by_cached_key`) instead of recomputing them on every comparison
- **Autotype dialog** — the embedded-RDP "type text" dialog is now an `adw::Dialog` (ToolbarView + HeaderBar) instead of a raw window, so it stays attached on Wayland
- **Touch targets** — header-bar icon buttons now meet the GNOME HIG 44×44px minimum tap target
- **Lint hygiene** — migrated `#[allow]` overrides to `#[expect]` (warns when a lint stops firing) and added the `clone_on_ref_ptr` restriction lint
- **IronRDP panic guard re-evaluated** — confirmed the `connect_finalize` catch_unwind wrapper is still needed (0.16 remains the latest release; upstream panic reports stay open) and refreshed the inline note

### Changed

- **Build** — narrowed the workspace `tokio` feature set from `full` to the exact features used, trimming compile time

### Dependencies

- **Updated**: rustls 0.23.40→0.23.41

## [0.16.13] - 2026-06-22

### Added

- **RDP RTT (latency) display** — embedded IronRDP sessions now show round-trip time in the toolbar status label when the server reports network characteristics via Auto-Detect PDU (MS-RDPBCGR 2.2.14.1.5). The Echo virtual channel (MS-RDPEECO) is also registered, enabling the server to measure RTT via echo request/response probes

### Improved

- **Dynamic RDP resolution change now works in embedded mode** — the Display Control Virtual Channel (MS-RDPEDISP) is now registered on the dynamic-channel client alongside the new Echo channel. Previously it was never registered, so every window resize fell back to a full reconnect (the 0.16.3 "Fit resolution to window" feature always took the reconnect path). On servers that advertise Display Control capabilities, the desktop is now re-sized seamlessly without dropping the session

### Dependencies

- **Updated**: ironrdp 0.15→0.16 (ironrdp-session 0.9→0.10, ironrdp-dvc 0.6→0.7, ironrdp-displaycontrol 0.6→0.7, ironrdp-server 0.11→0.12, ironrdp-rdpsnd 0.8→0.8.1, ironrdp-echo 0.2→0.3)
- **Added**: ironrdp-echo 0.3 (Echo DVC for RTT measurement)
- **Updated**: quote 1.0.45→1.0.46, time 0.3.49→0.3.51, time-macros 0.2.29→0.2.30, zlib-rs 0.6.3→0.6.4

## [0.16.12] - 2026-06-21

### Added

- **Workspace profiles** — save currently open sessions as a named workspace and restore them all at once. Access via *Tools → Workspaces...* menu. Features:
  - Save current set of active connections (with tab order) under a custom name
  - Open a saved workspace to reconnect all its entries in one click
  - Rename workspace profiles inline from the manager dialog
  - Delete workspace profiles that are no longer needed
  - Workspace profiles persist across restarts (`~/.config/rustconn/workspace_profiles.toml`)
  - Workspace entries auto-clean when a referenced connection is deleted

- **Port knocking** — built-in pre-connect port knock sequence, no external `knock` CLI needed. Configured per-connection (`Connection.knock_sequence`):
  - TCP and UDP knocks with configurable inter-knock delay and post-knock settle time
  - Parse from human-readable format: `"7000 8000/tcp 9000/udp"`
  - Inline validation in the connection editor — invalid format highlighted immediately
  - Works inside Flatpak sandbox (pure Rust, no shell command)
  - Each knock logged via tracing for diagnostics

- **fwknop Single Packet Authorization (SPA)** — built-in fwknop-compatible packet builder (AES-256-CBC + HMAC-SHA256, OpenSSL EVP_BytesToKey wire format). Sends an encrypted UDP packet to open firewall rules before connecting. Integrated into the pre-connect chain (knock → SPA → port check → connect). No external `fwknop` CLI needed — pure Rust implementation using existing `aes`/`cbc`/`ring` crates. Full GUI in the Advanced tab of the connection editor: Rijndael key, HMAC key (password entries with peek), access spec, destination port, and allow-IP mode (Source IP / Resolve Public / Explicit). Configure per-connection via `spa_config`

### Dependencies

- **Updated**: log 0.4.32→0.4.33
- **Added** (direct, previously transitive): aes 0.9, cbc 0.2, md-5 0.11 — for fwknop SPA packet builder

## [0.16.11] - 2026-06-20

### Fixed

- **Connection wizard's "Zero Trust" card showed only a custom-command field instead of the provider list** — the wizard reuses a single connection page, and selecting the "Custom Command" card runs a mode that hides the Zero Trust provider dropdown and retitles the group; `configure_for_protocol(ZeroTrust)` never restored them, so the "Zero Trust" card degraded to a bare command field. On top of that the dropdown defaulted to "Custom Command" (index 0), so even a fresh selection opened on a command field. The "Zero Trust" card now always restores the full provider picker (AWS SSM, GCP IAP, Azure Bastion/SSH, Cloudflare Access, Teleport, Tailscale SSH, Boundary, Hoop.dev) and defaults to AWS Session Manager, mirroring the Advanced editor; the "Custom Command" card still resets to the Generic command mode
- **RDP Mouse Jiggler never actually ran in embedded (IronRDP) mode** ([#185](https://github.com/totoshko88/RustConn/issues/185)) — the 0.16.10 keep-alive fix added the Scroll Lock keystroke, but the jiggler timer was only ever armed from `set_state`, and the embedded connection paths set the `Connected` state directly (via the event callback) without routing through `set_state`. As a result the timer was never started in embedded mode, so neither the mouse-move nor the Scroll Lock keep-alive was sent — the very mode the feature is documented to support. The jiggler is now armed (and stopped on disconnect/error) directly from the embedded `Connected`/`Disconnected`/`Error` event handlers, via a shared `JigglerHandles` struct also used by `set_state`
- **External RDP (sdl-freerdp) ignored its `sdl-freerdp.json` configuration in the Flatpak build** ([#183](https://github.com/totoshko88/RustConn/issues/183)) — the bundled FreeRDP was compiled with no JSON backend (the GNOME runtime ships neither cJSON, json-c, nor jansson), so WinPR fell back to `json-stub.c` and `WINPR_JSON_ParseFromFile()` always returned null. As a result the SDL client silently discarded `$XDG_CONFIG_HOME/freerdp/sdl-freerdp.json`, and the built-in SDL hotkeys (Right Shift + D disconnect, etc.) could not be disabled or remapped no matter where the config file was placed. A static **cJSON** module is now built ahead of FreeRDP in all Flatpak manifests, enabling `WITH_WINPR_JSON=ON`, so user prefs such as `{ "SDL_KeyModMask": ["KMOD_NONE"] }` take effect

- **RDP connections created through the New Connection wizard never stored the password** ([#188](https://github.com/totoshko88/RustConn/issues/188)) — typing a password on the wizard's authentication step and pressing *Save & Connect* (without opening *Advanced…*) silently dropped the secret: `build_connection()` collected `partial.password` but never set `password_source` or persisted the value, so the connection was created with `PasswordSource::None`. The result was an immediate "NLA authentication failed" with no usable credential, fixable only by manually editing the connection and changing Password Source. The wizard now marks a typed password as `Vault` (skipping key/agent/security-key auth) and persists it through `save_password_to_vault`, mirroring the full editor. The *Advanced…* hand-off also carries the typed password into the full dialog instead of discarding it
- **RDP through an RD Gateway rendered a broken/black session in embedded mode** ([#187](https://github.com/totoshko88/RustConn/issues/187)) — when a connection used an RD Gateway, IronRDP correctly bailed out (it has no MS-TSGU support), but `connect()` then fell through to embedded `wlfreerdp`. The embedded launch path (`thread.rs`) never emits the `/g:` / `/gu:` gateway arguments, so it connected straight to the gateway host on port 3389 with no tunnelling. The gateway answers on the socket — hence the immediate "FreeRDP connected" — but no real RDP session is established, leaving a black screen. Gateway connections now skip embedded `wlfreerdp` and go directly to the external client (`launcher::add_connection_args`), which wires up gateway routing correctly, mirroring the existing skip for RemoteApp

### Changed

- **Advanced connection editor has a distinct window title** — the full multi-tab editor (opened via *New Connection (Advanced)…*, Shift+Ctrl+N, or the wizard's *Advanced…* button) was titled "New Connection", identical to the simplified protocol-picker wizard, making the two indistinguishable. It is now titled "New Connection (Advanced)" through every entry point — including the wizard's *Advanced…* hand-off, which previously reset the title back to "New Connection"; edit mode still overrides this with "Edit Connection"

### Internal

- **Connection wizard no longer distinguishes the "Custom Command" card by its display string** — the protocol grid's *Custom Command* and *Zero Trust* cards both map to `ProtocolType::ZeroTrust`, and the custom-command mode was selected via `label == "Custom Command"`. That comparison would silently break the moment the label were wrapped in `i18n()`. Replaced with an explicit `is_custom_command` field on `ProtocolDef`

### Dependencies

- **Updated**: arrayvec 0.7.6→0.7.7
- **cJSON (Flatpak)** 1.7.18→1.7.19 — bundled JSON backend for FreeRDP/WinPR updated to the latest upstream release


## [0.16.10] - 2026-06-19

### Fixed

- **RDP Mouse Jiggler did not prevent the remote desktop from locking** ([#185](https://github.com/totoshko88/RustConn/issues/185)) — in Embedded (IronRDP) mode the jiggler sent only a tiny mouse-move every interval. That is enough to keep the RDP *session* from idle-disconnecting, but Windows does **not** refresh its workstation lock / screensaver timer (`GetLastInputInfo`) on RDP-injected pointer motion alone, so unattended desktops still locked after the configured inactivity limit (e.g. 10 minutes) regardless of a 10-second jiggle interval. Each jiggle tick now also taps **Scroll Lock twice** (toggle on, then off) — a layout-independent keyboard event that reliably resets the lock timer, produces no character, triggers no action, and leaves the Scroll Lock state unchanged. The mouse-move is kept for session-level keep-alive

### Known limitations

- **Mouse Jiggler only works in Embedded (IronRDP) mode** — the External RDP client runs as a separate FreeRDP process with no input channel back to RustConn, so neither the mouse-move nor the keyboard keep-alive can be injected into it. The setting is silently inactive in External mode; use Embedded mode if you need the jiggler. (Earlier docs incorrectly claimed external-mode support)

### Dependencies

- **Updated**: cc 1.2.64→1.2.65
- **FreeRDP (Flatpak)** 3.27.0→3.27.1 — bundled RDP backend updated to the latest upstream maintenance release

## [0.16.9] - 2026-06-19

### Removed

- **Dead ad-hoc broadcast controller** — the `BroadcastController` (`rustconn/src/broadcast.rs`) and its `TerminalNotebook` wrappers (`toggle_broadcast`, `is_broadcast_active`, `toggle_broadcast_terminal`, `is_broadcast_terminal_selected`, `broadcast_text`, `broadcast_controller`) implemented an ad-hoc "send keystrokes to several selected terminals" mode that was never wired to any action, menu, or shortcut. It was superseded by the split-view Broadcast toggle in the header bar (`win.toggle-broadcast`). The fields carried false `#[allow(dead_code, reason = "Public API — wired by app layer")]` annotations; the app layer never wired them. Deleted in full (YAGNI)
- **Unused virtual-scroll tuning API** — `VirtualScrollConfig` (`rustconn-core/src/connection/virtual_scroll.rs`) was exercised only by its own unit tests and never wired into the sidebar. Removed; the still-used `SelectionState` from the same module is kept
- **Unused protocol-layout builder setters** — `ProtocolLayoutBuilder::{max_size, tightening_threshold, spacing, margin}` were never called outside tests (all protocol panels use `new().build()` with defaults). Removed along with the module-wide `#![allow(dead_code)]`

### Changed

- **Removed stale `dead_code` overrides** — dropped the module-wide `#![allow(dead_code)]` from `dialogs/connection/ssh.rs`; all its functions (`create_ssh_options`, `create_port_forwarding_group`, `add_port_forward_row_to_list`) are in active use, so the override was masking nothing

### Documentation

- **Corrected stale doc comments** — `cmd_connect`'s `build_connection_command` no longer claims special `ZeroTrust` handling (it now goes through `ProtocolRegistry` like every other protocol; only `Sftp` is special-cased), and `GroupSyncExport::from_group_tree` no longer references unfinished "tasks 2.9–2.11" (the constructor is implemented and used by `SyncManager`)

### Fixed

- **RDP Quick Actions / Shell launchers typed wrong characters on non-QWERTY remote layouts** ([#184](https://github.com/totoshko88/RustConn/issues/184)) — the Run-dialog commands (Computer Management, Device Manager, Disk Management, Event Viewer, Registry Editor, Resource Monitor, Server Manager, Services) and the PowerShell/CMD shell launchers were typed character-by-character using hard-coded **US-QWERTY scancodes**. On a remote Windows session with a different keyboard layout (e.g. French AZERTY) the same physical scancodes produced different characters, so `compmgmt.msc` arrived as `co,p,g,t:,sc`. The command text is now sent via Unicode keyboard events (`TS_UNICODE_KEYBOARD_EVENT`, the same layout-independent autotype path already used for snippets), while only the layout-safe Win+R and Enter keys remain scancodes. Removes the entire `char_to_scancode` US-layout table

### Dependencies

- **Updated**: bitvec 1.0.1→1.1.1


## [0.16.8] - 2026-06-18

### Fixed

- **KeePassXC not detected in Flatpak ("keepassxc-cli not found")** ([#182](https://github.com/totoshko88/RustConn/issues/182)) — KeePassXC detection and all KDBX operations only searched the *sandbox* `PATH` (`which keepassxc-cli` plus host paths like `/usr/bin/keepassxc-cli`), which never resolve inside Flatpak, so the host's KeePassXC was never found even though the documentation promised automatic detection via `flatpak-spawn --host`. Unlike the bundled CLI tools (`bw`, `op`, `kubectl`), KeePassXC is the user's host GUI app and cannot be shipped as a Flatpak Component. Detection now resolves the host binary via `flatpak-spawn --host sh -lc 'command -v keepassxc-cli'`, and every `keepassxc-cli` invocation (show/add/ls/rm/verify/version) is routed through `flatpak-spawn --host` (which forwards stdin/stdout/stderr to the host process by default, so piped database/entry passwords reach it). Requires the host-shell permission (`--talk-name=org.freedesktop.Flatpak`, already in the manifest) and filesystem access to the `.kdbx` file
- **KDBX status text overflowed the row** ([#182](https://github.com/totoshko88/RustConn/issues/182)) — long status strings (e.g. "Invalid database password or key file") spilled past the "Connection Status" row next to the Check button. The status label now ellipsizes at a capped width and exposes the full text as a tooltip on hover
- **Settings → Interface: theme segmented control did not reflect the saved scheme** (libadwaita ≥ 1.7 builds) — on Flatpak/Fedora packages built with the `adw-1-7`/`adw-1-8` feature, the `AdwToggleGroup` for the color scheme was added to its row but the value returned to the loader was an empty placeholder box, so `load_ui_settings` had nothing to sync. Reopening Settings always showed "System" highlighted even when Dark or Light was saved and applied (the theme itself was correct — only the segmented control was out of step). The toggle group is now held inside its wrapper box (the same reparent-safe pattern used by the cursor shape/blink toggles) and the loader sets the active segment from the saved scheme

### Documentation

- **External FreeRDP keyboard shortcuts (Right Shift hotkeys)** ([#183](https://github.com/totoshko88/RustConn/issues/183)) — the User Guide now documents the built-in shortcuts of the FreeRDP SDL client used in External RDP mode (Right Shift + Enter/R/G/D/M for fullscreen/resize/grab/disconnect/minimize), why `Right Shift + D` drops the session, and that the "release input" key is `Right Shift + G` (not Win+Esc). It explains where to put `sdl-freerdp.json` for the Flatpak build — `~/.var/app/io.github.totoshko88.RustConn/config/freerdp/sdl-freerdp.json` (the bundled FreeRDP runs inside RustConn's own sandbox, so the `com.freerdp.FreeRDP` path and the host `/etc/FreeRDP/` do not apply), and gives ready-to-use JSON to either disable all hotkeys (`"SDL_KeyModMask": ["KMOD_NONE"]`) or remap only the disconnect key while keeping the grab toggle

### Dependencies

- **Updated**: ironrdp-graphics 0.8.0→0.8.1, ironrdp-rdpsnd 0.8.0→0.8.1, bytes 1.11.1→1.12.0, crypto-bigint 0.7.3→0.7.4, getrandom 0.4.2→0.4.3, syn 2.0.117→2.0.118

## [0.16.7] - 2026-06-16

### Fixed

- **Sidebar context menu rows invisible under KDE / Breeze GTK theme** ([#181](https://github.com/totoshko88/RustConn/issues/181)) — the connection/group right-click menu is a custom `Popover` of flat buttons whose text colour was inherited from the active GTK theme. Under a third-party theme (notably Breeze-GTK on KDE Plasma) the flat-button text was painted in a colour that clashed with the libadwaita popover background, so the rows rendered as white-on-white and the menu looked empty. The menu now pins its background and item text to the libadwaita popover palette (`@popover_bg_color` / `@popover_fg_color`) at application stylesheet priority, so the rows stay legible regardless of the system GTK theme, and forces normal font weight in case the theme bolds menu items
- **Smart-folder right-click menus had the same invisible-rows defect** — the context menus for a smart folder (Edit / Delete) and for a connection shown inside a smart folder (Connect, Edit, Copy Username/Password, Wake On LAN, Check if Online, Delete) are built from the same custom flat-button popover pattern and were also unreadable under Breeze-GTK on KDE. They now reuse the same popover palette styling; the destructive "Delete" entry keeps its red accent

### Dependencies (Flatpak)

- **FreeRDP** 3.26.0 → 3.27.0 — bundled RDP backend updated to the latest upstream maintenance release
- **fast_float** 8.0.2 → 8.2.10 — bundled FreeRDP build dependency updated within the 8.x series

## [0.16.6] - 2026-06-15

### Fixed

- **Activity Monitor did nothing on most connections; per-tab toggle was stuck on Off** ([#180](https://github.com/totoshko88/RustConn/issues/180)) — activity/silence monitoring was only wired on one connection path (`start_connection_with_split`) and only for synchronous connects. Connecting via the sidebar's "Connect" action, the command palette, or a cluster never set it up at all, and even the split path missed the common asynchronous port-check route (where the session is created later in a background callback). On top of that, a session whose effective mode was `Off` was never registered with the coordinator, so right-clicking the tab and clicking "Monitor: Off" did nothing — the menu could not turn monitoring on. Activity monitoring is now wired from a single choke point (the notebook's session-creation hook), so it works for every terminal protocol (SSH, Telnet, serial, Kubernetes, Mosh, Zero Trust), every connect entry point, and both synchronous and port-checked connections. Sessions are always registered (even when Off) so the per-tab "Monitor" menu can cycle Off → Activity → Silence on a live session without reconnecting. In-place reconnect re-arms monitoring as well
- **Silence notification could report the wrong connection name** — the coordinator's silence callback captured a single connection's name when it was wired, so with several monitored tabs open every silence toast/notification showed the most recently wired name. The name is now resolved per session when the notification fires

### Notes

- Changing a connection's Activity Monitor mode in its settings still applies to *new* sessions; for an already-open tab, use the right-click tab menu to change the mode live, or reconnect

### Dependencies

- **Updated**: h2 0.4.14→0.4.15

## [0.16.5] - 2026-06-14

### Fixed

- **Keyboard shortcuts did nothing under non-Latin keyboard layouts** — accelerators are registered with Latin keyvals (`<Control>n`), but GTK matches them against the keyval produced by the *active* layout, so under a Cyrillic/Greek/etc. layout pressing Ctrl+N yielded `Cyrillic_en` and the shortcut silently did nothing (the in-app recorder already normalized to Latin, but runtime matching did not). A capture-phase key controller now maps the hardware keycode back to its Latin keyval and activates the matching action when it is currently bound. User overrides and keyboard-passthrough mode are honored automatically (it queries the live accelerators), and Latin layouts are untouched
- **RDP external client closed immediately with no explanation** ([#177](https://github.com/totoshko88/RustConn/issues/177) follow-up) — when the embedded IronRDP first-frame watchdog (0.16.3) fell back to the external FreeRDP client — or when external mode was selected directly — the client's `stderr` was redirected to `/dev/null`, so a failed connection (authentication failure, rejected certificate, unsupported codec, wrong display backend) just made a window flash and close with no diagnostic anywhere. On top of that the widget was left in a phantom `Connected` state because the spawned process was never checked for an early exit. FreeRDP's `stderr` is now captured and forwarded to the application log, and a short startup watchdog (polls for ~3 s) detects an immediate exit and surfaces it as a clear, localized error instead of a silent blank tab
- **macOS: wrong default secret backend on a fresh install** — `SecretSettings` defaulted to `libsecret` on every platform, but libsecret does not exist on macOS, so a new install fell back through an unavailable backend instead of using the system Keychain. The default is now platform-aware (`MacOsKeychain` on macOS, `libsecret` elsewhere), and the Settings → Secrets backend selector shows and persists "macOS Keychain" for the system-keyring slot on macOS instead of the meaningless "libsecret" label
- **macOS: misleading "not installed" error for any terminal spawn failure** — the native-PTY launcher reported every failure (PTY allocation, fd duplication, controlling-terminal setup) as `Command not found` / `'…' is not installed`. Only a genuine missing-executable error (NotFound) now uses that wording; other failures surface the actual error text in the toast so the cause is diagnosable
- **macOS: `gtk-application-prefer-dark-theme` workaround fought the system theme** — the Linux/KDE/XFCE xsettings workaround that force-clears this property also ran on macOS, where the property mirrors the system `NSAppearance`. Clearing it interfered with "Follow system" dark mode and produced repeated misleading log lines. The workaround is now gated to non-macOS platforms

### Improved

- **RDP connection diagnostics** — the external FreeRDP launcher now logs the selected binary (`wlfreerdp3`/`sdl-freerdp3`/`xfreerdp3`), the session type (Wayland vs X11), and the full argument vector (the password is never on the command line, so this is safe) at debug level. The embedded IronRDP path now logs how long the first displayable frame took to arrive, making it possible to tell a genuine GFX/H.264-only server (no frame at all) apart from a slow first paint on a high-latency link. Run with `RUST_LOG=debug` to collect this when reporting RDP issues
- **Quieter logs: CSS theme-parser warning flood suppressed** — GTK4's CSS parser emits hundreds of harmless `Theme parser warning … Expected ';'` lines while reading the libadwaita ≥1.9 stylesheet (most visible on macOS/Homebrew). A GLib log writer now drops only those specific messages and forwards every other GTK/GLib message unchanged, so real warnings are no longer buried
- **macOS Keychain: secret bytes zeroized on decode failure** — if a stored Keychain value failed UTF-8 decoding, the raw bytes (potentially password material) were dropped without being wiped; they are now zeroized before the error is returned
- **macOS: native key symbols on the welcome screen** — the welcome screen's Keyboard Shortcuts column now renders combos with macOS symbols (`⌃ ⇧ ⌥ ⌫`, e.g. `⌃⇧T`) instead of the `Ctrl+Shift+T` text. The bindings themselves are unchanged (RustConn still uses Control on macOS, shown as `⌃`); this is presentation only. The Shortcuts dialog already renders natively via `AdwShortcutsItem`

### Translations

- **Italian** — updated translations contributed by [@albanobattistella](https://github.com/albanobattistella) ([#179](https://github.com/totoshko88/RustConn/pull/179))
- New string for the external-client early-exit error, translated across all 16 languages
- New strings for the macOS terminal spawn-failure messages (`Failed to start '{}'`, `Failed to start '{}': {}`)

### Dependencies

- **Updated**: yuv 0.8.15→0.8.16

## [0.16.4] - 2026-06-14

### Fixed

- **MobaXterm import/export lost nested folder structure** ([#178](https://github.com/totoshko88/RustConn/issues/178)) — `.mxtsessions` `SubRep` paths such as `Production\Web` were imported as a single flat group literally named `Production\Web` instead of a `Production` → `Web` tree, and export only emitted sections for folders that held direct sessions while building deeper paths in an order-dependent, sometimes-incorrect way. Import now splits each `SubRep` on the backslash separator and rebuilds the full folder tree (creating and reusing intermediate groups with correct `parent_id`), and export walks the parent chain to produce full paths, emits one section per group sorted so parents precede children, and includes intermediate folders even when they contain no direct sessions — so the hierarchy round-trips correctly between RustConn and MobaXterm
- **SecureCRT export mangled folders nested 3+ levels deep** — the directory-path builder updated paths in a single order-dependent pass, so a connection in `A/B/C` could be written to a truncated path like `B/C` when groups were processed child-first. Paths are now built by walking the parent chain (correct at any depth), and a directory is created for every group so empty intermediate folders are preserved
- **Asbru-CM export dropped parent links on deep hierarchies** — the group-to-UUID map was filled incrementally while emitting entries, so a child group serialized before its parent lost its `parent:` reference. The full map is now built up front, so nesting survives regardless of group order

### Dependencies

- **Updated**: time 0.3.47→0.3.49, time-core 0.1.8→0.1.9, time-macros 0.2.27→0.2.29

## [0.16.3] - 2026-06-13

### Added

- **RDP "Fit resolution to window" toolbar button** — a new left-aligned button (monitor icon) in the embedded RDP toolbar re-requests the session resolution to match the current window size, the same effect as resizing the window. This covers the edge case where the window was resized between connection start and the session becoming active (so the desktop did not fill the whole window) on servers without dynamic resolution. Uses the Display Control Channel (MS-RDPEDISP) for a seamless change when available, otherwise reconnects
- **Error details in Connection History** — failed connection entries now show an info button that opens a dialog with the captured error message. The connection toast is transient and disappears quickly; the stored error can now be re-read later for debugging. For embedded RDP sessions the specific, user-friendly error (auth failure, TLS, timeout, …) is persisted instead of a generic placeholder

### Fixed

- **Failed connections were missing from the history** — when a pre-connection port check timed out (host unreachable), the attempt only updated the sidebar status and showed a toast; no history entry was ever created, so the failure was lost. Failed port checks are now recorded in connection history with the reachability error, across all protocols (SSH, RDP, VNC, SPICE, Telnet, MOSH, SFTP)
- **RDP connects but the desktop never appears** ([#177](https://github.com/totoshko88/RustConn/issues/177)) — servers that only offer the GFX graphics pipeline with H.264/AVC444 (which the embedded IronRDP client cannot decode yet) completed the handshake successfully but never produced a displayable frame, leaving a blank session. A first-frame watchdog now detects this (connected but no frame within 8 s) and automatically falls back to the external FreeRDP client, which supports those codecs

### Dependencies

- **Updated**: openssl 0.10.80→0.10.81, openssl-sys 0.9.116→0.9.117, zeroize 1.8.2→1.9.0, zeroize_derive 1.4.3→1.5.0, js-sys 0.3.100→0.3.102, web-sys 0.3.100→0.3.102, wasm-bindgen 0.2.123→0.2.125 (and related macro/futures crates), wasip2 1.0.3→1.0.4


## [0.16.2] - 2026-06-13

### Improved

- **GNOME HIG follow-up audit**:
  - critical errors are no longer transient toasts: vault password save failure, secret load failure in the variables dialog, and smart folder deletion failure now show a blocking alert dialog (a toast could disappear before the user notices a lost credential)
  - error alert dialogs no longer style the OK button as destructive (red is reserved for irreversible actions)
  - sidebar drag-and-drop indicators use the libadwaita accent color (`@accent_bg_color`) instead of a hardcoded orange, following the user's accent color and high-contrast mode
- **Dead code cleanup**: removed the unused `ContainerState` machine and `is_split`/`is_welcome` from `TabPageContainer` (leftovers of an abandoned "Phase 2" refactoring), the never-called `load_variable_from_vault` wrapper, and stale `#[allow(dead_code)]` attributes that compiler verification (`#[expect]`) proved unnecessary

### Fixed

- **F10 still opened the application menu in keyboard passthrough mode** — the F10 primary-menu binding is GTK-internal (driven by the header-bar menu button's `primary` property), not an application accelerator, so disabling shortcuts via Ctrl+Shift+Backspace did not affect it. The `primary` flag is now dropped while passthrough is active, so F10 reaches the remote session (e.g. Midnight Commander) and is restored when passthrough is turned off
- **Ctrl+T (SSH Tunnel Manager) ignored keyboard passthrough and was not customizable** — the accelerator was registered directly instead of through the central keybinding registry; it is now a regular keybinding (visible in Settings → Keybindings, disabled in passthrough mode like all others)

### Dependencies

- **Updated**: block-buffer 0.12.0→0.12.1, cc 1.2.63→1.2.64, memchr 2.8.1→2.8.2, smallvec 1.15.1→1.15.2, yuv 0.8.14→0.8.15

## [0.16.1] - 2026-06-12

### Improved

- **Settings dialog GNOME HIG pass** — fixes from a HIG audit of the preferences dialog:
  - secret fields (Bitwarden master password / client secret, 1Password token, Passbolt passphrase, KeePass database password) migrated from `GtkPasswordEntry`-in-a-row to `adw::PasswordEntryRow` (built-in peek icon, caps-lock warning, focus on row click)
  - highlight-rules editor rebuilt: one expandable row per rule with proper `EntryRow`s, enable switch and delete button — replaces the raw entry grid inside a nested scrolled area (scrolling-inside-scrolling anti-pattern); rule name/pattern are reflected live in the row title/subtitle
  - rows with suffix controls (language, startup action, keybinding rows, SSH key rows, cloud-sync import) are now activatable via the row itself — keyboard and touch users no longer have to hit the small suffix widget; "Add SSH Key" row previously did nothing when clicked
  - "Reset All to Defaults" for keyboard shortcuts now asks for confirmation before wiping every customized shortcut
  - "Restart to apply" after a settings restore is a dialog with a "Quit now" option instead of a transient toast (persistent state must not auto-dismiss)
  - backup/restore failures caused by an inaccessible configuration directory are now shown to the user instead of only being logged
  - removed extra `suggested-action` styling from inline Add/Import buttons (one suggested action per dialog); error dialog headings unified to sentence case ("Backup failed", "Restore failed"); the "ZIP archives" file-filter name is now translatable

### Fixed

- **Settings dialog took 5+ seconds to appear with Bitwarden backend (flatpak)** — the Secrets tab's keyring auto-unlock chain (`bw status` / `bw unlock` / `bw sync`, each a 1–3 s Node.js cold start in the sandbox) was scheduled with `glib::spawn_future`, which runs on the GTK **main context**, not a worker thread — the dialog was mapped but its first frame waited until the whole chain finished. All blocking secret-CLI and keyring calls in the settings dialog (and the SSH-agent probe in the connection dialog) now run via `gio::spawn_blocking` on a real worker thread; measured first-frame time in flatpak dropped from ~5.3 s to the normal ~150 ms
- **UI froze for seconds right after startup with Bitwarden backend (flatpak)** — `resolve_bw_cmd()` probes CLI candidates by running `bw --version`, a Node.js cold start that takes 1–3 s inside the flatpak sandbox, and it ran on the GTK main thread during the startup idle handler. Clicking anything (e.g. opening Settings) during that window appeared to hang. The probe now runs on the background auto-unlock thread; the result is cached process-wide as before

- **Sidebar context menu dismissed on deeply nested rows (KDE Plasma)** ([#157](https://github.com/totoshko88/RustConn/issues/157)) — follow-up to the 0.16.0 fix: on rows at nesting level 3+ the menu opened via the ListView-level fallback and was cancelled by KWin ~45 ms later; the deferred `autohide=true` retry could not re-acquire a grab because its input serial was stale. Fallback- and keyboard-invoked menus now take the grab immediately (within the triggering event), and pointer-invoked menus no longer move keyboard focus into the menu (which itself triggered the compositor dismissal)

## [0.16.0] - 2026-06-11

### Added

- **Batch edit for multi-selected connections** — group-operations mode (multi-selection) gains a "Batch Edit" action in the bulk toolbar: change group, tags, or icon for all selected connections in one pass. Each field has an "apply" check so only the chosen fields are written; the result toast offers Undo that restores the previous values
- **Notes badge in the sidebar** — connections with a description now show a small `document-edit-symbolic` badge next to their name (with the note text in the tooltip and a screen-reader label), so it is visible at a glance which entries have documentation without opening each one. Inspired by recurring requests in competing tools
- **Search matches connection notes** — sidebar search and smart search now score the connection description (weight below name/host/tags), so "the server where we replaced the certificate" can be found by the words in its notes
- **Windows (WSL2) guide** ([#137](https://github.com/totoshko88/RustConn/issues/137)) — new [docs/WSL.md](docs/WSL.md) with a step-by-step setup for running RustConn on Windows via WSLg: WSL install/update, enabling systemd for D-Bus/Secret Service (the most common pitfall), OBS/.deb install, secret-storage options, known WSLg limitations (no tray, audio latency, usbipd for serial) and troubleshooting. Linked from README and INSTALL

### Changed

- **Structured validation errors in core** — `dialog_utils` validators (`validate_name`, `validate_host`, `validate_port`, `validate_icon`) now return a `ValidationError` enum (`thiserror`) instead of plain strings, so callers can pattern-match variants; user-facing messages are unchanged and still localized at the GUI call sites
- **RDP `catch_unwind` wrapper kept for 0.16** — re-evaluated per the in-code TODO: ironrdp is still at 0.15.0 and upstream has not confirmed that panics on malformed PDUs are gone, so the panic guard around `connect_finalize` stays (re-check on the next ironrdp bump)

### Fixed

- **Sidebar context menu failing to open on KDE Plasma** ([#157](https://github.com/totoshko88/RustConn/issues/157)) — two fixes based on the reporter's `RUST_LOG=debug` output: (1) the popover's `closed` handler unparented the widget synchronously inside GTK's popdown sequence, which could free the popover mid-emission and produce `gtk_popover_get_autohide: assertion 'GTK_IS_POPOVER' failed`; the unparent is now deferred to idle. (2) KWin cancels non-grabbing popups (our menus use `autohide=false` per #87), so the menu could be dismissed by the compositor immediately after opening; if that happens within 300 ms with no user interaction, the menu now re-opens once with `autohide=true` (grab taken — KWin keeps it)

### Internal

- **`dialogs/connection/dialog.rs` split into submodules** — the largest file in the project (5988 lines) is now a `dialog/` module with focused submodules (`build`, `construction`, `save`, `rows`, `populate`, `passwords`, `agent_variables`); pure code motion, no behavior changes
- **New tests for security-sensitive core modules** — property tests verify `shell_escape` output round-trips byte-for-byte through a real POSIX `sh` (drag-and-drop paths are an injection surface), and `smart_folder` rule matching gained unit tests for case-insensitive host patterns and multi-criteria AND logic

### Security

- **Password handling in RDP/SPICE clients documented and regression-tested** — the plain-`String` password copies required by the third-party ironrdp/spice-client APIs are now explicitly documented as an upstream API limitation, and new tests assert that `RdpClientConfig`/`SpiceClientConfig` Debug output never leaks the plaintext password

## [0.15.14] - 2026-06-11

### Improved

- **External RDP/VNC sessions no longer freeze the window for 1.5 s on connect** — launching FreeRDP blocked the GTK main thread with a `sleep(1500ms)` used to catch immediate failures (certificate/auth errors). The launcher now returns right after spawn and watches the process with a non-blocking 250 ms poll over the same 1.5 s window; early failures still close the tab, show the parsed FreeRDP error toast, and record the failure in history
- **Tray messages are now event-driven instead of polled** — the main loop woke ~20×/second to `try_recv()` tray clicks even when idle, costing CPU and battery on laptops. Tray menu events (Linux ksni, macOS) now arrive over an `async-channel` awaited on the main context, so the loop only wakes on real clicks; tray handling is skipped entirely when the tray icon is disabled in settings
- **Secret backend detection in Settings is parallel and cached** — probing KeePassXC, Bitwarden, 1Password, Passbolt, pass and secret-tool spawned 10+ CLI processes sequentially (1–5 s before statuses appeared). Probes now run in parallel scoped threads (latency = slowest probe) and the result is cached for 30 s, making a quick reopen of Settings instant; the result-delivery loop also no longer spins the main loop at 100% CPU while detection runs (idle source → 50 ms timer)
- **Connection history writes are debounced and off the main thread** — every session start/end serialized and wrote `history.toml` inline on the GTK main thread (twice per session). Changes now mark history dirty and a flusher coalesces a 2 s burst into a single write on a background thread; pending changes are flushed on shutdown
- **One suggested action per dialog (GNOME HIG)** — "Add Variable"/"Add Property" in the connection dialog's Data tab no longer compete with the dialog's primary action for the suggested style, and the tunnel wizard hides its "Next" footer button while the empty-state "New SSH Connection" call-to-action is shown

### Dependencies

- **Added**: async-channel 2.5.0 (event-driven tray message delivery)
- **Updated**: crypto-primes 0.7.1→0.7.2

## [0.15.13] - 2026-06-10

### Added

- **Menu key / Shift+F10 opens the sidebar context menu for the selected row** ([#157](https://github.com/totoshko88/RustConn/issues/157)) — standard GNOME HIG keyboard access to the connection/group context menu, anchored to the selected row. Requested as a reliable alternative on systems where right-click on nested rows misbehaves
- **Confirmation before closing with open sessions** — closing the window or pressing Ctrl+Q with open session tabs now asks "Close RustConn?" with the number of open tabs instead of silently disconnecting everything. Skipped when minimize-to-tray is enabled (the app keeps running). Both the window close button and the `app.quit` action (which bypasses `close_request`) share the same dialog
- **Recording indicator in the sidebar** — a red `media-record-symbolic` dot (with tooltip and screen-reader label) appears next to a connection while any of its sessions is being recorded; recording is privacy-sensitive and must be visible at a glance. Driven by a new `on_recording_changed` notebook callback so every start/stop path (manual action, auto-record, session end) updates it
- **Import duplicate handling** — importing connections whose names already exist now shows a dialog with the duplicate count and the choices Cancel / Skip Duplicates / Import All, instead of silently creating renamed copies
- **Persistent cloud-sync failure banner** — sync errors (manual Sync Now and background auto-export, which previously only logged to the journal) now reveal an `adw::Banner` below the header bar that stays until dismissed or the next successful sync; transient toasts are kept for success messages only (GNOME HIG: banner for state that needs attention)
- **Touch long-press opens the sidebar context menu** — `GestureLongPress` (touch-only) on the connection list, sharing the coordinate-based row resolution with the right-click fallback

### Improved

- **Context menu keyboard navigation and accessibility** — the custom sidebar context menu now supports Up/Down arrow navigation with wrap-around plus Home/End, focuses its first item on open (so the Menu-key path is immediately navigable), highlights the focused item, and announces itself as a menu (`AccessibleRole::Menu` / `MenuItem`) to screen readers
- **Error message quality (GNOME HIG writing style)** — removed "Please" from 10 validation messages (imperative voice), replaced three generic "OK" buttons with "Close", and rewrote raw error surfaces ("Error: {e}", "Unknown error", "Error loading log file") to explain what happened and what to do next; export/log-file failures now name the likely cause (permissions, disk space, moved/deleted file)
- **Sidebar bottom-toolbar buttons enlarged to the 44×44 px minimum tap target** (GNOME HIG pointer & touch); Ukrainian translations added for all new strings

### Packaging & CI

- **Snap: `rustconn-cli` granted `password-manager-service`** — the CLI could not reach the system keyring (`rustconn-cli secret` failed under strict confinement) while the GUI app could; plugs are now in sync
- **Snap: SSH agent limitation documented honestly** — snapd has no interface exposing the host SSH agent socket (unlike Flatpak's `--socket=ssh-auth`), so agent-based auth cannot work in the snap; docs/SNAP.md previously suggested checking `$SSH_AUTH_SOCK`, now it documents the limitation, workarounds, and a comparison-table row
- **AppImage recipe migrated from jammy to noble** — `packaging/obs/AppImageBuilder.yml` still targeted Ubuntu 22.04, which never shipped the GTK4 build of VTE, while the released AppImage is already built on ubuntu-24.04 runners; the recipe now matches reality (incl. `*t64` package renames); test images that cannot run a noble-based AppImage (fedora-30, ubuntu-focal) removed
- **Supply-chain hygiene for CI downloads** — `flatpak-cargo-generator.py` (3 workflows) and `linuxdeploy-plugin-gtk.sh` are now fetched from pinned commits and verified against SHA-256 checksums; `linuxdeploy` (continuous tag, unstable checksums) gets retry + ELF sanity check; the Homebrew source tarball download gets retry with backoff plus a `tar -tzf` integrity check
- **CI cargo caches can seed each other** — all six cache blocks now share a `restore-keys` fallback, so check/clippy/test/proptest/MSRV jobs reuse each other's registry and target artifacts instead of rebuilding from scratch on every key miss
- **OBS spec hardening** — Fedora/RHEL `cargo build` now passes `--offline` (belt-and-suspenders on top of vendored sources) and the installed desktop file is checked with `desktop-file-validate` (`desktop-file-utils` added to BuildRequires)
- **OBS debian packaging** — `Recommends` now lists `freerdp3-x11 | freerdp3-wayland` alternatives (matching the top-level debian/), and a header comment documents the intentional differences between the two debian packagings
- **`AppImageBuilder.yml` version is now synced by workflows** — both `obs-update.yml` and the release OBS job update its version field alongside spec/dsc/_service (previously only the local hook did)
- **Flatpak manifests de-drifted** — release and local manifests updated to inetutils 2.8 (Flathub already had it) and got the same `x-checker-data` blocks as the Flathub manifest (vte, inetutils, picocom, libsecret, mc, freerdp), so flatpak-external-data-checker reports outdated pins on all three manifests consistently

### Fixed

- **Sidebar context menu not opening for rows nested deeper than the root level on some systems (reported on KDE Plasma)** ([#157](https://github.com/totoshko88/RustConn/issues/157)) — the menu was shown only by a per-row `GestureClick` on each `TreeExpander` (CAPTURE phase); on the reporter's environment right-clicks on nested rows never reached that gesture. A fallback `GestureClick` on the `ListView` itself (BUBBLE phase) now resolves the clicked row from the pointer coordinates via `pick()` — independent of per-row event dispatch, so it works at any nesting depth. In the normal case the per-row gesture still fires first and claims the press, cancelling the fallback; when both fire (claim propagation differs between compositors), the result is idempotent — same selection, same menu position. Item-data extraction was deduplicated into a shared `show_context_menu_for_connection_item()` helper used by the per-row gesture, the fallback, and the new keyboard path

- **Crash (SIGSEGV) in pango when opening a new SSH tab or on screen unlock — follow-up to v0.15.9** ([#171](https://github.com/totoshko88/RustConn/issues/171)) — debug-symbol analysis of all five crash dumps (Ubuntu 24.04 dbgsym for pango 1.52.1 and vte 0.76.0) showed the same use-after-free: `pango_itemize` → `g_object_ref()` on a freed `PangoFont`, hit during VTE's first text measurement of a freshly created terminal (`FontInfo::get_unistr_info`) inside the GTK snapshot phase. Root cause: VTE reads `gtk-fontconfig-timestamp` only when it creates its cached `FontInfo` (the timestamp is part of the font-cache key) and never subscribes to changes, so after a fontconfig update (font installation, `fc-cache`, KDE pushing `Fontconfig/Timestamp` via XSettings on screen unlock) terminals keep stale Pango font references — which is why the crash always struck right when a new tab was opened (existing tabs render from VTE's glyph cache and rarely re-itemize) and why a restart always cleared it. Two changes: (1) RustConn now listens for `gtk-fontconfig-timestamp` changes and re-applies the font on every open terminal, forcing VTE to rebuild its font cache against the new fontconfig state (`vte_terminal_set_font` deliberately recreates the font even for an unchanged description); (2) all remaining widget/VTE work in the `child-exited` handler (disconnect indicator, `terminal.reset()`, reconnect banner, auto-reconnect setup) is now deferred to the next main-loop idle instead of running inside VTE's signal emission, closing the same race the v0.15.9 fix closed for `close_tab`

### Dependencies

- **Updated**: crypto-primes 0.7.0→0.7.1, ksni 0.3.4→0.3.5, regex 1.12.3→1.12.4 (with regex-syntax 0.8.10→0.8.11), zerocopy 0.8.50→0.8.52 (with zerocopy-derive)

## [0.15.12] - 2026-06-09

### Fixed

#### macOS

- **SSH password authentication always failed with "Permission denied"** ([#175](https://github.com/totoshko88/RustConn/issues/175)) — on macOS the terminal uses a hand-rolled native PTY (VTE's `spawn_async` does not connect the PTY to the child on the Homebrew build). The spawned child was given the PTY slave as stdin/stdout/stderr but never made it the *controlling terminal*. Since `ssh` reads the password from `/dev/tty` (not stdin) and RustConn disables askpass on macOS (`SSH_ASKPASS_REQUIRE=never`, #161), `ssh` could not read the password, tried an empty one three times, and failed with `Permission denied (publickey,password)`. The same affected `ssh` typed manually in a Local Shell tab. The child is now placed in a new session via `setsid(2)` and claims the slave PTY as its controlling terminal via `TIOCSCTTY`, so interactive password prompts work. `setsid(2)` also supplies the process-group leadership previously set by `process_group(0)`, so job control (Ctrl-C) is preserved.

### Changed

- **New `rustconn-pty-sys` crate** — the controlling-terminal setup needs `pre_exec` (an `unsafe` API), which conflicts with the workspace-wide `unsafe_code = "forbid"`. Per the project's `M-UNSAFE` guideline, the FFI is isolated in a small dedicated crate that exposes a single safe `set_controlling_terminal()` function with a documented safety contract. The main crates keep `unsafe_code = "forbid"` untouched.

### Dependencies

- Updated `uuid` 1.23.2 → 1.23.3
- Updated `wasm-bindgen` 0.2.122 → 0.2.123 and the related `js-sys` 0.3.99 → 0.3.100, `web-sys` 0.3.99 → 0.3.100, `wasm-bindgen-futures` 0.4.72 → 0.4.73 (transitive)

## [0.15.11] - 2026-06-07

### Fixed

#### Keybindings

- **Recorder did not register keystrokes on Flatpak/Wayland** ([#170](https://github.com/totoshko88/RustConn/issues/170), [#167](https://github.com/totoshko88/RustConn/issues/167)) — the inline recorder (0.15.7–0.15.10) attached an `EventControllerKey` to the toplevel window and depended on row focus, which was unreliable inside `AdwPreferencesDialog`. Replaced it with a dedicated modal `AdwDialog` (the pattern GNOME Control Center uses) that owns its own keyboard focus, so every key press is captured. Escape cancels, Backspace resets to default, conflicts still warn, and global accelerators are suspended during capture.
- **Custom shortcuts showed defaults after reopening Settings or restarting** ([#170](https://github.com/totoshko88/RustConn/issues/170)) — overrides were saved and applied correctly, but the UI always displayed the default. `move_groups` reparents the keybinding rows into the Interface page, leaving the page that `load_keybinding_settings` walked empty, so no label was updated. The accelerator labels are now tracked directly via a `HashMap<action, Label>` instead of walking the widget tree.

#### Snap

- **Package failed to start on Ubuntu 26.04** ([#174](https://github.com/totoshko88/RustConn/issues/174)) — the snap targeted `base: core26` and hand-rolled the GTK4 runtime because the `gnome` extension does not yet support core26 ([snapcraft#6185](https://github.com/canonical/snapcraft/issues/6185)), omitting `desktop-launch`, the GNOME platform and the matching AppArmor accesses. Moved to `base: core24` with `extensions: [gnome]`, which provides the complete, correctly-confined GTK4 environment. (The 0.15.10 note blaming `grade: devel` was wrong — `grade` only controls store channels.)
- **App could not register on the session D-Bus** — `g_application_register` was denied by AppArmor because a confined snap may only own names derived from the snap name, not the app ID. Added a `dbus` slot (`bus: session`, `name: io.github.totoshko88.RustConn`); the providing snap is auto-granted ownership. The Flatpak build is unaffected.
- **Transparent window and broken icons** — affected only the snap (native, Flatpak and other GTK4 snaps rendered fine). VTE must be staged (the platform omits it at runtime), but its `.deb` drags in a second copy of the whole GTK4 stack. The platform's libadwaita then bound against our `libgtk-4` (ABI mismatch → transparent window) and the platform's SVG loader against our newer `librsvg` (→ broken icons). A `prime` exclusion now drops every platform-provided GTK/GLib/render library, keeping only `libvte` itself, so a single matched copy is used process-wide.

### Changed

#### Keybindings

- **Shortcuts are stored layout-independently (Latin)** ([#170](https://github.com/totoshko88/RustConn/issues/170)) — recording under a non-Latin layout (e.g. Cyrillic) used to store the localised keyval, so pressing "F" produced `<Control>ф`, which stopped matching after switching back to Latin. The recorder now resolves the hardware keycode to its ASCII keyval. Function keys are unaffected.

#### Snap

- **Base `core26` → `core24`** (Ubuntu 24.04 LTS / GNOME 46 / libadwaita 1.5) — the GUI is now built **without** `--features adw-1-8`. The 1.6/1.7/1.8 widgets fall back to 1.5 equivalents (AdwSpinner → GtkSpinner, AdwToggleGroup → linked buttons, AdwShortcutsDialog → legacy dialog), preserving functionality with slightly less polish than the Flatpak (GNOME 50) build. Can return to core26 + adw-1-8 once the gnome extension supports core26.
- **CI installs Snapcraft from `latest/stable`** again (the `latest/candidate` 9.x pin was only needed for core26).
- **Added a `title`** (`RustConn`) so the Store listing and metadata linter no longer report a missing field.

## [0.15.10] - 2026-06-05

### Fixed

- **Keybinding overrides not displayed correctly after reopening Settings** — `load_keybinding_settings` and `refresh_accel_labels` used `zip` between DOM-ordered ActionRows and the `default_keybindings()` vector, but these orders diverged because Application category bindings appear in two separate groups in the vector (beginning and end) while in the UI they are merged into a single ExpanderRow. Starting from the 3rd row, each label was updated with the accelerator of a **different** action, making user-recorded shortcuts appear "not saved" on dialog reopen. Fixed by building the same category-grouped order used by `create_keybindings_page` before zipping ([#170](https://github.com/totoshko88/RustConn/issues/170))
- **Keybinding conflict detection ignored modifier order** — `find_accel_conflict` compared accelerator strings with plain `==`, so `<Control><Shift>w` and `<Shift><Control>w` (both representing Ctrl+Shift+W) were treated as different shortcuts. This allowed assigning a conflicting binding without any warning. Now comparison normalises modifier order (and the `<Primary>`/`<Ctrl>` aliases) before comparing key + modifiers, via a pure `accelerators_equivalent` helper in `rustconn-core` (unit-tested without a display)
- **Snap package fails to start on Ubuntu 26.04 (AppArmor error)** — the `snapcraft.yaml` still contained `build-base: devel` and `grade: devel` from the time when `core26` was experimental. Since core26 became stable (2026-04-29) and Snapcraft 9.0 added full support (2026-05-07), these keys are no longer required and caused the snap to be built with a restricted AppArmor profile that blocked access to `desktop`, `wayland`, and `opengl` interfaces. Removed `build-base: devel` and changed `grade` to `stable` ([#174](https://github.com/totoshko88/RustConn/issues/174))
- **SNAP.md and INSTALL.md still referenced the removed `host-usr-bin` interface** — the `system-files` plug was removed in 0.15.3 (rejected by Snap Store review), but the documentation was not updated. All references now correctly describe the on-demand CLI download mechanism via the Components dialog

### Dependencies

- Updated `bitflags` 2.12.1 → 2.13.0

## [0.15.9] - 2026-06-05

### Improved

- **Lazy init secret backends — only preferred backend is initialized** — previously opening the Settings → Secrets tab triggered keyring queries, decryption, and even Bitwarden CLI auto-unlock for ALL backends regardless of which one was configured as preferred. Now only the `preferred_backend` is initialized (decrypt + keyring load) both at startup and when opening Settings. Other backends' credentials are loaded on-demand when the user switches to them via the dropdown. Additionally, `dispatch_vault_op` now passes the service account token to 1Password, server URL and GPG passphrase to Passbolt backends, which were previously created without credentials
- **KeePass keyring failure toast at startup** — if the system keyring does not respond within 5 seconds or returns no password, a toast is shown after the main window appears: "KeePass password not loaded from keyring — re-enter it in Settings" with a clickable "Settings" button that opens the Preferences dialog directly. Previously the failure was only logged, leaving users confused when vault-based connections silently failed
- **Connection wizard: ComboRow model lazy init** — the auth method `ComboRow` model is now created empty and populated only when `configure_for_protocol()` is called, eliminating a brief flash of all 4 SSH methods during page transition animations for non-SSH protocols

### Fixed

- **macOS: Option key not producing composed characters in terminal** — on macOS with non-US keyboard layouts (German, French, etc.), `Option+L` should produce `@` (German) but was instead treated as Alt+L sending escape sequences to the PTY. Added a new "Option as Meta key" setting (Settings → Terminal → Behavior, macOS only) that defaults to off. When off, Option+key combinations produce the composed character from the active keyboard layout. When on, Option sends ESC-prefixed sequences (for vim/emacs users). The fix intercepts Option+key in GTK Capture phase before VTE's internal Alt handler, checks if GDK resolved a printable Unicode character from the macOS IMContext, and feeds it directly to the PTY ([#173](https://github.com/totoshko88/RustConn/issues/173))
- **KeePass vault credentials not resolved on Flatpak after restart** — when the KeePass database password was stored in the system keyring (default for Flatpak), it was not loaded at startup — only when the user opened Settings. This caused connections with `password_source = Vault` to show "Secret Backend Not Configured" or prompt for a password instead of reading it from the KDBX file. Now the keyring is queried during `AppState` initialization, matching the existing Bitwarden credential restore flow ([#170](https://github.com/totoshko88/RustConn/issues/170))
- **Crash (SIGSEGV) when opening new SSH tab or on screen lock/unlock** — three crash scenarios caused by VTE use-after-free during GTK widget snapshot phase. When `close_on_clean_exit` was enabled, `close_tab()` was called synchronously from within VTE's `child-exited` signal handler, destroying the widget while GTK still had a pending render scheduled for the current frame. Additionally, after SSH disconnect the VTE terminal could hold stale Pango font references that became invalid on screen lock/unlock (GPU context loss). Fix: (1) defer `close_tab` via `glib::idle_add_local_once` so the tab is destroyed after the current frame completes, (2) call `terminal.reset()` in `mark_tab_disconnected` to release stale Pango resources before the next snapshot cycle ([#171](https://github.com/totoshko88/RustConn/issues/171))
- **Connection wizard: auth method label overflow** — the "Method" label in the Authentication step was broken into individual characters ("M-e-t-h-od") because four radio buttons placed horizontally as an `ActionRow` suffix exceeded the available width. Replaced radio buttons with `adw::ComboRow` dropdown which fits any dialog width and follows GNOME HIG patterns ([#169](https://github.com/totoshko88/RustConn/issues/169))
- **Telnet connection not closed when closing the tab** — the `telnet` process was not terminated when the connection tab was closed, leaving established TCP connections in ESTABLISHED state in the background. The root cause: VTE closes the PTY master fd on widget destroy, which sends SIGHUP to the child, but telnet (and some other CLI clients) does not exit on SIGHUP while a TCP session is active. Fix: store the VTE child PID after `spawn_async` and explicitly send SIGTERM to the process group (`kill(-pid, SIGTERM)`) when the tab is closed. Also applies to Serial and other VTE-spawned protocols ([#172](https://github.com/totoshko88/RustConn/issues/172))
- **1Password/Passbolt credentials not passed in vault entry lookups** — `retrieve_by_vault_entry_name`, connection dialog password preview, and CLI `secret` commands created backend instances without service account token (1Password) or server URL and GPG passphrase (Passbolt). Custom vault entry names and CLI operations failed silently for these backends. Now all code paths pass credentials from settings consistently. Passbolt backend now accepts `--userPassword` flag for GPG passphrase authentication without relying on the CLI config file
- **Connection wizard: redundant Method dropdown for non-SSH protocols** — RDP/VNC/SPICE connections showed a ComboRow with a single "Password" option which was pointless. The method selector is now hidden for protocols that only support password authentication

## [0.15.8] - 2026-06-04

### Fixed

- **Keybinding reassignment not registering keystrokes** — the 0.15.7 fix (suspending accelerators) was necessary but not sufficient: after the Record button becomes insensitive it loses keyboard focus, leaving GTK4 with no target widget for key event propagation. Additionally, `AdwPreferencesDialog` with `search_enabled=true` installs a `key_capture_widget` on its internal `SearchEntry` that intercepts letter keys in bubble phase. Now the recorder (1) moves focus to the parent `ActionRow` so GTK4 has a valid propagation target, and (2) temporarily disables PreferencesDialog search during recording to eliminate `SearchEntry` interference. Both are restored on recording completion or cancellation ([#167](https://github.com/totoshko88/RustConn/issues/167))
- **Sidebar: right-click context menu not appearing for hosts in groups** — on Wayland with multiple groups, the `empty_space_gesture` on `ScrolledWindow` (bubble phase) could race with the per-item gesture on `TreeExpander` (capture phase): both handlers fired for the same right-click event, causing `empty_space_gesture` to call `close_active_popover()` and immediately destroy the item context menu that had just opened. Now `empty_space_gesture` checks via `pick()` + `ancestor(TreeExpander)` whether the click landed on an actual row and bails out if so. Additionally fixed a memory leak where `focus-widget` signal handlers accumulated on the window (never disconnected after popover close) and eliminated double `unparent()` calls that produced GTK critical warnings ([#168](https://github.com/totoshko88/RustConn/issues/168))
- **Secret variable with vault entry name wrote duplicate entry to vault** — when a secret variable had a custom "Vault entry" name (e.g., `AD Credentials`), saving still wrote the password under the default `rustconn/var/{name}` key, creating an unnecessary duplicate in Bitwarden/1Password/Passbolt/Pass. Now variables with a vault entry reference are treated as read-only — nothing is written back to the vault ([#166](https://github.com/totoshko88/RustConn/issues/166))
- **Sidebar: status icon size inconsistent with custom icons** — the connection status indicator (green checkmark / red stop) appeared larger for connections with a custom emoji icon because the sibling-based widget navigation found the wrong `Image` widget when an emoji label was prepended. Now status icons are located by CSS class (`status-icon`) and the main connection icon has a fixed `pixel_size(16)`, ensuring uniform 10px status indicators for all connection types

### Improved

- **Variable dialog: vault entry UX hints** — when the "Vault entry" field is filled, the password field placeholder changes to "Fetched from vault at connect time" to indicate no manual password input is needed. Updated tooltip explains that nothing is written back to the vault ([#166](https://github.com/totoshko88/RustConn/issues/166))

### Dependencies

- **Updated**: chrono 0.4.44→0.4.45, log 0.4.31→0.4.32, yoke 0.8.2→0.8.3

## [0.15.7] - 2026-06-03

### Improved

- **Variable password source: discoverability** — when "Variable" is selected as password source in the connection or group dialog, the row now shows a subtitle hint ("Create secret variables in Tools → Variables") and a "+" button that opens the global variables manager directly. Previously the dropdown appeared empty with no guidance, making the feature appear broken for users who had not yet created secret variables ([#166](https://github.com/totoshko88/RustConn/issues/166))
- **Variable password source: custom vault entry name** — secret variables can now reference an existing entry in Bitwarden, 1Password, Passbolt, or Pass by its exact name (e.g., "AD Credentials") instead of the default `rustconn/var/{name}` lookup key. This is the non-KeePass equivalent of the existing "KeePass entry" field — both allow reusing credentials already stored in the vault without duplication. Configure via Tools → Variables → mark as Secret → fill "Vault entry" field ([#166](https://github.com/totoshko88/RustConn/issues/166))

### Fixed

- **Proxmox SPICE: inline PEM CA certificate now saved automatically** — when importing a `.vv` file from Proxmox VE that contains an inline PEM CA certificate (common in SPICE tickets), the certificate is now automatically saved to `~/.local/share/rustconn/certs/ca-<hash>.pem` and the path is set in connection settings. Previously the import only showed a warning asking the user to save the certificate manually, which was impractical because Proxmox tickets expire in 30–40 seconds. Now the connection works immediately after import via file manager or `rustconn file.vv` ([#165](https://github.com/totoshko88/RustConn/issues/165))
- **Keybinding reassignment not working** — recording a new keyboard shortcut in Settings → Interface did not register keystrokes because global application accelerators (e.g. `Ctrl+W`) intercepted the key event before the recording controller could receive it. Now all accelerators are temporarily suspended during recording and the `EventControllerKey` uses the Capture phase, ensuring any key combination reaches the recorder ([#167](https://github.com/totoshko88/RustConn/issues/167))
- **Sidebar: right-click context menu still not working at depth ≥ 2** — the 0.15.6 fix moved the gesture from `TreeExpander` to `content_box`, but `content_box` does not cover the indent/arrow area that `TreeExpander` renders to the left of the content for nested items. Right-clicks landing in the indent area (which grows wider at each nesting level) never reached `content_box` and were silently ignored. Moved the gesture back to the `TreeExpander` widget with `BUTTON_SECONDARY` — this does not conflict with TreeExpander's internal expand/collapse handler which only listens for `BUTTON_PRIMARY` ([#157](https://github.com/totoshko88/RustConn/issues/157))

## [0.15.6] - 2026-06-02

### Added

- **VNC: Accept Certificate option** — new "Accept Certificate" toggle in VNC connection settings allows connecting to VNC servers with self-signed or untrusted TLS certificates (VeNCrypt). When enabled, the external viewer (TigerVNC) receives `-SecurityTypes VeNCrypt,TLSVnc,X509Vnc,VncAuth,None` arguments. The embedded VNC client auto-fallbacks to the external viewer with proper security types when VeNCrypt is detected. CLI `--ignore-certificate` now works for both RDP and VNC connections ([#164](https://github.com/totoshko88/RustConn/issues/164))

### Fixed

- **Welcome screen: "Remote host monitoring" icon missing on macOS** — replaced `speedometer-symbolic` (not part of the standard Adwaita icon theme bundle) with `power-profile-performance-symbolic` (same icon used in Settings → Monitoring tab) which is available across all platforms
- **Sidebar: right-click context menu not opening for nested items** — context menu gesture was attached to the `TreeExpander` widget, whose internal indent/arrow gesture silently swallowed right-click events for items at depth ≥ 1; moved the gesture controller to the content box inside the expander, bypassing the conflict ([#157](https://github.com/totoshko88/RustConn/issues/157))

### Dependencies

- **Updated**: bitflags 2.11.1→2.12.1, log 0.4.30→0.4.31, lzma-rust2 0.16.2→0.16.4. Removed: sha2 0.10.9 (unused transitive dependency)
- **Flathub**: inetutils 2.7→2.8

## [0.15.5] - 2026-06-01

### Added

- **IronRDP 0.15 (bulk compression)** — RDP sessions now negotiate XCRUSH (RDP 6.1) compression in Quality/Balanced modes and MPPC-64K in Speed mode, significantly reducing bandwidth for slow connections. Compression is handled transparently by the new `ironrdp-bulk` crate.
- **IronRDP 0.15 (slow-path rendering)** — sessions with servers that use slow-path output (XRDP, older Windows) now render correctly instead of showing blank screens. Both slow-path bitmap and pointer updates are routed through the existing rendering pipeline.
- **IronRDP 0.15 (alternate_shell/work_dir)** — RemoteApp `program` and `working_dir` are now passed via the native `alternate_shell`/`work_dir` fields in the Client Info PDU, enabling CyberArk PSM and custom shell scenarios without FreeRDP.
- **IronRDP 0.15 (improved compatibility)** — connection to GNOME Remote Desktop (grd) no longer fails on `ServerDeactivateAll` during CapabilitiesExchange; all colour depths are advertised per FreeRDP pattern (fixes Windows Server 2012+ with 24bpp); Auto-Detect Request PDUs no longer crash the session; bitmap updates exceeding buffer bounds after resize are safely skipped.
- **IronRDP 0.15 (multitransport dispatch)** — `MultitransportRequest` and `AutoDetect` PDUs are now logged instead of causing unhandled-PDU errors. UDP sideband transport is not yet implemented but the session stays alive.
- **IronRDP 0.15 (clipboard file contents)** — `SendFileContentsRequest`/`SendFileContentsResponse` clipboard messages are now gracefully handled (logged, not yet implemented for full file copy).
- **IronRDP 0.15 (pixel format fix)** — removed manual R↔B channel swap in `extract_region_data`; IronRDP 0.15 fixed the pixel format pipeline so BgrA32 output now directly matches Cairo's ARGB32 (both are B-G-R-A in memory on little-endian). This eliminates a per-frame O(w×h) loop, improving 4K rendering throughput.

### Fixed

- **macOS: passwords not saving to Keychain** — `dispatch_vault_op()` incorrectly used `LibSecretBackend` (which shells out to `secret-tool`, a Linux-only utility) for the `MacOsKeychain` backend type. Now correctly instantiates `MacOsKeychainBackend` (Security.framework) on macOS. Users saw a generic "Failed to save password to vault" toast with no further details.
- **macOS: tray icon missing when launched from .app bundle** — the root cause was `exec()` (re-exec) breaking the macOS LaunchServices "scene" registration for `NSStatusItem`. Replaced `setup_macos_bundle_env()` (which used `exec()` to set env vars) with `configure_macos_bundle()` which programmatically configures all subsystems without re-exec: `i18n::locale_dir()` now detects the bundle's `Contents/Resources/locale` path directly, icon search paths were already added programmatically in `register_app_icon()`, and `get_extended_path()` already handles PATH for child processes. `CFBundleExecutable` in Info.plist now points to the native `rustconn` binary with no wrapper or re-exec needed. **Note**: on macOS Sequoia 15.5, tray icon is not displayed when launched via Finder/Dock due to a GTK4 GDK macOS backend limitation (FrontBoardServices scene registration failure); works correctly when launched from terminal.
- **macOS: AWS SSM "session-manager-plugin not found"** — added `/usr/local/sessionmanagerplugin/bin` to `get_extended_path()` on macOS for users who install the plugin via the official AWS installer (not Homebrew). Documented the separate installation requirement in `ZERO_TRUST.md` and `MACOS_BUILD.md`.

### Changed

- **Compact mode: denser sidebar rows** — vertical margins reduced from 6px to 3px per row, allowing ~60px more visible content for 10 connections
- **Compact mode: slimmer sidebar bottom toolbar** — button min-height reduced from default to 22px with tighter padding
- **Compact mode: smaller search/filter bar** — search entry min-height reduced to 26px with slightly smaller font
- **Compact mode: popover menu item padding** — all popover menus (hamburger, context, tab) use less vertical padding per item, significantly reducing menu height on small screens
- **Compact mode: smaller protocol filter buttons** — denser filter pills matching overall compact density
- **Compact mode: denser RDP/VNC/SPICE toolbar** — embedded session toolbar (Copy, Paste, Autotype, Ctrl+Alt+Del) uses reduced margins and button height in compact mode, giving more vertical space to the remote desktop viewport
- **Compact mode enabled by default on macOS** — new installations on macOS start with compact interface active; existing users keep their saved preference
- **Hamburger menu restructured into submenus** — "Tools" (Snippets, Clusters, Templates, Variables, Password Generator, Wake On LAN, SSH Tunnels) and "Sessions" (Active Sessions, History, Statistics, Recordings) are now submenus; Import/Export/Copy/Paste merged into a single "File" section. Top-level menu reduced from 24 items to ~14, dramatically reducing vertical height on macOS

### Dependencies

- **Upgraded**: ironrdp 0.14→0.15, ironrdp-tokio 0.8→0.9, ironrdp-connector 0.8→0.9, ironrdp-session 0.8→0.9, ironrdp-cliprdr 0.5→0.6, ironrdp-rdpdr 0.5→0.6, ironrdp-rdpsnd 0.7→0.8, ironrdp-dvc 0.5→0.6, ironrdp-displaycontrol 0.5→0.6, ironrdp-graphics 0.7→0.8, ironrdp-pdu 0.7→0.8, ironrdp-core 0.1→0.2, ironrdp-svc 0.6→0.7. New transitive: ironrdp-bulk 0.1.
- **Updated**: inotify 0.11.1→0.11.2, ironrdp-tls 0.2.0→0.2.1, rustls-native-certs 0.8.3→0.8.4, unicode-segmentation 1.13.2→1.13.3

## [0.15.4] - 2026-05-31

### Fixed

- **macOS: UI hang when editing connection with broken ssh-agent** — when the system `ssh-agent` is unhealthy or launchd-throttled, opening the Edit Connection dialog no longer freezes the GTK main thread; `ConnectionDialog::refresh_agent_keys()` now probes the agent asynchronously on a background thread with a 5-second timeout, showing "Loading agent keys…" while the probe runs; if the agent does not respond in time the child process is killed and the dropdown shows "No keys loaded" without blocking the UI ([#163](https://github.com/totoshko88/RustConn/issues/163))
- **macOS: SSH Tunnel Manager showed native traffic lights** — migrated from `adw::Window` to `adw::Dialog` for consistent cross-platform presentation (× close button in header bar, same as all other dialogs)
- **macOS: Settings dialog tabs truncated** — increased `content_width` to 1000px on macOS so all 6 ViewSwitcher tabs display without ellipsis (Linux remains 800px)
- **macOS: clippy warnings** — fixed `useless_conversion` in `rdpdr.rs` (inverted `cfg_attr`), `new_without_default` and `redundant_clone` in `macos_keychain.rs`, `case_sensitive_file_extension_comparisons` in `main.rs`, `collapsible_if` in `macos_pty.rs` and `window/mod.rs`, `single_match_else` / `while_let_loop` / `useless_conversion` / `if_not_else` in `tray.rs`, `needless_return` in `edit_actions.rs`, `items_after_statements` in `dialog.rs`, unused `PtyFlags` import in `terminal/mod.rs`
- **`release.sh` crash on macOS (bash 3.2)** — replaced `declare -A` associative arrays (bash 4+ only) with parallel indexed arrays compatible with macOS default bash
- **`release.sh` clippy failure on macOS** — added platform detection to run clippy with `--no-default-features --features tray-macos,...` on Darwin, avoiding the missing `gtk4-wayland` pkg-config error

### Added

- **`scripts/macos-build.sh`** — one-command build + `.app` bundle creation for macOS development. Handles cargo build with correct features, icon generation, locale compilation, Adwaita icon bundling, ad-hoc code signing, and launch. Supports `--release`, `--no-launch`, `--clean` flags.

### Dependencies

- **Updated**: kqueue 1.1.1→1.2.0, libz-sys 1.1.28→1.1.29, rpassword 7.5.3→7.5.4

## [0.15.3] - 2026-05-30

Snap packaging now reaches feature parity with the Flatpak build and is on a current GNOME runtime.

### Changed

- **Snap base bumped to `core26` (Ubuntu 26.04 / GNOME 50).** The previous `core24` base only provides libadwaita 1.5, while the UI targets libadwaita 1.8 — the same version the Flatpak GNOME 50 runtime ships. The snap now builds the GUI crate with `--features adw-1-8` via an explicit workspace `cargo build` in `override-build` (the plugin's `rust-features` key applies `--features` per-crate, which would fail on `rustconn-cli`, so we drive cargo directly, mirroring the Flatpak manifest).
- **Removed the `system-files` (`host-usr-bin`) plug from the snap.** The Snap Store review rejected executing host binaries (`-1` on the privileged request); bundled embedded clients plus on-demand CLI downloads replace it. The `personal-files` plugs (`aws`/`gcloud`/`azure`/`oci`/`kube-credentials`) remain and are intended for manual connection, matching the reviewer's `+1`.
- **Renamed the "Flatpak Components" dialog/menu to "Components".** The external-CLI download manager is no longer Flatpak-specific.

### Added

- **External CLI tools can now be downloaded on demand inside the snap**, mirroring the Flatpak mechanism. CLIs install into `$SNAP_USER_DATA/cli/` and are added to the connection-launch `PATH`; cloud CLI config dirs (gcloud, Azure, Teleport, OCI) are redirected to writable `$SNAP_USER_DATA/.config/<tool>` locations with credential bootstrap from the host's read-only `personal-files` mounts. `python3` is now staged so the pip-based `az`/`oci` CLIs work.
- **`rustconn_core::is_sandboxed()`** — single predicate (`is_snap() || is_flatpak()`) now gates all sandbox-specific CLI logic instead of Flatpak-only checks, so snap and Flatpak share one code path.
- **Snap confinement guidance for personal-files plugs.** `get_confinement_message()` now detects when cloud credential directories exist on the host but are inaccessible (plug not connected) and shows the exact `sudo snap connect rustconn:<plug>` command to run.

### Fixed

- **Broken Ukrainian translation for the Protocol Clients section.** The Components dialog subtitle was garbled ("Убудовані Першорядні - це клієнти…") — now correctly reads "Необов'язкові для зовнішніх з'єднань RDP/VNC/SPICE. Перевагу мають вбудовані клієнти (IronRDP, vnc-rs)."
- **All 16 language catalogs updated for the "Components" rename.** Removed stale "Flatpak" from translations and cleared fuzzy flags so the menu/dialog title is properly localized in all languages.

### Documentation

- **Zero Trust guide updated for snap.** Added a "Sandbox note (Flatpak & Snap)" block explaining how CLI tools are installed and how credential directories are accessed in both sandboxed distributions.
- **Bitwarden setup guide updated for snap.** Sections now reference both Flatpak and Snap paths; "Flatpak Components" → "Components" throughout.

### Dependencies

- **Updated**: hyper 1.10.0→1.10.1, typenum 1.20.0→1.20.1, uuid 1.23.1→1.23.2, zbus 5.15.0→5.16.0, zbus_macros 5.15.0→5.16.0, zerocopy 0.8.49→0.8.50, zerocopy-derive 0.8.49→0.8.50, zvariant 5.11.0→5.12.0, zvariant_derive 5.11.0→5.12.0, zvariant_utils 3.3.1→3.4.0

## [0.15.2] - 2026-05-29

A small code-hygiene and documentation release. No user-facing behaviour changes — the goal is to remove misleading lint reasons, drop a dead field, replace a `unwrap()` in the CLI with a proper error path, and bring the keyboard-shortcut documentation back in sync with the code.

### Improved

- **Corrected misleading `#[allow]`/`#[expect]` reasons.** A copy-pasted `reason = "kept alive for GTK widget lifecycle / future API exposure"` had spread into `rustconn-core` (a crate that never touches GTK) and into property tests, where it made no sense. Each site now carries an accurate reason: float comparisons against exactly-representable constants (`0.0`, `100.0`), bounded `f64`→`u32` truncation in drop-index tests, reserved test fixtures and proptest strategies, and the long sequential body of `start_collector`. Lints that fire are now `#[expect]`; genuinely-reserved test helpers stay `#[allow(dead_code)]` with honest reasons.
- **Removed a dead field in `DirectoryWatcher`.** `notify_tx` stored a throwaway `mpsc::channel().0` (the real sender lives inside the watcher closure) and only existed behind an `#[allow(dead_code)]`. The field and its allow attribute are gone; behaviour is unchanged.
- **Keyboard-shortcut help dialog now lists every action.** The in-app shortcuts dialog (Ctrl+?) was a hand-maintained list that had drifted from the keybinding registry: it was missing SSH Tunnel Manager (Ctrl+T), Open Primary Menu (F10), the Ctrl+W close-tab alias, and the font-zoom shortcuts (Ctrl+Plus / Ctrl+Minus / Ctrl+0). Those are now included, and a new unit test (`registry_accelerators_are_documented_in_dialog`) fails if a future registry entry is ever left undocumented.

### Fixed

- **`rustconn-cli update` no longer relies on `unwrap()`.** Locating the connection index after `find_connection` used `.position(...).unwrap()`; it now returns a `CliError::Config` instead of panicking in the (logically unreachable) case where the connection vanishes between lookups, satisfying the project's no-`unwrap` rule.

### Documentation

- **Corrected keyboard shortcuts in the User Guide.** "Create Group" was documented as Ctrl+Shift+N (which actually opens New Connection (Advanced)); the correct shortcut is **Ctrl+Shift+G**. Removed the non-existent **Ctrl+K** "Search" binding (only Ctrl+F is registered) and added the missing **Ctrl+Shift+B** (Toggle Split Broadcast) row.
- **Zero Trust guide synced with the implementation.** Bumped the version header to 0.15.2; corrected the Generic Command section — the command template is run verbatim through `sh -c` and is **not** processed for `${host}`/`${user}`/`${port}` placeholders (the previous text wrongly promised substitution); and fixed the Hoop.dev Flatpak section — `~/.hoop/` is **not** mounted by default (the permission was rejected by Flathub lint), so access must be granted manually via `flatpak override` or supplied through the `HOOP_*` environment variables. The misleading placeholder comment on `GenericZeroTrustConfig` in `rustconn-core` was corrected to match.
- **Bitwarden setup guide corrected.** Removed a duplicated paragraph in the install section; the "Architecture Notes" now describe the actual vault layout (item name `RustConn: rustconn/<connection-name>`, URI `rustconn://<lookup-key>`, notes holding only the connection's domain as a plain string — not JSON, and no key passphrase); and the troubleshooting command now uses the correct keyring attribute (`secret-tool search application rustconn`, not `service`).

### Dependencies

- **Updated**: cc 1.2.62→1.2.63, cfg-expr 0.20.7→0.20.8, shlex 1.3.0→2.0.1, target-lexicon 0.13.3→0.13.5 (transitive build dependencies; no API impact)

## [0.15.1] - 2026-05-29

A focused fix for the Flatpak language-switch bug ([#158](https://github.com/totoshko88/RustConn/issues/158)).

### Fixed

- **Flatpak: language switch had no effect for any of the 16 translations.** Translations were installed under `/app/share/locale/<lang>/`, which `flatpak-builder` automatically splits into per-language subsets of `org.gnome.Platform.Locale`. The host pulls in only the subset matching the user's system locale, so a user with `LANG=en_US` literally had no `rustconn.mo` file in the sandbox and could not switch the UI language to anything else, no matter what was selected in Settings. Translations now install to `/app/share/rustconn/locale/` (a path `flatpak-builder` does not touch) and the manifest sets `--env=LOCALEDIR=/app/share/rustconn/locale`. The existing `i18n::locale_dir()` resolution order already honors `LOCALEDIR` first, so no application code changed.
  - Affects the published Flathub manifest, the GitHub release manifest, and the local development manifest.
  - Verified: a `find /app -name 'rustconn.mo'` in the sandbox now lists all 16 languages instead of zero.

### Dependencies

- **Updated**: mio 1.2.0→1.2.1, socket2 0.6.3→0.6.4

## [0.15.0] - 2026-05-27

A focused quality-and-cleanup release. No new user-facing features — the goal is to retire technical debt and make the codebase easier to evolve. 

### Security

- **RDP RemoteApp no longer passes `/p:` on the command line.** RemoteApp launches now write a single-use `.rdp` args file in `$XDG_RUNTIME_DIR` with mode 0600 containing only the password switch, and FreeRDP reads it via `/args-from:file:<path>`. The temp file is removed on `Drop` even if the launcher panics or returns early. Closes the [#153](https://github.com/totoshko88/RustConn/issues/153) Known Issue from 0.14.10.

### Improved

- **`#[allow]` → `#[expect(reason = "…")]` migration.** Every clippy/compiler override now carries an explanatory `reason = "…"` string and uses `#[expect]` where the lint actively fires (around 350 sites across all three crates). Stale overrides surface as warnings during code review instead of silently accumulating. Workspace lint `clippy::allow_attributes_without_reason = "warn"` enforces the new rule, and the migration uncovered roughly 50 overrides whose underlying lint no longer triggers — those were dropped entirely.
- **Manual `Debug` impls for secret backends.** `BitwardenBackend`, `KeePassXcBackend`, `LibSecretBackend`, `MacOsKeychainBackend`, `OnePasswordBackend`, `PassBackend`, `PassboltBackend`, `SecretManager`, and `CredentialResolver` now render meaningful `Debug` output (backend kind, server address, whether a session is held) without leaking session keys or passwords. Each backend file ships a `debug_does_not_leak_secret` unit test.
- **CLI `# Errors` documentation.** Every public `cmd_*` function in `rustconn-cli/src/commands/` now lists the `CliError` variants it can return, satisfying `clippy::missing_errors_doc` for the package.
- **GUI spacing follows GNOME HIG.** All `set_margin_*` / `set_spacing` calls in `rustconn/src/` use the HIG steps (6 / 12 / 18 / 24 px); legacy 4 px values were rounded up to 6 px and 8 px values rounded up to 12 px (about 520 sites).
- **`EntryRowBuilder::new` requires pre-translated titles.** Callers now pass `&i18n("Kubeconfig")` instead of `"Kubeconfig"`, so `xgettext` extracts every UI label and translators see them in `po/*.po`. Previously several rows in Kubernetes / Serial / Telnet / Cluster / Smart Folder dialogs stayed untranslated forever.

### Fixed

- **Settings-save failures now surface a toast.** `update_settings` results in `rustconn/src/window/smart_folders.rs` and `rustconn/src/window/connection_actions.rs` are no longer dropped silently with `let _ = …`; recoverable errors log a `tracing::warn!` and the active window shows a toast so the user knows the change did not stick.

## [0.14.10] - 2026-05-27

This release focuses on hardening how passwords flow through the app, removing several places where plaintext credentials could linger on the heap longer than necessary, plus a few startup-robustness and HIG-compliance fixes.

### Security

- **Vault save paths now take `SecretString` directly.** `save_password_to_vault`, `save_group_password_to_vault` and `save_variable_to_vault` no longer accept `&str`. Callers in `edit_group.rs`, `connection_dialogs.rs`, `dialogs/connection/builders.rs` and the variable-setup dialog wrap the contents of password entries into `SecretString` (or `Zeroizing<String>`) immediately on capture, so plaintext is never stored in a long-lived `String`.
- **Backend deserializers wrap secrets at parse time.** Bitwarden, KeePassXC, Passbolt, RDM and libvirt responses deserialize their password fields straight into `Option<SecretString>` via a shared helper. Previously the JSON/XML parser would allocate a plain `String` on the heap and that value lingered until the caller manually wrapped it.
- **Bitwarden GUI unlock no longer logs `password_len`.** Logging the length of a master password narrows brute-force search space; replaced with a `has_password` boolean. The master password is held in `Zeroizing<String>` for the duration of the `bw unlock` invocation.

### Added

- **Close tab on clean session exit** — new switch in Settings → Terminal. When enabled, tabs auto-close after a clean shell exit (typing `exit` or `logout`, exit code 0) instead of showing the reconnect overlay. Disabled by default. ([#162](https://github.com/totoshko88/RustConn/issues/162))
- **`Ctrl+W` closes the active tab** — alongside the existing `Ctrl+Shift+W`. On macOS this maps to `Cmd+W` via GTK4's modifier translation. ([#162](https://github.com/totoshko88/RustConn/issues/162))
- **F10 opens the primary menu.** The header-bar burger button is now marked as the primary menu, so GTK4 binds F10 to it automatically — required by GNOME HIG.

### Improved

- **Graceful exit when Tokio runtime fails to start.** Instead of panicking, RustConn now prints a hint about ulimits / sandbox restrictions and exits with code 2.
- **Password generator surfaces RNG failures.** A failure inside `ring::SystemRandom::fill` (rare; possible in heavily restricted sandboxes) now returns `PasswordGeneratorError::RngError` instead of panicking.
- **`SyncManager::try_recv_export` returns `Option<Uuid>`** instead of `Result<Uuid, ()>` — the absence of a queued export is normal control flow, not an error.
- **Named timeout constants** for HTTP downloads (`HTTP_DOWNLOAD_TIMEOUT`), Bitwarden unlock (`BITWARDEN_UNLOCK_TIMEOUT`), and vault retrieval (`VAULT_RETRIEVE_TIMEOUT`) replace eight scattered `Duration::from_secs(...)` literals.
- **Doc comments**: added `# Errors` sections to the public KeePass helpers in `secret/status.rs` (`get_password_from_kdbx_with_key`, `get_password_from_kdbx_exact`, `verify_kdbx_credentials`, `validate_key_file_path`) and to `vault_ops::save_variable_to_vault`.
- **`Debug` for protocol handlers and small types** — derived for the eleven zero-sized `*Protocol` handlers, `SshTunnelParams`, `TunnelPreviewParams`; manual implementation for `BusyStack` / `BusyGuard` that does not expose the internal callback.

### Fixed

- **macOS: SSH fails with `ssh-askpass: No such file or directory`.** SSH was trying to invoke the XQuartz askpass binary at `/usr/X11R6/bin/ssh-askpass` even though RustConn handles password input natively via VTE injection. The terminal environment now sets `SSH_ASKPASS_REQUIRE=never` on macOS to suppress external askpass invocation. ([#161](https://github.com/totoshko88/RustConn/issues/161))

### Dependencies

- `displaydoc` 0.2.5 → 0.2.6
- `hyper` 1.9.0 → 1.10.0
- `libredox` 0.1.16 → 0.1.17
- `memchr` 2.8.0 → 2.8.1
- `toml_edit` 0.25.11 → 0.25.12
- `zerocopy` 0.8.48 → 0.8.49
- `zerocopy-derive` 0.8.48 → 0.8.49

### Known Issues

- **RDP RemoteApp passed the password via the `/p:` cmdline argument**, visible in `/proc/PID/cmdline` to other processes of the same uid. Tracked in [#153](https://github.com/totoshko88/RustConn/issues/153) and **fixed in 0.15.0** via single-use args files in `$XDG_RUNTIME_DIR`.

## [0.14.9] - 2026-05-26

### Added

- **Server Manager** quick action in the RDP embedded toolbar (Windows admin tools menu).

### Improved

- Windows admin tools menu is now grouped with a separator: quick shortcuts (Settings, Task Manager) are separated from admin consoles (sorted alphabetically).

### Fixed

- Split view: panels with sessions moved via Select Tab lost their content ("Loading...") on subsequent splits — terminal was not stored in the internal map for restoration after `rebuild_widgets()`.
- RemoteApp: sessions now correctly use external FreeRDP mode instead of embedded wlfreerdp — RAIL protocol requires its own window management which is incompatible with Wayland subsurface embedding. ([#153](https://github.com/totoshko88/RustConn/issues/153))
- RemoteApp: use `xfreerdp3` via `flatpak-spawn --host` in Flatpak sandbox since only xfreerdp supports RAIL/RemoteApp (wlfreerdp and sdl-freerdp do not). ([#153](https://github.com/totoshko88/RustConn/issues/153))
- RemoteApp: updated `/app:` argument format to FreeRDP 3.x syntax (`/app:program:<path>,cmd:<args>,name:<name>`). ([#153](https://github.com/totoshko88/RustConn/issues/153))
- RemoteApp: force NTLM authentication (`/auth-pkg-list:ntlm`) for host-launched xfreerdp3 to avoid Kerberos realm misconfiguration on standalone servers. ([#153](https://github.com/totoshko88/RustConn/issues/153))
- RemoteApp: pass password via `/p:` argument for RemoteApp sessions (FreeRDP 3.x `/from-stdin` requires interactive input incompatible with pipe-based stdin). ([#153](https://github.com/totoshko88/RustConn/issues/153))

### Known Issues

- RemoteApp (RAIL) windows do not appear on Wayland sessions due to an upstream xfreerdp3 bug — RAIL app windows fail to create via XWayland (`xf_Pointer: Invalid appWindow`). Workaround: use an X11 session or full desktop RDP instead. ([FreeRDP#8071](https://github.com/FreeRDP/FreeRDP/issues/8071), [FreeRDP#12485](https://github.com/FreeRDP/FreeRDP/issues/12485))

## [0.14.8] - 2026-05-26

### Added

- **Compact interface mode** — toggle in Settings → Interface reduces header bar height to 32 px, tab bar to 28 px, and button padding throughout. Saves ~28-32 px of vertical space. ([#157](https://github.com/totoshko88/RustConn/issues/157))
- **Split-view broadcast toggle** — header-bar button mirrors keystrokes from any panel to all other panels in the same split. Hidden when no split is active. Shortcut: `Ctrl+Shift+B`. ([#160](https://github.com/totoshko88/RustConn/issues/160))

### Improved

- Broadcast toggle gets accent background + pulse animation when active; one-shot discoverability toast on first eligible split.
- Tray icon uses a dedicated SVG with cream halo for visibility on dark KDE/Plasma panels. ([#157](https://github.com/totoshko88/RustConn/issues/157))

### Fixed

- Broadcast toggle now appears after panels filled via Select Tab (uses `active_session_count()` instead of `session_count()`).
- Broadcast no longer doubles every typed character (shared `broadcast_busy` re-entrancy guard across all wired handlers).
- Broadcast rewired around split view instead of clusters — works for any terminal session, not just cluster members. ([#160](https://github.com/totoshko88/RustConn/issues/160))
- Broadcast wires new sessions placed via Select Tab while broadcast is already on (previously required toggle off/on).
- Language change now applies in Flatpak — only `LANGUAGE` is passed across re-exec; `LC_MESSAGES` inherited from host. ([#158](https://github.com/totoshko88/RustConn/issues/158))
- Cluster dialog no longer shows defunct "Broadcast mode" switch; CLI `--broadcast` flag emits deprecation warning.

### Dependencies

- `http` 1.4.0 → 1.4.1
- `log` 0.4.29 → 0.4.30

## [0.14.7] - 2026-05-25

### Added

- **Visual Tunnel Builder** — 3-step wizard dialog for creating and editing SSH tunnels with a visual path diagram (localhost → bastion → target), SSH command preview, and real-time status indicators. Step 1: select SSH connection, name the tunnel, optionally configure jump host. Step 2: add port forwarding rules (Local -L, Remote -R, Dynamic -D), configure auto-start/reconnect. Step 3: review configuration, see generated SSH command, copy to clipboard. Replaces the previous flat dialog in the Tunnel Manager window.
- **Keyboard passthrough mode** — press Ctrl+Shift+Backspace to disable all application shortcuts and pass keys directly to the remote session (VTE terminal, embedded RDP/VNC/SPICE viewer). Essential for using nvim, tmux, or other TUI apps that conflict with RustConn keybindings. Toggle via shortcut, menu (☰ → Keyboard Passthrough), or command palette. Only Quit (Ctrl+Q), Fullscreen (F11), and the passthrough toggle itself remain active. A toast notification confirms the mode change. ([#159](https://github.com/totoshko88/RustConn/issues/159))

### Improved

- **Visual Tunnel Builder: live diagram on Step 1** — the path diagram now updates in real time when changing the SSH connection or jump host selection, providing immediate visual feedback of the tunnel chain
- **Visual Tunnel Builder: live diagram on Step 2** — the path diagram now updates automatically when adding, editing, or deleting port forwarding rules, reflecting the first rule's direction and target
- **Visual Tunnel Builder: validation before Review** — Step 2 now validates all port forwarding rules (remote host required for Local/Remote) before allowing navigation to Step 3
- **Visual Tunnel Builder: "Add Forward" button placement** — moved to `PreferencesGroup` header suffix (GNOME HIG pattern) for better discoverability
- **Visual Tunnel Builder: accessibility** — status changes now announce both the tunnel path and status to screen readers (e.g., "Tunnel: localhost:8080 → bastion → target:80. Status: Running"); diagram container uses `Role::Img` for correct AT semantics
- **Visual Tunnel Builder: empty state** — replaced manual icon+label layout with `adw::StatusPage` for consistent GNOME HIG empty state presentation
- **Visual Tunnel Builder: UTF-8 safe error truncation** — error messages in tooltip are now truncated by character count instead of byte offset, preventing potential panics on multi-byte text
- **Keyboard passthrough: persistent header bar indicator** — a warning-styled "Passthrough" button now appears in the header bar while passthrough mode is active, providing persistent visual feedback and one-click disable; complements the existing toast notification
- **VNC client: connection timeout** — embedded VNC client now respects the configured `timeout_secs` (default 30s) instead of relying on OS-level TCP timeout which could hang for 2+ minutes on unreachable hosts
- **Document encryption: key zeroization** — derived encryption keys are now wrapped in `Zeroizing<[u8; 32]>` ensuring automatic memory clearing on drop (defense-in-depth hardening)
- **Welcome screen: streamlined shortcuts** — combined "Split vertical" and "Split horizontal" into a single entry; added keyboard passthrough shortcut for discoverability

## [0.14.6] - 2026-05-23

### Improved

- **Variables dialog: collapsible rows** — each variable is now shown as a collapsible expander displaying only the name; when adding a new variable, all existing rows collapse and the new one opens expanded with focus on the name field; improves usability with many variables
- **Variables dialog: duplicate name validation** — saving is blocked when two or more variables share the same name (case-insensitive); duplicate entries are highlighted with error styling and auto-expanded so the user can fix them
- **RDP toolbar: deduplicated admin menu** — removed PowerShell and CMD from the quick actions menu (⋮) since they are already available in Shell Launchers (+); replaced with Registry Editor and Device Manager for better coverage of Windows admin tools without duplication
- **RDP scripts: instant clipboard paste** — scripts/snippets are now sent via clipboard + Ctrl+V instead of character-by-character autotype; a 2000-char script that took ~16s now executes instantly; controlled by `script_paste_via_clipboard` in RDP connection config (enabled by default)
- **Snippet delivery mode** — each snippet now has a "Delivery" setting (Auto / Clipboard / Autotype) allowing per-snippet control over how it's sent to the remote session; "Clipboard" is instant for large scripts, "Autotype" is reliable for Citrix or legacy consoles

### Dependencies

- **Updated**: rpassword 7.5.2→7.5.3

## [0.14.5] - 2026-05-22

### Added

- **RDP Scripts v2: Shell Launchers + Autotype** — redesigned the "Scripts" dropdown in the RDP toolbar: now split into "Shell Launchers" (PowerShell, PowerShell Admin, CMD, CMD Admin — open shells via Win+R) and "Scripts" (user snippets sent via autotype into the already-open shell); removes timing hacks — the user controls when the shell is ready.
- **Snippet target platform** — snippets can now be marked as "Terminal", "Windows", or "Any"; the terminal context menu hides Windows-only snippets, and RDP sessions show only Windows-compatible ones

### Improved

- **Edit Group dialog redesigned** — the single-page Edit Group form is now split into 5 tabs: Identity, SSH Inheritance, Cloud Sync, Dynamic Folder, and Automation; Cloud Sync tab auto-hides for non-root groups
- **Connection dialog adapts to narrow windows** — the New/Edit Connection dialog now responds to window width and keeps content readable on small screens
- **Connection Wizard: Security Key (FIDO2)** — added "Security Key (FIDO2)" authentication option in the wizard for SSH, Mosh, and SFTP protocols
- **Connection Wizard: back navigation** — each wizard page now has its own header bar with a back button (GNOME HIG); dialog height increased to reduce scrolling
- **Redundant Cancel buttons removed** — dialogs that already close on Escape no longer show a separate Cancel button (password, document, snippet variable dialogs)
- **Color scheme selector modernized** — the Light/Dark/System toggle in Settings now uses the native libadwaita toggle group widget (or a combo row on older versions)
- **Dialog sizes unified** — all dialogs now use consistent dimensions; fixed minimum-size warnings on some screens
- **All dialogs migrated to modern adaptive style** — 25+ dialogs (Connection, Snippets, Templates, Clusters, Recordings, Import, Export, Statistics, Variables, Smart Folder, Password Generator, Sessions, Shortcuts, Terminal Search, Documents, Groups, SSH Agent passphrase) now support bottom-sheet on narrow screens, close on Escape, and drag-to-close
- **`--window-mode` CLI flag scoped to RDP/VNC/SPICE** — using `--window-mode` with protocols that don't support it (SSH, Telnet, etc.) now shows a warning instead of silently ignoring the value; SPICE added to supported protocols
- **SSH Agent passphrase no longer written to disk** — the askpass helper now passes the passphrase via an environment variable instead of a temporary file; safe on copy-on-write filesystems (btrfs, APFS, ZFS)
- **Bitwarden CLI resilience** — `--nointeraction` flag added to all `bw` commands to prevent hangs in sandboxed environments; 30-second timeout on all CLI calls; session key trusted without `bw status` verification (workaround for Bitwarden CLI v2026.4.x reporting "locked" despite valid session)

### Fixed

- **Quick Connect: user's color theme not applied ([#156](https://github.com/totoshko88/RustConn/issues/156))** — new connections from Quick Connect now use the configured terminal theme instead of always defaulting to "Dark"
- **Bitwarden: vault unlock not persisted across operations** — `mark_verified()` was missing after successful unlock via saved password or keyring, causing subsequent `retrieve`/`store` calls to re-check `bw status` (which incorrectly reported "locked" on Bitwarden CLI v2026.4.1); session key is now trusted immediately after unlock
- **Bitwarden: "Open Settings" button in backend-missing dialog** — fixed `ActionGroupExt` trait error; now uses `WidgetExt::activate_action` with correct `win.settings` prefix

### Dependencies

- `js-sys` 0.3.98 → 0.3.99
- `wasm-bindgen` 0.2.121 → 0.2.122
- `wasm-bindgen-futures` 0.4.71 → 0.4.72
- `wasm-bindgen-macro` 0.2.121 → 0.2.122
- `web-sys` 0.3.98 → 0.3.99

## [0.14.4] - 2026-05-20

### Added

- **CLI: `history`, `pin`/`unpin`, `tag`, `move`, `monitor` commands** — full set of connection management commands (history, favorites, tags, group moves, monitoring)
- **CLI: `import --auto` / `--dry-run`** — auto-detect sources (Asbru, Remmina, SSH config) and preview imports without saving
- **CLI: `export --csv-delimiter`, `--csv-fields`** — customize CSV export format
- **CLI: `add`/`update` — full GUI parity** — all advanced fields for SSH, RDP, VNC, SPICE, MOSH, Serial; metadata (`--tags`, `--group`, `--domain`, `--skip-port-check`)
- **Config file locking** — exclusive advisory lock (`fs2`) on write; GUI + CLI no longer conflict
- **SSH agent: `add_key()` accepts `&SecretString`** — intermediate strings wrapped in `Zeroizing`
- **Quick Connect: history persisted across sessions** — up to 15 entries in `config.toml`, no passwords
- **RDP Quick Actions: 3 new Windows admin tools** — Disk Management (`diskmgmt.msc`), Resource Monitor (`resmon`), Computer Management (`compmgmt.msc`); 9 total actions in the "⋮" menu

### Fixed

- **Settings: Azure/gcloud/OCI CLI not detected in Flatpak** — relevant env vars now passed through
- **Settings: `.version` file fallback** for CLI tools installed via Components dialog
- **Settings: SSH/RDP/Waypipe version strings** — parser now extracts version token only
- **Command Palette: "New Group"** — fixed shortcut display Ctrl+Shift+N → Ctrl+Shift+G

### Improved

- **Pre-connect probe bypass: cleanup** — replaced inline checks with `conn.bypasses_direct_probe()`
- **Property tests: +8 tests** in `connection_probe_tests.rs` (VNC/SPICE/RDP/SFTP jump_host, invariant)
- **vault_ops: deduplicated `collect_descendant_groups`** — replaced O(n²) recursive local function with `rustconn_core::models::collect_descendant_group_ids()` (O(n) BFS with parent→children index)

### Dependencies

- `fs2` 0.4 (new — advisory file locking)
- `either` 1.15.0 → 1.16.0


## [0.14.3] - 2026-05-20

### Fixed

- **Settings: removed duplicate group titles above collapsible sections** — `PreferencesGroup` title was visually duplicated with the `ExpanderRow` title for System Tray, Session Restore, Logging, and Highlight Rules sections; removed the redundant group-level titles per GNOME HIG (ExpanderRow already carries the section label)
- **CLI: `secret set --password` wrapped in `Zeroizing` immediately** — the plain `String` from argv is now wrapped in `zeroize::Zeroizing` as soon as clap parses it, minimizing heap lifetime; intermediate strings before `SecretString::from()` are zeroed on drop; added `--password-stdin` flag as the recommended secure alternative (reads one line from stdin pipe, never appears in `/proc/cmdline`); `--password` is deprecated with a runtime warning 

### Improved

- **External window: migrated to libadwaita** — `external_window.rs` now uses `adw::ApplicationWindow` + `adw::ToolbarView` + `adw::HeaderBar` instead of plain `gtk4::ApplicationWindow` + `gtk4::HeaderBar`; consistent with the rest of the application, inherits Adwaita styling and color scheme support
- **Settings: collapsible sections (GNOME HIG)** — wrapped secondary settings groups into `AdwExpanderRow` to reduce visual clutter on overloaded pages; collapsed by default, users expand only what they need:
  - Terminal tab: Logging section (7 controls) collapsed into a single "Session Logging" expander
  - Interface tab: System Tray (2 controls) and Session Restore (3 controls) collapsed into expanders
  - Interface tab: Keybindings — each category (Terminal, Navigation, Tabs, etc.) is now a collapsible expander inside a single "Keyboard Shortcuts" group instead of separate full-width groups
  - Highlight Rules already used an expander (unchanged)
- **Edit Connection: collapsible sections (GNOME HIG)** — wrapped secondary settings groups in the connection dialog into `AdwExpanderRow` to reduce visual clutter:
  - Advanced tab: Terminal Theme, Activity Monitor, Automatic Reconnection, Highlight Rules, and Wake On LAN collapsed into expanders; Remote Monitoring, Session Recording, and Connection Behavior remain as single-toggle groups
  - Automation tab: Pattern Tester, Pre-Connect Task, and Post-Disconnect Task collapsed into expanders; Expect Rules list remains expanded as the primary function
- **Settings: credential storage as a 3-state ComboRow** — replaced the per-backend pair of "Save password" + "Save to system keyring" CheckButtons (with hand-rolled mutual-exclusion logic) with a single `AdwComboRow` offering three canonical choices: "Don't save" / "Encrypted file (machine-specific)" / "System keyring (recommended)". Applies to all four secret backends: KeePassXC database password, Bitwarden master password, 1Password service-account token, and Passbolt GPG passphrase. Settings storage layout is unchanged — the previous `*_password_encrypted` and `*_save_to_keyring` fields are retained as the persistence sink, with a `CredentialStorage` enum and `*_storage()` / `set_*_storage()` helpers in `rustconn-core` providing the canonical API. Old configs round-trip through the new selector without a migration step
- **Settings UI: GNOME HIG compliance** — converted 25 toggle controls from `CheckButton`-in-`AdwActionRow` pattern to `AdwSwitchRow` across Interface, Terminal, Logging, and Monitoring pages; switches are the canonical libadwaita widget for boolean preferences and provide better touch targets, larger hit areas, and consistent rendering across themes; no behavioural changes 
  - Interface tab: 7 toggles (Color tabs by protocol, Show protocol filters, Remember size, Show tray icon, Minimize to tray, Session restore Enabled, Ask first)
  - Terminal tab: 8 toggles (Scroll on output / keystroke, Scrollbar, Hyperlinks, Hide pointer, Bell, SFTP via mc, Copy on select)
  - Logging tab: 4 toggles (Activity, User Input, Terminal Output, Timestamps)
  - Monitoring tab: 6 toggles (CPU / Memory / Disk / Network / Load / System info usage)
- **Settings dialog field types unified** — internal `SettingsDialog` struct fields renamed/retyped from `gtk4::CheckButton` to `adw::SwitchRow` for the migrated controls

### Dependencies
- `pastey` 0.2.2 → 0.2.3

## [0.14.2] - 2026-05-19

### Added

- **Centralized pre-connect probe bypass** — `Connection::bypasses_direct_probe()` and `Connection::should_pre_connect_check()` in `rustconn-core`; single source of truth for jump host, RDP Gateway, SSH ProxyCommand, and SPICE proxy detection; replaces 5+ scattered inline checks; SPICE proxy now correctly skips port check (previously missed)
- **Auto-reconnect: attempt N/M in banner** — "Auto-reconnecting (attempt 2/5)" with live updates via background→UI channel

### Fixed

- **CLI: `add --protocol web` port always 0** — Web branch now uses resolved port (default 443 or `--port`)
- **CLI: SecureCRT export/import missing** — added `secure-crt` format to both `ExportFormatArg` and `ImportFormatArg`
- **CSV import: silent port overflow** — values > 65535 or 0 now skip the row with a warning instead of silently using default
- **Credential memory: backend passwords as plain String** — `get_bw_password_from_keyring()` and 3 similar functions now return `SecretString` directly
- **Tooltip: "New Group (Ctrl+Shift+N)"** — corrected to actual shortcut Ctrl+Shift+G

### Improved

- **i18n: 15 new translatable strings** — connection dialog validation (10), "Ctrl+Alt+Del" button, AlertDialog "OK", search syntax help popover (6 lines), auto-reconnect attempt progress
- **Documentation: CLI_REFERENCE.md** — version header updated to 0.14.2

### Dependencies
- `asn1-rs` 0.7.1 → 0.7.2
- `num-conv` 0.2.1 → 0.2.2
- `tar` 0.4.45 → 0.4.46
- `tower-http` 0.6.10 → 0.6.11

## [0.14.1] - 2026-05-18

### Added

- **Predefined connection templates** — 20 built-in templates for common CLI tools (RustDesk, Docker, Podman, LXC, Incus, Distrobox, Virsh, Proxmox, IPMI, Picocom, WireGuard, Teleport, Ansible, and more) with emoji icons, grouped into 6 categories: Remote Desktop, Containers, Virtualization, Hardware, Cloud Access, Automation
- **Template grid in Connection Wizard** — Custom Command mode (Step 2) now shows a grid of template buttons below the command field; user ZeroTrust templates appear first, predefined fill remaining slots; "More…" button opens a popover with all templates grouped by category
- **Template icon field** — templates now support a custom icon (emoji or GTK icon name); shown in Manage Templates list and inherited by connections created from the template
- **Icon inheritance** — connections created via wizard template buttons automatically inherit the template's emoji icon for sidebar display
- **Per-connection "Skip port check" toggle** — new switch in the Advanced tab → Connection Behavior section to bypass the pre-connect TCP probe for the selected connection; useful for low-bandwidth links and hosts only reachable through an RDP Gateway or jump host (#153)

### Fixed

- **"Use Template" freezes UI** — clicking "Use Template" in the template manager opened the new-connection dialog as a modal child of the manager window, which was immediately closed; the orphaned modal blocked all input. The dialog is now parented to the main application window (#155)
- **RDP Gateway: "Host unreachable" before connection** — the pre-connect port check probed the target RDP host directly even when an RDP Gateway was configured, so connections through corporate gateways always failed with "Connection failed. Host unreachable.". The check is now skipped automatically when `RdpConfig.gateway` is set, mirroring the existing jump-host behaviour, so FreeRDP can route via `/g:gateway:443` as expected (#153)
- **Highlight overlay: colored underlines persisted after `clear`** — overlay was reading rows from absolute buffer position 0 instead of the visible viewport, so after `clear` (which pushes the previous screen into scrollback) the overlay re-rendered highlights from the now-hidden scrollback lines on top of the cleared viewport pixels; fixed by anchoring the read range to `vadjustment.value()` (#154)

### Dependencies
- **Updated (Flatpak)**: vte 0.80.3→0.80.5, freerdp 3.25.0→3.26.0

## [0.14.0] - 2026-05-18

### Added

- **Connection Wizard (Ctrl+N)** — new step-by-step dialog for creating connections; 3 steps: protocol selection → connection details → authentication/finish; all 11 protocols supported; "Advanced…" escape hatch on every step opens the full ConnectionDialog; "Save" and "Save & Connect" final actions (#0140)
  - **Step 1: 4-column protocol grid** — protocols grouped into Secure Shell, Remote Desktop, Terminal, Other columns with icon + label + descriptive subtitle for each protocol
  - **Step 2: Adaptive fields** — form adapts per protocol (host/port/username for SSH, device/baud for Serial, pod/namespace for Kubernetes, provider fields for Zero Trust, URL for Web)
  - **Step 3: Auth + Appearance** — SSH auth method selection (password/key/agent), VTE color profile for terminal protocols, connection icon
  - **Jump Host** — SSH tunnel dropdown on Step 2 for SSH, MOSH, SFTP, RDP, VNC, SPICE
  - **VTE Color Profile** — terminal theme selector on Step 3 for VTE-based protocols; maps to per-connection `ConnectionThemeOverride`
  - **Wizard → Advanced pre-fill** — "Advanced…" transfers all entered data into the full ConnectionDialog
  - **Duplicate via Wizard** — "Duplicate via Wizard…" context menu in sidebar; pre-fills wizard from existing connection for clone & modify workflows
- **`win.new-connection-advanced` action** — opens the full ConnectionDialog directly (Ctrl+Shift+N)
- **Quick Connect runtime history** — last 15 sessions remembered during app lifetime; "Recent" section with type-ahead filtering; one-click fills protocol, host, port, username
- **Zero Trust: Custom Command shortcut** — dedicated button on Step 1 for running any CLI tool as a connection, without navigating Zero Trust provider list

### Fixed

- **Highlight overlay: colored underlines not removed by `clear`** — added `cursor-moved` signal as additional repaint trigger; `contents-changed` alone did not fire reliably for erase-display escape sequences, leaving ghost underlines until the next output (#154)
- **Wizard: Zero Trust provider fields lost** — `PartialConnection::to_connection()` now correctly maps all provider-specific fields for all 10 Zero Trust providers
- **Wizard: Mosh ssh_port ignored** — `MoshConfig::ssh_port` now populated from wizard's port field
- **Wizard: Serial baud rate defaulted to 115200** — baud rate selection (9600–460800) now correctly maps to `SerialBaudRate` enum
- **Quick Connect: port always shown in history** — now displays port only when it differs from protocol default

### Changed

- **Ctrl+N** — opens Connection Wizard instead of full ConnectionDialog
- **Wizard: `adw::Dialog`** — migrated from `adw::Window` for better focus management, bottom-sheet on narrow windows, auto close-on-Escape
- **Wizard Step 1: true 4-column layout** — horizontal columns with centered headers, equal spacing, buttons expand to fill vertical space; subtitles under each protocol for discoverability
- **Highlight Rules in Settings** — collapsed into `adw::ExpanderRow`, hidden by default
- **`build_zt_provider_config` refactored** — moved to `ZeroTrustProviderConfig::from_wizard_fields()` in `rustconn-core`; signature uses `Option<&str>` instead of `&Option<String>`

### Improved

- **Web protocol: URL validation** — "Next" disabled until valid URL entered; red border on invalid input


## [0.13.17] - 2026-05-16

### Added

- **Web bookmark connections** — new protocol type "Web" for storing website URLs with credentials; opens in the default browser via GTK4 UriLauncher (portal-aware, works in Flatpak); credentials stored in the configured secret backend (KeePassXC, Bitwarden, etc.) for copy-to-clipboard via context menu; no embedded browser — delegates to the system default; icon `web-browser-symbolic` in sidebar (#151)
- **RDP RemoteApp support** — launch individual remote applications instead of a full desktop session; configure program path, arguments, and display name in the connection dialog; automatically uses FreeRDP (IronRDP does not support RAIL protocol); imported from `.rdp` files (`remoteapplicationprogram`, `remoteapplicationcmdline`, `remoteapplicationname` fields) (#153)

### Fixed

- **Cloud Sync in Flatpak** — detect XDG Document Portal paths when selecting sync directory; show a warning dialog with `flatpak override` command instead of silently saving an unusable portal path; dialog body now instructs user to adjust the path if needed (#152)
- **Highlight overlay not cleared by `clear` command** — colored underlines and background highlights now disappear immediately when the terminal screen is erased; replaced 32ms timeout throttle with idle-based redraw and added whitespace-only line filtering (#154)

### Improved

- **RDP RemoteApp: FreeRDP availability warning** — the connection dialog now shows a warning row when RemoteApp is configured but no FreeRDP binary is detected on the system, preventing a confusing error at connect time
- **Documentation: RemoteApp** — added "RemoteApp (RAIL)" section to User Guide with configuration steps, program path format table, import notes, and limitations; updated "Supported .rdp Fields" list
- **Documentation: Cloud Sync portal detection** — added note about automatic XDG Document Portal path detection to the Flatpak Cloud Sync section
- **Documentation: Highlight overlay limitation** — documented that whitespace-only lines are excluded from highlight processing

### Dependencies
- **Updated**: libbz2-rs-sys 0.2.4→0.2.5, openssl 0.10.79→0.10.80, openssl-sys 0.9.115→0.9.116

## [0.13.16] - 2026-05-16

### Added — macOS Port

First macOS release with full native support:

- **Native PTY** — VTE `spawn_async` workaround via `openpty()` + `Pty::foreign_sync()` with job control
- **Tray icon** (`tray-macos` feature) — NSStatusItem via `tray-icon` + `muda` with dynamic menu (Show/Hide, Recent, Quick Connect, Quit)
- **macOS Keychain** — native credential storage via Security.framework
- **DMG packaging** — automated build with bundled Adwaita icons, locales, GSettings schemas, ad-hoc code signing
- **Homebrew formula** — `brew tap totoshko88/rustconn && brew install rustconn`
- **PATH extension** — `.app` bundle injects Homebrew paths for CLI tool detection (KeePassXC, bw, op, etc.)
- **Platform-aware URL opener** — `open` on macOS, `xdg-open` on Linux

### Fixed — macOS

- **Tray icon** — main-thread init, Retina 44px template icon, diagnostic logging on failure
- **DMG bundle** — `cp -RL` resolves Homebrew symlinks; fixed iconset glob; wrapper exports full PATH
- **Icon theme** — `register_app_icon()` discovers bundle's `Resources/share/icons` and Homebrew paths
- **PTY** — child process cleanup (no zombies), handle race prevention via `std::mem::forget`
- **Secret detection** — unified `detection_command()` with extended PATH, removed invalid Cellar paths
- **X11 fallback** — skipped on macOS via `#[cfg(not(target_os = "macos"))]`

### Fixed — General

- **RDP error messages** — IronRDP CredSSP/NLA errors now show specific cause (invalid credentials, account locked, password expired) instead of generic "Connection failed"
- **Settings dialog width** — increased to 800px to prevent tab label truncation on localized builds
- **SFTP file manager ignores "Disable MC" setting** — in Flatpak, `is_flatpak()` override forced Midnight Commander regardless of user preference; now respects the saved `sftp_use_mc` toggle (default remains `true` in Flatpak for new installs); shows a warning toast when using external file manager in Flatpak since it cannot access the sandbox SSH agent

### Dependencies
- **Updated**: tray-icon 0.19→0.20 (fixes muda version conflict), winnow 1.0.2→1.0.3

## [0.13.15] - 2026-05-14

### Added
- **Local Shell: custom command** — new "Command" field in Settings → Terminal → Local Shell allows specifying a custom command to run instead of the default login shell (e.g. `fish`, `bash --norc`, `neofetch && bash`, environment setup scripts); applies only to Local Shell tabs, not remote connections

### Fixed
- **Split screen snippet execution** — snippets now execute in the focused pane of a split terminal tab instead of always targeting the first pane; uses per-session split bridge to resolve the correct focused session before sending text

### Improved
- **Dynamic snippet context menu** — when ≤5 snippets exist, they appear as individual items directly in the terminal right-click menu for one-click execution; when more than 5 exist, the previous "Execute Snippet…" picker is shown instead

### Dependencies
- **Updated**: aws-lc-rs 1.16.3→1.17.0, aws-lc-sys 0.40.0→0.41.0, kqueue-sys 1.1.1→1.1.2, kurbo 0.13.0→0.13.1

## [0.13.14] - 2026-05-13

### Added
- **Welcome page: Import button** — added "Import" action button alongside "New Connection" and "Quick Connect" on the welcome page, making it easier for users migrating from PuTTY, Remmina, mRemoteNG, or SecureCRT to get started
- **Template Manager: empty state** — added `adw::StatusPage` placeholder with icon and description when no templates exist, consistent with Recordings and History dialogs
- **Reconnect banner: auto-reconnect indicator** — when auto-reconnect is active, the disconnected session banner now shows "Auto-reconnecting…" status label so users know background polling is in progress

### Fixed
- **Credential memory safety** — intermediate password strings from `expose_secret().to_string()` are now wrapped in `zeroize::Zeroizing` across VNC, RDP, and document password flows; passwords are zeroed in memory on drop instead of lingering as plain `String`
- **Potential panic in resize debounce** — replaced `unwrap()` on `Instant::checked_sub()` with `unwrap_or_else` fallback in terminal resize handler (`window/mod.rs`)
- **CLI `show` command panic** — replaced `expect("json object")` with proper `let-else` error propagation in `rustconn-cli`
- **Port overflow in SecureCRT/libvirt importers** — replaced truncating `as u16` casts with `u16::try_from().ok()` fallback to default port; prevents silent corruption when imported files contain port values > 65535
- **Sync file path traversal** — added `validate_sync_filename()` that rejects absolute paths, `..` components, and directory separators in `sync_file` field; prevents writing outside the configured sync directory via crafted `.rcn` files

### Dependencies
- **Updated**: filetime 0.2.28→0.2.29, libbz2-rs-sys 0.2.3→0.2.4, open 5.3.4→5.3.5, zerofrom 0.1.7→0.1.8

## [0.13.13] - 2026-05-12

### Added
- **SSH ProxyCommand support** — new "ProxyCommand" field in the SSH connection dialog allows routing connections through a custom proxy (e.g., `ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p` for Tor `.onion` hidden services); skips pre-connect port check when set, since the destination is only reachable through the proxy (fixes [#146](https://github.com/totoshko88/RustConn/issues/146))

### Fixed
- **SSH Startup Command not executing** — the startup command configured in the SSH connection dialog was never appended to the SSH invocation in the GUI terminal; now correctly passed after `user@host` so the remote shell executes it immediately after login (fixes [#147](https://github.com/totoshko88/RustConn/issues/147))
- **SSH ProxyCommand port format** — jump hosts with non-standard ports now correctly use `-p port user@host` instead of invalid `user@host:port` inside `ProxyCommand` (fixes [#144](https://github.com/totoshko88/RustConn/issues/144))
- **RDP/VNC/SPICE tunnel through nested jump hosts** — SSH tunnel now resolves the full recursive jump host chain; previously only the immediate jump host was used, causing tunnel failure when the jump host itself required another jump host to be reachable

### Improved
- **StringInterner: `HashSet` instead of `HashMap`** — reduced memory overhead by 50% per entry (key and value were identical `Arc<str>`)
- **ConfigManager: cache `ensure_config_dir()` result** — skips filesystem check after first successful call, eliminates 1 syscall per debounced save
- **ConnectionManager: `collect_descendant_groups()` O(n) instead of O(n²)** — builds parent→children index instead of scanning all groups on each iteration
- **ConnectionManager: `sort_all()` refactor** — extracted `sort_ids_by_name()` helper, removed 4× duplicated sort-by-lowercase pattern (~60 lines)
- **WolDialog: migrated to `adw::Dialog`** — better focus management, auto close-on-Escape, bottom-sheet behavior on narrow windows (GNOME HIG)

## [0.13.12] - 2026-05-11

### Added
- **Auto-reconnect: per-connection RetryConfig with exponential backoff** — new "Automatic Reconnection" section in the connection dialog Advanced tab; configurable retry behavior: enable/disable, max attempts (1–10), initial delay (100–30000ms), max delay (1000–120000ms); exponential backoff with 2× multiplier; `RetryConfig` serialized with `#[serde(default)]` for backward compatibility
- **Import: multi-file batch import** — new "Multiple Files (batch)" source in the Import dialog; select multiple files at once (CSV, RDP, VV, RCN, JSON, RTSZ, MobaXterm, XML, YAML); sequential import with per-file progress; `BatchImporter` for large sets (>10) with configurable batch sizes and cancellation
- **ExpectEngine: new methods for GUI integration** — `match_line()` with auto-trimming and priority; `remove_by_id(Uuid)`; `remove_expired()` and `remove_expired_individual()` for per-rule timeouts

### Improved
- **Import: success messages now use i18n_f()** for proper localization
- **AutomationSession uses ExpectEngine from core** — delegates to `ExpectEngine` for priority-sorted matching, duplicate ID detection, pattern validation, and timeout handling; `Trigger` struct removed
- **SplitView legacy UUID layer partially removed** — external consumers use `get_pane_session()` instead of `panes_ref_clone()`; `TerminalPane` reduced to `pub(crate)`; `panes_ref()`, `panes_ref_clone()` removed from public API
- **`performance/mod.rs` decomposed** — 2210-line monolith split into 10 submodules; public API unchanged
- **`cli_download.rs` decomposed** — 3391-line monolith split into 10 submodules; public API unchanged
- **MainWindow credential resolution extracted** — 9 methods (~920 lines) moved to `window/credentials.rs`
- **MainWindow session lifecycle extracted** — 5 methods (~760 lines) moved to `window/session_lifecycle.rs`; `window/mod.rs` reduced by 31%

### Fixed
- **SSH: identity key `-i` duplicated in command** — `build_command_args()` and `spawn_ssh()` both added `-i`; now deduplicated in both connect and reconnect paths
- **CSV import: panic on empty group path** — `resolve_group_path()` now returns `ImportError::InvalidEntry` instead of `expect()`
- **VNC/RDP/SPICE embedded: hangs when remote port unreachable through SSH tunnel** — `probe_tunnel_remote()` now verifies end-to-end connectivity after tunnel readiness; fails immediately with clear error instead of hanging
- **Terminal: per-connection white color displayed as grey** — `apply_theme_override_with_base()` now rebuilds full 16-color palette via `set_colors()`, replacing palette entries 7+15 and 0+8 ([#145](https://github.com/totoshko88/RustConn/issues/145))

### Dependencies
- `clap_complete` 4.6.4 → 4.6.5
- `kqueue-sys` 1.1.0 → 1.1.1
- `nix` 0.31.2 → 0.31.3

## [0.13.11] - 2026-05-10

### Improved
- **RDP: better diagnostics for IronRDP fallback to FreeRDP** — when the embedded IronRDP client encounters a protocol incompatibility (e.g. GNOME Remote Desktop sending unexpected PDU during capabilities exchange), the error detection now includes detailed comments explaining the upstream limitation (IronRDP connector 0.8.0 does not handle `ServerDeactivateAll` during `CapabilitiesExchange`); submitted fix upstream ([Devolutions/IronRDP#1253](https://github.com/Devolutions/IronRDP/issues/1253)); narrowed fallback detection patterns to avoid false positives on generic network errors (e.g. "unexpected end of stream" no longer triggers fallback)
- **Security: pre/post-connect tasks now use TaskExecutor from core** — pre-connect and post-disconnect automation tasks are now executed through `TaskExecutor` instead of raw `sh -c`; this adds: timeout enforcement (`timeout_ms` field is now respected — previously commands could hang indefinitely; timed-out processes are now killed instead of orphaned), environment sanitization (removes `BW_SESSION`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AWS_ACCESS_KEY_ID`, `OP_SESSION`, `GITHUB_TOKEN`, `GH_TOKEN` from child process to prevent credential leakage), variable substitution (`${var}` references in task commands are now resolved from both global and connection-scoped variables — `${host}`, `${port}`, `${username}`, `${name}` are available automatically), and conditional execution (`only_first_in_folder` / `only_last_in_folder` conditions are now evaluated correctly via a shared `FolderConnectionTracker` in `AppState`)
- **UX: pre-connect task failure toast now shows error details** — instead of a generic "Pre-connect task failed. Connection aborted." message, the toast now includes the specific error (e.g. "Task timed out after 5000ms", "Task failed with exit code 1") via `i18n_f()` for proper localization
- **Code quality: ConnectionDialog decomposed** — extracted `create_rdp_options` (640 lines), `create_vnc_options` (310 lines), `create_spice_options` (290 lines), and `create_zerotrust_options` (480 lines) from the monolithic `dialog.rs` (8746 → 6968 lines, −20%) into their respective protocol modules (`rdp.rs`, `vnc.rs`, `spice.rs`, `zerotrust.rs`); replaced dead-code placeholder implementations with the actual production code; all protocol tab modules now follow the same pattern as the existing `ssh.rs`, `telnet.rs`, `serial.rs`, `kubernetes.rs`

### Removed
- **Dead code: `ConnectionFallback` module removed from rustconn-core** — the generic fallback chain (`ConnectionFallback`, `ConnectionStrategy`, `FallbackError`, `StrategyAttempt`) was never integrated into the GUI; the RDP fallback (IronRDP → wlfreerdp → xfreerdp) uses a purpose-built ad-hoc chain that is tightly coupled to GTK widget lifecycle; the generic module added complexity without benefit and can be restored from git history if needed
- **Dead code: `VirtualScrollConfig` removed from public API** — `VirtualScrollConfig` was re-exported from `rustconn-core` but never imported by any consumer (GUI or CLI); removed from `lib.rs` and `connection/mod.rs` re-exports, reduced visibility to `pub(crate)` for internal module tests only
- **UI: removed "Window Mode" section from Advanced tab** — the Display Mode dropdown (Embedded/External/Fullscreen) and Remember Position checkbox were shown for all protocols but only worked for RDP and VNC; for SSH, Telnet, SPICE, Serial, Kubernetes, and Mosh it was completely ignored; for RDP/VNC it duplicated the existing "Client Mode" setting in the Protocol tab; the `window_mode` field remains in the data model for backward compatibility but is no longer editable from the dialog ([#130](https://github.com/totoshko88/RustConn/issues/130))

### Dependencies
- `clap_complete` 4.6.3 → 4.6.4
- `hybrid-array` 0.4.11 → 0.4.12

## [0.13.10] - 2026-05-09

### Added
- **Import/Export: SecureCRT session support** — import connections from SecureCRT's `Config/Sessions/` directory (individual `.ini` files); export connections back to SecureCRT INI format as a directory tree; supports SSH2, Telnet, RDP, VNC protocols with hostname, port, username, SSH key path, X11/agent forwarding, compression; folder hierarchy preserved as connection groups; available in GUI export dialog, CLI, and programmatic API ([#140](https://github.com/totoshko88/RustConn/issues/140))

### Fixed
- **Backup/Restore: global variables lost after restore** — restoring settings from a ZIP archive would show "Restored N files" but closing the settings dialog afterwards overwrote the restored `config.toml` with stale in-memory state (which had empty `global_variables`); the dialog now reloads `AppSettings` from disk immediately after a successful restore so that the in-memory state matches the restored file ([#142](https://github.com/totoshko88/RustConn/issues/142))
- **SSH: ControlMaster sockets now actually closed on application exit** — the shutdown handler scanned `active_sessions()` to find SSH connections needing socket cleanup, but by the time GTK's `connect_shutdown` fires, all sessions are already in `Terminated` state (GTK destroys widgets first), so the list was always empty and no sockets were ever closed; replaced with a filesystem scan of the runtime directory for `rc-*` socket files, which works regardless of session state; stale sockets that don't respond to `ssh -O exit` are force-removed ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **KeePass: custom entry path for variables ignored RustConn/ prefix** — when a user specified a custom "KeePass entry" path for a secret variable, the lookup still prepended `RustConn/` and tried fallback paths, making it impossible to reference entries at arbitrary locations in the database; added `get_password_from_kdbx_exact()` that queries the entry at the exact user-specified path without any prefix or fallback logic; default (no custom path) behaviour unchanged ([#143](https://github.com/totoshko88/RustConn/issues/143))
- **KeePass: "Variable Not Configured" dialog loop when database password not set** — the dialog appeared on every connection because: (1) `save_variable_to_vault` silently failed when KeePass couldn't unlock the database (`kdbx_password = None`), (2) the retry always re-read from KeePass which also failed, (3) the user's backend choice in the dialog dropdown was ignored; fixed by: respecting the user-selected backend from the dialog, showing an error toast when save fails instead of silently retrying, adding LibSecret fallback for both read and write when KeePass is unavailable and `enable_fallback` is enabled; the dialog now shows only the configured preferred backend + LibSecret as options ([#143](https://github.com/totoshko88/RustConn/issues/143))

### Dependencies
- `hashbrown` 0.17.0 → 0.17.1

## [0.13.9] - 2026-05-09

### Fixed
- **Flatpak: Zero Trust Generic commands now execute on host** — custom commands in the Generic Zero Trust provider were failing with "No command specified" because the double `sh -c` wrapping broke argument parsing for `flatpak-spawn`; now Generic commands in Flatpak are automatically wrapped with `flatpak-spawn --host -- script -qfc '...' /dev/null` (same PTY-allocating approach as Local Shell), so host-side binaries resolve correctly without manual `flatpak-spawn` prefixes; single quotes in command templates are properly escaped ([#132](https://github.com/totoshko88/RustConn/issues/132))
- **Split View: focus border not updating on click** — clicking a terminal pane in split view would send input to the correct terminal but the focus border (colored outline) remained on the previously focused pane; root cause: each call to `setup_all_panel_click_handlers` added a new `GestureClick` controller without removing the previous one, causing duplicate gesture controllers to compete for the click event; the first (stale) handler would claim the event before the current handler could update focus styling; fixed by removing existing primary-button gesture controllers before adding a new one
- **Zero Trust Generic: skip vault credential lookup** — connecting via Generic Command no longer triggers a ~14-second Bitwarden/KeePass vault lookup; the custom command handles its own authentication interactively in the terminal, so vault resolution is unnecessary and was only adding startup delay ([#132](https://github.com/totoshko88/RustConn/issues/132))

### Improved
- **RDP: real disk space reported to Windows via shared folders** — the RDPDR backend now queries actual filesystem statistics using `nix::sys::statvfs` instead of returning hardcoded values; Windows Explorer and applications connected via shared folders now see correct total/available disk space; values are normalized to 4096-byte allocation units matching the reported sector geometry; graceful fallback to defaults if the statvfs call fails

### Dependencies
- `cc` 1.2.61 → 1.2.62
- `filetime` 0.2.27 → 0.2.28
- `quick-xml` 0.39.3 → 0.39.4
- `tokio` 1.52.2 → 1.52.3
- Added `nix` 0.31.2 (feature: `fs`) to rustconn-core for safe statvfs access

## [0.13.8] - 2026-05-08

### Fixed
- **Per-connection monitoring toggle not saving state** — the "Enable Monitoring" switch in the connection dialog Advanced tab always appeared enabled and did not persist the user's choice; root cause: `update_connection()` in `ConfigManager` unconditionally overwrote `monitoring_config` with the old value from the existing connection (`updated.monitoring_config = existing.monitoring_config.clone()`), discarding the user's change from the dialog; additionally, the save logic stored `None` when the toggle was ON (meaning "use global default") instead of an explicit override, so per-connection enable didn't work when global monitoring was disabled; fixed both: removed the overwrite in `update_connection`, and save now stores explicit `Some(true)` / `Some(false)` so the per-connection toggle correctly overrides the global setting in both directions ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **Flatpak: CLI tools not found by protocol detection** — `which_binary()` in protocol detection only checked `/app/bin` and system PATH; CLI tools installed via Flatpak Components (e.g. hoop, boundary, tailscale) in `~/.var/app/io.github.totoshko88.RustConn/cli/` were not discovered; now searches all `get_cli_path_dirs()` directories and passes the full resolved path to version detection
- **Hoop.dev CLI version not displayed** — `hoop version` outputs JSON (`{"version":"1.59.3",...}`) which the generic version parser did not recognize; added `parse_json_version()` to extract the version field from JSON output; also fixed `get_version()` to set extended PATH in Flatpak so the binary can be executed from its install directory

## [0.13.7] - 2026-05-08

### Improved
- **Flatpak CLI: automatic version resolution for 7 components** — Tailscale, kubectl, Teleport, Boundary, Hoop.dev, Bitwarden CLI, and 1Password CLI no longer use hardcoded version URLs; instead, the latest version is detected at install/update time from upstream APIs (Kubernetes stable.txt, GitHub releases, HashiCorp checkpoint, Tailscale packages page, Hoop latest.txt, AgileBits update API); this eliminates the need to manually bump versions in source code and ensures users always get the latest release; `scripts/check-cli-versions.sh` updated to verify endpoint reachability instead of comparing pinned versions

### Fixed
- **SSH: monitoring no longer triggers a second agent confirmation** — monitoring now waits up to 5 seconds for the main session's ControlMaster socket to appear, then connects as `ControlMaster=no` (slave only); previously monitoring used `ControlMaster=auto` which could race with the main session and create a separate master connection, causing a second Bitwarden/SSH agent prompt; falls back to creating its own master only if the socket never appears ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **SSH: ControlMaster sockets cleaned up on application exit** — all SSH ControlMaster sockets are now gracefully closed (`ssh -O exit`) when the application shuts down; previously sockets lingered in the filesystem until `ControlPersist` timeout expired ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **SSH: control socket path shortened for macOS compatibility** — the ControlPath format changed from `rustconn-ssh-{host}-{port}-%r` to `rc-{host}-{port}-%r`; on macOS, `/tmp` is used instead of `$TMPDIR` (which is ~52 chars under `/var/folders/...`) to stay within the 104-byte Unix socket path limit; long hostnames are truncated to 40 characters with `floor_char_boundary` for UTF-8 safety
- **Auto-reconnect: no longer loops infinitely on rapid crashes** — if a session crashes within 5 seconds of starting (e.g., SIGSEGV in VTE), auto-reconnect is skipped to prevent an infinite reconnect loop; previously a terminal crash would trigger immediate reconnect → crash → reconnect indefinitely at ~17ms intervals
- **Flatpak: local shell PTY resize now propagates to host** — when the VTE widget resizes, the new dimensions are forwarded to the host-side PTY via `flatpak-spawn --host -- stty rows R cols C` with 200ms debounce; previously the host shell retained its initial 24×80 size regardless of window resizing, causing incorrect line wrapping in bash, vim, htop, etc. ([#122](https://github.com/totoshko88/RustConn/issues/122))
- **Shutdown: socket cleanup limited to active sessions only** — previously attempted to close ControlMaster sockets for all saved SSH connections; now only closes sockets for sessions that were actually connected, using `futures::join_all` for parallel execution

## [0.13.6] - 2026-05-07

### Improved
- **Preferences: Monitoring moved to its own page** — monitoring settings (global enable, polling interval, visible metrics, activity monitor) are now on a dedicated "Monitoring" tab with the `utilities-system-monitor-symbolic` icon instead of being buried at the bottom of the Connection page; the subtitle on the global enable switch now clarifies that per-connection overrides are configured in the Advanced tab of the connection dialog ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **CLI: machine-readable output for AI agents and scripting** — `show`, `test`, and `stats` commands now accept `--format json|csv|table`; `group show` also supports `--format`; all default to JSON when stdout is piped (non-TTY); `list --format json` output now includes `description`, `group_id`, `tags`, `last_connected` fields; `test --format json` returns structured results with `latency_ms`, `pass_rate`, and per-connection details; `stats --format json` returns all metrics as a single JSON object; `group show --format json` includes child groups and connections list ([#132](https://github.com/totoshko88/RustConn/issues/132))
- **i18n: activity monitor mode and export format names now translatable** — `MonitorMode::display_name()` values ("Off", "Activity", "Silence") and `ExportFormat::display_name()` values ("Ansible Inventory", "SSH Config", etc.) are now wrapped in `i18n()` at call sites so they appear translated in the UI; previously these were hardcoded English strings passed as arguments to `i18n_f()` without translation
- **Security: intermediate password strings now zeroized on drop** — cached RDP/VNC credentials extracted via `expose_secret().to_string()` are now wrapped in `zeroize::Zeroizing<String>` so the plaintext is overwritten in memory when the variable goes out of scope; previously these intermediate `String` copies remained in heap memory until the allocator reused the page
- **Reliability: host check no longer panics on runtime creation failure** — `tokio::runtime::Runtime::new().expect()` in WoL/reconnect polling replaced with proper `Result` propagation via a new `HostCheckError::Io` variant; if the tokio runtime cannot be created (e.g. file descriptor exhaustion), the error is reported gracefully instead of crashing

### Fixed
- **SSH: single authentication prompt for connection + monitoring** — the main VTE terminal SSH connection now always uses `ControlMaster=auto` with a shared `ControlPath`; the monitoring subsystem reuses the same socket instead of opening a separate SSH session, eliminating the second key/passphrase/agent prompt that previously appeared when monitoring started; if the user already enabled ControlMaster manually in connection settings, the shared ControlPath is still injected so monitoring can find the socket ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **Repository hygiene: removed committed vim swap file and msgmerge backups** — `rustconn-core/src/.ssh_tunnel.rs.swp` removed from git tracking; `*.swp`, `*.swo`, `*.po~` patterns added to `.gitignore` to prevent future accidental commits

## [0.13.5] - 2026-05-07

### Added
- **Drag & Drop file paths into VTE terminals (SSH/Telnet)** — dragging files from a file manager onto a terminal session inserts their shell-escaped paths (single-quoted with `'\''` for embedded quotes), separated by spaces; matches GNOME Terminal behavior; visual highlight on the terminal when dragging over it; works with multiple files simultaneously ([#74](https://github.com/totoshko88/RustConn/issues/74))
- **Drag & Drop files to RDP clipboard (embedded IronRDP)** — dragging files onto an embedded RDP session announces them to the remote server via the CLIPRDR `FileGroupDescriptorW` channel (`CF_HDROP`); the server can then paste the files as if they were copied locally; includes a circuit breaker that auto-disables after 3 consecutive failures with an `adw::Toast` notification and "Try Again" button to re-enable; servers that don't support `STREAM_FILECLIP_ENABLED` are detected at capability negotiation and the feature is disabled gracefully ([#74](https://github.com/totoshko88/RustConn/issues/74))
- **RDP: "Reconnect on Resize" option** — new checkbox in the RDP connection dialog (Advanced → Features) that forces a full reconnect on window resize instead of using the Display Control Channel; useful for legacy RDP servers (Windows Server 2008/2012) that don't support MS-RDPEDISP, or when the server ignores dynamic resolution changes; disabled by default (dynamic resize is preferred)
- **Smart Folders: custom emoji icons** — smart folders now support a custom emoji icon displayed in the sidebar instead of the default 📁; set via the "Icon" field in the New/Edit Smart Folder dialog or `--icon` CLI flag (`rustconn-cli smart-folder create --icon "🚀"`); use "none" to clear in edit mode; the emoji is persisted in settings and shown in the sidebar folder row ([#133](https://github.com/totoshko88/RustConn/issues/133))

### Fixed
- **RDP: automatic reconnect on resize for servers without Display Control Channel** — when the server does not support MS-RDPEDISP (e.g. Windows Server 2008/2012/2016 without the Display Control extension), RustConn now automatically falls back to a full reconnect with the new resolution instead of leaving the session at the old resolution with distorted scaling; previously, if "Reconnect on Resize" was unchecked, the resize was silently ignored for such servers; now the dynamic resize path is always attempted first, and if the server reports Display Control unavailable, a reconnect is triggered regardless of the checkbox state; the "Reconnect on Resize" option now means "always reconnect immediately without trying dynamic resize first" — useful when you know the server doesn't support it and want to skip the initial attempt ([#131](https://github.com/totoshko88/RustConn/issues/131))
- **RDP: Copy/Paste toolbar buttons don't actually copy or paste** — the Copy button set the local GTK clipboard but didn't suppress the `clipboard-changed` handler, causing a feedback loop that re-announced the same text back to the server; the Paste button only updated the server's clipboard buffer via CLIPRDR `FormatList` but never simulated Ctrl+V, so text was never actually pasted into the active remote window; fixed: Copy now suppresses the sync handler for 100ms to prevent the feedback loop; Paste now sends `SendKeySequence` (Ctrl+V) after a 150ms delay to let the server process the clipboard data before pasting ([#126](https://github.com/totoshko88/RustConn/issues/126))
- **RDP: dynamic resize without reconnect via Display Control Channel** — window resize no longer triggers a full session disconnect/reconnect cycle; instead, the new resolution is sent in-place via the MS-RDPEDISP Display Control Virtual Channel (`SetDesktopSize` → `encode_resize`), which is the same mechanism used by mstsc, xfreerdp, and Remmina; the session continues seamlessly with the server-side desktop resized to match the new widget dimensions; debounce (500ms) and threshold (50px) remain to avoid flooding the server with resize requests; FreeRDP external mode still falls back to reconnect since it has no DVC access from the widget side ([#131](https://github.com/totoshko88/RustConn/issues/131))
- **SSH agent key selection not remembered in connection dialog** — when editing a saved connection that uses SSH agent authentication, the previously selected key was always reset to the first entry in the dropdown; root cause: `set_connection()` tried to restore the key selection before `refresh_agent_keys()` loaded the key list, then `refresh_agent_keys()` unconditionally reset the dropdown to index 0; now the selected fingerprint/comment is stored as a pending selection and restored after the agent keys are loaded ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **SSH agent: multiple authentication prompts from monitoring** — the monitoring subsystem spawned a new SSH process every 3 seconds to collect metrics, each requiring a separate SSH agent confirmation (e.g., Bitwarden desktop app); now monitoring uses `ControlMaster=auto` with `ControlPersist=30` so all polling commands multiplex over a single authenticated SSH connection; the agent is only prompted once when monitoring starts ([#125](https://github.com/totoshko88/RustConn/issues/125), [#135](https://github.com/totoshko88/RustConn/issues/135))
- **Smart Folders: double-click and context menu not working for connections** — connections inside smart folders did not respond to double-click (connect) or right-click (context menu); root cause: the inner `ListBox` used `SelectionMode::None` which blocked `row_activated` signal, and the `connect-to` window action was never registered; fixed by switching to `SelectionMode::Single` with `activate_on_single_click(false)`, registering a new `win.connect-to` action that accepts a connection ID parameter, and adding a right-click context menu on connection rows with Connect, Edit, Copy Username/Password, Wake On LAN, Check if Online, and Delete actions
- **Toast error messages showing only "Error" without details** — the retry toast for port check failures (SSH, VNC, SPICE, Telnet, MOSH) displayed only a generic "Error" icon+label with no message text; root cause: `set_custom_title()` replaces the entire toast title widget, so the message passed to `Toast::new()` was never rendered; fixed by including the message in the custom title widget (format: "Error: <message>"); also replaced the generic "Connection failed. Host unreachable." text with the actual `PortCheckError` details (e.g. "Port 22 on '12.23.34.45' is not reachable: connection timed out")

## [0.13.4] - 2026-05-05

### Added
- **RDP Autotype: send text as keystrokes bypassing clipboard restrictions** — new "Type Clipboard" and "Type Text…" toolbar buttons in embedded RDP sessions; sends text character-by-character using `TS_UNICODE_KEYBOARD_EVENT` PDU which is keyboard-layout independent (works regardless of DE/US/other layout mismatches); solves scenarios where server-side paste is blocked by GPO, Citrix policy, UAC dialogs, or password fields that reject Ctrl+V; "Type Clipboard" reads the local clipboard and types it into the remote session; "Type Text…" opens a dialog where the user enters text (with optional password mode) that never touches the system clipboard — ideal for sensitive strings; per-connection configurable timing: inter-character delay (5–200ms, default 20ms) and initial delay before typing starts (0–5000ms, default 0ms); higher delays needed for Citrix gateways that drop characters when events arrive too fast; iterates by Unicode grapheme clusters so composed characters (é, ñ) are sent as single units; only available for embedded IronRDP mode (external FreeRDP runs in a separate process where keystroke injection is not possible) ([#127](https://github.com/totoshko88/RustConn/issues/127))
- **KeePass custom entry path for secret variables** — secret variables can now reference an existing entry in the KeePass database instead of the default `RustConn/rustconn/var/{name}` path; in the Variables dialog (Menu → Tools → Variables), when a variable is marked as Secret and the KeePass backend is active, a new "KeePass entry" field appears where you can specify the full path to an existing entry (e.g., `Internet/MyRouter` or `Network/Switches/RADIUS`); the password is read directly from that entry's Password attribute — no need to duplicate secrets under the RustConn hierarchy; when a custom path is set, RustConn does not attempt to create or overwrite the entry on save ([#114](https://github.com/totoshko88/RustConn/issues/114))

### Fixed
- **RDP toolbar Copy/Paste buttons do nothing on Wayland (COSMIC, GNOME)** — the clipboard buttons used `drawing_area.display().clipboard()` which on Wayland may not have clipboard ownership because the clipboard is tied to the focused surface; replaced with `root().native().display().clipboard()` which uses the top-level window surface — the reliable clipboard owner on all Wayland compositors; also fixed: Paste button silently swallowed errors when local clipboard was empty or unreadable (now shows status feedback: "Local clipboard is empty" / "Cannot read clipboard" / "Clipboard channel not ready"); CLIPRDR `client_capabilities` now advertises `USE_LONG_FORMAT_NAMES` flag required by Windows Server 2016+ for proper format list exchange — without it some servers never announce clipboard formats, making the Copy button permanently disabled; added tracing for all clipboard button operations to aid future debugging ([#126](https://github.com/totoshko88/RustConn/issues/126))

### Dependencies
- unicode-segmentation 1.13.2 (new)
- tower-http 0.6.8 → 0.6.9

## [0.13.3] - 2026-05-05

### Improved
- **GNOME HIG: application menu restructured** — "Fullscreen" (F11) added to the app menu; monolithic Tools section (11 items) split into three logical subsections: Managers (Snippets, Clusters, Templates, Variables), Monitoring & History (Sessions, History, Statistics, Recordings), Security & Network (Password Generator, Wake On LAN, SSH Tunnels); Settings separated from Shortcuts/About/Quit into its own section per HIG convention; "About" renamed to "About RustConn" per GNOME HIG naming pattern
- **GNOME HIG: Fullscreen menu item is now stateful** — the "Fullscreen" entry in the app menu uses a stateful `GAction` so GNOME Shell displays a checkmark (✓) when fullscreen is active; previously the menu item gave no visual indication of the current state
- **GNOME HIG: manager dialogs unified** — all dialog windows now use standard window close buttons (×) instead of text "Close"/"Cancel" buttons; action buttons moved to consistent positions: primary actions as icon buttons on the left of header bar (Add `+`, Refresh, Copy, Reset, Test, Connect, Import), save/submit actions on the right (Save, Create, Export, Import, Send, Rename); Snippets manager replaced bottom text buttons with inline icon buttons per row (▶ ✏ 🗑); Templates moved "Use Template" to bottom action bar; Variables added `+` and Save to bottom; History moved "Connect" to bottom right; Connection dialog uses Test icon (`network-transmit-receive-symbolic`) on the left; Quick Connect uses Connect icon on the left; Statistics uses Reset icon (`edit-clear-all-symbolic`); Password Generator uses Copy icon; Log Viewer and Terminal Search now use standard × close; applies to: Snippets, Clusters, Templates, Variables, Sessions, History, Statistics, Recordings, Quick Connect, Password Generator, Wake On LAN, Import, Export, Group Edit, New Group, Smart Folder, Connection Dialog, Rename, Log Viewer, Terminal Search
- **GNOME HIG: RDP Security Layer dropdown accessibility** — added `accessible::Property::Label` to the Security Layer dropdown in the RDP connection dialog for screen reader support
- **Tray menu i18n** — all system tray menu strings ("Show Window", "Hide Window", "Quick Connect...", "Recent Connections", "Local Shell", "Active Session(s)", "About RustConn", "Quit", tooltip) now use `gettext()` for translation; translations provided for all 16 languages; previously hardcoded in English

### Added
- **RDP Security Layer / TLS Compatibility options** — new "Security Layer" dropdown (Negotiate/RDP/TLS/NLA) and "TLS Security Level" spin (0–5) in the RDP connection dialog; enables connections to legacy servers (Windows Server 2012 / Windows 7) that require lower TLS levels or RDP Security Layer instead of NLA; when RDP or TLS security layer is selected (or TLS level < 2), IronRDP embedded mode automatically falls back to external FreeRDP which supports legacy protocols; TLS level row is shown only when RDP or TLS mode is selected; CLI `show` command displays non-default security settings ([#124](https://github.com/totoshko88/RustConn/issues/124))

### Fixed
- **SSH agent: multiple authentication prompts for saved connections** — when a saved connection uses `SshKeySource::Agent` and the key comment contains a file path, `build_command_args()` previously added `-i <path> -o IdentitiesOnly=yes`; this caused SSH to first attempt file-based auth (triggering an agent confirmation in Bitwarden), then fall back to agent auth (triggering a second confirmation); fixed by removing `-i` and `IdentitiesOnly` for Agent auth entirely — the agent now offers keys naturally with a single prompt, matching Quick Connect behavior ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **AdwTabOverview "exceeds AdwApplicationWindow size" warning in Flatpak** — embedded RDP and VNC sessions caused `AdwTabOverview` to request more space than the window provides (e.g. 1540×865 vs 1200×800 available); the RDP `DrawingArea` had no size constraint and the VNC `DrawingArea` set `content_width(1280)`/`content_height(720)` which GTK reported as natural size; fixed by setting `content_width`/`content_height` to 0 on both drawing areas (they expand via `hexpand`/`vexpand` instead) and setting `overflow: hidden` on the `TabOverview` widget
- **False "KeePassXc backend unavailable" toast when KeePassXc is running** — `check_secret_backend_available` checked `SecretManager.is_available()` for all non-LibSecret backends, but `build_from_settings` registers `LibSecretBackend` (not `KeePassXcBackend`) for KeePassXc/KdbxFile because KDBX credentials are resolved via direct file access in `resolve_credentials_blocking`; the availability probe therefore tested whether `secret-tool` could be spawned within a 5-second `block_on` timeout — which can fail in Flatpak sandboxes or when D-Bus is slow at startup — and incorrectly reported KeePassXc as unavailable; now KeePassXc/KdbxFile availability is determined by checking `kdbx_enabled && kdbx_path.exists()` instead of probing the unrelated LibSecretBackend ([#123](https://github.com/totoshko88/RustConn/issues/123))
- **Flatpak Local Shell: "no job control" warnings and broken PTY** — `flatpak-spawn --host` only forwards stdio without creating a host-side PTY, so the shell cannot become a session leader — causing `tcgetpgrp failed`, `setpgid: Inappropriate ioctl for device` warnings and broken job control (Ctrl-Z, fg, bg); now wraps the host shell in `script -qfc` (util-linux) which allocates a real PTY on the host, giving bash/zsh/fish a proper controlling terminal ([#122](https://github.com/totoshko88/RustConn/issues/122))

### Dependencies
- h2 0.4.13 → 0.4.14
- kqueue-sys 1.0.4 → 1.1.0
- openssl 0.10.78 → 0.10.79, openssl-sys 0.9.114 → 0.9.115
- quick-xml 0.39.2 → 0.39.3
- redox_syscall 0.7.4 → 0.7.5
- serdect 0.4.2 → 0.4.3
- tokio 1.52.1 → 1.52.2

## [0.13.2] - 2026-05-04

### Fixed
- **Mouse scroll not working in terminal sessions** — `set_enable_fallback_scrolling(false)` was incorrectly tied to the "Mouse passthrough" setting, which disabled VTE's scrollback scrolling for normal shell sessions (bash, zsh); VTE natively forwards scroll events to programs that request mouse tracking regardless of this flag — disabling it only broke scrollback for programs without mouse tracking; now always enabled ([#121](https://github.com/totoshko88/RustConn/issues/121))
- **Flatpak local shell provides only a sandboxed minimal shell** — "Local Shell" in Flatpak now spawns the user's host shell via `flatpak-spawn --host` with `--login`, giving full access to system tools, dotfiles, and home directory; added `--talk-name=org.freedesktop.Flatpak` permission to both Flatpak manifests ([#122](https://github.com/totoshko88/RustConn/issues/122))

### Removed
- **"Mouse passthrough" setting removed from Terminal preferences** — the toggle was based on a misunderstanding of VTE's `enable_fallback_scrolling` API; VTE handles mouse event forwarding to terminal apps (mc, vim, htop) automatically via escape sequences — no user-facing setting is needed; removed field from `TerminalSettings`, checkbox from UI, and obsoleted translations in all 16 languages

### Added
- **Per-connection monitoring toggle in connection dialog** — Advanced tab now has a "Remote Monitoring" section with an "Enable Monitoring" switch; when disabled, the monitoring collector does not open a separate SSH session to the remote host, preventing IP bans on devices with concurrent session limits (e.g. network routers); uses the existing `MonitoringConfig` backend — the toggle was already supported in `rustconn-core` but had no GUI ([#106](https://github.com/totoshko88/RustConn/issues/106))

## [0.13.1] - 2026-05-04

### Fixed
- **Crash when typing in sidebar search field** — `SearchEngine::find_case_insensitive` used raw byte-position iteration (`0..=len`) and direct byte slicing (`haystack[i..i+n]`) which panics when connection names or hosts contain multi-byte UTF-8 characters (Cyrillic, CJK, emoji, etc.); the same byte-boundary bug existed in `fuzzy_score_case_insensitive` and `fuzzy_score_optimized` prefix checks; fixed by iterating only over valid `char_indices()` boundaries and using `str::get()` for safe slicing with `is_char_boundary()` guards ([#116](https://github.com/totoshko88/RustConn/issues/116))
- **Export/re-import loses folder hierarchy, icons, SSH settings, and smart folders** — native `.rcn` export/import had four data-loss bugs: (1) group hierarchy flattened because binary root-vs-child sort didn't guarantee parents were created before children — replaced with topological sort; (2) folder icons (emoji), description, SSH auth settings (`ssh_auth_method`, `ssh_key_path`, `ssh_proxy_jump`, `ssh_agent_socket`), login defaults (`username`, `domain`, `password_source`), automation (`expect_rules`, `post_login_scripts`), and `dynamic_folder` config were silently dropped because `create_group_with_parent` only sets name+parent — now copies all fields via `update_group` after creation; (3) smart folders were not included in the export format at all — added `smart_folders` field to `NativeExport` (format version 3, backward-compatible via `#[serde(default)]`); (4) smart folder `filter_group_id` references were not remapped to new group UUIDs — now remapped during import; CLI export/import updated accordingly ([#118](https://github.com/totoshko88/RustConn/issues/118))
- **Settings dialog loses Passbolt, 1Password, Pass, and KeePassXC keyring settings on save** — the close handler used dummy widgets for 9 secret backend fields (passbolt passphrase/URL/save-to-keyring, 1password token/save-to-keyring, pass store dir, kdbx save-to-keyring); replaced with clones of real widgets so all secret backend settings are now correctly persisted
- **Statistics Reset button had no confirmation dialog** — destructive action now requires confirmation via `adw::AlertDialog`
- **Cluster and Template delete had no confirmation dialog** — both now show `adw::AlertDialog` before deletion
- **Template protocol filter dropdown was not connected** — filter now correctly filters templates by SSH/RDP/VNC/SPICE

### Improved
- **GNOME HIG compliance audit across all menu-accessible dialogs** — comprehensive review and fixes: empty states with `adw::StatusPage` (History, Statistics, Sessions, Recordings); accessible labels on status icons (History); search/filter in Recordings dialog; auto-refresh after recording import; confirmation dialogs for all destructive actions; "Add Variable" button style corrected (`suggested-action` → `flat`); Template header button renamed to "Use Template" for clarity; duplicate close buttons removed (Snippets manager); xgettext marker functions for indirect `i18n()` strings (dialog buttons, keyboard shortcuts); WoL dialog auto-sized to content; Export dialog height increased to fit all fields; Sessions window enlarged for StatusPage; copy-to-clipboard toast in Password Generator

### Documentation
- **Remmina Flatpak import troubleshooting** — added instructions to User Guide for importing Remmina connections when both apps are Flatpaks; covers `flatpak override`, Flatseal, and symlink workarounds ([#120](https://github.com/totoshko88/RustConn/issues/120))

### Added
- **Automation section in group edit dialog** — Expect Rules and Post-login Scripts can now be configured directly in the group edit dialog (Edit Group → Automation); full rule editor with pattern/response fields, priority/timeout controls, enabled/one-shot checkboxes, ↑↓🗑️ action buttons per rule, "Insert Variable" (➕) popover for `${password}`, `${username}`, `${host}`, `${port}`, `\n` insertion, "From Template" menu with 5 built-in presets marked with protocol hints (e.g., "Sudo Password (SSH)"), "Clear All" button, collapsible Pattern Tester for real-time rule matching, post-login scripts as individual entries with per-row delete, confirmation dialog on automation disable, stable window width via `set_size_request` + `tightening_threshold` matching Clamp, overlay scrolling to prevent layout shifts; CLI: `group edit --add-expect-rule` (JSON), `--clear-expect-rules`, `--add-post-login-script`, `--clear-post-login-scripts`; `group show` now displays automation config ([#117](https://github.com/totoshko88/RustConn/issues/117))

### Improved
- **Connection dialog Automation tab unified with group dialog** — expect rule editor now uses the same vertical layout as the group dialog: ↑↓🗑️ action buttons at top-right of each rule, "Insert Variable" (➕) popover on Response field, template picker with "(SSH)" protocol hints, variable substitution info banner, removed inner ScrolledWindow (scroll-in-scroll anti-pattern per GNOME HIG), overlay scrolling and `tightening_threshold` matching Clamp to prevent layout width shifts ([#117](https://github.com/totoshko88/RustConn/issues/117))

## [0.13.0] - 2026-05-03

### Fixed
- **Mouse not working in Midnight Commander and ncurses apps** — split view panel click handlers used `EventSequenceState::None` in capture phase, which prevented VTE from receiving raw button-press events for mouse tracking; changed to `EventSequenceState::Denied` so VTE processes mouse clicks for ncurses apps (mc, vim, htop); also unified TERM to `xterm-256color` everywhere (Flatpak previously used `rustconn-256color` which broke mouse tracking)
- **SFTP connection slow with Bitwarden backend** — double-clicking an SFTP connection triggered full Bitwarden credential resolution (~12s of sequential `bw status` + `bw list items` calls) even though mc uses SSH key in agent, not vault passwords; connections with `password_source` set to None or Prompt now skip async vault resolution entirely, benefiting all protocols — not just SFTP
- **External RDP tab shows only toolbar, content area empty** — `EmbeddedSessionTab::new()` determined embed mode from `DisplayServer::supports_embedding()` which returns `true` on both Wayland and X11; when `start_external_rdp_session` created the tab, it got an empty `DrawingArea` (meant for XEmbed) instead of the `StatusPage` with FreeRDP hotkeys; added `force_external` parameter so external sessions always render the informational StatusPage; also added `show_tab_view_content()` call for layout consistency and `markup_escape_text` to protect against special characters in connection names
- **Group edit: SSH settings toggle reopens confirmation dialog in a loop** — clicking "Clear" in the "Clear SSH Settings?" confirmation dialog triggered the `enable_expansion_notify` signal recursively, causing the dialog to reopen immediately; added a guard flag to prevent re-entry
- **Incomplete translations for Dynamic Folder strings** — "Dynamic Folder", "Generate connections from a script", "Working Directory", "Timeout (seconds)", "Shell command executed via sh -c", "Refreshed" toast, and error messages were marked fuzzy in all 16 `.po` files and displayed in English; provided correct translations and removed fuzzy flags for all languages
- **Bulk actions: "Move to Group" icon missing** — `folder-move-symbolic` is not available in all icon themes; replaced with standard `folder-drag-accept-symbolic` from Adwaita
- **External RDP (FreeRDP) fails on changed server certificate** — external xfreerdp always used `/cert:tofu` with no way to override; when a server certificate changed, the connection was silently rejected; added `ignore_certificate` field to `RdpConfig` and "Accept Certificate" checkbox in the RDP connection dialog; when enabled, removes stored FreeRDP certificate (`~/.config/freerdp/server/` and `known_hosts2`) and passes `/cert:ignore`; also fixed: password not passed to `sdl-freerdp3` (now uses `/p:`), `disable_nla` not forwarded to external client (now passes `/sec:nla:off`), connection errors not shown to user (now displays toast with parsed FreeRDP error), tab remained open after FreeRDP window closed (now auto-closes via process monitor), tab placeholder showed keyboard shortcuts and auto-close hint ([#112](https://github.com/totoshko88/RustConn/issues/112))

### Added
- **Smart Folders in sidebar** — the Smart Folders section is now visible in the sidebar below the connection list; dynamically groups connections based on tag, protocol, host pattern, and group filters using AND logic; shows match count badge per folder; "Add" button and right-click context menu (Edit / Delete) fully wired; "New Smart Folder" option added to empty-space context menu; smart folders auto-refresh on every sidebar reload; hidden by default, toggled via toolbar button with persisted visibility state ([#111](https://github.com/totoshko88/RustConn/issues/111))
- **Group-level Expect rules & post-login scripts inheritance** — `ConnectionGroup` now supports `expect_rules` and `post_login_scripts` fields; connections with empty automation config automatically inherit from their parent group chain (same walk-up-hierarchy pattern as SSH/credential inheritance with cycle detection); applies to all terminal protocols: SSH, Telnet, Serial, Kubernetes, MOSH, Zero Trust; set expect rules once on a group and all 600+ connections inherit them automatically ([#110](https://github.com/totoshko88/RustConn/issues/110))
- **Mouse passthrough option for VTE terminal** — new "Mouse passthrough" toggle in Settings → Terminal → Behavior; when enabled (default), mouse clicks and scroll wheel events are forwarded to terminal applications that request mouse tracking (Midnight Commander buttons, vim visual selection, htop scrolling); disables VTE's fallback scrolling so scroll wheel is sent as mouse events on alternate screen; hold Shift to select text when mouse passthrough is active
- **CLI: `--keep-alive-interval`, `--keep-alive-count`, `--ssh-verbose`, `--ignore-certificate` flags** — `add` and `update` commands now support SSH keep-alive settings (`ServerAliveInterval`, `ServerAliveCountMax`), SSH verbose/debug output (`-v` flag), and RDP certificate acceptance; previously these settings were only configurable via the GUI connection dialog
- **CLI: `snippet edit`, `template edit`, `cluster edit`, `smart-folder edit` subcommands** — all four resource types now have full CRUD via CLI; `snippet edit` supports `--new-name`, `--command`, `--description`, `--category`, `--tags`; `template edit` supports `--new-name`, `--host`, `--port`, `--user`, `--description`; `cluster edit` supports `--new-name`, `--broadcast`; `smart-folder edit` supports `--new-name`, `--protocol`, `--host-pattern`, `--tags` (use `"none"` to clear a filter); previously editing required delete+create

### Documentation
- **Auto-login with stored passwords** — added comprehensive troubleshooting section to User Guide explaining how to configure auto-login: Password Source must be "Vault", lookup key formats for each backend (KeePass hierarchical, libsecret flat, Bitwarden/1Password prefixed), common issues table with fixes ([#114](https://github.com/totoshko88/RustConn/issues/114))

### Improved
- **Vault entry missing toast notification** — when credential resolution finds no vault entry for a connection, a warning toast now shows "Vault entry not found for '{name}'" instead of silently falling back to a password prompt; helps users understand why auto-login didn't work ([#114](https://github.com/totoshko88/RustConn/issues/114))
- **"Test credential resolution" button in connection dialog** — new ✓ button next to the password field performs a vault lookup using the current connection name, host, protocol, and group; shows the exact lookup key used and whether the vault returned a password; helps users verify their vault configuration before connecting ([#114](https://github.com/totoshko88/RustConn/issues/114))
- **Multi-language SSH password prompt detection** — VTE password injection now recognizes password prompts from SSH servers configured in non-English locales; supported: German (Passwort/Kennwort), French (mot de passe), Spanish (contraseña), Portuguese (senha), Ukrainian/Belarusian (пароль), Polish (hasło), Czech/Slovak (heslo), Dutch (wachtwoord), Swedish (lösenord), Danish (adgangskode), Chinese (密码/密碼), Japanese (パスワード), Korean (비밀번호); previously only English prompts triggered auto-fill ([#114](https://github.com/totoshko88/RustConn/issues/114))
- **GNOME HIG: sidebar toolbar decluttered** — removed Import/Export buttons from the bottom toolbar (accessible via hamburger menu Ctrl+I and context menu); reduces button count from 8 to 6 for cleaner appearance per GNOME HIG recommendations

### Dependencies
- Boundary CLI 0.21.2 → 0.21.3 (security: CVE fixes in pgx/v5 and go-ntlmssp)
- Hoop.dev CLI 1.59.3 → 1.62.0
- Tailscale CLI 1.96.5 → 1.96.4 (1.96.5 was platform-specific, not available for Linux)
- rpassword 7.5.1 → 7.5.2
- zvariant 5.10.1 → 5.11.0

## [0.12.9] - 2026-05-02

### Fixed
- **Export group exports entire tree instead of selected subtree** — when exporting a specific group via the Export dialog's group filter, all groups were included in the output file even though connections were correctly filtered; now both connections and groups are filtered to the selected group and its descendants; previously importing such an export recreated the entire group hierarchy instead of just the selected branch

### Added
- **Snippet variable substitution from Global Variables** — snippets containing `${VARIABLE}` placeholders now automatically resolve values from Global Variables (Menu → Tools → Variables) before execution; if all variables are resolved, the snippet executes immediately without showing the input dialog; partially resolved variables pre-fill the dialog with known values; resolution order: Global Variables → snippet-defined defaults → manual input; uses the same `VariableManager` and vault-backed secret resolution as Expect rules and SSH connections
- **Dynamic Folders** — new `DynamicFolderConfig` on `ConnectionGroup` allows generating connections from an external script; the script runs via `sh -c` with configurable timeout (default 30s) and optional working directory; output is a JSON array of `[{name, host, port?, protocol?, username?, group?, tags?, description?}]`; connections are read-only (`is_dynamic` flag) with stable deterministic UUIDs across refreshes; supports SSH, RDP, VNC, SPICE, Telnet, and MOSH protocols; async executor in `rustconn-core/src/dynamic_folder.rs` with entry validation, warnings for empty name/host, and `thiserror`-based error types; model in `rustconn-core/src/models/dynamic_folder.rs`; **GUI**: group edit dialog with ExpanderRow for script/timeout/working directory/refresh interval configuration; context menu "Refresh Dynamic Folder" action with async execution and toast notifications; **CLI**: `dynamic-folder list`, `dynamic-folder show`, `dynamic-folder refresh` subcommands with table/JSON/CSV output

### Improved
- **CLI `group edit` extended** — `group edit` now supports `--new-name`, `--parent` (use "none" for root), `--description`, and `--icon` in addition to the existing SSH inheritance fields; enables full group management from the CLI without GUI
- **CLI `dynamic-folder set/remove`** — full CRUD for dynamic folders via CLI: `set` creates or updates the script configuration on any group, `remove` clears the configuration and deletes generated connections

## [0.12.8] - 2026-05-01

### Added
- **Generic async cache `Cached<T>`** — new `rustconn-core/src/cache.rs` module providing a thread-safe, TTL-based cache with automatic refresh via the `LoadCacheObject` trait; uses double-checked locking with `tokio::sync::RwLock` for concurrent read access; supports incremental updates through `previous_value` parameter, explicit invalidation, and configurable TTL (default 60s); replaces ad-hoc caching patterns across the codebase 
- **Busy-state indicator `BusyStack`** — new `rustconn-core/src/busy.rs` module providing a thread-safe RAII counter for tracking in-flight operations; callback fires on 0→1 (busy) and 1→0 (idle) transitions; nested operations handled correctly without extra callbacks; `Clone` for sharing across components; **integrated into GUI** — header bar spinner appears during connection attempts via `glib::MainContext` channel bridge 
- **Extended `ProtocolCapabilities`** — added 9 new capability flags: `multi_monitor`, `usb_redirection`, `port_forwarding`, `wayland_forwarding`, `x11_forwarding`, `session_recording`, `remote_monitoring`, `command_snippets`, `wake_on_lan`; enables UI to adapt controls per-protocol 
  - SSH: `port_forwarding`, `wayland_forwarding`, `x11_forwarding`, `session_recording`, `remote_monitoring`, `command_snippets`
  - RDP: `multi_monitor`
  - SPICE: `multi_monitor`, `usb_redirection`, `audio`
  - All terminal protocols: `session_recording`, `remote_monitoring`, `command_snippets`, `wake_on_lan`
  - All graphical protocols: `wake_on_lan`
- **Connection fallback chain `ConnectionFallback<T>`** — new `rustconn-core/src/connection/fallback.rs` module providing a generic mechanism for trying multiple connection strategies in priority order; `ConnectionStrategy` trait with `name()`, `is_available()`, and async `connect()`; unavailable strategies are skipped automatically; `FallbackError` collects all attempt details for diagnostics; integrated with `tracing` for structured logging 
- **Virt-viewer `.vv` file open support** — RustConn can now open `.vv` files (SPICE/VNC from libvirt, Proxmox VE, oVirt) directly from the file manager or command line (`rustconn file.vv`); adds `StartupAction::VvFile`, `VirtViewerImporter::parse_vv_file()` convenience method, `application/x-virt-viewer` MIME type registration in desktop file and metainfo, and MIME XML definition; completes parity with `.rdp` file support 
- **Connection failure toast** — when a connection fails to start, an error toast now shows the connection name (`"Connection to 'name' failed"`); previously the sidebar turned red with no further feedback

### Dependencies
- Teleport CLI 18.7.5 → 18.7.6 (security: authorization bypass in encrypted session recordings, cross-node recording access, SSRF via AWS database endpoint)

## [0.12.7] - 2026-05-01

### Fixed
- **Group credentials: Variable source shows password field instead of variable selector** — when editing a group and choosing "Variables" as the credential type, the dialog incorrectly displayed a password entry field; now shows a dropdown populated with secret global variables, matching the behavior of individual connection editing ([#109](https://github.com/totoshko88/RustConn/issues/109))
- **Group credentials: saving Variable source stored empty string** — selecting "Variable" and saving the group produced `PasswordSource::Variable("")` instead of the actual variable name; now correctly reads the selected variable from the dropdown
- **Group credentials: no validation for empty variable selection** — saving with "Variable" source when no secret global variables exist silently stored an empty variable name; now shows a validation error prompting the user to select a variable

### Improved
- **GNOME HIG: accessible labels for group credential widgets** — added `LabelledBy` relations for the password source dropdown, password entry, and variable dropdown in the group edit dialog so screen readers can announce their purpose
- **GNOME HIG: menu button tooltip shows F10 shortcut** — the hamburger menu button tooltip now reads "Menu (F10)", consistent with other header bar buttons that show their keyboard shortcut
- **GNOME HIG: "SSH Tunnels" moved to Tools section** — SSH Tunnels was in the App section alongside Settings/About/Quit; moved to the Tools section where it logically belongs with other management dialogs
- **GNOME HIG: "Settings" menu item ellipsis** — "Settings" now shows as "Settings..." to indicate it opens a window, per GNOME HIG ellipsis convention
- **GNOME HIG: "Keyboard Shortcuts" added to app menu** — the existing `app.shortcuts` action (F1) was not discoverable from the hamburger menu; added "Keyboard Shortcuts..." entry in the App section before About, matching standard GNOME app layout

### Documentation
- **User Guide: group credentials rewritten** — replaced outdated KeePass/Keyring/Bitwarden password source list with the current unified model (Prompt, Vault, Variable, Inherit, None); documented that Variable source shows a dropdown of secret global variables
- **User Guide: F10 shortcut** — added F10 (Open Menu) to the Application keyboard shortcuts table

## [0.12.6] - 2026-04-30

### Fixed
- **Expect script variables not substituted** — `${MY_PASSWORD}` and other `${VAR}` references in Expect rule responses were sent as literal text instead of being resolved to their actual values; now uses `VariableManager` to substitute global variables before creating automation triggers ([#105](https://github.com/totoshko88/RustConn/issues/105))
- **GNOME HIG: missing ellipsis on menu items** — "Active Sessions" and "SSH Tunnels" in the hamburger menu now use ellipsis ("Active Sessions...", "SSH Tunnels...") to indicate they open a dialog/window, per GNOME HIG conventions
- **False `c-format` flag on command palette search string** — `xgettext` incorrectly marked `"Search connections, > commands, @ tags, # groups, % tabs"` as a C format string; `% T` was interpreted as a format specifier, causing `msgfmt --check` to fail in 14 languages; removed the flag from POT and all 16 PO files
- **Corrupted `Plural-Forms` in uk.po** — `%` characters in the Ukrainian plural formula were replaced with `{}` placeholders during a previous i18n update; restored the correct `nplurals=3` formula

### Added
- **SSH verbose mode for connection debugging** — new "Verbose" toggle in SSH connection settings adds `-v` flag to the SSH command, showing detailed debug output in the terminal to help diagnose connection issues such as resets by remote devices ([#106](https://github.com/totoshko88/RustConn/issues/106))
- **Sidebar width setting** — new "Sidebar width" control in Settings → Appearance allows adjusting the sidebar width from 260 to 500 pixels; applied immediately and persisted across sessions ([#96](https://github.com/totoshko88/RustConn/issues/96))
- **SSH Tunnel Manager** — standalone window for managing headless SSH port-forwarding tunnels without terminal sessions; supports Local/Remote/Dynamic forwards, auto-start on launch, auto-reconnect, and references existing SSH connections for host/key configuration; accessible via menu or Ctrl+T ([#96](https://github.com/totoshko88/RustConn/issues/96))

### Improved
- **GNOME HIG: tunnel delete confirmation** — deleting a tunnel from the SSH Tunnel Manager now shows an `AdwAlertDialog` confirmation ("Delete Tunnel? — Tunnel "…" will be permanently removed.") with a destructive "Delete" button; previously the tunnel was removed immediately without confirmation
- **GNOME HIG: tunnel dialog inline validation** — the Save button in the Add/Edit Tunnel dialog is disabled while the tunnel name is empty, preventing accidental saves of unnamed tunnels; previously the dialog silently refused to save without any visual feedback

### Documentation
- **User Guide: SSH Tunnel Manager** — complete section with create/manage workflow, tunnel options table, use case examples, and comparison with per-connection port forwarding
- **User Guide: SSH verbose mode** — new section under SSH → Session Options with configuration steps and when-to-use guidance
- **User Guide: Expect variable substitution** — documented `${VARIABLE_NAME}` placeholder support in Expect rule responses with multi-step login example
- **User Guide: Sidebar width** — added to Settings → Appearance documentation
- **User Guide: context menu corrections** — removed non-existent "View Details" entry, fixed "Pin to Favorites"→"Pin / Unpin" to match code, added undocumented "Run Snippet..." and "Start/Stop Recording" entries, clarified Copy/Paste Connection scope (sidebar focus + hamburger menu only)
- **User Guide: Ctrl+T shortcut** — added SSH Tunnel Manager shortcut to Keyboard Shortcuts table

### Translations
- All 16 languages (be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn) aligned to 1748 translatable strings with 0 untranslated entries
- **Chinese Simplified (zh-cn)** — merged community translation update from GaaChun ([PR #108](https://github.com/totoshko88/RustConn/pull/108)); filled 36 new strings from 0.12.6; 20 fuzzy entries remaining (upstream review pending)

## [0.12.5] - 2026-04-29

### Fixed
- **Settings dialog overflows after Hoop.dev install** — `hoop version` outputs JSON (`{"version":"1.59.3","git_commit":"...","build_date":"..."}`) which was displayed as-is in the version suffix label, stretching the `AdwToolbarView` to 1331 px and breaking the 700 px settings dialog layout; added a dedicated `hoop` parser that extracts only the `"version"` field
- **kubectl version not shown in settings** — `kubectl version --client --short` fails on kubectl ≥ 1.28 (`error: unknown flag: --short`); switched to `kubectl version --client` and parse `Client Version: vX.Y.Z`
- **Tray icon SIGSEGV on restart** — `connect_shutdown` did not drop the `TrayManager`, so D-Bus callbacks could reference already-finalized GObjects during GTK teardown; now explicitly drops the tray manager in the shutdown handler before flushing persistence
- **Teleport CLI download URL 404** — pinned version 18.7.6 did not exist on the CDN; corrected to 18.7.5

### Dependencies
- Hoop.dev CLI 1.56.1 → 1.59.3
- Teleport CLI 18.7.6 → 18.7.5 (URL fix)

## [0.12.4] - 2026-04-29

### Fixed
- **"Copy Password" from context menu resolves from vault** — previously only worked with cached credentials (required connecting first); now falls back to `resolve_credentials_gtk` to fetch the password directly from the configured secret backend (KeePass, Bitwarden, 1Password, etc.) when no cached credentials are available

### Cleaned
- **Removed dead `mosh.rs` dialog module** — standalone MOSH options panel was never wired into the connection dialog; MOSH settings are already integrated into the SSH tab via `ssh::create_ssh_options()`
- **Removed legacy `connect_password_load_button` wrapper** — unused passthrough method in `ConnectionDialog` that delegated to `connect_password_load_button_with_groups` with empty groups; all callers already use the `_with_groups` variant directly

### Added
- **Import button in Cloud Sync settings** — "Available in Cloud" section now shows an "Import" button on each unimported `.rcn` file; clicking it creates an Import group and triggers an immediate sync, importing all connections from the file

### Dependencies
- rpassword 7.4.0 → 7.5.0
- rustls 0.23.39 → 0.23.40

## [0.12.3] - 2026-04-28

### Fixed
- **Sync toast shows raw placeholders instead of values** — the Cloud Sync notification displayed `%1`, `%2`, `%3`, `%4` instead of actual connection counts because `i18n_f()` only supports `{}` placeholders; changed both sync message strings and all 16 translations to use `{}` format

### Accessibility
- **Icon-only buttons missing accessible labels** — added `update_property(accessible::Property::Label)` with `i18n()` wrappers to 24 icon-only buttons across 15 files (password generator, terminal search, history, cluster management, split view, SSH agent, settings tabs, flatpak components); screen readers now correctly announce button purpose instead of just the icon name

### Dependencies
- Teleport CLI 18.7.4 → 18.7.6
- clap_complete 4.6.2 → 4.6.3
- gio 0.22.5 → 0.22.6, glib 0.22.5 → 0.22.6, glib-macros 0.22.2 → 0.22.6
- gtk4 0.11.2 → 0.11.3
- pango 0.22.4 → 0.22.6
- zbus 5.14.0 → 5.15.0, zvariant 5.10.0 → 5.10.1

## [0.12.2] - 2026-04-26

### Fixed
- **Flatpak SFTP ssh-add fails with missing askpass** — `ssh-add` inherited the host's `SSH_ASKPASS` (e.g. `ksshaskpass` on KDE) which doesn't exist inside the Flatpak sandbox, causing "No such file or directory" and blocking mc/file-manager SFTP; now strips `SSH_ASKPASS` from the environment for all bare `ssh-add` calls ([#102](https://github.com/totoshko88/RustConn/issues/102))
- **Blocking operations on GTK main thread** — `has_secret_backend()` and `refresh_secret_backend_cache()` called `block_on(is_available())` without timeout on the main thread, freezing the UI if the secret backend was unresponsive; added 5-second timeouts to both methods
- **Missing timeouts on blocking async operations** — `flush_persistence()` (app shutdown), `resolve_with_hierarchy()` (credential fallback), `auto_unlock()` (Bitwarden), and all vault store/retrieve/delete operations in `dispatch_vault_op()` could hang indefinitely; added timeouts (5s for persistence, 30s for credential resolution and Bitwarden unlock, 10s for vault operations) to prevent infinite blocking

### Translations
- All 16 languages (be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn) aligned to 1697 translatable strings
- **Italian (it)** — fixed syntax error in PO file (curly quotes breaking msgfmt)
- **Chinese Simplified (zh-cn)** — 20 fuzzy entries remaining (community-contributed, pending upstream review)

### Dependencies
- FreeRDP 3.24.1 → 3.25.0 (switched to pub.freerdp.com release tarballs)

## [0.12.1] - 2026-04-25

### Fixed
- **Split view content disappearing on panel focus switch** — clicking between split panels caused the content to vanish because the click handler called `switch_to_tab()` which navigated the TabView away from the split-owner's tab (where the split widget lives) to the clicked session's placeholder tab; removed the `switch_to_tab()` call — focus is now handled entirely via `set_focused_pane()` and `grab_focus()` ([#101](https://github.com/totoshko88/RustConn/issues/101))
- **Flatpak SFTP mc host key prompt on every connect** — mc FISH uses SSH internally but could not find the Flatpak-writable `known_hosts` file because `~/.ssh` is read-only in the sandbox; now creates a thin SSH wrapper script that injects `StrictHostKeyChecking=accept-new` and the correct `UserKnownHostsFile`, prepended to `$PATH` for the mc process ([#102](https://github.com/totoshko88/RustConn/issues/102))
- **cargo-deny CI failure** — removed deprecated `unlicensed` and `copyleft` keys from `deny.toml` `[licenses]` section (removed in cargo-deny v2, see [PR #611](https://github.com/EmbarkStudios/cargo-deny/pull/611))
- **cargo-audit CI failure** — added `RUSTSEC-2023-0071` (rsa Marvin Attack) to `[advisories].ignore` in `deny.toml`; transitive dependency via ironrdp/sspi and spice-client with no upstream fix available

### Dependencies
- Bitwarden CLI 2026.3.0 → 2026.4.1 (fixes supply chain attack in 2026.4.0)
- kubectl 1.35.4 → 1.36.0

## [0.12.0] - 2026-04-24

### Added
- **Cloud Sync** — synchronize connection configurations between devices and team members through any shared cloud directory (Google Drive, Syncthing, Nextcloud, Dropbox, USB)
  - **Group Sync** — per-group `.rcn` files with Master/Import access model and name-based merge
  - **Simple Sync** — single-file bidirectional sync with UUID-based merge and tombstone deletion tracking
  - **SSH Key Inheritance** — group-level SSH settings (key path, auth method, proxy jump, agent socket) inherited by child connections; `ssh_key_path` remains local-only per device
  - **Credential Resolution UX** — interactive `AdwAlertDialog` prompts when variables or secret backends are missing at connect time
  - **File Watcher** — automatic import on `.rcn` file changes via `notify` crate with 3s debounce
  - **Cloud Sync Settings page** — `AdwPreferencesPage` with sync directory, device name, synced groups, available files, and Simple Sync toggle
  - **Sidebar sync indicators** — `emblem-synchronizing-symbolic` for synced groups, `dialog-warning-symbolic` for errors
  - **Import group UI restrictions** — synced fields read-only, local fields editable, context menu restrictions
  - **CLI sync commands** — `sync status`, `sync list`, `sync export`, `sync import`, `sync now`
- **Accessible labels** — added `update_property` accessible labels to icon-only buttons (password visibility toggle, password load, RDP quick actions)
- **cargo-deny + cargo-audit in CI** — security advisory checks, license allow-list, ban wildcards, source registry restrictions
- **Document dirty badge** — CSS dot indicator replaces text `"• "` prefix for unsaved documents in sidebar
- **Tab Overview** — grid view of all open tabs (GNOME Web-style) via button on the tab bar or **Ctrl+Shift+O**; makes navigating 10+ tabs significantly easier ([#100](https://github.com/totoshko88/RustConn/issues/100))
- **Tab Switcher in Command Palette** — `%` prefix in Command Palette (or **Ctrl+%**) opens fuzzy search across all open tabs; shows protocol and tab group in results ([#100](https://github.com/totoshko88/RustConn/issues/100))
- **Tab Pinning** — right-click a tab → Pin Tab to keep it always visible at the left edge of the tab bar; pinned tabs don't scroll away ([#100](https://github.com/totoshko88/RustConn/issues/100))
- **Custom terminal themes** — create, edit, and delete custom color themes (background, foreground, cursor, full 16-color ANSI palette) from Settings → Terminal → Colors; custom themes are persisted to `~/.config/rustconn/custom_themes.json` and appear alongside built-in themes in the dropdown ([#98](https://github.com/totoshko88/RustConn/issues/98))
- **Group Jump Host dropdown** — group SSH settings now include a Jump Host dropdown (select from existing SSH connections) in addition to the manual ProxyJump text field; stored as `ssh_jump_host_id` with inheritance support via `resolve_ssh_jump_host_id()`

### Improved
- **Tab Overview + Split View architecture** — complete refactoring of the TabView/SplitView architecture so that split layouts live inside TabPages instead of a global container; Tab Overview now renders correct thumbnails for all tabs including split-view tabs without SIGSEGV crashes or blank previews
- **Split view "Select Tab" popover** — the session picker popover in empty split panels now shows color indicators for sessions already displayed in other split views
- **Split view placeholder** — when a session is moved to another tab's split layout, its own tab shows a "Displayed in Split View" status page with a "Go to Split View" button for quick navigation
- **Split color indicators preserved** — switching between tabs no longer clears the colored dot indicators on split-view tabs
- **Group settings: GNOME HIG enable switches** — Default Credentials and SSH Settings sections now use `AdwExpanderRow` with `show_enable_switch(true)`; when disabled, all fields are cleared to `None`, giving clear semantics of "not configured" vs "configured but empty"
- **SSH tunnel password authentication** — SSH tunnels (used by RDP, VNC, SPICE jump host connections) now support password-authenticated jump hosts via `SSH_ASKPASS` mechanism; previously `BatchMode=yes` was unconditional, silently blocking password auth
- **VTE passphrase prompt guard** — VTE password auto-fill now explicitly rejects SSH key passphrase prompts (`"Enter passphrase for key"`) to prevent sending the wrong secret when SSH auth method is PublicKey
- **Connection dialog: protocol-aware Password Source** — Password Source dropdown is now hidden for protocols that don't use stored passwords (Telnet, Serial, MOSH, Kubernetes, Zero Trust); previously visible but non-functional for these protocols
- **Credential Resolution UX fully wired** — `CredentialResolutionResult` enum now drives the connection flow: `VariableMissing` shows the variable setup `AdwAlertDialog` (enter value + select backend → Save & Connect), `BackendNotConfigured` shows the backend missing dialog (Enter Manually / Open Settings), `VaultEntryMissing` falls through to the protocol's password prompt; previously the resolver silently returned `None` on all failure paths
- **Sidebar sync error indicators** — synced groups now show `dialog-warning-symbolic` with error tooltip when the last sync operation failed (e.g. parse error, missing file); previously always showed the generic synced icon regardless of error state
- **Custom themes atomic write** — `custom_themes.json` now uses temp file + rename (atomic write) with `0600` permissions and `tracing::warn` on errors; consistent with sync file write pattern

### Dependencies
- notify 7 (new — file watching for Cloud Sync)
- hostname 0.4 (new — default device name)
- slug 0.1 (new — sync filename generation)
- Tailscale CLI 1.96.4 → 1.96.5
- cc 1.2.60 → 1.2.61, data-encoding 2.10.0 → 2.11.0, hybrid-array 0.4.10 → 0.4.11, libc 0.2.185 → 0.2.186, rustls-pki-types 1.14.0 → 1.14.1

### Fixed
- **System tray SIGSEGV and empty menu** — tray icon menu could randomly appear empty or crash the application with `object_ref: assertion '!object_already_finalized' failed` (SIGSEGV) on startup; root cause was `ksni::Handle::update()` calling `block_on()` on the GTK main thread which deadlocked with the D-Bus service loop competing for the `TrayState` mutex, and conflicted with the application's tokio runtime guard; moved all D-Bus updates to a dedicated `tray-updater` background thread with coalescing `sync_channel(1)`, moved `TrayManager` creation to a `tray-init` background thread, added re-activation guard in `build_ui`, and ensured polling timers stop when the window is finalized
- **Tab Overview SIGSEGV with split-view tabs** — opening Tab Overview when split-view tabs were active caused Pango `size >= 0` assertion failures and crashes because `AdwTabOverview` attempted to snapshot `TabPage` children with 0×0 allocation; refactored to keep `TabView` always visible with per-tab `TabPageContainer` wrappers that guarantee non-zero allocation
- **Tab Overview blank previews** — split-view tabs showed empty thumbnails in Tab Overview because terminals were reparented to a global split container outside `TabView`; terminals now stay inside `TabPage` children at all times
- **Terminal theme reset when Settings dialog is closed** — closing the Settings dialog applied the global terminal color theme to all terminals, overwriting per-connection theme overrides (custom background/foreground/cursor colors); now re-applies connection-specific theme overrides after global settings are applied ([#99](https://github.com/totoshko88/RustConn/issues/99))
- **Pango assertion failure on zero font size** — guarded against `font_size == 0` in terminal configuration and settings collection to prevent `pango_font_description_set_size: assertion 'size >= 0' failed` crashes when the settings dialog returns an invalid value
- **Highlight rules show color instead of hover-only underline** — VTE's `match_add_regex()` only underlines text on mouse hover without color; added a Cairo `DrawingArea` overlay that reads visible terminal text, runs `CompiledHighlightRules::find_matches()` per line, and draws colored background rectangles and foreground underlines in real time; `SourcePattern` now carries `foreground_color`/`background_color` from the rule ([#97](https://github.com/totoshko88/RustConn/issues/97))

## [0.11.7] - 2026-04-23

### Fixed
- **Monitoring bar broken after scrollbar addition** — the terminal scrollbar (added in 0.11.6) changed the session container from vertical to horizontal layout, causing the monitoring bar to appear side-by-side with the terminal instead of below it; wrapped the horizontal terminal+scrollbar row in a vertical outer container so the monitoring bar is correctly appended underneath
- **Monitoring collector keeps running in split view** — when a session entered split view the monitoring bar was removed but the SSH exec collector continued polling the remote host every 3 seconds; added `suspend_monitoring`/`resume_monitoring` to `MonitoringCoordinator` that stops the collector on split entry and restarts it (with stored connection params) when the session returns to tab view

### Documentation
- **User Guide restructured** — reorganized USER_GUIDE.md from 41 flat sections (~4000 lines) into 13 logically grouped sections (~2000 lines); protocols, sessions, organization, and productivity tools are now grouped by topic instead of scattered across the document
- **CLI Reference extracted** — moved the full CLI command reference (~700 lines) to a dedicated [CLI_REFERENCE.md](docs/CLI_REFERENCE.md) for easier navigation
- **Zero Trust Providers extracted** — moved all Zero Trust provider documentation (~220 lines) to a dedicated [ZERO_TRUST.md](docs/ZERO_TRUST.md)
- **FAQ and Troubleshooting merged** — combined the previously separate FAQ, Troubleshooting, and Migration Guide sections to reduce duplication

### Dependencies
- clap_mangen 0.2.33 → 0.3.0

## [0.11.6] - 2026-04-23

### Added
- **Terminal scrollbar** — VTE terminals now display a vertical scrollbar (using a standalone `GtkScrollbar` connected to VTE's `vadjustment`, the same approach as GNOME Terminal); scrollbar is shown by default and can be toggled in Settings → Terminal → Scrolling ([#95](https://github.com/totoshko88/RustConn/issues/95))
- **"Execute Snippet…" in terminal context menu** — right-clicking inside a terminal now shows an "Execute Snippet…" option that opens the snippet picker; follows GNOME HIG (no nested submenus, verb label with ellipsis) ([#95](https://github.com/totoshko88/RustConn/issues/95))

### Fixed
- **Sidebar status stays gray after reconnect** — clicking "Reconnect" on a disconnected SSH/VTE session now immediately sets the sidebar status to "connecting" (yellow) instead of leaving it gray; the status then transitions to "connected" (green) once the session is established ([#96](https://github.com/totoshko88/RustConn/issues/96))
- **Context menu intermittently fails to open on right-click** — reverted sidebar popover from `autohide(true)` back to `autohide(false)` because GTK4's pointer grab consumed right-click events before the gesture handler could fire; added manual Escape key handler and window `focus-widget` tracking to auto-dismiss the menu when a dialog opens ([#87](https://github.com/totoshko88/RustConn/issues/87))

### Dependencies
- pastey 0.2.1 → 0.2.2
- rustls 0.23.38 → 0.23.39

## [0.11.5] - 2026-04-22

### Added
- **Simplified Chinese (zh-cn) translation** — complete translation of all 1573 UI strings; contributed by GaaChun ([PR #94](https://github.com/totoshko88/RustConn/pull/94))
- **User Guide: libvirt NSS hostname resolution** — added troubleshooting section explaining how to resolve VM hostnames via the libvirt NSS module when connecting with RDP/VNC from Flatpak or native installs ([#91](https://github.com/totoshko88/RustConn/issues/91))

### Dependencies
- picky-asn1-der 0.5.5 → 0.5.6
- rustls-webpki 0.103.12 → 0.103.13
- winnow 1.0.1 → 1.0.2
- kubectl 1.35.3 → 1.35.4

## [0.11.4] - 2026-04-21

### Fixed
- **Sidebar flashes red during SSH connection** — connecting via SSH (and other protocols with port check) briefly showed "failed" (red) status before switching to "connected" (green); introduced `ConnectionStartResult` enum to distinguish async port check in progress (`Pending`) from real failures (`Failed`); the sidebar now stays yellow ("connecting") until the port check completes
- **Context menu stays open when dialog opens** — the sidebar context menu remained visible when opening a dialog via keyboard shortcut or toolbar button (e.g. "New Connection"); switched the popover from `autohide(false)` to `autohide(true)` so GTK4 automatically dismisses it when focus moves elsewhere ([#93](https://github.com/totoshko88/RustConn/issues/93))
- **Sidebar stays "connecting" after cancelling password dialog** — closing the VNC or RDP password prompt without entering credentials left the sidebar status stuck on yellow ("connecting"); the status is now cleared on cancel
- **VNC/RDP with "None" password source prompts immediately** — when Password Source is set to "None", the first connection attempt now uses an empty password; the password dialog is only shown on retry (second attempt) if authentication fails
- **Cannot save SSH connection with default key** — validation incorrectly required an explicit SSH key path even when Key Source was set to "Default"; the check now only applies when Key Source is "File"

### Dependencies
- Teleport CLI 18.7.3 → 18.7.4
- 1Password CLI 2.33.1 → 2.34.0

## [0.11.3] - 2026-04-21

### Added
- **CLI: `--jump-host` flag for `add` and `update`** — set a jump host (SSH bastion) when creating or updating SSH, SFTP, RDP, VNC, and SPICE connections via CLI; accepts connection name or UUID; validates that the referenced connection exists and prevents self-referencing
- **SSH Jump Host for VNC and SPICE** — VNC and SPICE connections now support SSH jump host tunnelling via `ssh -L` local port forwarding; the tunnel process is managed automatically and killed on tab close; port check is skipped when jump host is configured
- **SSH tunnel stderr capture** — SSH tunnel process stderr is now read in a background thread and logged via `tracing::warn`; diagnostic messages (auth failures, port unreachable) are available via `SshTunnel::stderr()` and logged on process exit
- **SSH tunnel health monitoring** — `SshTunnel::is_alive()` checks whether the SSH process is still running; `wait_for_tunnel_ready()` now detects early process exit and fails fast with a descriptive error instead of polling until timeout
- **CLI: `show` displays Jump Host** — `rustconn-cli show` now prints the resolved jump host name for SSH, SFTP, RDP, VNC, and SPICE connections

### Fixed
- **RDP via jump host stuck at "connecting"** — embedded IronRDP connections through an SSH tunnel could hang indefinitely when the remote host was unreachable (firewall DROP); the handshake timeout for tunnel connections is now capped at 15 seconds (down from 60s) and produces a clear error message ([#92](https://github.com/totoshko88/RustConn/issues/92))
- **Flatpak: kubectl and Hoop.dev missing from settings and PATH** — kubectl and Hoop.dev CLI were not shown in the Settings → Clients detection tab and their install directories were missing from the Flatpak PATH extension; added "Container Orchestration" section to settings, added Hoop.dev to "Zero Trust Clients", and registered both directories in `get_cli_path_dirs()` and `find_in_flatpak_cli_dir()`
- **Sidebar status not set on connection start** — "connecting" (yellow) status is now shown immediately on double-click, before credential resolution or tunnel creation begins; previously the status only appeared after the tunnel was established
- **Sidebar status not cleared on RDP error** — non-protocol errors (timeout, unreachable host) now fire the `on_state_changed(Error)` callback, which closes the tab and sets "failed" (red) status; previously the sidebar stayed yellow after a timeout
- **Sidebar "failed" status overridden by Disconnected** — the `Disconnected` handler no longer calls `decrement_session_count` for sessions that were never connected; this prevents the "failed" status set by the Error handler from being cleared back to empty
- **RefCell panic on RDP error** — `handle_ironrdp_error` now uses take-invoke-restore pattern for `on_state_changed` and `on_error` callbacks; the previous `borrow()` approach caused a re-entrancy panic when the callback triggered `close_tab` → `adw_tab_view_close_page` → `Disconnected` state change
- **RDP error toast** — a toast notification ("RDP connection failed. Check that the remote host is reachable.") is now shown when an embedded RDP connection fails before ever connecting

### Improved
- **RDP handshake phase logging** — debug log messages now mark each handshake phase (X.224 negotiation, TLS upgrade, NLA/capabilities) so the exact hang point is visible in logs
- **TCP_NODELAY for tunnel connections** — Nagle's algorithm is disabled on the TCP stream to the tunnel, reducing latency for the RDP handshake
- **Tunnel timeout error message** — tunnel connections show "Connection failed: RDP handshake timed out after 15s — the remote host may be unreachable through the SSH tunnel or the RDP service is not running" instead of generic "Operation timed out"

## [0.11.2] - 2026-04-20

### Fixed
- **Reconnect reuses existing tab for all VTE protocols** — clicking "Reconnect" on a disconnected session now respawns the process in the same terminal tab instead of closing and creating a new one; works for SSH, Telnet, Serial, Kubernetes, ZeroTrust (all providers), and MOSH; tab position, tab group, and split view state are fully preserved ([#89](https://github.com/totoshko88/RustConn/issues/89))
- **RDP port check skipped with jump host** — pre-connect TCP port check is now skipped for RDP connections that have a jump host configured; the destination is only reachable through the SSH tunnel, so direct probing always timed out
- **Hoop.dev CLI download** — `releases.hoop.dev` removed the `latest` URL alias (HTTP 403); switched to versioned URL format; pinned to 1.56.1
- **Azure/gcloud/OCI CLI wrapper test in Flatpak** — `az --version` verification after pip install crashed with `Read-only file system`; now sets Flatpak-writable config dirs during wrapper script test
- **Flatpak SFTP always uses mc** — SFTP in Flatpak now always opens via Midnight Commander; `xdg-open sftp://` is unreachable from the sandbox

### Improved
- **Reconnect banner consistent across all protocols** — RDP, VNC, and SPICE sessions now show the "Session disconnected / Reconnect" banner at the bottom of the tab (same position as SSH/Telnet) instead of a button in the top-right toolbar
- **Sidebar width tuned for HiDPI** — default sidebar width lowered from 360px to 320px and fraction from 30% to 27%; saved widths from older versions are reset on upgrade; fixes overly wide sidebar on 4K displays with 200% scaling while keeping all protocol filter icons visible

### Added
- **SSH Jump Host for RDP** — SSH jump host selector is now available for RDP connections; the session is tunnelled through the selected SSH bastion host via `ssh -L` local port forwarding; tunnel process is managed automatically and killed on tab close ([#90](https://github.com/totoshko88/RustConn/issues/90))
- **Tab context menu: Close Others / Left / Right / All / Ungrouped** — right-click a tab for browser-style close actions: close all other tabs, close tabs to the left or right, close all ungrouped tabs, or close all tabs
- **CLI: all protocols and Zero Trust providers** — `rustconn-cli add` now supports all 10 protocols (`ssh`, `rdp`, `vnc`, `spice`, `sftp`, `telnet`, `serial`, `mosh`, `k8s`, `zt`) and all 11 Zero Trust providers with provider-specific flags (`--aws-region`, `--gcp-zone`, `--resource-group`, `--boundary-target`, etc.)

### Documentation
- **Complete CLI reference in User Guide** — comprehensive documentation for all 23 CLI commands with syntax, options tables, examples for every protocol and Zero Trust provider, shell completions, Flatpak usage with alias, and scripting examples

### Dependencies
- open 5.3.3 → 5.3.4
- openssl 0.10.77 → 0.10.78
- openssl-sys 0.9.113 → 0.9.114
- typenum 1.19.0 → 1.20.0
- Hoop.dev CLI pinned to 1.56.1

## [0.11.1] - 2026-04-18

### Fixed
- **Reconnect preserves tab position** — clicking "Reconnect" on a disconnected session now opens the new tab at the same position in the tab bar instead of appending it to the end; fixes workflow disruption when managing 10+ SSH sessions ([#89](https://github.com/totoshko88/RustConn/issues/89))
- **Context menu handoff between items** — right-clicking a second sidebar item while a context menu is already open now correctly closes the first menu and opens the new one; previously the second menu failed to appear due to GTK4 popover lifecycle conflicts ([#87](https://github.com/totoshko88/RustConn/issues/87))
- **Stale highlight on right-click** — right-clicking multiple sidebar items in succession no longer leaves residual selection highlights on previously clicked rows; the context menu gesture now claims the event sequence to prevent GTK4 from applying sticky `:active` / `:focus-within` pseudo-classes to row widgets
- **Context menu requires single right-click** — switching the context menu between sidebar items now works with a single right-click instead of requiring two clicks (first to dismiss, second to open); achieved by disabling `autohide` on the popover and managing dismissal explicitly via gesture handlers

### Improved
- **Context menu layout follows GNOME HIG** — sidebar context menu items reordered to match GNOME Files conventions: primary action (Connect) at top, organisation (Rename / Duplicate / Move) next, utilities (Copy credentials, SFTP, WOL) in the middle, creation and properties (New Connection, Edit) before the destructive action (Delete) at the bottom
- **MSRV bumped to 1.95** — required by `constant_time_eq` 0.4.3 (transitive dependency via `zip`)

### Improved
- **`SshOptionsWidgets` tuple replaced with named struct** — the 24-element tuple type alias in `ssh.rs` is now a proper struct with named fields; adding new SSH options is a single-point change instead of updating ~6 destructuring sites across `dialog.rs`
- **Split view context menu shares popover lifecycle with sidebar** — split view panel right-click menu now uses the same `ACTIVE_POPOVER` tracking as the sidebar; right-clicking panel B while panel A's menu is open correctly closes the first menu; also fixes cross-component conflicts where a sidebar menu and split view menu could fight for the GTK4 popover grab; menu labels now wrapped in `i18n()` for localization
- **Auto-reconnect guard for closed tabs** — polling callback now checks if the session still exists in `sessions_map` before triggering reconnect; prevents creating an orphan tab if the user manually closes the tab while background polling is active
- **SSH config importer applies `Host *` defaults** — `Host *` entries in `~/.ssh/config` are now parsed as global defaults and merged into each host entry (host-specific values take priority); previously `Host *` was skipped entirely, losing settings like `ServerAliveInterval 60` that apply to all hosts

### Added
- **SSH Keep-Alive settings** — dedicated `Keep-Alive Interval` and `Keep-Alive Count` spin rows in the SSH Connection options group; generates `-o ServerAliveInterval=N` and `-o ServerAliveCountMax=M` flags to prevent idle disconnects caused by firewalls or server timeouts; new connections default to 60s interval / 3 retries; custom_options take precedence if the same key is set manually ([#88](https://github.com/totoshko88/RustConn/issues/88))
- **SSH Config import/export for Keep-Alive** — `ServerAliveInterval` and `ServerAliveCountMax` from `~/.ssh/config` are now mapped to dedicated fields instead of only `custom_options`; exporter outputs them as separate directives with deduplication

## [0.11.0] - 2026-04-18

### Added
- **General tab migrated to adw:: widgets** — connection dialog General tab rebuilt with `adw::PreferencesGroup`, `adw::EntryRow`, `adw::SpinRow`, `adw::ComboRow`, and `adw::PasswordEntryRow`; replaces manual Grid+Label+Entry layout with native GNOME HIG sections (Identity, Connection, Authentication, Organization); 30-element tuple replaced with `BasicTabWidgets` struct; content wrapped in `adw::Clamp` (max 600px) for consistent width; Entry suffix widgets constrained with `width_chars`/`max_width_chars`
- **Legacy XOR encryption migration warning** — credentials still using XOR obfuscation are transparently migrated to AES-256-GCM on load; a toast notification shows the count of migrated credentials; XOR support will be removed in v0.12
- **State access helpers** — `with_state()`, `try_with_state()`, `with_state_mut()`, `try_with_state_mut()` helper functions reduce RefCell borrow panics; documented in ARCHITECTURE.md
- **Runtime warning for `block_on_async`** — logs `tracing::warn` when GTK main thread is blocked for >100ms, suggesting `spawn_async` instead
- **Accessible label for Command Palette list** — screen readers now announce the results list as "Search results"
- **Desktop entry translations** — added `Comment[lang]` translations for uk, de, fr, es, cs

### Improved
- **RDP connection state structured** — `handle_ironrdp_error()` 13-parameter signature replaced with `RdpConnectionContext` struct
- **Automation task validation hardened** — import warnings for connections with automation/expect rules; sensitive env vars (`BW_SESSION`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`) cleared before task execution
- **Localized constants and port descriptions** — `(Root)`, `(None)`, `(No keys loaded)`, and port range labels (`Well-Known`, `Registered`, `Dynamic`) now wrapped in `i18n()` for translation
- **Sidebar GNOME HIG color consistency** — sidebar pane and tab bar backgrounds unified with `@headerbar_bg_color` for GNOME Files-like appearance; bottom toolbar buttons use `.flat` style; separator between search and list hidden for seamless look; works correctly in both light and dark themes
- **KeePass button visibility** — active vault button now uses normal icon color instead of `.suggested-action` (which rendered white-on-white in light theme); inactive state uses `.dim-label`
- **Focus border only in split view** — `.focused-panel` accent border is now hidden when only one panel exists; previously showed a distracting border around the welcome screen and single-tab sessions

### Fixed
- **Split view tab colors preserved across Settings** — opening the Settings dialog no longer resets colored indicators on split view tabs; the root cause was that `apply_protocol_color()` / `clear_protocol_color()` guards relied on an unpopulated `session_tab_ids` map, so they always overwrote split indicators when `set_color_tabs_by_protocol()` was called on dialog close
- **Group Operations mode no longer breaks sidebar layout** — replaced text buttons with compact icon-only pill buttons matching the protocol filter bar style; toolbar wrapped in animated `Revealer` (SlideDown 200ms) instead of abrupt `set_visible()`; delete button uses `@error_color` for visual distinction
- **Split view context menu Copy/Paste/Select All now works** — action group `terminal.*` was installed on the TabView container which is lost when the terminal is reparented into a split panel; moved to the VTE terminal widget itself so actions follow the widget through reparenting

### Security
- **Automation env var sanitization** — task executor removes sensitive environment variables before spawning shell commands
- **Lazy Bitwarden credential decryption** — Bitwarden master password and API credentials are now decrypted at startup only when Bitwarden is the preferred backend; previously they were unconditionally decrypted into memory even when KeePass or other backends were active

### Dependencies
- libbz2-rs-sys 0.2.2 → 0.2.3
- rand 0.8.5 → 0.8.6
- rtoolbox 0.0.4 → 0.0.5

## [0.10.22] - 2026-04-17

### Fixed
- **Terminal context menu Copy/Paste now works** — replaced custom `GestureClick` popover with VTE's native `set_context_menu_model()` API; the old approach broke clipboard actions because the popover stole focus from VTE before callbacks could run ([#84](https://github.com/totoshko88/RustConn/issues/84))
- **No more `gdk_clipboard_write_async` assertion** — Copy action now caches selected text via `text_selected()` before VTE clears the selection on right-click, preventing the `mime_type != NULL` GDK critical warning
- **Blank menus on X11 (MATE, XFCE)** — GTK4's NGL renderer causes popovers to render blank until hovered on some X11 compositors; RustConn now auto-detects X11 sessions and falls back to the Cairo renderer via process re-exec ([#85](https://github.com/totoshko88/RustConn/issues/85))

### Improved
- **Context menu labels localized** — Copy, Paste, Select All strings now wrapped in `i18n()` for translation

### Dependencies
- pxfm 0.1.28 → 0.1.29
- tokio 1.52.0 → 1.52.1
- uuid 1.23.0 → 1.23.1

## [0.10.21] - 2026-04-16

### Security
- **Machine key encryption hardened** — removed predictable `hostname+username` fallback from `get_machine_key()`; `/etc/machine-id` fallback now uses HKDF-SHA256 with app-specific salt; `.machine-key` file created with `0600` permissions

### Fixed
- **Groups expand/collapse on double-click** — double-clicking anywhere on a group row now toggles expand/collapse, not just the expander icon ([#83](https://github.com/totoshko88/RustConn/issues/83))
- **Ctrl+K no longer hijacks terminal** — removed `Ctrl+K` from the global search shortcut; only `Ctrl+F` focuses the search box now, so `Ctrl+K` passes through to terminal applications like nano ([#83](https://github.com/totoshko88/RustConn/issues/83))
- **Right-click context menu on all SSH profiles** — set gesture propagation phase to `Capture` so the right-click handler fires before `TreeExpander` internal handlers that could swallow the event ([#83](https://github.com/totoshko88/RustConn/issues/83))
- **Filter bar opens below search box** — swapped layout order so protocol filters appear below the search entry instead of above it, preventing UI jump ([#83](https://github.com/totoshko88/RustConn/issues/83))

### Improved
- **Sidebar accessible labels localized** — wrapped `"Search connections"`, `"Search syntax help"`, `"Connection list"`, and `"Filter by {protocol} protocol"` in `i18n()` / `i18n_f()` for screen reader localization

### Dependencies
- aws-lc-rs 1.16.2 → 1.16.3
- aws-lc-sys 0.39.1 → 0.40.0
- clap 4.6.0 → 4.6.1

## [0.10.20] - 2026-04-15

### Fixed
- **RDP shared folders only used first folder path** — RDPDR backend now maps each drive to its own base path via `device_id`, so multiple shared folders work correctly in embedded IronRDP mode ([#82](https://github.com/totoshko88/RustConn/issues/82))
- **Tailscale CLI download broken by macOS-only release** — pinned version 1.96.5 only existed for macOS; downgraded to 1.96.4 (latest Linux build) and switched from static checksum to `SkipLatest` policy to prevent future platform-specific release breakage ([#81](https://github.com/totoshko88/RustConn/issues/81))
- **SSH Port Forwarding section missing from connection dialog** — the Port Forwarding group was silently not added because fragile widget tree navigation (`first_child → downcast → child → ...`) failed; now uses the content box directly from `create_ssh_options()` return value ([#80](https://github.com/totoshko88/RustConn/issues/80))

### Docs
- **Flatpak shared folders troubleshooting** — added "RDP Shared Folders in Flatpak" section to User Guide with `flatpak override` commands for granting filesystem access ([#82](https://github.com/totoshko88/RustConn/issues/82))

## [0.10.19] - 2026-04-15

### Added
- **Shell button in header bar** — moved the Local Shell button from the sidebar filter bar to the main header bar as a prominent accent-colored pill button with icon and label; always visible even when sidebar is hidden ([#76](https://github.com/totoshko88/RustConn/issues/76))
- **Optional protocol filter bar** — protocol filters can now be toggled on/off via a button in the search bar or in Settings → Interface → "Show protocol filters"; state is persisted across sessions; hidden by default for a cleaner interface ([#76](https://github.com/totoshko88/RustConn/issues/76))
- **Toggle protocol filters action** — `win.toggle-protocol-filters` window action with sidebar toggle button that persists visibility state to config
- **Tab group chooser dialog** — "Set Group..." dialog now shows existing groups as clickable pill buttons for quick selection, with a text field for creating new groups; no  manual retyping of group names
- **Close All in Group** — new context menu action on grouped tabs; shows a confirmation dialog with tab count and group name, then closes all tabs belonging to that group
- **Group name in tab tooltip** — hovering over a grouped tab now shows `[GroupName]` in the tooltip, visible even when split view colors are active
- **Group name as tab title prefix** — tab groups now display as a `[GroupName]` prefix in the tab title instead of a colored indicator icon; this separates group identity from split view / protocol color indicators, so both are visible simultaneously

### Fixed
- **Terminal not auto-focused after connection** — newly opened SSH session tabs now automatically grab keyboard focus so the user can type immediately; uses idle callback with selected-page guard to prevent focus-stealing when multiple tabs open concurrently ([#79](https://github.com/totoshko88/RustConn/issues/79))
- **SIGSEGV on rapid right-click on tab** — triple right-clicking a terminal tab caused a segfault because each click created a new popover without unparenting the previous one; now tracks the active popover and tears it down before creating a new one
- **Tray menu labels empty when "Minimize to tray" enabled** — the ksni tray `menu()` callback runs on a D-Bus worker thread where `gettext` is not initialised, causing `i18n()` to return empty strings; tray menu now uses plain English labels to avoid the thread-safety issue; window visibility is synced via periodic polling so the Show/Hide toggle stays correct
- **Tab group color conflict with split view** — tab groups and split view previously competed for the same `indicator_icon` slot; groups now use a title prefix while split view keeps the colored indicator, eliminating the conflict

### Improved
- **Wider sidebar** — increased minimum sidebar width from 160px to 360px for better readability of nested items and long hostnames; increased OverlaySplitView max from 280px to 360px default with up to 600px maximum
- **Filter bar cleanup on hide** — active protocol filters are automatically cleared when the filter bar is hidden to prevent invisible filtering confusion

### Dependencies
- bitflags 2.11.0 → 2.11.1
- clap_complete 4.6.1 → 4.6.2
- FreeRDP 3.24.0 → 3.24.1 (security fixes)
- hyper-rustls 0.27.8 → 0.27.9
- rand 0.9.3 → 0.9.4
- rayon 1.11.0 → 1.12.0
- rustls-webpki 0.103.11 → 0.103.12
- tokio 1.51.1 → 1.52.0
- VTE 0.80.0 → 0.80.3

## [0.10.18] - 2026-04-13

### Added
- **Terminal font zoom** — dynamically scale terminal font size using Ctrl+Scroll wheel, Ctrl+Plus/Minus keyboard shortcuts, and Ctrl+0 to reset; uses VTE's native `font_scale` for per-session zoom (0.5×–4.0×) ([#77](https://github.com/totoshko88/RustConn/issues/77))
- **Copy on select** — optional X11-style auto-copy: selected text is automatically copied to the clipboard; enable in Settings → Terminal → Behavior ([#78](https://github.com/totoshko88/RustConn/issues/78))

### Improved
- **Export group filter** — export dialog now includes a group selector to export only connections from a specific group and its subgroups; defaults to "All connections"
- **Import/Export format ordering** — RustConn Native (.rcn) is now the default format in both import and export dialogs; remaining formats sorted alphabetically

### Dependencies
- gio 0.22.4 → 0.22.5
- glib 0.22.4 → 0.22.5
- hyper-rustls 0.27.7 → 0.27.8
- libc 0.2.184 → 0.2.185
- openssl 0.10.76 → 0.10.77
- openssl-sys 0.9.112 → 0.9.113
- pkg-config 0.3.32 → 0.3.33
- rtoolbox 0.0.3 → 0.0.4
- rustls 0.23.37 → 0.23.38

## [0.10.17] - 2026-04-12

### Fixed
- **`clear` command not working in Flatpak SSH sessions** — the Flatpak sandbox inherits `TERM=dumb` from the host, and the previous fix only set `rustconn-256color` for local shells; remote commands (SSH, Telnet, etc.) kept the inherited `dumb` value, breaking `clear`, `htop`, `mc`, `tmux` on remote hosts; now force `TERM=xterm-256color` for all remote commands in Flatpak ([#25](https://github.com/totoshko88/RustConn/issues/25))
- **Sidebar scroll position lost after editing/moving connections** — `restore_state()` scheduled group expansion, scroll restoration, and selection as three independent idle callbacks that raced against each other; scroll was applied before groups finished expanding (which changes content height), causing the sidebar to jump to the top; now runs expansion and selection synchronously in one callback, then restores scroll in a chained second callback
- **Sorting collapsed all expanded groups** — `sort_connections()` and `sort_recent()` rebuilt the sidebar store without saving/restoring expanded group state; now preserves which groups were open before sorting

### Dependencies
- clap_complete 4.6.0 → 4.6.1
- rand 0.9.2 → 0.9.3
- Tailscale CLI 1.96.4 → 1.96.5

## [0.10.16] - 2026-04-10

### Fixed
- **Sidebar context menu actions still not working** — the v0.10.15 fix using `insert_action_group()` proxy was insufficient: `PopoverMenu` inside a `ListView`/`TreeExpander` hierarchy cannot reliably resolve `win.*` actions regardless of where the action group is injected; replaced `PopoverMenu` + `gio::Menu` with a plain `Popover` containing `Button` widgets that directly call `window.activate_action()`, completely bypassing GTK4 action-group resolution ([#75](https://github.com/totoshko88/RustConn/issues/75))

### Dependencies
- cc 1.2.59 → 1.2.60
- gif 0.14.1 → 0.14.2
- hashbrown 0.16.1 → 0.17.0
- indexmap 2.13.1 → 2.14.0
- js-sys 0.3.94 → 0.3.95
- ksni 0.3.3 → 0.3.4
- libredox 0.1.15 → 0.1.16
- redox_syscall 0.7.3 → 0.7.4
- rustls-webpki 0.103.10 → 0.103.11
- wasm-bindgen 0.2.117 → 0.2.118
- web-sys 0.3.94 → 0.3.95

## [0.10.15] - 2026-04-10

### Fixed
- **`clear` command not working in Flatpak** — the `clear` binary from ncurses-utils was missing inside the Flatpak sandbox; added a minimal ANSI escape sequence wrapper (`\033[H\033[2J\033[3J`) to all three Flatpak manifests so `clear` works out of the box ([#25](https://github.com/totoshko88/RustConn/issues/25))
- **Sidebar context menu items not working** — after migration to `PopoverMenu` in v0.10.14, clicking menu items did nothing because the popover lacked access to the window's action group; fixed by explicitly proxying `win.*` actions into the popover via `insert_action_group()` ([#75](https://github.com/totoshko88/RustConn/issues/75))
- **Keyboard shortcuts dialog showed wrong bindings** — 19 discrepancies between the shortcuts help dialog (`shortcuts.rs`) and the actual GTK accelerators (`keybindings.rs`): Ctrl+G was labeled "New group" (actually Password Generator), Ctrl+T was labeled "Open local shell" (actually Ctrl+Shift+T), Ctrl+\` was labeled "Focus terminal" (actually Focus Next Pane), F1 was labeled "Show about dialog" (actually Keyboard Shortcuts); all corrected to match the real bindings
- **Shortcuts dialog missing entries** — added 13 missing shortcuts: Quick Connect, Export, Command Palette, Focus Terminal, Close Pane, Connection History, Statistics, Password Generator, Wake On LAN, Toggle Fullscreen, Toggle Sidebar, and alternative accelerators

### Improved
- **FreeRDP stays at 3.24.1** — 3.24.2 release assets not yet published upstream; keeping 3.24.1 which includes all prior security fixes

### Documentation
- **Keyboard shortcuts fully synchronized** — User Guide shortcuts tables now match the actual keybindings registry; added missing entries for Ctrl+K (Search), Ctrl+PageDown/PageUp (tab switching), Ctrl+Shift+T (Local Shell), Ctrl+H (History), Ctrl+G (Password Generator), Ctrl+Shift+I (Statistics), Ctrl+Shift+L (Wake On LAN)
- **Terminal clear troubleshooting** — added User Guide section explaining VTE's Ctrl+L behavior (scrolls instead of erasing) and workarounds for `clear` command in Flatpak

## [0.10.14] - 2026-04-09

### Dead code cleanup
- **Removed unused CSS classes** — removed `.tab-icon`, `.tab-label`, `.tab-label-disconnected`, `.tab-close-button` (replaced by AdwTabView), `.focused-pane`/`.unfocused-pane` (replaced by `.focused-panel`), `notebook > header > tabs > tab` selector (no longer using GtkNotebook), and stale comment placeholders; updated section headers for clarity

### Improved
- **Success notifications use Toast instead of modal dialogs** — snippet creation, cluster creation now show non-blocking `adw::Toast` instead of `adw::AlertDialog` (GNOME HIG compliance); remaining `show_success` calls with detailed counts (import/export/delete) kept as alerts
- **Fixed missing i18n for export/connection test dialogs** — `"Export Complete"`, `"Connection Test Successful"`, and `"Connection successful! Latency: Xms"` were hardcoded English; now wrapped in `i18n()`/`i18n_f()` for proper localization
- **Accessible labels for status icons and split panels** — sidebar connection status icons (`Connected`, `Connecting`, `Connection failed`) now use `i18n()` for localized screen reader announcements; split panel containers have accessible `"Terminal panel"` label
- **Sidebar context menus migrated to PopoverMenu** — replaced manual `Button`-based `Popover` with `PopoverMenu` + `gio::Menu` for both connection/group and empty-space context menus; provides native GNOME HIG look, keyboard arrow navigation, and screen reader accessibility out of the box

### Fixed
- **Sidebar context menu missing Delete action** — context menu for both connections and groups was cut off at the bottom, hiding the Delete item; fixed by attaching popover to the clicked widget instead of the window, allowing GTK to properly calculate available space and scroll long menus

### Documentation
- **RDP File Transfer** — added User Guide section documenting shared folders (drive redirection) and clipboard file transfer (IronRDP embedded mode "Save Files" button)
- **Complete translations for all 15 languages** — filled all empty/fuzzy translations for be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz; fixed broken PO headers in 10 files; updated version to 0.10.14

## [0.10.13] - 2026-04-08

### Fixed
- **SSH auto-reconnect infinite loop** — when an SSH session failed with "Permission denied" (exit code 255), the auto-reconnect polling detected the host as online (TCP port open) and immediately triggered a reconnect, which failed again with the same auth error, creating an exponential loop of sessions. Fixed by skipping auto-reconnect for SSH authentication failures (exit code 255); the user can still reconnect manually via the overlay button
- **Duplicate `child-exited` handlers for SSH/Telnet** — `setup_child_exited_handler` was called twice per session (before and after spawn), registering two GLib signal handlers. Each exit event fired both handlers, spawning two parallel auto-reconnect polls per failure cycle and doubling the session count on every iteration

### Dependencies
- FreeRDP 3.24.0 → 3.24.1 (security fix: CVE patches for credential zeroing, codec fixes)
- Boundary CLI 0.21.1 → 0.21.2 (search sorting flags)
- tokio 1.51.0 → 1.51.1, toml_edit 0.25.10 → 0.25.11

## [0.10.12] - 2026-04-07

### Security
- **VNC password stored as `SecretString`** — `VncConfig.password` changed from plain `String` to `secrecy::SecretString`, matching RDP/SSH/SPICE credential handling; password is now zeroized on drop and protected from accidental logging via `Debug` trait
- **VNC pixel buffer max resolution guard** — `VncPixelBuffer::new()` and `resize()` now clamp dimensions to 16384×16384 (1 GB max), preventing OOM from a malicious VNC server claiming absurd resolution

### Improved
- **RDP 4K frame conversion zero-copy** — `convert_to_bgra()` now returns `Cow<[u8]>` instead of `Vec<u8>`; when pixel data is already in BGRA format (the common IronRDP case), the function returns a borrowed slice instead of cloning the entire frame buffer (33 MB at 4K per frame)
- **Sidebar search highlight regex cached** — `highlight_match()` now accepts a pre-compiled `Regex` via new `compile_highlight_regex()` helper; the regex is compiled once per query change instead of once per visible list item per keystroke
- **Log sanitization custom patterns pre-compiled** — `SanitizeConfig` now pre-compiles custom regex patterns at construction time instead of recompiling on every call to `sanitize_output()`; affects every line of terminal output when session logging is enabled
- **Log sanitization redundant `to_lowercase()` removed** — `SENSITIVE_PATTERNS` are already lowercase constants; removed unnecessary `pattern.to_lowercase()` allocation on every pattern comparison

### Dead code cleanup
- **Removed `wayland_surface.rs`** — ~1050-line stub module with no callers; all types (`WaylandSubsurface`, `EmbeddedRenderer`, `ShmBuffer`, `DamageRect`, `RenderingMode`) were unused; native Wayland subsurface support can be restored from git history when needed
- **Removed `TracingOutput::OpenTelemetry` variant** — deprecated placeholder that was never constructed; match arm fell back to stderr
- **Removed RDPDR `FileLock` struct and `notify_directory_change()` stub** — dead code placeholders for unimplemented fcntl integration
- **Removed commented-out code** — `set_allow_bold` (VTE4 incompatible), `--full-screen` SPICE arg

### Dependencies
- **Updated**: fastrand 2.4.0→2.4.1, gdk4 0.11.1→0.11.2, gdk4-sys 0.11.1→0.11.2, gio 0.22.2→0.22.4, glib 0.22.3→0.22.4, gtk4 0.11.1→0.11.2, gtk4-sys 0.11.1→0.11.2, libz-sys 1.1.26→1.1.28, pango 0.22.0→0.22.4, zip 8.5.0→8.5.1
- **CLI downloads** — TigerVNC 1.16.1→1.16.2 (security fix for x0vncserver), Teleport 18.7.2→18.7.3, Bitwarden CLI 2026.2.0→2026.3.0

## [0.10.11] - 2026-04-05

### Added
- **RDP Mouse Jiggler** — prevents idle disconnect by sending periodic mouse movements; configurable interval (10–600 seconds, default 60); auto-starts when RDP session connects, auto-stops on disconnect; works with both IronRDP embedded and FreeRDP external modes; settings in Connection Dialog → RDP → Features
- **Connect All in Folder** — right-click a group in the sidebar → "Connect All" opens all connections in that group simultaneously
- **Copy Username / Copy Password from context menu** — right-click a connection → "Copy Username" or "Copy Password" copies credentials to clipboard; password auto-clears from clipboard after 30 seconds for security; uses cached credentials resolved during previous connection
- **Host Online Check** — right-click a connection → "Check if Online" starts async TCP port probing (polls every 5s for up to 2 minutes); auto-connects when host becomes reachable; shows toast notifications for status updates
- **WoL + Auto-Connect** — Wake On LAN now automatically polls the host after sending the magic packet (up to 5 minutes) and auto-connects when the host comes online; replaces the previous fire-and-forget WoL behavior
- **Auto-reconnect on session failure** — when an SSH session disconnects unexpectedly (server reboot, network failure), RustConn automatically starts polling the host (every 5s for up to 5 minutes) and reconnects when the server comes back online; the reconnect banner is still shown for manual reconnect if auto-reconnect times out
- **Host check module** (`rustconn-core::host_check`) — async TCP connect probe with configurable timeout, polling interval, and max duration; cancellation support via `AtomicBool`; `check_host_online()` for single probe, `poll_until_online()` for continuous monitoring
- **Terminal Activity Monitor** — per-session activity and silence detection for terminal tabs, inspired by KDE Konsole ([#72](https://github.com/totoshko88/RustConn/issues/72)); three monitoring modes: Off (default), Activity (notify when new output appears after a configurable quiet period), and Silence (notify when no output occurs for a configurable timeout); notifications delivered through tab indicator icons, in-app toasts, and desktop notifications (when window is unfocused); per-connection config overrides global defaults; settings in Connection Dialog → Advanced → Activity Monitor and Settings → Monitoring → Activity Monitor; tab context menu "Monitor: Off/Activity/Silence" for quick mode cycling; property-based tests for mode cycling, serde round-trip, config resolution, and timeout clamping

### Fixed
- **RDP tabs auto-close on initial connection failure** — RDP tabs that fail during initial connection (CredSSP auth error, connection refused, timeout) now close automatically instead of showing a useless "failed" tab; disconnected tabs are still shown for sessions that were previously connected (for reconnect)
- **Group context menu detection** — fixed `is_group` detection in sidebar context menu to use `ConnectionItem.is_group()` instead of icon name check; groups with custom emoji icons now correctly show group-specific menu items (Connect All, New Connection in Group)

### Dependencies
- **Updated**: fastrand 2.3.0→2.4.0

## [0.10.10] - 2026-04-04

### Changed
- **Flatpak: removed extra sandbox permissions rejected by Flathub lint** — reverted `--filesystem=home/.hoop:ro`, `--filesystem=xdg-run/gnupg:ro`, `--filesystem=home/.var/app/com.bitwarden.desktop/data:ro`, and `--filesystem=xdg-run/ssh-agent:ro` from Flatpak and Flathub manifests; these permissions are now added manually by users via `flatpak override` after installation (see [Flatpak Sandbox Overrides](docs/USER_GUIDE.md#flatpak-sandbox-overrides)); prompted by [flathub-infra/flatpak-builder-lint#972](https://github.com/flathub-infra/flatpak-builder-lint/pull/972#pullrequestreview-4051168156)

### Added
- **User Guide: Flatpak Sandbox Overrides section** — documents how to add filesystem permissions for alternative SSH agent sockets (KeePassXC, Bitwarden, GPG agent, 1Password) and Hoop.dev CLI config after Flatpak installation ([User Guide → Flatpak Sandbox Overrides](docs/USER_GUIDE.md#flatpak-sandbox-overrides))

### Improved
- **Bulk delete dialog migrated to AdwAlertDialog** — replaced custom `adw::Window` with `adw::AlertDialog` using `set_close_response("cancel")` and `ResponseAppearance::Destructive`, following GNOME HIG for destructive confirmation dialogs
- **Background thread result delivery** — `spawn_blocking_with_callback` now uses event-driven `glib::MainContext::channel()` instead of 16ms polling timer, reducing unnecessary main loop wake-ups
- **vault_ops unit tests** — added 14 tests for `select_backend_for_load` (8 backend selection scenarios including KeePass fallback logic) and `generate_store_key` (6 key format scenarios across LibSecret, Bitwarden, 1Password, Pass backends)

### Dependencies
- **Updated**: cc 1.2.58→1.2.59, coreaudio-rs 0.14.0→0.14.1, indexmap 2.13.0→2.13.1, libz-sys 1.1.25→1.1.26, semver 1.0.27→1.0.28, tokio 1.50.0→1.51.0, tokio-macros 2.6.1→2.7.0, writeable 0.6.2→0.6.3, yuv 0.8.12→0.8.13
- **CLI downloads** — TigerVNC 1.16.0→1.16.1

## [0.10.9] - 2026-04-02

### Added
- **Hoop.dev Zero Trust provider** — added Hoop.dev as the 11th Zero Trust provider; supports `hoop connect <connection-name>` with optional `--api-url` and `--grpc-url` flags; includes data model (`HoopDevConfig`), CLI detection (`detect_hoop()`), Flatpak CLI download component, GUI fields in connection dialog, CLI support (`--provider hoop_dev --hoop-connection-name`), Flatpak `~/.hoop:ro` permission, serialization round-trip, i18n, and property-based tests
- **Custom SSH agent socket override** — users can now specify a custom `SSH_AUTH_SOCK` path at two levels: a global setting in Settings → SSH Agent tab (applies to all connections) and a per-connection override in Connection Dialog → SSH tab (overrides global and auto-detected socket); resolves the Flatpak limitation where `--socket=ssh-auth` hard-overwrites `SSH_AUTH_SOCK`, preventing use of alternative agents like KeePassXC or Bitwarden SSH agent ([#71](https://github.com/totoshko88/RustConn/issues/71))
- **CLI `--ssh-agent-socket`** — `rustconn-cli add` and `update` commands accept `--ssh-agent-socket <PATH>` to set per-connection SSH agent socket; `show` command displays the value when set
- **Socket path validation** — real-time feedback in both Settings and Connection dialogs: green for valid socket, yellow for path not found (non-blocking), red for non-absolute path
- **Flatpak: alternative SSH agent socket access** — added `--filesystem` permissions for GPG agent (`xdg-run/gnupg`), Bitwarden SSH agent (`home/.var/app/com.bitwarden.desktop/data`), and custom sockets (`xdg-run/ssh-agent`) in Flatpak and Flathub manifests

### Fixed
- **Orphaned subgroups on group delete** — deleting a group containing only empty subgroups (0 connections) via the GUI now cascade-deletes all descendant subgroups instead of reparenting them to root; CLI `group delete` now delegates to `ConnectionManager` instead of manual `groups.retain()`, fixing dangling `parent_id` references on child groups
- **Startup error dialog orphaned window** — `show_error_dialog` no longer creates a temporary `ApplicationWindow` that lingers after dismissal; now presents via `app.active_window()` parent

### Security
- **Tar archive path traversal (defense-in-depth)** — CLI component downloads now validate each tar entry path against `..` traversal and absolute paths before extraction, matching the existing `enclosed_name()` protection for zip archives; pinned `tar >= 0.4.45` (CVE-2026-33056)
- **RDP certificate validation** — changed default `ignore_certificate` from `true` to `false`; FreeRDP now uses `/cert:tofu` (trust-on-first-use) by default instead of unconditional `/cert:ignore`; applies to all RDP paths (external FreeRDP, embedded launcher, embedded thread)
- **Bitwarden session key no longer exposed in process list** — session key is now passed via `BW_SESSION` environment variable instead of `--session` CLI argument, preventing exposure in `/proc/PID/cmdline`
- **1Password credentials no longer exposed in process list** — password field values are now piped via stdin instead of passed as CLI arguments to `op item create/edit`
- **Export file permissions hardened** — KDBX XML exports and all connection export files now set `0600` (owner-only) permissions on Unix, preventing world-readable credential/topology data
- **Bitwarden session key cleared on vault lock** — `lock_vault()` now calls `clear_session_key()` alongside `clear_verified()`, ensuring the session key does not persist in memory after lock
- **VNC custom args blocklist** — dangerous VNC viewer arguments (`-via`, `-passwd`, `-passwordfile`, `-securitytypes`, `-proxyserver`, `-listen`) are now blocked, matching the existing RDP custom args blocklist
- **FreeRDP extra args blocklist** — `extra_args` in FreeRDP external mode now filtered through the same dangerous-prefix blocklist (`/p:`, `/password:`, `/shell:`, `/proxy:`) as RDP `custom_args`
- **Pass backend path traversal prevention** — `build_pass_path()` now sanitizes `connection_id` and `field` by replacing `/`, `\`, `.` with `_`, preventing directory traversal in the password store
- **Log sanitization expanded** — added `passphrase:`, `client_secret:`, `authorization:` to sensitive prompt patterns; added GitHub (`ghp_*`), GitLab (`glpat-*`), and JWT (`eyJ*`) token detection to value patterns

### Corrected
- **Flatpak `--device=all` clarification** — v0.9.11 release notes incorrectly stated Flatpak permissions were "scoped to `--device=serial`"; Flatpak has no granular `--device=serial` option — the actual permission is `--device=all`, which is required for serial port access via picocom

### Improved
- **Asbru import regex cached** — `convert_asbru_variables()` now uses `LazyLock<Regex>` instead of compiling the regex on every call, matching the pattern used throughout the rest of the codebase
- **Snippet validation strings translated** — "Snippet name is required" and "Command is required" wrapped in `i18n()` for localization
- **Framebuffer fallback warning** — RDP, VNC, and SPICE embedded viewers now log `tracing::warn!` (once per session) when the legacy `to_vec()` pixel buffer copy path is activated instead of `CairoBackedBuffer`
- **Clippy suppressions scoped to GUI crate** — 8 GTK-specific clippy suppressions (`redundant_clone`, `needless_borrow`, `needless_pass_by_value`, `unused_self`, `wildcard_imports`, `needless_borrows_for_generic_args`, `redundant_closure_for_method_calls`, `redundant_closure`) moved from workspace `Cargo.toml` to `rustconn/Cargo.toml`; `rustconn-core` now linted under stricter rules

### Dependencies
- **Updated**: aws-lc-sys 0.39.0→0.39.1, cc 1.2.57→1.2.58, cmake 0.1.57→0.1.58, hybrid-array 0.4.8→0.4.10, hyper 1.8.1→1.9.0, libc 0.2.183→0.2.184, mio 1.1.1→1.2.0, simd-adler32 0.3.8→0.3.9, system-deps 7.0.7→7.0.8, toml_edit 0.25.8→0.25.10, uuid 1.22.0→1.23.0, winnow 1.0.0→1.0.1, zerocopy 0.8.47→0.8.48, zip 8.4.0→8.5.0, zune-jpeg 0.5.14→0.5.15
- **CLI downloads** — Tailscale 1.96.2→1.96.4

## [0.10.8] - 2026-03-27

### Fixed
- **Flatpak: gcloud install fails with read-only filesystem** — `install.sh` now runs with `CLOUDSDK_CONFIG` pointing to the writable sandbox directory, preventing `OSError: [Errno 30]` on `~/.config/gcloud/`

### Improved
- **SPICE/VNC embedded rendering performance** — replaced per-frame `to_vec()` pixel buffer copy with persistent `CairoBackedBuffer` (in-place surface updates + `mark_dirty_rectangle`); eliminates 8–33 MB allocation per frame depending on resolution; same zero-copy pattern already used by embedded RDP since 0.10.7
- **`CairoBackedBuffer` extracted to shared module** — `cairo_buffer.rs` is now used by RDP, VNC, and SPICE embedded widgets instead of three separate implementations
- **`parse_version` regex cached** — `secrets_tab.rs` now reuses `VERSION_REGEX` from `rustconn-core` instead of compiling a new regex on every call
- **`VARIABLE_REGEX` deduplicated** — identical regex was compiled in three modules (`variables/manager.rs`, `snippet/manager.rs`, `utils.rs`); now defined once and re-exported

## [0.10.7] - 2026-03-26

### Changed
- **RDP default quality mode** — new RDP connections now default to Quality (RemoteFX) instead of Balanced; existing connections with explicitly saved Balanced or Speed settings are not affected

### Fixed
- **SPICE fallback viewer reported as failed** — `connect_with_fallback()` returned an error even when the external SPICE viewer launched successfully; now returns `Ok(())` so the GUI correctly shows the connected state
- **SPICE embedded mouse clicks at wrong position** — click and release events sent coordinates (0,0) instead of the actual cursor position; now applies the same widget-to-framebuffer coordinate transformation as mouse motion
- **RDP file import ignores gateway port** — `.rdp` parser read gateway port from `gatewayaccesstoken` instead of the standard `gatewayport` field; gateway connections now use the correct port
- **Session type misclassified for terminal protocols** — only SSH was classified as embedded; Telnet, Serial, Kubernetes, and MOSH sessions are now correctly classified as terminal-embedded
- **MOSH `--ssh` argument not parsed correctly** — `--ssh=ssh -p PORT` was passed as a single argument; now split into `--ssh` and `ssh -p PORT` as two separate arguments for correct parsing
- **MOSH connections accepted port 0** — `validate_connection()` now rejects port 0, consistent with SSH and other protocols
- **Config file corruption on power failure** — synchronous `save_toml_file` now calls `sync_all()` before atomic rename, matching the async version's durability guarantee
- **CLI `delete` auto-confirms in non-interactive mode** — piped input no longer auto-confirms destructive operations; use `--force` to bypass confirmation in scripts
- **CLI `add` allows duplicate connection names** — now returns an error if a connection with the same name already exists
- **CLI `group delete` leaves orphaned connections** — connections belonging to a deleted group now have their `group_id` cleared
- **CLI `update` uses case-sensitive exact match** — now uses `find_connection` for case-insensitive and fuzzy matching, consistent with other commands
- **FreeRDP 2.x flagged as version-incompatible** — detection entries for `wlfreerdp`/`xfreerdp` (2.x) had `min_version("3.0.0")`; corrected to `"2.0.0"`
- **External window saves default size instead of current** — `setup_close_handler` now uses `window.width()`/`height()` to capture actual dimensions after user resize
- **Cluster dialog buttons break on layout change** — Select All / Deselect All buttons are now stored as struct fields instead of being found via fragile `parent()` traversal
- **Whitespace-only group and snippet names accepted** — `validate_group` and `validate_snippet` now trim names before checking emptiness
- **Tray dirty-check hash collision** — replaced simple timestamp sum with `DefaultHasher` combining connection IDs and timestamps
- **`Connection::default_port` duplicated `ProtocolType::default_port`** — now delegates to `self.protocol.default_port()`

### Security
- **Script credential resolver password not zeroed** — intermediate `String` holding the password from script output is now zeroed via `zeroize::Zeroize` after wrapping in `SecretString`
- **Encrypted credential changes not detected** — `SecretSettings::PartialEq` now includes all `*_encrypted` fields so save-if-changed logic detects credential updates

### Improved
- **Highlight rules performance** — `CompiledHighlightRules` now uses `RegexSet` for fast initial filtering before running individual regexes; avoids executing every pattern on every terminal line
- **Command palette sort performance** — `SearchEngine` is now created once before sorting instead of inside every comparator call
- **GTK main loop polling** — `poll_for_result` uses `timeout_add_local` at 16ms intervals instead of `idle_add_local_once` to avoid busy-spinning
- **Terminal themes cached** — `all_themes()` and `theme_names()` use `OnceLock` to avoid repeated allocation
- **Fuzzy search allocation** — `fuzzy_score_optimized` replaced `to_lowercase()` with allocation-free case-insensitive search
- **Export runs on background thread** — large exports no longer freeze the UI
- **CLI download default allocation** — reduced from 10MB to 1MB for small downloads
- **Group descendant collection** — `collect_descendant_groups` uses `HashSet` for O(1) lookups instead of O(n) `Vec::contains`
- **`parse_args` supports quoted strings** — uses `shell_words::split()` so RDP arguments with spaces and quotes are parsed correctly
- **Tray menu translated** — all tray menu strings wrapped in `i18n()`
- **Password generator tips translated** — security tip strings wrapped in `i18n()`
- **Session restore version validation** — `from_json` now warns on version mismatch for forward compatibility
- **ZeroTrust protocol registry documented** — `get_by_type()` explains that ZeroTrust delegates to provider-specific protocols
- **Wayland subsurface code documented** — dead Wayland native paths annotated as future extension points
- **Duplicate CSS rules removed** — `.status-connected` and `.status-connecting` were defined twice in sidebar CSS
- **Dead Flatpak config helpers removed** — unused `get_flatpak_boundary_config_dir` and `get_flatpak_cloudflared_config_dir`
- **`CredentialResolutionContext` struct** — replaces 8-argument function with a bundled context struct
- **Embedded RDP 4K performance** — replaced per-frame 33MB pixel buffer clone (`data.to_vec()`) with a persistent Cairo `ImageSurface` that is updated in-place via `surface.data()` + `mark_dirty_rectangle()`; eliminates the main bottleneck that caused near-slideshow rendering at 4K resolution; old `PixelBuffer` path kept as fallback for FreeRDP external mode
- **RDP frame extraction optimized** — `extract_region_data` replaced per-pixel copy+swap loop with row-based `memcpy` + bulk R↔B channel swap; full-frame fast path avoids row-by-row copy when region covers entire image; LLVM auto-vectorizes the swap loop into SIMD on x86_64
- **RDP cursor artifacts (random pixels below cursor)** — cursor bitmaps from IronRDP are padded to 32×32 or 64×64 with transparent rows; on HiDPI the downscale + compositor upscale caused color bleeding at transparency edges; now crops transparent padding before downscale and uses premultiplied alpha (`B8g8r8a8Premultiplied`) to prevent bleed; R↔B channel swap moved from session layer to cursor handler to avoid double-swap

## [0.10.6] - 2026-03-24

### Fixed
- **Passbolt CLI integration broken with CLI 0.4.2** — `PassboltResourceDetail` deserialization failed because serde looked for `"_id"`, `"_name"`, `"_uri"`, `"_description"` instead of lowercase `"id"`, `"name"`, `"uri"`, `"description"` returned by Passbolt CLI 0.4.2; added `serde(rename)` for all underscore-prefixed fields; made `_id` and `_name` optional since `get resource` no longer returns `id`; added `folder_parent_id` field; same fix applied to `PassboltResource` for `_username` and `_uri` ([#69](https://github.com/totoshko88/RustConn/issues/69))
- **Blurry/artifact RDP image on HiDPI displays** — embedded IronRDP framebuffer was double-scaled on HiDPI (device→CSS→device) because Cairo surface lacked `set_device_scale`; now sets device scale on the pixel buffer surface so Cairo renders 1:1 at native resolution; also uses adaptive filter (Nearest for 1:1, Bilinear for actual scaling)
- **1Password JSON parse errors silently ignored** — `op item list` parse failures were swallowed by `unwrap_or_default()`, masking real issues; now logs warning via `tracing::warn!`

### Changed
- **CLI downloads** — 1Password CLI 2.33.0→2.33.1

### Dependencies
- **Updated**: ipconfig 0.3.2→0.3.4, libredox 0.1.14→0.1.15, proptest 1.10.0→1.11.0

## [0.10.5] - 2026-03-24

### Fixed
- **KeePassXC CLI integration not working** — all vault write/rename/delete/copy operations passed `None` as database password to `keepassxc-cli`, causing "Invalid credentials" errors when the KDBX file is password-protected; now correctly passes `kdbx_password` from settings in all 10 call sites across GUI (`vault_ops.rs`) and CLI (`secret.rs`) ([#68](https://github.com/totoshko88/RustConn/issues/68))
- **KeePassXC CLI silent error swallowing** — `get_password_from_kdbx` silently returned `Ok(None)` for unrecognized errors; `get_password_from_kdbx_with_key` silently skipped failed path attempts; now logs warnings via `tracing::warn!`/`tracing::debug!` for all failure paths
- **KeePassXC CLI missing `-q` flag** — added `-q` (quiet) flag to all `keepassxc-cli show` commands and `verify_kdbx_credentials` to suppress interactive password prompts in scripted usage
- **GTK warnings on application startup** — suppressed `Adwaita-WARNING: gtk-application-prefer-dark-theme` on KDE/XFCE by clearing the deprecated property before `adw::init()`; removed unsupported `@media (prefers-reduced-motion)` CSS media query that caused GTK theme parser warning

### CI
- **GitHub Actions Node.js 20 deprecation** — replaced `flathub-infra/flatpak-github-actions/flatpak-builder@master` (Node.js 20) with `flatpak/flatpak-github-actions/flatpak-builder@v6` (Node.js 24)

### Dependencies
- **Updated**: deflate64 0.1.11→0.1.12, toml 1.0.7→1.1.0, zip 8.3.1→8.4.0

## [0.10.4] - 2026-03-22

### Fixed
- **Flatpak: Zero Trust CLIs crash on read-only filesystem** — gcloud, Azure CLI, Teleport, and OCI CLI need writable config directories; Flatpak mounts host dirs as read-only or doesn't mount them at all; now redirects CLI config paths to writable sandbox directories via environment variables (`CLOUDSDK_CONFIG`, `AZURE_CONFIG_DIR`, `TELEPORT_HOME`, `OCI_CLI_CONFIG_FILE`); bootstraps credentials from host mounts where available; Boundary uses system keyring via D-Bus (works natively in Flatpak); Cloudflare Access SSH uses browser-based auth (no persistent config needed); GCP IAP also gets `--ssh-key-file` and `--strict-host-key-checking=no` to handle read-only `~/.ssh/`
- **Flatpak: Zero Trust CLI tools not found** — `is_host_command_available()` used default PATH which doesn't include Flatpak CLI directories (`~/.var/app/.../cli/`); now uses extended PATH from `get_cli_path_dirs()` so AWS SSM, gcloud, and other installed CLIs are detected correctly
- **Failed connections stuck in "connecting" (yellow) state** — when `start_connection()` returned `None` (e.g. missing CLI, validation error), sidebar status was never reset; now transitions to "failed" (red) on connection launch failure
- **VTE runtime warning on regex match registration** — `match_add_regex()` requires `PCRE2_MULTILINE` compile flag; highlight rules and search highlight regexes were compiled with flags=0, causing `_vte_regex_has_multiline_compile_flag` assertion warning

### Improved
- **Flatpak manifests: FreeRDP and Waypipe modules** — added missing `freerdp` module to `packaging/flatpak/io.github.totoshko88.RustConn.yml` and `packaging/flathub/io.github.totoshko88.RustConn.yml`; added missing `waypipe` module to `packaging/flatpak/io.github.totoshko88.RustConn.yml` — matches documentation claim "FreeRDP 3.24.0 bundled in Flatpak"
- **i18n: 3 untranslated UI strings wrapped** — `"Failed to start"` in settings, `"Enter text above to test patterns"` and `"No patterns matched"` in connection dialog highlight rules, `"Import Failed"` in import dialog, `"Pasted {} chars"` in VNC clipboard — all translated across 15 languages
- **Snap license corrected** — `GPL-3.0+` → `GPL-3.0-or-later` (SPDX)
- **ARM64 release builds** — added `build-deb-arm64`, `build-rpm-arm64`, and `build-appimage-arm64` jobs to release workflow using QEMU emulation

- Updated: `moka` 0.12.14→0.12.15, `yuv` 0.8.11→0.8.12
- **CLI downloads** — Tailscale 1.94.2→1.96.2
- **Libvirt daemon import** — new import source "Libvirt Daemon (virsh)" queries running libvirtd for VMs via `virsh dumpxml`, reusing the existing XML parser; supports `qemu:///session`, `qemu:///system`, and remote URIs ([#63](https://github.com/totoshko88/RustConn/issues/63))

## [0.10.3] - 2026-03-21

### Security
- **RDP password no longer exposed in `/proc`** — legacy `RdpLauncher` passed password as `/p:{pass}` CLI argument visible to all system users; now uses `/from-stdin` pipe matching `SafeFreeRdpLauncher` behavior
- **SSH agent askpass script zeroized before deletion** — passphrase temp file in `/tmp/rustconn-askpass-*/` is now overwritten with zeros and fsynced before `remove_dir_all`, preventing recovery after abnormal termination
- **CLI `--password` flag shows security warning** — `rustconn-cli secret set --password` now prints a warning that the value is visible in process listings and recommends the interactive prompt
- **Legacy XOR credential decryption now logged** — transparent XOR→AES-256-GCM migration now emits `tracing::warn!` so administrators can track remaining legacy credentials

### Fixed
- **Highlight rules not applied without per-connection rules** — built-in defaults (ERROR, WARNING, CRITICAL, FATAL) and global highlight rules were skipped when a connection had no per-connection rules; removed the `is_empty()` guard so highlights always apply ([#66](https://github.com/totoshko88/RustConn/issues/66))
- **CLI `add --protocol zerotrust` silently created SSH connection** — now returns an error instead of logging and falling back to SSH
- **Config file corruption on crash** — sync `save_toml_file` now uses atomic temp-file + rename pattern matching the async version
- **Blocking DNS in async `check_port_async`** — replaced `to_socket_addrs()` with `tokio::net::lookup_host()` to avoid blocking the tokio worker thread

### Improved
- **Sidebar shows full connection name on hover** — tooltip displays full name and host for truncated entries; removed `max_width_chars` limit so labels use all available sidebar space
- **Log sanitization performance** — `sanitize_output()` regex patterns compiled once via `LazyLock` instead of on every call; `SENSITIVE_PATTERNS` deduplicated from 29 to 16 lowercase-only entries
- **CLI `parse_protocol` consolidated** — three duplicate implementations in `add.rs`, `template.rs`, `smart_folder.rs` replaced with shared `parse_protocol_type()` + `default_port_for_protocol()` in `util.rs`
- **`ProtocolResult<T>` deduplication** — removed duplicate type alias from `protocol/mod.rs`, now re-exported from `error.rs`
- **OpenTelemetry tracing variant marked deprecated** — `TracingOutput::OpenTelemetry` now has `#[deprecated]` attribute until implementation is complete
- **Dead code cleanup** — removed unused `AppStateError`, `VncLauncher`, `FieldValidator`/`FormValidator` framework, `initialize_secret_backends()`, `create_async_resolver()`

- Updated: `rustls-webpki` 0.103.9→0.103.10, `zune-jpeg` 0.5.13→0.5.14
## [0.10.2] - 2026-03-20

### Fixed
- **MOSH connections not working** — `start_connection()` dispatch was missing the `"mosh"` arm; MOSH connections silently failed. Added `start_mosh_connection()` with port check, binary detection, and CLI feedback
- **Auto-recording not triggered** — `session_recording_enabled` toggle in connection dialog had no effect; wired auto-recording into SSH, Telnet, Serial, Kubernetes, and MOSH connection handlers using `connect_contents_changed` callback
- **Highlight rules not applied** — per-connection `highlight_rules` were saved but never passed to `TerminalNotebook`; wired `set_highlight_rules()` call into all protocol handlers after terminal tab creation
- **`script` command visible on recording start** — replaced synchronous `feed()` erase with 100ms delayed erase via `glib::timeout_add_local_once` so PTY echo arrives before the clear sequence; added leading space for `HISTCONTROL=ignorespace`
- **Double exit and UI freeze on recording stop** — replaced `exit\n` with `\x04` (Ctrl+D/EOF) to terminate `script` sub-shell without visible echo; moved SCP file retrieval and remote cleanup to background thread via `spawn_blocking_with_callback`
- **Lost commands in recording playback** — added `strip_script_command_echo()` that removes the echoed `script -q -f --log-out …` line from recording data with timing entry adjustment, analogous to existing `strip_script_header()`
- **.rdp files not opening on double-click** — created `application/x-rdp` MIME type XML definition (`io.github.totoshko88.RustConn-rdp.xml`); installed in all packaging formats: Flatpak, Flathub, OBS RPM/DEB, native install script ([#64](https://github.com/totoshko88/RustConn/issues/64))
- **Sidebar stretching with long connection names** — added `ellipsize(End)` and `max_width_chars(35)` to sidebar connection label ([#64](https://github.com/totoshko88/RustConn/issues/64))
- **picocom not detected in Flatpak** — `picocom --help` returns exit code 1 on v3.x causing detection failure; added `which_binary()` fallback that confirms binary existence without running it ([#62](https://github.com/totoshko88/RustConn/issues/62))
- **RDP "indefinite connection" with no feedback** — improved error message when FreeRDP is not installed: now shows "Install FreeRDP 3.x (xfreerdp3 or wlfreerdp3)" instead of raw error ([#61](https://github.com/totoshko88/RustConn/issues/61))
- **IronRDP debug log spam** — filtered `ironrdp`, `ironrdp_session`, `ironrdp_tokio` crates to `warn` level in tracing subscriber; suppresses noisy `Non-32 bpp compressed RLE_BITMAP_STREAM` messages

### Improved
- **CSV import auto-detects delimiter** — `.tsv` files use tab; for `.csv` files, heuristic compares comma/semicolon/tab counts in the first line and picks the most frequent separator
- **Script credentials test feedback** — "Test Script" button now runs the configured command with 30s timeout, shows success with masked output preview or failure with stderr and exit code
- **Config sync documentation** — added "Configuration Sync Between Machines" section to User Guide with Git, Syncthing/rsync, CLI export/import, and built-in Backup/Restore instructions

- New: `shell-words` 1.x added to `rustconn` crate (script credential test button)
- Updated: `aws-lc-rs` 1.16.1→1.16.2, `aws-lc-sys` 0.38.0→0.39.0, `itoa` 1.0.17→1.0.18, `tar` 0.4.44→0.4.45

## [0.10.1] - 2026-03-19

### Note
Thank you to **Todor Todorov** for the support and for pointing out that the donation link was broken. The donation service has been changed and is now working. Today marks 8 months of active development on RustConn. If you'd like to support the project financially, I'd be very grateful: [https://donatello.to/totoshko88](https://donatello.to/totoshko88)

### Added
- **MOSH protocol** — new protocol type with predict mode (Adaptive/Always/Never), SSH port, UDP port range, server binary path, and custom arguments; `MoshProtocol` handler with `build_command()`, `detect_mosh()` in detection module; GUI tab in connection dialog; CLI support
- **CSV import/export** — RFC 4180 compliant CSV parsing and generation; auto column mapping from headers (`name`, `host`, `port`, `protocol`, `username`, `group`, `tags`, `description`); configurable delimiter (comma, semicolon, tab); GUI import dialog with column mapping preview; CLI `import --format csv` and `export --format csv` with `--delimiter` and `--fields` options
- **Session recording** — scriptreplay-compatible format (data + timing files); per-connection toggle in Advanced tab; `●REC` indicator in tab title; sanitization of sensitive output; recordings saved to `$XDG_DATA_HOME/rustconn/recordings/`
- **Text highlighting rules** — regex-based pattern matching with foreground/background colors; per-connection and global rules; built-in defaults for ERROR (red), WARNING (yellow), CRITICAL/FATAL (red background); rules editor in Settings and Connection Dialog; VTE integration
- **Ad-hoc broadcast** — send keystrokes to multiple terminals simultaneously; toolbar toggle button with keyboard shortcut; per-terminal checkboxes for selection; separate from existing cluster broadcast
- **Smart Folders** — dynamic connection grouping with filter criteria: protocol type, tags (AND logic), host glob pattern (`*.prod.example.com`), parent group; sidebar section with read-only connection list; create/edit/delete dialogs; CLI `smart-folders list/show/create/delete` subcommands
- **Script credentials** — `PasswordSource::Script` variant for dynamic credential resolution; shell command parsed via `shell-words`; 30-second timeout via `tokio::time::timeout`; stdout trimmed to `SecretString`; GUI entry with Test button in Auth tab
- **Per-connection terminal theming** — color overrides (background, foreground, cursor) per connection in `#RRGGBB` or `#RRGGBBAA` format; 3 `ColorDialogButton` widgets in Advanced tab; Reset button; VTE `set_color_background/foreground/cursor` integration
- **15 new language translations** — all new UI strings for 8 features translated across uk, de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk, uz

- New: `csv` 1.x (RFC 4180 parsing), `glob` 0.3 (Smart Folder host matching), `shell-words` 1.x (script credential argument splitting)
### Fixed
- Flatpak SSH key paths become stale after rebuild — keys copied to stable `~/.var/app/<app-id>/.ssh/` with fallback resolution ([#62](https://github.com/totoshko88/RustConn/issues/62))
- SFTP `ssh-add` uses stale portal key path — resolved via `resolve_key_path()` before use
- SFTP mc opens even when `ssh-add` fails — now aborts with toast error and "failed" status
- `script` command format updated to `--log-out`/`--log-timing` for modern util-linux
- Remote SSH recording used local paths — now extracts SSH config for remote `script` execution
- Recording playback showed `Script started on …` header — stripped with timing adjustment
- `script` invocation visible in terminal — erased via ANSI escape after `feed_child`
- SCP host key verification prompts in `stop_recording()` — added `-o StrictHostKeyChecking=no`
- RDP sidebar status not clearing after disconnect — `decrement_session_count` called with correct flag
- `PlaybackToolbar` GtkSearchEntry finalization warning — `Drop` unparents popover
- `cargo/config` deprecation warning in Flatpak build — renamed to `config.toml`
- Flatpak local manifest runtime updated from GNOME 50beta to GNOME 50
- Dependencies: euclid 0.22.14, toml 1.0.7, zerocopy 0.8.47, zip 8.3

## [0.10.0] - 2026-03-16

> **Note:** Flatpak release will follow after March 18, 2026, when GNOME 50 runtime is published on Flathub.

### Added
- **RDP file import in GUI** — `.rdp` files can now be imported via the Import dialog (Ctrl+I); previously only available through file association and CLI
- **CLI import: 4 new formats** — `rustconn-cli import` now supports `--format rdp`, `rdm`, `virt-viewer`, and `libvirt` in addition to the existing 7 formats
- **Split view for Telnet, Serial, Kubernetes** — split view now works with all VTE terminal-based protocols, not just SSH/SFTP/Local Shell
- **Statistics: Most Used & Protocol Distribution** — statistics dialog now shows top-3 most used connections and protocol usage breakdown with progress bars
- **5 new customizable keybindings** — Toggle Sidebar (F9), Connection History (Ctrl+H), Statistics (Ctrl+Shift+I), Password Generator (Ctrl+G), Wake On LAN (Ctrl+Shift+L); total now 31 actions
- **Sidebar keyboard shortcuts** — F2 renames selected connection/group, Ctrl+C/Ctrl+V copies/pastes connections, Ctrl+M moves to group; all scoped to sidebar focus so they don't intercept VTE terminal or embedded viewer input
- **Dynamic inventory sync** — new `rustconn-cli sync` command synchronizes connections from external JSON/YAML inventory files; matches by source tag + name + host; supports `--remove-stale` to clean absent connections and `--dry-run` for preview ([#56](https://github.com/totoshko88/RustConn/issues/56))
- **RDP file association** — double-clicking an `.rdp` file opens RustConn and connects automatically; supports address, credentials, gateway, resolution, audio, and clipboard fields ([#54](https://github.com/totoshko88/RustConn/issues/54))
- **FreeRDP bundled in Flatpak** — FreeRDP 3.24.0 SDL3 client built into the Flatpak; external RDP works out of the box on Wayland without `DISPLAY`
- **`sdl-freerdp3` detection** — FreeRDP detection now includes SDL3 variants (`sdl-freerdp3`, `sdl-freerdp`); Wayland priority: `wlfreerdp3` > `wlfreerdp` > `sdl-freerdp3` > `sdl-freerdp` > `xfreerdp3`

### Improved
- **i18n: hardcoded English strings wrapped** — ~40 user-visible strings across sidebar, embedded viewers (RDP, VNC, SPICE), session status overlays, and toolbar buttons now use `i18n()` for translation
- **i18n: accessible labels translatable** — ~25 `update_property` accessible labels in sidebar, window UI, embedded toolbar, and viewer controls wrapped with `i18n()`
- **i18n: protocol display names** — wrapped `display_name()` call sites with `i18n()` and added translations for 15 strings across all 15 languages
- **User-friendly VNC error messages** — raw error variants in VNC session toasts replaced with actionable messages ("Authentication failed. Check your credentials.", "Connection error")
- **VTE context menu moved off terminal widget** — `GestureClick` controller for the right-click context menu moved from the VTE terminal to its container widget; prevents interference with VTE's internal mouse event processing in ncurses/slang applications
- **VTE terminal no longer wrapped in ScrolledWindow** — redundant `ScrolledWindow` wrappers removed since VTE implements `GtkScrollable` natively
- **Monitoring module property tests** — 12 new tests covering `MonitoringSettings`, `MonitoringConfig`, `MetricsParser`, and `MetricsComputer`
- **Stale X11 comment removed** — `embedded.rs` comment referencing `GtkSocket` / X11 embedding updated to reflect native protocol clients

### Fixed
- **Secret backend default mismatch** — `SecretBackendType` default changed from `KeePassXc` to `LibSecret` to match User Guide and provide a universal out-of-the-box experience on all Linux desktops

#### Flatpak sandbox
- **waypipe not detected** — C-only build installs as `waypipe-c`, not `waypipe`; added `post-install` symlink in Flatpak manifest; `detect_waypipe()` now also tries `waypipe-c` as fallback; `which_binary()` checks `/app/bin/` directly in sandbox
- **SFTP file manager ignores SSH key** — external file managers (Dolphin, Nautilus) launched via `xdg-open` run outside the sandbox and cannot access the sandbox's SSH agent; `sftp_use_mc` now defaults to `true` in Flatpak so Midnight Commander (bundled) is used instead
- **ssh-agent socket in read-only `~/.ssh`** — `ensure_ssh_agent()` now uses `-a $XDG_RUNTIME_DIR/rustconn-ssh-agent.sock` inside Flatpak so the agent socket is created in a writable directory
- **KeePassXC not detected** — `keepassxc-cli` on the host system is now detected and executed via `flatpak-spawn --host`; all KDBX operations work transparently inside the sandbox; "Open Password Manager" button launches KeePassXC on the host
- **SSH jump host broken** — replaced `-J` with `-o ProxyCommand=ssh -W %h:%p ...` that passes `StrictHostKeyChecking`, `UserKnownHostsFile`, and identity file to the jump host process
- **mc wrapper not found** — stripped host-exported `mc()` bash function via `--unset-env=BASH_FUNC_mc%%`; installed sandbox wrapper for correct directory-change-on-exit
- **ZeroTrust and Kubernetes connections broken** — CLI tools (`aws`, `gcloud`, `az`, `kubectl`) now detected and executed via `flatpak-spawn --host`; cloud CLI config dirs mounted into sandbox so credentials are shared between sandbox and host
- **mc mouse clicks produce artifacts** — the `xterm-256color` terminfo entry's `XM` extended capability tells ncurses/slang to negotiate SGR mouse mode (1006) with VTE 0.80; mc cannot parse SGR-encoded mouse events, causing raw escape fragments like `7;6M7;6m` on every click; fix: compiled a custom `rustconn-256color` terminfo entry (identical to `xterm-256color` but without `XM`); VTE child processes in Flatpak use `TERM=rustconn-256color` to prevent the negotiation; additionally switched mc build from ncurses to slang and mc SFTP uses `-g` (`--oldmouse`) flag as defense-in-depth

#### Terminal / mc
- **mc SFTP: initial window not fullscreen** — mc read terminal dimensions before VTE widget received its GTK size allocation; added 150ms delay before spawning mc
- **Split view: text selection broken** — `GestureClick` handler no longer claims clicks on `VteTerminal` widgets

#### RDP
- **Crash on RDP connect (RefCell already borrowed)** — the IronRDP event polling loop held an immutable `client_ref.borrow()` while `handle_ironrdp_error` attempted `client_ref.borrow_mut().take()`, causing a double-borrow panic; error handling is now deferred until after the borrow is dropped ([#57](https://github.com/totoshko88/RustConn/issues/57))
- **Crash on RDP connect (ironrdp-tokio panic)** — upstream bug in `ironrdp-tokio 0.8.0` causes `copy_from_slice` panic on 64-bit systems during KDC TCP response parsing; `connect_finalize` is now wrapped in `catch_unwind` so the panic is converted to an error and the GUI falls back to FreeRDP instead of crashing
- **RDP gateway ignored in embedded mode** — IronRDP doesn't support RD Gateway; now falls back to external xfreerdp with a toast ([#53](https://github.com/totoshko88/RustConn/issues/53))
- **External RDP sidebar icon stays green after tab close** — fixed session ID / connection ID mismatch in `add_embedded_session_tab`; external xfreerdp process is now killed on tab close

#### SSH
- **Custom options format unclear** — subtitle now reads "Key=Value, comma-separated" with an example placeholder so users don't have to guess the format ([#58](https://github.com/totoshko88/RustConn/issues/58))
- **`UserKnownHostsFile` defaults to Flatpak path on native build** — `is_flatpak()` now requires `FLATPAK_ID` to match our app ID, preventing false positives when the env var leaks from another Flatpak process ([#59](https://github.com/totoshko88/RustConn/issues/59))

#### Terminal
- **Ctrl+W closes tab instead of deleting word** — changed close-tab shortcut from Ctrl+W to Ctrl+Shift+W (GNOME standard); Ctrl+W now passes through to the shell for backward-kill-word; close-pane moved to Ctrl+Shift+X ([#60](https://github.com/totoshko88/RustConn/issues/60))

#### UI / Clippy
- **Default window size too small on first start** — minimum size increased to 800×500; welcome screen adapts to narrow windows ([#55](https://github.com/totoshko88/RustConn/issues/55))
- **CSS parser warning: `@media (prefers-reduced-motion)`** — GTK4 CSS parser requires explicit value; changed to `@media (prefers-reduced-motion: reduce)`
- **Clippy: `RdpCommand::Connect` large enum variant** — boxed `RdpConfig` payload to reduce enum size from 240 to 16 bytes
- **Clippy: case-sensitive `.rdp` extension check** — now uses `Path::extension()` with `eq_ignore_ascii_case`
- **Clippy: collapsible `if` and `if-not-else`** — cleaned up nested conditionals in protocols, window, and main modules

### Changed
- **GTK4/libadwaita/VTE crate upgrade** — gtk4 0.10→0.11, libadwaita 0.8→0.9, vte4 0.9→0.10; unlocks GNOME 48–50 APIs
- **MSRV bumped to 1.92** — required by updated GTK-rs bindings
- **Flatpak runtime bumped to GNOME 50** — all three manifests now use `org.gnome.Platform` 50 with VTE 0.80
- **AdwSpinner migration** — replaces `gtk::Spinner` in export dialog; cfg-gated `adw-1-6`
- **AdwShortcutsDialog migration** — replaces deprecated `gtk::ShortcutsWindow`; cfg-gated `adw-1-8`
- **AdwSwitchRow migration** — replaces manual `ActionRow` + `Switch` in monitoring, logging, and secrets settings tabs
- **AdwWrapBox for protocol filters** — sidebar filters wrap on narrow sidebars; cfg-gated `adw-1-7` with `GtkBox` fallback
- **Welcome screen refreshed** — updated feature highlights, replaced performance internals with Quick Access tips, added Command Palette / Import / Settings shortcuts
- **CSS `prefers-reduced-motion`** — transitions disabled when reduced motion is requested
- **Tiered distro feature flags** — `adw-1-8` for Tumbleweed/Fedora 43+, `adw-1-6` for Leap 16.0/Fedora 42, baseline for older distros
- **Codebase cleanup** — removed 25+ unused CSS classes, consolidated `futures-util` into `futures`, fixed metainfo.xml duplicates, added k8s keywords, removed dead code

- clap 4.5.60→4.6.0, gtk4 0.11.0→0.11.1, gdk4 0.11.0→0.11.1, gsk4 0.11.0→0.11.1, glib 0.22.2→0.22.3, openssl 0.10.75→0.10.76, tracing-subscriber 0.3.22→0.3.23
- Transitive: anstream 0.6.21→1.0.0, anstyle 1.0.13→1.0.14, anstyle-parse 0.2.7→1.0.0, cc 1.2.56→1.2.57, clap_complete 4.5.66→4.6.0, clap_mangen 0.2.31→0.2.33, colorchoice 1.0.4→1.0.5, glib-sys 0.22.0→0.22.3, once_cell 1.21.3→1.21.4, roff 0.2.2→1.1.0, tinyvec 1.10.0→1.11.0, uds_windows 1.2.0→1.2.1

## [0.9.15] - 2026-03-11

### Added
- **Hide local cursor option for embedded viewers** — new "Show Local Cursor" checkbox in RDP, VNC, and SPICE connection dialogs (Features section) allows hiding the local OS cursor over embedded viewers to eliminate the "double cursor" effect; enabled by default for backward compatibility ([#51](https://github.com/totoshko88/RustConn/issues/51))

### Fixed
- **VNC session ignores Display Mode setting** — the "Display Mode" dropdown (Embedded/External/Fullscreen) in the Advanced tab was saved correctly but had no effect on VNC sessions; now Fullscreen maximizes the main window (same as RDP), and External forces the external VNC viewer (TigerVNC/vncviewer) instead of the embedded vnc-rs client ([#50](https://github.com/totoshko88/RustConn/issues/50))
- **SSH port forwarding via UI broken** — `window/protocols.rs` built SSH args manually, skipping `port_forwards`, X11 forwarding (`-X`), compression (`-C`), and `ControlPersist=10m` from `SshConfig`; refactored to delegate to `SshConfig::build_command_args()` which has the complete logic ([#49](https://github.com/totoshko88/RustConn/issues/49))
- **SSH custom options `-o` prefix not stripped** — `parse_custom_options()` expected `Key=Value` format but users pasted `-o Key=Value` from CLI; now silently strips the `-o` prefix ([#49](https://github.com/totoshko88/RustConn/issues/49))
- **SSH custom options placeholder misleading** — dialog showed `-o StrictHostKeyChecking=no` format but parser expected comma-separated `Key=Value`; updated placeholder and subtitle to clarify correct format ([#49](https://github.com/totoshko88/RustConn/issues/49))

## [0.9.14] - 2026-03-10

### Fixed
- **SSH connection fails in Flatpak on KDE** — host `SSH_ASKPASS` environment variable (e.g. `ksshaskpass`) was inherited by the VTE child process but not available inside the sandbox, causing `Permission denied` before the password prompt appeared; now stripped from the terminal environment since RustConn uses native VTE password injection ([#48](https://github.com/totoshko88/RustConn/issues/48))
- **Header bar buttons clipped when sidebar + monitoring enabled** — monitoring bar's system info label could request more width than available in the content area, causing overflow that pushed header bar buttons out of bounds; fixed by adding `ellipsize` to variable-length labels and `overflow: hidden` on the monitoring bar container ([#47](https://github.com/totoshko88/RustConn/issues/47))

- tokio 1.49→1.50, uuid 1.21→1.22, regex 1.11→1.12, proptest 1.9→1.10, tempfile 3.23→3.26, zip 8.1→8.2, criterion 0.8.1→0.8.2, rpassword 7.3→7.4
- Transitive: hybrid-array 0.4.7→0.4.8, image 0.25.9→0.25.10, libc 0.2.182→0.2.183, libz-sys 1.1.24→1.1.25, moxcms 0.7.11→0.8.1, quinn-proto 0.11.13→0.11.14, schannel 0.1.28→0.1.29, zerocopy 0.8.40→0.8.42

## [0.9.13] - 2026-03-09

### Fixed
- **RDP handshake timeout on loaded servers** — Phase 3 (TLS upgrade + NLA + connect_finalize) now wrapped in `tokio::time::timeout` with `timeout_secs × 2` (minimum 60s); previously only TCP connect had a timeout, causing indefinite hangs when the remote server was under heavy load
- **ARM64 binary download mismatch** — `download_url_for_arch()` on aarch64 no longer falls back to x86_64 URL when no ARM64 binary exists; `get_available_components()` now filters out components unavailable for the current architecture (affects TigerVNC Viewer and Bitwarden CLI)

### Added
- **RDP Quick Actions menu** — new dropdown button on the embedded RDP toolbar with 6 Windows admin shortcuts: Task Manager (Ctrl+Shift+Esc), Settings (Win+I), PowerShell, CMD, Event Viewer, Services; actions send scancode sequences via `SendKeySequence` command with 30ms inter-key delay

## [0.9.12] - 2026-03-08

### Security
- **Removed sshpass dependency** — interactive SSH sessions now use native VTE password injection via `feed_child()`; monitoring SSH uses `SSH_ASKPASS` mechanism with temporary script instead of `SSHPASS` environment variable (no longer visible in `/proc/PID/environ`)
- **Bitwarden master password zeroized on drop** — `unlock_vault()` now wraps the temporary plain-text password copy in `Zeroizing<String>` so heap memory is scrubbed when the blocking task completes
- **SSH monitoring askpass script cleaned up on drop** — temporary `SSH_ASKPASS` helper script is now deleted automatically when the monitoring session ends (RAII wrapper with `Drop` impl)

### Improved
- **Reduced state.rs complexity** — extracted vault operations (~979 lines) into `vault_ops.rs`, trimming `state.rs` from 3143 to 2167 lines
- **Reduced window/mod.rs complexity** — extracted `setup_edit_actions` (637 lines), `setup_terminal_actions` (298 lines), and `setup_split_view_actions` (746 lines) into separate modules, trimming `window/mod.rs` from 5316 to 3648 lines

### Changed
- **SPICE embedded client enabled by default** — `spice-embedded` feature flag now included in default features for both `rustconn-core` and `rustconn` crates; native SPICE client (via `spice-client` crate) is now the primary connection method with `remote-viewer` as fallback

### Removed
- **sshpass** — removed from all packaging manifests (Flatpak, Flathub, Debian, OBS RPM, Snap); no longer a runtime dependency

## [0.9.11] - 2026-03-07

### Security
- **Bitwarden session key now uses SecretString** — session key was stored as plain `String` in memory without zeroization; migrated to `SecretString` with `expose_secret()` only at CLI invocation point
- **Config files written with 0600 permissions** — connection data (hostnames, usernames, port forwards) was world-readable on multi-user systems; config directory now created with 0700
- **SSH monitoring host key verification** — removed unconditional `StrictHostKeyChecking=no`; now uses `accept-new` by default (accepts first-seen keys, rejects changed keys)
- **Session log sanitization active by default** — built-in sensitive patterns (password prompts, API keys, tokens) were defined but never wired into the sanitizer; now active in `SanitizeConfig::default()`
- **Flatpak device permissions documented** — `--device=all` retained in Flatpak manifests with justification comment (serial ports for picocom require it; Flatpak has no granular `--device=serial` option)
- **Monitoring password uses SecretString** — `ssh_exec_factory` password parameter migrated from plain `String` to `SecretString` with zeroization; `expose_secret()` used only at `SSHPASS` env var injection point
- **RDP TLS certificate policy documented** — `establish_connection` now documents that IronRDP does not validate server certificates (standard for RDP self-signed certs); added `tracing::warn!` on each connection

### Fixed
- **Encrypted document format ambiguity** — legacy salt byte could be misinterpreted as encryption strength byte (~1.2% chance); introduced V2 magic header `RCDB_EN2` for unambiguous format detection

### Added
- **Monitoring: remote host private IP** — monitoring bar now shows the primary private IP address in the system info section; hovering shows hostname, all IPv4 and IPv6 addresses grouped separately
- **Monitoring: live uptime counter** — uptime in the system info tooltip now updates on every metrics polling tick instead of remaining static until the next full system info refresh
- **Monitoring: stopped indication** — when the metrics collector stops (3 consecutive failures), the monitoring bar dims to 50% opacity, shows a warning icon, and the tooltip displays "⚠ Monitoring stopped"
- **Monitoring: all mount points** — disk section now shows root filesystem in the level bar and all mounted real filesystems in the tooltip (mount point, used/total, percentage); virtual filesystems (tmpfs, devtmpfs, squashfs, overlay) and snap loop mounts are filtered out

### Removed
- **Dead `read_import_file_async`** — unused async import helper removed from `rustconn-core/src/import/traits.rs`

## [0.9.10] - 2026-03-07

### Fixed
- **Connection dialog Basic tab clipped** — removed redundant outer `ScrolledWindow` wrapping the `ViewStack`; each tab already provides its own scroller, so the nested scroll stole height allocation and clipped the Basic tab content
- **Dialog minimum sizes missing** — added `set_size_request` to Import, Export, and Shortcuts dialogs to prevent UI breakage on small screens
- **Remmina import fails in Flatpak** — importer now also checks the host path `~/.local/share/remmina/` when running inside Flatpak sandbox ([#44](https://github.com/totoshko88/RustConn/issues/44))

### Improved
- **Connection dialog default height** — increased from 500→670px so the Basic tab fields (including Description) are fully visible without scrolling on typical displays

- serde_yaml_ng 0.9→0.10, cfg-expr 0.20.6→0.20.7, inotify 0.11.0→0.11.1, socket2 0.6.2→0.6.3, toml 1.0.4→1.0.6
- CLI downloads: Teleport 18.7.1→18.7.2

## [0.9.9] - 2026-03-06

### Fixed
- **sshpass not installed in Flatpak** — SSH password-authenticated connections broken in Flatpak 0.9.8 ([#42](https://github.com/totoshko88/RustConn/issues/42))
- **Jump host connections fail port check** — pre-connect TCP probe always timed out for destinations reachable only through a jump host; now skipped when `jump_host_id` or `proxy_jump` is configured ([#41](https://github.com/totoshko88/RustConn/issues/41))
- **Jump host dropdown hard to use** — added host address to dropdown labels (`Name (host)`) and enabled search filtering for quick lookup
- **Jump host monitoring fails** — monitoring SSH commands now include `-J` jump host chain so metrics collection works through bastion hosts ([#41](https://github.com/totoshko88/RustConn/issues/41))
- **Jump host false positive connection status** — SSH status detection now checks terminal text for failure patterns (`Connection timed out`, `Connection refused`, etc.) before marking jump host connections as established ([#41](https://github.com/totoshko88/RustConn/issues/41))

- Bitwarden CLI 2026.1.0→2026.2.0, uuid 1.21.0→1.22.0, winnow 0.7.14→0.7.15

## [0.9.8] - 2026-03-05

### Security
- **RDP password no longer exposed on command line** — FreeRDP fallback now uses `/from-stdin` instead of `/p:{password}` argument

### Fixed
- **SSH connection status not turning green** — VTE cursor position axes were swapped; status detection callbacks were skipped when async port check is enabled
- **Automation cursor tracking** — expect-script automation read wrong cursor axis from VTE
- **RDP keyboard input duplication** — deduplicated key press/release handlers via shared `send_ironrdp_key()`
- **Username placeholder on empty `$USER`** — falls back to `$LOGNAME`, then generic placeholder

### Added

**Connection dialog — protocol improvements:**
- **SSH** — password source validation on save, key source "Default" explanation, custom options placeholder, port forwarding duplicate detection
- **RDP** — gateway port/username fields, disable NLA checkbox, clipboard sharing toggle, dynamic resolution info
- **VNC** — encoding dropdown (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), performance mode auto-sync, auth info
- **SPICE** — proxy field for Proxmox VE, CA certificate validation, TLS/skip-verification sensitivity logic
- **Serial** — device auto-detection (`/dev/ttyUSB*`, `/dev/ttyACM*`, `/dev/ttyS*`), dialout group warning
- **Kubernetes** — pod name validation, busybox mode sensitivity
- **Telnet** — plaintext transmission security warning
- **Zero Trust** — CLI availability check, OCI Bastion SSH key/TTL fields, generic command placeholder docs

**Connection dialog — general:**
- Domain field hidden for non-RDP protocols
- MAC address format validation for Wake-on-LAN
- Granular per-connection logging options (activity, input, output, timestamps)
- Password source ↔ SSH auth method auto-sync

**Other:**
- **SFTP mc in split view** — mc-based SFTP sessions now support horizontal/vertical split like SSH
- **Context menu "New Connection"** — opens dialog with the connection's group pre-selected

### Improved
- **Connection dialog decomposition** — extracted 4 tab modules from monolithic `dialog.rs` (~7500→~1500 lines)
- **Embedded RDP decomposition** — extracted 5 modules from monolithic `mod.rs` (~2900→~500 lines)
- **Code quality** — structured tracing fields, i18n coverage, deduplication of clipboard/callback/resize patterns, module-level lint allows removed

- binrw 0.15.0→0.15.1, proc-macro-crate 3.4.0→3.5.0, toml 1.0.3→1.0.4, toml_edit 0.23.10→0.25.4, uds_windows 1.1.0→1.2.0

## [0.9.7] - 2026-03-04

### Fixed
- **Connection group not saved** — connection dialog used a separate `Rc` for `groups_data` in the save closure vs the struct field, so `set_groups()` updated the struct but the save handler always read the initial `[(None, "(Root)")]`; connections now correctly land in the selected subgroup on both create and edit ([#40](https://github.com/totoshko88/RustConn/issues/40))
- **Secret variable values lost after settings reopen** — secret variables had their values cleared before persisting to disk (stored in vault), but were never restored from vault when reopening the Variables dialog or substituting `${VAR}` in connections; added `resolve_global_variables()` that loads secret values from the configured vault backend
- **Crash on session reconnect** — `close_tab` held an immutable borrow on `sessions` while `tab_view.close_page()` synchronously fired the `close-page` signal handler which needed a mutable borrow, causing a `BorrowMutError` panic; separated the borrow from the close call ([#39](https://github.com/totoshko88/RustConn/issues/39))

### Changed
- **Bitwarden credential lookup speed** — removed per-retrieve `bw sync` (network round-trip) and added a 120-second verification cache for `bw status` checks; vault syncs once on unlock instead of on every credential lookup, making reconnect and batch operations significantly faster

- tokio 1.49→1.50, aws-lc-rs 1.16.0→1.16.1, aws-lc-sys 0.37.1→0.38.0, getrandom 0.4.1→0.4.2, ipnet 2.11→2.12, quote 1.0.44→1.0.45, tokio-macros 2.6.0→2.6.1, zip 8.1→8.2

## [0.9.6] - 2026-03-02

### Fixed
- **Bitwarden Flatpak session key** — `build_command` now falls back to the global in-process session store when the instance-level key is absent, so `SecretManager.is_available()` correctly sees an unlocked vault after `auto_unlock` ([#28](https://github.com/totoshko88/RustConn/issues/28))
- **Bitwarden Settings auto-unlock path** — secrets tab auto-unlock now uses `get_bw_cmd()` (globally resolved path) instead of the local `Rc<RefCell>` which may still hold the bare `"bw"` before detection completes
- **Connection dialog credential download** — lookup key now uses `generate_store_key()` (UUID-based) instead of `"{name} ({protocol})"` format, matching the key used by Bitwarden/1Password/Passbolt store operations
- **Vault credential resolve for non-KeePass backends** — `resolve_credentials_blocking` now has a direct `PasswordSource::Vault` block that calls `dispatch_vault_op` with `auto_unlock` for Bitwarden and other backends, instead of falling through to `CredentialResolver` which created a fresh `BitwardenBackend` without session
- **Inherit condition for non-KeePass backends** — group password inheritance no longer blocked when `kdbx_enabled=true` but preferred backend is Bitwarden/1Password/Passbolt/Pass; condition changed from `!kdbx_enabled` to `!matches!(preferred_backend, KeePassXc | KdbxFile)`
- **Group password load from any backend** — group edit dialog password load button now dispatches to the configured default secret backend via `select_backend_for_load` + `dispatch_vault_op`, instead of hardcoded KeePass/Keyring-only branches
- **SSH known_hosts not persisting in Flatpak** — SSH connections now use `-o UserKnownHostsFile=~/.var/app/<app-id>/.ssh/known_hosts` in Flatpak sandbox where `~/.ssh` is mounted read-only; directory is auto-created; applies to interactive SSH, sshpass, Quick Connect, and monitoring; respects user-set `UserKnownHostsFile` in custom options
- **Duplicate reconnect banner** — `TerminalNotebook` now tracks shown reconnect banners per session to prevent duplicates on repeated child-exit signals
- **SSH dialog key fields for Keyboard Interactive** — auth method visibility now correctly hides key path/passphrase fields for Keyboard Interactive (index 2), same as Password (index 0)

### Changed
- **Dependency updates** — moka 0.12.13→0.12.14, pxfm 0.1.27→0.1.28, zlib-rs 0.6.2→0.6.3; kubectl pinned 1.35.1→1.35.2

## [0.9.5] - 2026-03-02

### Fixed
- **SSH/Telnet pre-connect port check** — fail fast with retry toast instead of hanging in "Connecting" state
- **Vault credential lifecycle** — orphaned credentials cleaned on trash empty; paste duplicates credentials; group rename/move migrates KeePass entries
- **Consistent credential keys** — unified `generate_store_key()` across all backends; fixed silent lookup failures from key format mismatch
- **SecretManager cache TTL** — entries expire after 5 minutes, preventing stale credentials
- **Inherit cycle protection** — `HashSet<Uuid>` visited guard prevents infinite loops in group hierarchy
- **Group change in connection dialog** — selecting a different group now correctly persists on save
- **Monitoring race condition** — waits for SSH handshake before opening monitoring channel

### Security
- **SecretString migration** — RDP/SPICE event credentials, GUI password structs, CLI input, and `Variable` (zeroize on Drop) all use `SecretString`

### Changed
- **Backend dispatch consolidation** — `VaultOp` enum + `dispatch_vault_op()` replaces ~200 lines of duplicated match blocks
- **Mutex lock safety** — ~50 `unwrap()` on `Mutex::lock()` replaced with `lock_or_log()` helper
- **Error logging** — `let _ =` on persistence ops replaced with `tracing::warn!`; remaining `eprintln!` migrated
- **CSS extraction** — 595-line inline CSS moved to `rustconn/assets/style.css`
- **i18n consistency** — hardcoded English strings wrapped with `i18n()` / `i18n_f()`
- **CI** — `--all-features` added to test jobs for feature-gated code coverage

### Removed
- Dead code: `StateAccessError`, unused state accessors, legacy dialog tabs, ~30 unused sidebar methods

- js-sys 0.3.90→0.3.91, pin-project-lite 0.2.16→0.2.17, wasm-bindgen 0.2.113→0.2.114, web-sys 0.3.90→0.3.91
## [0.9.4] - 2026-03-01

### Added
- **Session Reconnect** — disconnected VTE tabs show a "Reconnect" banner to re-launch in one click
- **Recursive Group Delete** — three-option dialog: keep children, cascade delete, or cancel
- **Connection History** — search/filter by name/host/protocol; per-entry delete
- **Cluster from sidebar** — "Create Cluster" pre-selects checked connections
- **Shortcut conflict detection** — warning when a keybinding is already assigned
- **Settings Backup/Restore** — export/import all config as ZIP via Settings → Interface
- **Libvirt / GNOME Boxes import** — VNC, SPICE, RDP from domain XML; auto-scans qemu dirs ([#38](https://github.com/totoshko88/RustConn/issues/38))
- **Automation templates** — 5 built-in expect rule presets (Sudo, SSH Host Key, Login, etc.)
- **TemplateManager** — centralized template CRUD with search, protocol filtering, import/export
- **Snippet shell safety** — warns about dangerous metacharacters in variable values before `--execute`

### Fixed
- **Password inheritance** — `PasswordSource::Variable` now resolved in group hierarchy ([#37](https://github.com/totoshko88/RustConn/issues/37))
- **New connection in wrong group** — context menu now pre-selects the target group ([#37](https://github.com/totoshko88/RustConn/issues/37))
- **Toast system** — severity icons, "Retry" on port-check failures, `AlertDialog` fallback, i18n
- **VTE spawn failure** — missing command shows "Command not found" banner + error toast instead of silent empty terminal
- **Cluster broadcast** — keyboard input now actually broadcasts to all cluster terminals; session lifecycle wired; disconnect-all button; full i18n
- **Pango markup** — escaped ampersand in "Backup & Restore" settings title
- **Adwaita dark theme warning** — suppressed on KDE/XFCE desktops

### Improved
- **User Guide** — major rewrite: Zero Trust, Security, FAQ, Migration Guide, expanded all sections
- **Automation engine** — one-shot rules, per-rule timeout, regex validation, template picker, pre-connect/post-disconnect tasks, key sequences on connect
- **Template management** — CLI and GUI migrated to `TemplateManager`; GUI keeps document integration

- **Updated**: js-sys 0.3.90→0.3.91, pin-project-lite 0.2.16→0.2.17, wasm-bindgen 0.2.113→0.2.114, web-sys 0.3.90→0.3.91
## [0.9.3] - 2026-02-27

### Added
- **Waypipe Support** — Wayland application forwarding for SSH connections via `waypipe`; auto-detected on Wayland sessions when `waypipe` binary is available on PATH; per-connection toggle in SSH Session options; graceful fallback to direct SSH when unavailable ([#36](https://github.com/totoshko88/RustConn/issues/36))
- **IronRDP Clipboard Integration** — Bidirectional clipboard sync between local desktop and remote RDP session via cliprdr channel; server→client text is auto-synced to local GTK clipboard; local clipboard changes are automatically announced to the server; Copy/Paste buttons remain as manual fallback; feedback loop prevention via suppression flag

### Fixed
- **Missing icons on KDE and non-GNOME desktops** — Replaced all non-standard icon names (`emblem-ok-symbolic`, `emblem-system-symbolic`, `call-start-symbolic`, `modem-symbolic`, `application-x-executable-symbolic`, etc.) with freedesktop-standard equivalents; replaced icons missing from Adwaita (`emblem-default-symbolic`, `emblem-synchronizing-symbolic`, `utilities-system-monitor-symbolic`, `view-sidebar-start-symbolic`, `tag-symbolic`) with available alternatives; forced Adwaita icon theme via `GtkSettings` for consistent icon availability on all desktops; unified protocol icons via single source of truth in `icons.rs`, eliminating hardcoded duplicates across sidebar, tabs, dialogs, templates, and cluster views ([#35](https://github.com/totoshko88/RustConn/issues/35))
- **Serial connection creation failed** — Serial and Kubernetes connections no longer require host/port validation (they use device path / pod name instead); previously "Host cannot be empty" error blocked saving these connections
- **Serial/Kubernetes missing client toast** — Shows user-friendly toast when picocom (Serial) or kubectl (Kubernetes) is not installed, and when Kubernetes pod/container configuration is incomplete; fixed toast overlay discovery that failed on `adw::ApplicationWindow` internal widget hierarchy
- **libsecret password storage panic** — Fixed `debug_assert` crash in libsecret backend that rejected non-UUID lookup keys (e.g. `"test (vnc)"`); libsecret uses `name (protocol)` format, not UUIDs
- **libsecret password retrieval** — Fixed `is_available()` check that always returned `false` because `secret-tool --version` is not a valid subcommand (exits with code 2); the store path bypassed this check but the retrieve path went through `SecretManager` which skipped the backend, causing saved passwords to never be found on connection
- **VNC/RDP identical icons** — VNC now uses `video-joined-displays-symbolic` (two monitors) instead of `video-display-symbolic` which was identical to RDP's `computer-symbolic` in Adwaita
- **SFTP via mc opens root instead of home** — mc FISH VFS URI now includes `/~` suffix to open the remote user's home directory; mc is launched via `sh -c` wrapper for correct terminal sizing
- **SSH agent not inherited by VTE terminals** — `spawn_command` now injects `SSH_AUTH_SOCK`/`SSH_AGENT_PID` from the global `OnceLock<SshAgentInfo>` into VTE-spawned processes; previously mc, ssh, and other terminal commands could not reach the SSH agent when RustConn started its own agent (Rust 2024 edition forbids `set_var`)

### Improved
- **Client Detection** — Added waypipe to Settings → Clients detection tab
- **Documentation** — Added Waypipe section to User Guide and Architecture docs
- **Translations** — Added waypipe-related strings to all 18 languages

- **Updated**: deflate64 0.1.10→0.1.11, dispatch2 0.3.0→0.3.1, objc2 0.6.3→0.6.4, zerocopy 0.8.39→0.8.40

## [0.9.2] - 2026-02-26

### Added
- **Custom Icons** — Set emoji/unicode or GTK icon names on connections and groups ([#23](https://github.com/totoshko88/RustConn/issues/23))
- **Remote Monitoring** — MobaXterm-style monitoring bar below SSH/Telnet/K8s terminals showing CPU, memory, disk, and network usage from remote Linux hosts; agentless via `/proc/*` parsing; per-connection and global toggle in Settings ([#26](https://github.com/totoshko88/RustConn/issues/26))

### Fixed
- New connections and groups now append to end of list instead of jumping to position 0
- **IronRDP fallback to FreeRDP** — When IronRDP fails during RDP protocol negotiation (e.g. xrdp `ServerDemandActive` incompatibility), the session now auto-falls back to external FreeRDP instead of showing a raw error; shows a user-friendly toast on fallback ([#33](https://github.com/totoshko88/RustConn/issues/33))
- **Monitoring SSH password auth** — Remote monitoring now works with password-authenticated SSH connections via `sshpass`; previously `BatchMode=yes` blocked password auth causing "Permission denied" errors
- **Monitoring error spam** — Monitoring collector now stops after 3 consecutive failures instead of retrying indefinitely and flooding logs
- **Bitwarden CLI not found in Flatpak** — All `bw` command invocations now use a dynamically resolved path instead of hardcoded `"bw"`; `resolve_bw_cmd()` probes Flatpak CLI dir, Snap, `/usr/local/bin`, and `PATH` at startup ([#28](https://github.com/totoshko88/RustConn/issues/28))

### Improved
- **Documentation** — Added User Guide sections for Remote Monitoring and Custom Icons; added monitoring architecture to ARCHITECTURE.md; updated README features table; rewrote Settings section to match the current 4-page `PreferencesDialog` layout (Terminal, Interface, Secrets, Connection); fixed all cross-references to old tab names throughout User Guide; added `docs/BITWARDEN_SETUP.md` step-by-step guide covering Flatpak sandbox, self-hosted servers, API key auth, and troubleshooting
- **Translations** — Completed all 14 language translations to 100% coverage (de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk, uz); added Uzbek (uz) as a new language; fixed corrupted .po file formatting from previous patching

## [0.9.1] - 2026-02-24

### Added
- **Command Palette** — VS Code-style quick launcher (`Ctrl+P` / `Ctrl+Shift+P`) with fuzzy search for connections and `>` / `@` / `#` prefixes for commands, tags, and groups
- **Favorites / Pinning** — Pin connections to a dedicated "Favorites" section at the top of the sidebar via context menu
- **Pass (passwordstore.org) secret backend** — Store and retrieve credentials via `pass` with GPG-encrypted files, custom `PASSWORD_STORE_DIR`, Settings UI, and CLI support ([#32](https://github.com/totoshko88/RustConn/pull/32), contributed by [@h3nnes](https://github.com/h3nnes))
- **Tab coloring by protocol** — Optional colored circle indicator on terminal tabs (SSH=green, RDP=blue, VNC=purple, SPICE=orange, Serial=yellow, K8s=cyan); toggle in Settings → Appearance
- **Snippet timestamps** — `created_at` and `updated_at` fields on `Snippet` model with backward-compatible deserialization
- **Tab grouping** — Right-click context menu on tabs to assign named groups ("Production", "Staging") with color-coded indicators
- **Custom Keybindings** — Fully customizable keyboard shortcuts via Settings → Keybindings with 30+ actions, Record button, per-shortcut Reset, and Reset All

### Fixed
- Command Palette not dismissible via Escape or click-outside
- Favorites group not updating immediately on pin/unpin
- KDBX group visibility regression when loading saved backend preference in Settings
- Doc-comment misplacement in `state.rs` for Pass helper functions

### Improved
- **i18n coverage** — Connection dialog tabs (Basic, Protocol, Data, Logging, Automation, Advanced) and all their content strings now translatable; translations added to all 14 languages
- **User Guide** — Added "Terminal Keybinding Modes" section (vim/emacs in Bash, Zsh, Fish)

- **Updated**: uuid 1.11→1.21, proptest 1.6→1.9, tempfile 3.15→3.23, plus 18 transitive dependency bumps via `cargo update`
### Internal
- Deduplicated `PassBackend` construction in CLI and GUI
- Cached `has_secret_backend()` result in `AppState` to avoid repeated `block_on` calls

## [0.9.0] - 2026-02-21

### Added
- **Startup action** — configure which session opens automatically when RustConn starts: local shell, or any saved connection. Set in Settings → Appearance → Startup, or override via CLI flags `--shell` / `--connect <name|uuid>` ([#30](https://github.com/totoshko88/RustConn/issues/30))

### Security
- All password fields (`FreeRdpConfig`, `RdpConfig`, `SpiceClientConfig`, `KdbxEntry`, `PasswordDialogResult`, `ConnectionDialogResult`) migrated to `SecretString` — credentials are now exposed only at point of use
- FreeRDP embedded thread no longer passes password via CLI arg — uses `/from-stdin` + stdin pipe
- Bitwarden `BW_SESSION` replaced with thread-safe in-process `RwLock` storage instead of `set_var`
- KDBX functions migrated to `SecretString` + `SecretResult` throughout
- SSH `custom_options` now filtered against dangerous directives (`ProxyCommand`, `LocalCommand`, etc.) before passing to `ssh -o`
- Hand-rolled base64 in Bitwarden backend replaced with `data-encoding` crate

### Improved
- **Ukrainian translation** — 674 translations professionally reviewed by Mykola Zubkov for accuracy and modern orthography
- SVG icon optimized and simplified per GNOME HIG; 48×48 and 64×64 PNG removed — GTK renders SVG at any size; 128×128 and 256×256 PNG regenerated from SVG
- Welcome page logo now uses GTK themed icon lookup (same as About dialog) — renders SVG at native HiDPI resolution instead of fixed-size raster
- Flathub metainfo.xml overhauled: description condensed, brand colors improved, screenshots replaced with HiDPI windowed captures with shadows, localized screenshots for uk/be/cs, added translate and contribute URLs
- 8 dialogs migrated to `adw::Dialog` (libadwaita 1.5+) with adaptive sizing and proper modal behavior
- Password field uses `PasswordEntry` with built-in peek icon
- Screen reader support: accessible label relations added to password and connection dialogs
- `adw::Clamp` added to dialogs to prevent content stretching on wide screens
- Dialog header bar pattern deduplicated via shared `dialog_header()` helper
- Clear History now requires confirmation via `adw::AlertDialog`
- Search history popover items are now clickable
- All `eprintln!` calls replaced with structured `tracing`

### Fixed
- **VNC RSA-AES auto-fallback** — servers using RSA-AES security type (type 129, e.g. wayvnc) now automatically fall back to external VNC viewer (TigerVNC) instead of showing a raw error. User sees a friendly toast message ([#31](https://github.com/totoshko88/RustConn/issues/31))
- Embedded RDP cursor size corrected on HiDPI displays — server-sent device-pixel bitmaps now downscaled to logical pixels before GTK cursor creation
- Pango markup warning on welcome page — ampersand in "Embedded & external clients" escaped for GTK label rendering
- Variable password source (`PasswordSource::Variable`) now resolves correctly at connection time — `SecretManager` is initialized with backends from settings, and variable lookup uses the same backend as save
- Locale `.mo` files now included in Debian, RPM, and local Flatpak packages
- Debian build no longer enables `spice-embedded` feature without build dependencies
- AppStream metainfo.xml: categories added explicitly (`Network`, `RemoteAccess`), generic `GTK` category removed
- Debian `Recommends` updated for FreeRDP 3 / Wayland support
- Build dependencies corrected for `gettext` across Debian and RPM

### Removed
- Dead code cleanup: unused credential caching, split view adapter methods, toast helpers, deprecated flatpak host command functions

- **Updated**: deranged 0.5.6→0.5.8, js-sys 0.3.86→0.3.88, wasm-bindgen 0.2.109→0.2.111, wasm-bindgen-futures 0.4.59→0.4.61, web-sys 0.3.86→0.3.88
### Internal
- `Project-Id-Version` updated to `0.9.0` in all `.po` files
- Duplicate `SessionResult` type alias removed from `session/manager.rs` — canonical definition in `error.rs`
- Tray stub no longer allocates orphaned `mpsc` channel when `tray` feature is disabled
- Migrated to Rust 2024 edition (167 files changed across all three crates):
  - Eliminated all `unsafe` `set_var`/`remove_var` calls — SSH agent info stored in `OnceLock<SshAgentInfo>` with `apply_agent_env()` helper, language switching via process re-exec with sentinel guard, Bitwarden session token in `RwLock`
  - Renamed `gen` keyword usages to `generator`/`pw_gen`/`counter` in password generator, dialog, and RDP modules
  - Fixed `ref` binding patterns in match arms across source and test files (Rust 2024 match ergonomics)
  - Hundreds of `collapsible_if` patterns rewritten as let-chains (`if let ... && let ...`)
  - Import ordering updated to Rust 2024 `style_edition` rules via `cargo fmt`

## [0.8.9] - 2026-02-20

### Security
- Input validation hardening across all protocols — `custom_args`, device paths, shell paths, hostnames, proxy URLs, and shared folder names are now validated against injection attacks (null bytes, newlines, shell metacharacters, path traversal)
- SSH config export blocks dangerous directives (`ProxyCommand`, `LocalCommand`, etc.) with inline comments
- KeePassXC socket responses capped at 10 MB; reduced password exposure lifetime
- Async import enforces the same 50 MB file size limit as sync path
- VNC and RDP client passwords migrated to `SecretString` — exposed only at point of use
- FreeRDP external launcher uses `/from-stdin` instead of `/p:{password}` on command line

### Added
- **SSH port forwarding** — Local (`-L`), remote (`-R`), and dynamic SOCKS (`-D`) port forwarding rules can be configured per connection; rules are persisted in `SshConfig.port_forwards` and passed as CLI flags to `ssh` ([#22](https://github.com/totoshko88/RustConn/issues/22))
- **Deferred secret backend initialization** — Bitwarden vault unlock and KDBX password decryption now run asynchronously after the main window is presented, eliminating the 1–3 second startup delay when a secret backend is configured

### Fixed
- `localhost` no longer rejected as placeholder during import
- Bitwarden: fixed duplicate vault writes, false "unlocked" status at startup, auto-unlock after restart, and compatibility with CLI v2026.1.0 including automatic `logout → login → unlock` recovery on "key type mismatch" ([#28](https://github.com/totoshko88/RustConn/issues/28))
- Bitwarden GUI unlock no longer clears password field, preventing stale encrypted password on next save ([#28](https://github.com/totoshko88/RustConn/issues/28))
- Generic ZeroTrust `custom_args` now embedded into shell command instead of passed as positional parameters
- RefCell borrow panic in EmbeddedRdpWidget; VNC polling mutex contention; RDP polling timer leak
- FreeRDP now uses native Wayland backend (removed `QT_QPA_PLATFORM=xcb` override)
- Several `unwrap()` panics replaced with safe fallbacks (VNC, TaskExecutor, tray, build.rs)
- EmbeddedRdpWidget resize signal handler properly cleaned up on disconnect
- Quick connect RDP fails with "Got empty identity" CredSSP error — NLA is now auto-disabled when username or password is not provided, letting the server prompt for credentials ([#29](https://github.com/totoshko88/RustConn/issues/29))
- Bitwarden vault unlock moved to a background thread — eliminates "application not responding" dialog on startup when Bitwarden is the configured secret backend

### Changed
- **CLI downloads** — Tailscale 1.94.1→1.94.2, Teleport 18.6.8→18.7.0, kubectl 1.35.0→1.35.1
- **Documentation** — Updated README, ARCHITECTURE, and USER_GUIDE with SSH port forwarding and deferred secret backend initialization

### Improved
- ~40 `eprintln!` calls migrated to structured `tracing` across GUI crate
- VNC client warns about unencrypted connections

### Internal
- `tracing` moved to workspace dependencies; deprecated flatpak re-exports removed
- API surface migrated from flat re-exports to modular paths (`rustconn_core::models::*`, etc.)
- Architecture audit: 51 findings, 49 resolved

- **serde_yaml** replaced with **serde_yaml_ng** 0.9 (maintained fork; transparent rename)
- **cpal** `0.17.1` → `0.17.3`
- **clap** `4.5.59` → `4.5.60`

## [0.8.8] - 2026-02-18

### Security
- **AES-256-GCM for stored credentials** — Replaced XOR obfuscation with AES-256-GCM + Argon2id key derivation for KeePassXC, Bitwarden, 1Password, and Passbolt passwords in settings; transparent migration from legacy format on first save
- **FreeRDP password via stdin** — Passwords are now passed using `/from-stdin` instead of `/p:{password}` command-line argument, preventing exposure via `/proc/PID/cmdline`

### Changed
- **FreeRDP detection unified** — Single `detect_best_freerdp()` function with Wayland-first candidate ordering (`wlfreerdp3` → `wlfreerdp` → `xfreerdp3` → `xfreerdp`); all detection paths delegate to it
- **RDP `build_args()` decoupled** — New `build_args()` and `build_command_with_binary()` methods on `RdpProtocol` separate argument construction from binary name; callers determine the binary via runtime detection
- **ZeroTrust validation** — Provider-specific `validate()` on `ZeroTrustConfig` checks required fields (AWS SSM target, GCP IAP instance/zone/project, Teleport cluster, Tailscale hostname, Generic command template) before save
- **ZeroTrust CLI detection** — CLI tool availability (`aws`, `gcloud`, `tsh`, `tailscale`) is verified before connection launch; missing tools show a toast and log a warning
- **ZeroTrust tracing** — Connection launch attempts and failures are now logged via `tracing` in both GUI and CLI paths
- **Native export format v2** — `NativeExport` now includes `snippets` field; backward-compatible with v1 imports via `#[serde(default)]`

- **native-tls** `0.2.14` → `0.2.18` — Removed version pin; 0.2.18 fixes the `Tlsv13` compile error from 0.2.17 ([#367](https://github.com/rust-native-tls/rust-native-tls/issues/367))
- **toml** `0.8` → `1.0` — Major version bump; no API changes required (re-export crate, fully compatible)
- **zip** `2.2` → `8.1` — Major version bump; replaced deprecated `mangled_name()` with `enclosed_name()` which adds path traversal validation
### Fixed
- **RDP HiDPI scaling on 4K displays** — IronRDP now sends `desktop_scale_factor` to the Windows server (e.g. 200% on a 2× display), so remote UI elements render at the correct logical size instead of appearing tiny; previously hardcoded to 0
- **RDP mouse coordinate mismatch on HiDPI** — Widget dimensions used for mouse→RDP coordinate transform now store CSS pixels (matching GTK event coordinates) instead of device pixels, fixing misaligned clicks on scaled displays
### Removed
- **Dashboard module** — Removed unused `ConnectionDashboard` GUI widget, core types (`SessionStats`, `DashboardFilter`), and property tests; session monitoring is handled by Active Sessions manager and sidebar indicators
- **5 dead GUI modules** — Removed `adaptive_tabs.rs`, `empty_state.rs`, `error_display.rs`, `floating_controls.rs`, `loading.rs` (all replaced by native adw/GTK4 equivalents)
- **`tab_split_manager` remnants** — Removed unused field from `MainWindow` and `SharedTabSplitManager` type alias; split view fully handled by `SplitViewBridge`

## [0.8.7] - 2026-02-17

### Security
- **Variable injection prevention** — All variable substitution in command-building paths now validates resolved values, rejecting null bytes, newlines, and control characters to prevent command injection
- **Checksum policy for CLI downloads** — Replaced placeholder SHA256 strings with `ChecksumPolicy` enum (`Static`, `SkipLatest`, `None`) for explicit integrity verification
- **Sensitive CLI arguments masked** — Password-like arguments (`/p:`, `--password`, `token=`, etc.) are masked in log output
- **Configurable document encryption** — `EncryptionStrength` enum (Standard/High/Maximum) with per-level Argon2 parameters; backward-compatible with legacy format
- **SSH Agent passphrase handling** — `add_key()` now uses `SSH_ASKPASS` helper script with `SSH_ASKPASS_REQUIRE=force` to securely pass passphrases to `ssh-add` without PTY; temporary script is cleaned up immediately after use

### Added
- **Internationalization (i18n)** — gettext support via `gettext-rs` with system libintl; `i18n` module with `i18n()`, `i18n_f()`, `ni18n()` helpers; translations for 14 languages: uk, de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk; closes [#17](https://github.com/totoshko88/RustConn/issues/17)
- **SPICE proxy support** — `SpiceConfig.proxy` field stores proxy URL from virt-viewer `.vv` imports; `remote-viewer` receives `--spice-proxy` flag for Proxmox VE tunnelled connections; fixes [#18](https://github.com/totoshko88/RustConn/issues/18)
- **RDP HiDPI fix** — IronRDP embedded client now multiplies widget dimensions by `scale_factor()` to negotiate device-pixel resolution on HiDPI displays, eliminating blurry upscaling; fixes [#16](https://github.com/totoshko88/RustConn/issues/16)
- **Property tests for variable injection** — 8 proptest properties validating command injection prevention
- **CLI delete confirmation** — Interactive prompt with `--force` flag to skip
- **CLI `--verbose` / `--quiet`** — Global flags for controlling output verbosity
- **CLI `--no-color` / `NO_COLOR`** — Per [no-color.org](https://no-color.org/) convention
- **CLI shell completions** — `completions <shell>` for bash, zsh, fish, elvish, PowerShell
- **CLI `--dry-run` for connect** — Prints command without executing
- **CLI pager for long output** — Pipes through `less` when output exceeds 40 lines
- **CLI auto-JSON when piped** — List commands switch to JSON when stdout is not a terminal
- **CLI fuzzy suggestions** — "Did you mean: x, y, z?" on connection name mismatch
- **CLI man page generation** — `man-page` subcommand via `clap_mangen`
- **Ctrl+M "Move to Group"** — Keyboard shortcut for moving sidebar items between groups
- **Search history navigation** — Up/Down arrows cycle through sidebar search history
- **CI version check workflow** — Weekly GitHub Action checks upstream CLI versions
- **Client detection caching** — 5-minute cache for CLI version checks
- **Flathub x-checker-data** — Automated dependency tracking for vte, libsecret, inetutils, picocom, mc
- **Flathub device metadata** — `<requires>`, `<recommends>`, `<supports>` in metainfo.xml

### Fixed
- **CLI `--config` flag** — Was declared but never used; now threads through all 43 `ConfigManager` call sites
- **Flatpak components dialog** — Hides unusable protocol clients in sandbox; shows only network-compatible tools
- **SPDX license** — `GPL-3.0+` → `GPL-3.0-or-later` in metainfo.xml

### Changed
- **VTE** — Flatpak manifests use VTE 0.78.7 (LTS branch for GNOME 46/47); `vte4` Rust crate 0.9 with `v0_72` feature
- **CLI modularized** — Split 5000+ line `main.rs` into 18 handler modules
- **CLI structured logging** — `tracing` replaces `eprintln!` with `--verbose`/`--quiet` control
- **VNC viewer list deduplicated** — Single `VNC_VIEWERS` constant shared across detection
- **Protocol icon mapping unified** — `get_protocol_icon_by_name()` in core replaces duplicate match blocks
- **Protocol command building unified** — `Protocol::build_command()` trait; CLI delegates to `ProtocolRegistry`
- **Send Text dialog** — Migrated to `adw::Dialog` per GNOME HIG
- **Sidebar minimum width** — Reduced from 200px to 160px
- **Tray polling optimized** — Split into 50ms message handling + 2s state sync with dirty-flag tracking

### Deprecated
- **Flatpak host command functions** — `host_command()`, `host_has_command()`, etc. in `flatpak.rs`; `flatpak-spawn --host` disabled since 0.7.7

### Improved
- **Accessible labels** — Added to 20+ icon-only buttons for screen reader compatibility
- **Czech translation (cs)** — Native speaker review by [p-bo](https://github.com/p-bo); 45 translations improved ([PR #19](https://github.com/totoshko88/RustConn/pull/19))
- **Remmina RDP import** — Now imports `gateway_server`, `gateway_username`, and `domain` fields from Remmina RDP profiles ([#20](https://github.com/totoshko88/RustConn/issues/20))

## [0.8.6] - 2026-02-16

### Fixed
- **Embedded RDP keyboard layout** — Fixed incorrect key mapping for non-US keyboard layouts (e.g. German QWERTZ) in IronRDP embedded client ([#15](https://github.com/totoshko88/RustConn/issues/15))
- **Secrets management** — Comprehensive fixes to vault credential storage, backend dispatch, and Bitwarden integration ([#14](https://github.com/totoshko88/RustConn/issues/14)):
  - All vault operations now respect `Settings → Secrets → preferred_backend` instead of being hardcoded to libsecret
  - Bitwarden encrypted password is decrypted and vault auto-unlocked at startup when preferred backend is Bitwarden
  - `PasswordSource::Inherit` resolves group passwords through non-KeePass backends with correct hierarchy traversal
  - RDP and VNC password prompts auto-save entered passwords to vault when `password_source == Vault`
  - Toast notifications shown on all vault save error paths
- **Flatpak component checksums** — Fixed kubectl installation failing with `ChecksumMismatch`; updated boundary v0.21.0 checksum
- **Flatpak component uninstall/reinstall** — Fixed `AlreadyInstalled` error when reinstalling AWS CLI and Google Cloud CLI
- **Terminal search Highlight All** — Fixed checkbox toggling to next match instead of highlighting

### Changed
- **Dependencies** — Updated: `futures` 0.3.31→0.3.32, `libc` 0.2.181→0.2.182, `uuid` 1.20.0→1.21.0, `bitflags` 2.10.0→2.11.0, `syn` 2.0.114→2.0.116, `native-tls` 0.2.14→0.2.16, `png` 0.18.0→0.18.1, `cc` 1.2.55→1.2.56

## [0.8.5] - 2026-02-15

### Added
- **Kubernetes Protocol** — Shell access to Kubernetes pods via `kubectl exec -it` ([#14](https://github.com/totoshko88/RustConn/issues/14)):
  - `KubernetesConfig` model with kubeconfig, context, namespace, pod, container, shell, busybox toggle
  - Two modes: exec into existing pod, or launch temporary busybox pod
  - GUI: Connection dialog Kubernetes tab, sidebar K8s quick filter
  - CLI: `kubernetes` subcommand with full flag support
  - Sandbox: kubectl as Flatpak downloadable component
- **Virt-Viewer (.vv) Import** — Import SPICE/VNC connections from virt-viewer files ([#13](https://github.com/totoshko88/RustConn/issues/13)):
  - Parses `[virt-viewer]` INI sections: host, port, tls-port, password, proxy, CA cert, title
  - Supports `type=spice` (with TLS detection) and `type=vnc`
  - Compatible with libvirt, Proxmox VE, and oVirt generated `.vv` files
- **Serial Console Protocol** — Full serial console support via `picocom` ([#11](https://github.com/totoshko88/RustConn/issues/11)):
  - `SerialConfig` model with device path, baud rate (9600–921600), data bits, stop bits, parity, flow control
  - GUI, CLI, and Flatpak sandbox support with bundled `picocom`
- **SFTP File Browser** — SFTP integration for SSH and standalone SFTP connections ([#10](https://github.com/totoshko88/RustConn/issues/10)):
  - "Open SFTP" action via `gtk::UriLauncher` (portal-aware)
  - "SFTP via mc" option with Midnight Commander FISH VFS
  - Standalone `ProtocolType::Sftp` connection type
- **Responsive / Adaptive UI** — Improved dialog sizing and window breakpoints ([#9](https://github.com/totoshko88/RustConn/issues/9))
- **Terminal Rich Search** — Regex, highlights, case-sensitive, wrap-around ([#7](https://github.com/totoshko88/RustConn/issues/7))

### Changed
- **Session Logging moved to Logging tab** — Better discoverability
- **CLI component versions updated** — Bitwarden CLI 2024.12.0→2026.1.0, Teleport 17.1.2→18.6.8, Boundary 0.18.1→0.21.0, 1Password CLI 2.30.0→2.32.1, kubectl 1.32.0→1.35.0

### Fixed
- **Flathub linter `finish-args-home-filesystem-access`** — Replaced `--filesystem=home` with `--filesystem=xdg-download:create`
- **Flathub linter `module-rustconn-source-git-no-commit-with-tag`** — Added explicit `commit` hash
- **ZeroTrust icon inconsistency** — Changed to `security-high-symbolic` across all UI
- **SFTP tab icon** — Correct `folder-remote-symbolic` icon
- **SFTP sidebar status** — Shows connecting/connected status and increments session count

## [0.8.4] - 2026-02-14

### Added
- **FIDO2/SecurityKey SSH authentication** — `SshAuthMethod::SecurityKey` variant for hardware key auth
- **CLI auth-method support** — `--auth-method` flag for `add` and `update` commands

### Fixed
- **CLI version check timeout** — Increased from 3 to 6 seconds for Azure CLI
- **Settings dialog startup delay** — Replaced blocking `is_secret_tool_available_sync()` with cached async detection
- **WoL MAC Entry Disabled on Edit** — Fixed sensitivity conflict between widget and group-level control
- **secret-tool detection** — Replaced invalid `secret-tool --version` with `which secret-tool`
- **Settings version label race condition** — Added `detection_complete` flag
- **Unequal split panel sizes** — Set `size_request(0, 0)` on panel containers

### Refactored
- **ConnectionManager watch channels** — Replaced `Arc<Mutex<Option<Vec<T>>>>` with `tokio::sync::watch`
- **Embedded RDP module directory** — Reorganized into `embedded_rdp/` with 6 submodules
- **Window module directory** — Reorganized 14 flat files into `window/` directory
- **OverlaySplitView sidebar** — Replaced `gtk::Paned` with `adw::OverlaySplitView`
- **Protocol trait capabilities** — Extended with `capabilities()` and `build_command()`

### Changed
- **Dependencies** — Updated `resvg` 0.46→0.47

## [0.8.3] - 2026-02-13

### Added
- **Wake On LAN from GUI** — Send WoL magic packets directly from the GUI ([#8](https://github.com/totoshko88/RustConn/issues/8))

### Fixed
- **Flatpak libsecret Build** — Disabled `bash_completion` (EROFS in sandbox)
- **Flatpak libsecret Crypto Option** — Renamed `gcrypt` to `crypto`
- **Thread Safety** — Removed `std::env::set_var` from FreeRDP spawned thread
- **Flatpak Machine Key** — App-specific key file in `$XDG_DATA_HOME`
- **Variables Dialog Panic** — Replaced `expect()` with `if let Some(window)` pattern
- **Keyring `secret-tool` Check** — Returns `SecretError::BackendUnavailable` if not installed
- **Flatpak CLI Paths** — No longer adds hardcoded paths when running inside Flatpak
- **Settings Dialog Performance** — Moved all detection to background threads; dialog opens instantly
- **Settings Clients Tab Performance** — Parallelized CLI detection; ~15s → ~3s
- **Settings Dialog Visual Render Blocking** — Replaced `glib::spawn_future` with `std::thread::spawn` + `glib::idle_add_local`

## [0.8.2] - 2026-02-11

### Added
- **Shared Keyring Module** — Generic `store()`, `lookup()`, `clear()` for all secret backends
- **Keyring Support for All Backends** — Bitwarden, 1Password, Passbolt, KeePassXC
- **Auto-Load Credentials from Keyring** — Automatic restore on settings load
- **Flatpak `secret-tool` Support** — `libsecret` 0.21.7 as Flatpak build module
- **Passbolt Server URL Setting** — New field in `SecretSettings`
- **Unified Credential Save Options** — Consistent "Save password" / "Save to keyring" across all backends

## [0.8.1] - 2026-02-11

### Added
- **Passbolt Secret Backend** — Passbolt password manager integration ([#6](https://github.com/totoshko88/RustConn/issues/6)):
  - `PassboltBackend` implementing `SecretBackend` trait via `go-passbolt-cli`
  - Store, retrieve, and delete credentials as Passbolt resources
  - CLI detection and version display in Settings → Secrets
  - Server configuration status check (configured/not configured/auth failed)
  - `PasswordSource::Passbolt` option in connection dialog password source dropdown
  - `SecretBackendType::Passbolt` option in settings backend selector
  - Credential resolution and rename support in `CredentialResolver`
  - Requires `passbolt configure` CLI setup before use

### Changed
- **Unified Secret Backends** — Replaced individual `PasswordSource` variants (KeePass, Keyring, Bitwarden, OnePassword, Passbolt) with single `Vault` variant:
  - Connection dialog password source dropdown: Prompt, Vault, Variable, Inherit, None
  - Serde aliases preserve backward compatibility with existing configs
  - `PasswordSource` is now `Clone` only (no longer `Copy`) due to `Variable(String)`
- **Variable Password Source** — New `PasswordSource::Variable(String)` reads credentials from a named secret global variable:
  - Connection dialog shows variable dropdown when "Variable" is selected
  - Dropdown populated with secret global variables only
- **Variables Dialog Improvements** — Show/Hide and Load from Vault buttons for secret variables:
  - Toggle password visibility with `view-reveal-symbolic`/`view-conceal-symbolic` icon
  - Load secret value from vault with key `rustconn/var/{name}`
  - Secret variable values auto-saved to vault on dialog save, cleared from settings file

### Fixed
- **Secret Variable Vault Backend** — Fixed secret variables always using libsecret instead of configured backend:
  - Save/load secret variable values now respects Settings → Secrets backend (KeePassXC, libsecret)
  - Added `save_variable_to_vault()` and `load_variable_from_vault()` functions using settings snapshot
  - Toast notification on vault save/load failure with message to check Settings
- **Variable Dropdown Empty in Connection Dialog** — Fixed Variable dropdown showing "(Немає)" when editing connections:
  - `set_global_variables()` was never called when creating/editing connections
  - Added call to all three `ConnectionDialog` creation sites (new, edit, template)
  - Edit dialog: `set_global_variables()` called before `set_connection()` so variable selection works
- **Telnet Backspace/Delete Key Handling** — Fixed keyboard settings not working correctly for Telnet connections ([#5](https://github.com/totoshko88/RustConn/issues/5)):
  - Replaced `stty erase` shell wrapper approach with VTE native `EraseBinding` API
  - Backspace/Delete settings now applied directly on the VTE terminal widget before process spawn
  - `Automatic` mode uses VTE defaults (termios for Backspace, VT220 `\e[3~` for Delete)
  - `Backspace (^H)` sends ASCII `0x08`, `Delete (^?)` sends ASCII `0x7F` as expected
  - Fixes Delete key showing `3~` escape artifacts on servers that don't support VT220 sequences
- **Split View Panel Sizing** — Fixed left panel shrinking when splitting vertically then horizontally:
  - Use model's fractional position (0.0–1.0) instead of hardcoded `size / 2` for divider placement
  - Disable `shrink_start_child`/`shrink_end_child` to prevent panels from collapsing below minimum size
  - One-shot position initialization via `connect_map` prevents repeated resets on widget remap
  - Save user-dragged divider positions back to the model via `connect_notify_local("position")`
  - Each split now correctly divides the current panel in half without affecting other panels

## [0.8.0] - 2026-02-10

### Added
- **Telnet Backspace/Delete Configuration** — Configurable keyboard behavior for Telnet connections ([#5](https://github.com/totoshko88/RustConn/issues/5)):
  - `TelnetBackspaceSends` and `TelnetDeleteSends` enums with Automatic/Backspace/Delete options
  - Connection dialog Keyboard group with two dropdowns for Backspace and Delete key behavior
  - `stty erase` shell wrapper in `spawn_telnet()` to apply key settings before connecting
  - Addresses common backspace/delete inversion issue reported by users
- **Flatpak Telnet Support** — GNU inetutils built as Flatpak module:
  - `telnet` binary available at `/app/bin/` in Flatpak sandbox
  - Built from inetutils 2.7 source with `--disable-servers` (client tools only)
  - Added to all three Flatpak manifests (flatpak, flatpak-local, flathub)

### Changed
- **Dependencies** — Updated: `libc` 0.2.180→0.2.181, `tempfile` 3.24.0→3.25.0, `unicode-ident` 1.0.22→1.0.23

### Fixed
- **OBS Screenshot Display** — Updated `_service` revision from `v0.5.3` to current version tag for proper AppStream metadata processing on software.opensuse.org
- **Flatpak AWS CLI** — Replaced `awscliv2` pip package (Docker wrapper) with official AWS CLI v2 binary installer from `awscli.amazonaws.com`; `aws --version` now shows real AWS CLI instead of Docker error
- **Flatpak Component Detection** — Fixed SSM Plugin, Azure CLI, and OCI CLI showing as "Not installed" after installation:
  - Added explicit search paths for SSM Plugin (`usr/local/sessionmanagerplugin/bin`) and AWS CLI (`v2/current/bin`)
  - Increased recursive binary search depth from 3 to 5/6 levels
- **Flatpak Python Version** — Wrapper scripts for pip-installed CLIs (Azure CLI, OCI CLI) now dynamically detect Python version instead of hardcoding `python3.13`

## [0.7.9] - 2026-02-09

### Added
- **Telnet Protocol Support** — Full Telnet protocol implementation across all crates ([#5](https://github.com/totoshko88/RustConn/issues/5)):
  - Core model: `TelnetConfig`, `ProtocolType::Telnet`, `ProtocolConfig::Telnet` with configurable host, port (default 23), and extra arguments
  - Protocol trait implementation with external `telnet` client
  - Import support: Remmina, Asbru, MobaXterm, RDM importers recognize Telnet connections
  - Export support: Remmina, Asbru, MobaXterm exporters write Telnet connections
  - CLI: `rustconn-cli telnet` subcommand with `--host`, `--port`, `--extra-args` options
  - GUI: Connection dialog with Telnet-specific configuration tab
  - Template dialog: Telnet protocol option with default port 23
  - Sidebar: Telnet filter button with `network-wired-symbolic` icon
  - Terminal: `spawn_telnet()` method for launching telnet sessions
  - Quick Connect: Telnet protocol option in quick connect bar
  - Cluster dialog: Telnet connections selectable for cluster membership
  - Property tests: All existing property tests updated with Telnet coverage

### Fixed
- **Sidebar Icon Missing** — Added missing `"telnet"` mapping in sidebar `get_protocol_icon()` function; Telnet connections now display the correct icon in the connection tree
- **Telnet Icon Mismatch** — Changed Telnet protocol icon from `network-wired-symbolic` to `call-start-symbolic` across all views (sidebar, filter buttons, dialogs, templates); the previous icon resembled a shield in breeze-dark theme, which was misleading for an insecure protocol
- **ZeroTrust Sidebar Icon** — Unified ZeroTrust sidebar icon to `folder-remote-symbolic` for all providers; previously showed provider-specific icons that were inconsistent with the filter button icon

## [0.7.8] - 2026-02-08

### Added
- **Remmina Password Import** — Importing from Remmina now automatically transfers saved passwords into the configured secret backend (libsecret, KeePassXC, etc.); connections are marked with `PasswordSource::Keyring` so credentials resolve seamlessly on first connect

### Fixed
- **Import Error Swallowing** — Replaced 14 `.unwrap_or_default()` calls in import dialog with proper error propagation; import failures now display user-friendly messages instead of silently returning empty results
- **MobaXterm Import Double Allocation** — Removed unnecessary `.clone()` on byte buffer during UTF-8 conversion; recovers original bytes from error on fallback path instead of cloning upfront

### Improved
- **Import File Size Guard** — Added 50 MB file size limit check in `read_import_file()` to prevent OOM on accidentally selected large files
- **Native Export Streaming I/O** — `NativeExport::to_file()` now uses `BufWriter` with `serde_json::to_writer_pretty()` instead of serializing entire JSON to `String` first; eliminates intermediate allocation
- **Native Import Streaming I/O** — `NativeExport::from_file()` now uses `BufReader` with `serde_json::from_reader()` instead of reading entire file to `String`; reduces peak memory by ~50%
- **Native Import Version Pre-Check** — Version validation now runs before full deserialization; rejects unsupported format versions without parsing all connections and groups
- **Export File Writing** — Added centralized `write_export_file()` helper with `BufWriter` for consistent buffered writes across all exporters

### Refactored
- **Export Write Consolidation** — Replaced duplicated `fs::write` + error mapping boilerplate in SSH config, Ansible, Remmina, Asbru, Royal TS, and MobaXterm exporters with shared `write_export_file()` helper
- **TOCTOU Elimination** — Removed redundant `path.exists()` checks before file reads in importers; the subsequent `read_import_file()` already returns `ImportError` on failure
- **Unused Imports Cleanup** — Removed unused `ExportError` import from Asbru exporter and moved `std::fs` import to `#[cfg(test)]` in MobaXterm exporter

- Updated `memchr` 2.7.6 → 2.8.0
- Updated `ryu` 1.0.22 → 1.0.23
- Updated `zerocopy` 0.8.38 → 0.8.39
- Updated `zmij` 1.0.19 → 1.0.20
## [0.7.7] - 2026-02-08

### Fixed
- **Keyboard Shortcuts** — `Delete`, `Ctrl+E`, and `Ctrl+D` no longer intercept input when VTE terminal or embedded viewers have focus; these shortcuts now only activate from the sidebar ([#4](https://github.com/totoshko88/RustConn/issues/4))

### Improved
- **Thread Safety** — Audio mutex locks use graceful fallback instead of `unwrap()`, preventing potential panics in real-time audio callbacks
- **Thread Safety** — Search engine mutex locks use graceful recovery patterns throughout `DebouncedSearchEngine`
- **Security** — VNC client logs a warning when connection is attempted without a password

### Refactored
- **Runtime Consolidation** — Replaced 23 redundant `tokio::runtime::Runtime::new()` calls across GUI code with shared `with_runtime()` pattern, reducing resource overhead
- **Collection Optimization** — Snippet tag collection uses `flat_map` with `iter().cloned()` instead of `clone()`, and `sort_unstable()` for better performance
- **Dead Code Removal** — Removed 3 deprecated blocking credential methods from `AppState` (`store_credentials`, `retrieve_credentials`, `delete_credentials`)
- **Dead Code Removal** — Removed unused `build_pane_context_menu` from `MainWindow`

## [0.7.6] - 2026-02-07

### Added
- **Flatpak Components Manager** — On-demand CLI download for Flatpak environment:
  - Menu → Flatpak Components... (visible only in Flatpak)
  - Download and install CLIs to `~/.var/app/io.github.totoshko88.RustConn/cli/`
  - Supports: AWS CLI, AWS SSM Plugin, Google Cloud CLI, Azure CLI, OCI CLI, Teleport, Tailscale, Cloudflare Tunnel, Boundary, Bitwarden CLI, 1Password CLI, TigerVNC
  - Python-based CLIs installed via pip, .deb packages extracted automatically
  - Install/Remove/Update with progress indicators and cancel support
  - SHA256 checksum verification (except AWS SSM Plugin which uses "latest" URL)
  - Settings → Clients detects CLIs installed via Flatpak Components

- **Snap Strict Confinement** — Migrated from classic to strict confinement:
  - Snap-aware path resolution for data, config, and SSH directories
  - Interface connection detection with user-friendly messages
  - Uses embedded clients (IronRDP, vnc-rs, spice-gtk) — no bundled external CLIs
  - External CLIs accessed from host via `system-files` interface

### Changed
- **Flatpak Permissions** — Simplified security model:
  - Removed `--talk-name=org.freedesktop.Flatpak` (no host command access)
  - SSH available in runtime, embedded clients for RDP/VNC/SPICE
  - Use Flatpak Components dialog to install additional CLIs

- **Snap Package** — Strict confinement with host CLI access:
  - Added plugs for ssh-keys, personal-files, system-files
  - Data stored in `~/snap/rustconn/current/`
  - Smaller package (~50 MB) using host-installed binaries

- **Settings → Clients** — Improved client detection display:
  - All protocols (SSH, RDP, VNC, SPICE) show embedded client status
  - Blue indicator (●) for embedded clients, green (✓) for external
  - Fixed AWS SSM Plugin detection (was looking for wrong binary name)

### Improved
- **UI/UX** — GNOME HIG compliance:
  - Accessible labels for status icons and protocol filter buttons
  - Sidebar minimum width increased to 200px
  - Connection dialog uses adaptive `adw::ViewSwitcherTitle`
  - Toast notifications with proper priority levels

- **Thread Safety** — Mutex poisoning recovery in FreeRDP thread

### Fixed
- **RDP Variable Substitution** — Global variables now resolve in username/domain fields

### Refactored
- **Dialog Widget Builders** — Reusable UI components (`CheckboxRowBuilder`, `EntryRowBuilder`, `SpinRowBuilder`, `DropdownRowBuilder`, `SwitchRowBuilder`)
- **Protocol Dialogs** — Applied widget builders to SSH, RDP, VNC, SPICE panels
- **Legacy Cleanup** — Removed unused `TabDisplayMode`, `TabLabelWidgets` types

### Documentation
- **New**: `docs/SNAP.md` — Snap user guide with interface setup
- **Updated**: `docs/INSTALL.md`, `docs/USER_GUIDE.md`

## [0.7.5] - 2026-02-06

### Refactored
- **Code Quality Audit** - Comprehensive codebase analysis and cleanup:
  - Removed duplicate SSH options code from `dialog.rs` (uses `ssh::create_ssh_options()`)
  - Removed duplicate VNC/SPICE/ZeroTrust options code from `dialog.rs` (~830 lines)
  - Removed duplicate RDP options code from `dialog.rs` (~350 lines, uses `rdp::create_rdp_options()`)
  - Removed legacy dialog functions (`create_automation_tab`, `create_tasks_tab`, `create_wol_tab`) (~250 lines)
  - Extracted shared folders UI into reusable `shared_folders.rs` module
  - Extracted Zero Trust UI into `zerotrust.rs` module (~450 lines)
  - Created `protocol_layout.rs` with `ProtocolLayoutBuilder` for consistent protocol UI
  - Consolidated `with_runtime()` into `async_utils.rs` (removed duplicate from `state.rs`)
  - Changed FreeRDP launcher to Wayland-first (`force_x11: false` by default)
  - Removed legacy no-op methods from terminal module (~40 lines)
  - **Total dead/duplicate code removed: ~1850+ lines**

### Fixed
- **Wayland-First FreeRDP** - External RDP client now uses Wayland backend by default:
  - Changed `SafeFreeRdpLauncher::default()` to set `force_x11: false`
  - X11 fallback still available via `with_x11_fallback()` constructor

### Changed
- **Dependencies** - Updated: proptest 1.9.0→1.10.0, time 0.3.46→0.3.47, time-macros 0.2.26→0.2.27
- **Architecture Documentation** - Updated `docs/ARCHITECTURE.md` with:
  - Current architecture diagram
  - Recommended layered architecture for future refactoring
  - Module responsibility guidelines
  - New modules: `protocol_layout.rs`, `shared_folders.rs`

## [0.7.4] - 2026-02-05

### Fixed
- **Split View Protocol Restriction** - Split view is now disabled for RDP, VNC, and SPICE tabs:
  - Only SSH, Local Shell, and ZeroTrust tabs support split view
  - Attempting to split an embedded protocol tab shows a toast notification
  - Prevents UI issues with embedded widgets that cannot be reparented
- **Split View Tab Close Cleanup** - Closing a tab now properly clears its panel in split view:
  - Panel shows "Empty Panel" placeholder with "Select Tab" button after tab is closed
  - Works for both per-session split bridges and global split view
  - Added `on_split_cleanup` callback to `TerminalNotebook` for proper cleanup coordination
  - Fixes issue where terminal content remained visible after closing tab
- **Document Close Dialog** - Fixed potential panic when closing document without parent window:
  - `CloseDocumentDialog::present()` now gracefully handles missing parent window
  - Logs error and calls callback with `None` instead of panicking
- **Zero Trust Entry Field Alignment** -додай зміни в чендлог і онови architecture.md в doc Fixed inconsistent width of input fields in Zero Trust provider panels:
  - Converted all Zero Trust provider fields from `ActionRow` + `Entry` to `adw::EntryRow`
  - All 10 provider panels (AWS SSM, GCP IAP, Azure Bastion, Azure SSH, OCI Bastion, Cloudflare, Teleport, Tailscale, Boundary, Generic) now have consistent field widths
  - Follows GNOME HIG guidelines for proper libadwaita input field usage

### Refactored
- **Import File I/O** - Extracted common file reading pattern into `read_import_file()` helper:
  - Reduces code duplication across 5 import sources (SSH config, Ansible, Remmina, Asbru, Royal TS)
  - Consistent error handling with `ImportError::ParseError`
  - Added async variant `read_import_file_async()` for future use
- **Protocol Client Errors** - Consolidated duplicate error types into unified `EmbeddedClientError`:
  - Merged `RdpClientError`, `VncClientError`, `SpiceClientError` (~60 lines reduced)
  - Type aliases maintain backward compatibility
  - Common variants: `ConnectionFailed`, `AuthenticationFailed`, `ProtocolError`, `IoError`, `Timeout`
- **Config Atomic Writes** - Improved reliability of configuration file saves:
  - Now uses temp file + atomic rename pattern
  - Prevents config corruption on crash during write
  - Applied to `save_toml_file_async()` in `ConfigManager`
- **Connection Dialog Modularization** - Refactored monolithic `connection.rs` into modular structure:
  - Created `rustconn/src/dialogs/connection/` directory with protocol-specific modules
  - `dialog.rs` - Main `ConnectionDialog` implementation (~6,600 lines)
  - `ssh.rs` - SSH options panel (~460 lines, prepared for future integration)
  - `rdp.rs` - RDP options panel (~414 lines, prepared for future integration)
  - `vnc.rs` - VNC options panel (~249 lines, prepared for future integration)
  - `spice.rs` - SPICE options panel (~240 lines, reuses rdp:: folder functions)
  - Improves code organization and maintainability

### Added
- **Variables Menu Item** - Added "Variables..." menu item to Tools menu for managing global variables:
  - Opens Variables dialog to view/edit global variables
  - Variables are persisted to settings and substituted at connection time
  - Accessible via Tools → Variables...
- **GTK Lifecycle Documentation** - Added module-level documentation explaining `#[allow(dead_code)]` pattern:
  - Documents why GTK widget fields must be kept alive for signal handlers
  - Prevents accidental removal of "unused" fields that would cause segfaults
- **Type Alias Documentation** - Added documentation explaining why `Rc` is used instead of `Arc`:
  - GTK4 is single-threaded, so atomic operations are unnecessary overhead
  - `Rc<RefCell<_>>` pattern matches GTK's single-threaded model
  - Documented in `window_types.rs` module header

### Changed
- **Dialog Size Unification** - Standardized dialog window sizes for visual consistency:
  - Connection History: 750×500 (increased from 550 for better content display)
  - Keyboard Shortcuts: 550×500 (increased from 500 for consistency)
- **Code Quality** - Comprehensive cleanup based on code audit:
  - Removed legacy `TabDisplayMode`, `SessionWidgetStorage`, `TabLabelWidgets` types
  - Standardized error type patterns with `#[from]` attribute
  - Reduced unnecessary `.clone()` calls in callback chains
  - Improved `expect()` messages to clarify provably impossible states
  - Added `# Panics` documentation for functions with justified `expect()` calls
- **Dependencies** - Updated: clap 4.5.56→4.5.57, criterion 0.8.1→0.8.2, hybrid-array 0.4.6→0.4.7, zerocopy 0.8.37→0.8.38

### Tests
- Updated property tests for consolidated error types
- Verified all changes pass `cargo clippy --all-targets` and `cargo fmt --check`

## [0.7.3] - 2026-02-03

### Fixed
- **Azure CLI Version Parsing** - Fixed version detection showing "-" instead of actual version:
  - Added dedicated parser for Azure CLI's unique output format (`azure-cli  2.82.0 *`)
  - Version now correctly extracted and displayed in Settings → Clients
- **Teleport CLI Version Parsing** - Fixed version showing full output instead of clean version:
  - Added dedicated parser for Teleport's output format (`Teleport v18.6.5 git:...`)
  - Now displays clean version like `v18.6.5`
- **Flatpak XDG Config** - Removed unnecessary `--filesystem=xdg-config/rustconn:create` permission:
  - Flatpak sandbox automatically provides access to `$XDG_CONFIG_HOME`
  - Configuration now stored in standard Flatpak location (`~/.var/app/io.github.totoshko88.RustConn/config/`)
- **Teleport CLI Detection** - Fixed detection using wrong binary name (`teleport` → `tsh`)

### Changed
- **RDP Client Detection** - Improved FreeRDP detection with Wayland support:
  - Priority order: FreeRDP 3.x (wlfreerdp3/xfreerdp3) → FreeRDP 2.x (wlfreerdp/xfreerdp) → rdesktop
  - Wayland-native clients (wlfreerdp3/wlfreerdp) now checked before X11 variants
  - Updated install hint to recommend freerdp3-wayland package
- **Client Install Hints** - Unified and improved package installation messages:
  - Format: `Install <deb-package> (<rpm-package>) package`
  - SSH: `openssh-client (openssh-clients)`
  - RDP: `freerdp3-wayland (freerdp)`
  - VNC: `tigervnc-viewer (tigervnc)`
  - Zero Trust CLIs: simplified to package names only
- **Dependencies** - Updated: bytes 1.11.0→1.11.1, flate2 1.1.8→1.1.9, regex 1.12.2→1.12.3

### Refactored
- **Client Detection** - Unified detection logic in `rustconn-core`:
  - Removed duplicate version parsing from `clients_tab.rs` (~200 lines)
  - Added `detect_spice_client()` to core detection module
  - Added `ZeroTrustDetectionResult` struct for all Zero Trust CLI clients
  - GUI now uses `ClientDetectionResult` and `ZeroTrustDetectionResult` from core

## [0.7.2] - 2026-02-03

### Added
- **Flatpak Host Command Support** - New `flatpak` module for running host commands from sandbox:
  - `is_flatpak()` - Detects if running inside Flatpak sandbox
  - `host_command()` - Creates command that runs on host via `flatpak-spawn --host`
  - `host_has_command()`, `host_which()` - Check for host binaries
  - `host_exec()`, `host_spawn()` - Execute/spawn host commands
  - Enables external clients (xfreerdp, vncviewer, aws, gcloud) to work in Flatpak

### Changed
- **Dependencies** - Updated: hyper-util 0.1.19→0.1.20, system-configuration 0.6.1→0.7.0, zmij 1.0.18→1.0.19
- **Flatpak Permissions** - Extended sandbox permissions for full functionality:
  - `xdg-config/rustconn:create` - Config directory access
  - `org.freedesktop.Flatpak` - Host command execution (xfreerdp, vncviewer, aws, etc.)
  - `org.freedesktop.secrets` - GNOME Keyring access
  - `org.kde.kwalletd5/6` - KWallet access
  - `org.keepassxc.KeePassXC.BrowserServer` - KeePassXC proxy
  - `org.kde.StatusNotifierWatcher` - System tray support

### Fixed
- **Flatpak Config Access** - Added `xdg-config/rustconn:create` permission to Flatpak manifests:
  - Connections, groups, snippets, and settings now persist correctly in Flatpak sandbox
  - Previously, Flatpak sandbox blocked access to `~/.config/rustconn`
- **Split View Equal Proportions** - Fixed split panels having unequal sizes:
  - Changed from timeout-based to `connect_map` + `idle_add` for reliable size detection
  - Panels now correctly split 50/50 regardless of timing or rendering delays
  - Added `shrink_start_child` and `shrink_end_child` for balanced resizing

## [0.7.1] - 2026-02-01

### Added
- **Undo/Trash Functionality** - Safely recover deleted items (COMP-FUNC-01):
  - Deleted items are moved to Trash and can be restored via "Undo" notification
  - Implemented persisted Trash storage for recovery across sessions
- **Group Inheritance** - Simplify connection configuration (COMP-FUNC-03):
  - Added ability to inherit Username and Domain from parent Group
  - "Load from Group" buttons auto-fill credential fields from group settings

### Changed
- **Dependencies** - Updated: bytemuck 1.24.0→1.25.0, portable-atomic 1.13.0→1.13.1, slab 0.4.11→0.4.12, zerocopy 0.8.36→0.8.37, zerocopy-derive 0.8.36→0.8.37, zmij 1.0.17→1.0.18
- **Persistence Optimization** - Implemented debounced persistence for connections and groups (TECH-02):
  - Changes are now batched and saved after 2 seconds of inactivity
  - Reduces disk I/O during rapid modifications (e.g., drag-and-drop reordering)
  - Added `flush_persistence` to ensure data safety on application exit
- **Sort Optimization** - Improved rendering performance (COMP-FUNC-02):
  - Sorting is now skipped when data order hasn't changed, reducing CPU usage
  - Optimized `sort_all` calls during UI updates
- **Connection History Sorting** - History entries now sorted by date descending (newest first)

### Fixed
- **Credential Inheritance from Groups** - Fixed password inheritance not working for connections:
  - Connections with `password_source=Inherit` now correctly resolve credentials from parent group's KeePass entry
  - Added direct KeePass lookup for group credentials in `resolve_credentials_blocking`
- **GTK Widget Parenting** - Fixed `gtk_widget_set_parent` assertion failure in split view:
  - `set_panel_content` now checks if widget has parent before calling `unparent()`
- **Connection History Reconnect** - Fixed reconnecting from Connection History not opening tab:
  - History reconnect now uses `start_connection_with_credential_resolution` for proper credential handling
  - Previously showed warning about missing credentials for RDP connections
- **Blocking I/O** - Fixed UI freezing during save operations by moving persistence to background tasks (Async Persistence):
  - Added global Tokio runtime to main application
  - Implemented async save methods in `ConfigManager`
  - `ConnectionManager` now saves connections and groups in non-blocking background tasks
- **Code Quality** - Comprehensive code cleanup and optimization:
  - Fixed `future_not_send` issues in async persistence layer
  - Resolved type complexity warnings in `ConnectionManager`
  - Removed dead code and unused imports across sidebar modules
  - Enforced `clippy` pedantic checks for better robustness

### Refactored
- **Sidebar Module** - Decomposed monolithic `sidebar.rs` into focused submodules (TECH-03):
  - `search.rs`: Encapsulated search logic, predicates, and history management
  - `filter.rs`: centralized protocol filter button creation and state management
  - `view.rs`: Isolated UI list item creation, binding, and signal handling
  - `drag_drop.rs`: Prepared structure for drag-and-drop logic separation
  - Improved compile times and navigation by splitting 2300+ line file
- **Drag and Drop Refactoring** - Replaced string-based payloads with strongly typed `DragPayload` enum (TECH-04):
  - Uses `serde_json` for robust serialization instead of manual string parsing
  - Centralized drag logic in `drag_drop.rs`
  - Improved type safety for drag-and-drop operations

### UI/UX
- **Search Highlighting** - Added visual feedback for search matches (TECH-05):
  - Matched text substrings are now highlighted in bold
  - Implemented case-insensitive fuzzier matching with Pango markup
  - Improved `Regex`-based search logic

## [0.7.0] - 2026-02-01

### Fixed
- **Asbru Import Nested Groups** - Fixed group hierarchy being lost when importing from Asbru-CM:
  - Groups with subgroups (e.g., Group1 containing Group11, Group12, etc.) now correctly preserve parent-child relationships
  - Previously, HashMap iteration order caused child groups to be processed before their parents were added to the UUID map, resulting in orphaned root-level groups
  - Now uses two-pass algorithm: first creates all groups and populates UUID map, then resolves parent references
  - Special Asbru parent keys (`__PAC__EXPORTED__`, `__PAC__ROOT__`) are now properly skipped
- **Asbru Export Description Field** - Fixed description not being exported for connections and groups:
  - Connection description now exports from `connection.description` field directly
  - Falls back to legacy `desc:` tags only if description field is empty
  - Group description now exports when present

### Added
- **Group Description Field** - Groups can now have a description field for storing project info, contacts, notes:
  - Added `description: Option<String>` to `ConnectionGroup` model
  - Asbru importer now imports group descriptions
  - Edit Group dialog now includes Description text area for viewing/editing
  - New Group dialog now includes Description text area (unified with Edit Group)
- **Asbru Global Variable Conversion** - Asbru-CM global variable syntax is now converted during import:
  - `<GV:VAR_NAME>` is automatically converted to RustConn syntax `${VAR_NAME}`
  - Applies to username field (e.g., `<GV:US_Parrallels_User>` → `${US_Parrallels_User}`)
  - Plain usernames remain unchanged
- **Variable Substitution at Connection Time** - Global variables are now resolved when connecting:
  - `${VAR_NAME}` in host and username fields are replaced with variable values
  - Works for SSH, RDP, VNC, and SPICE connections
  - Variables are defined in Settings → Variables

### Changed
- **Export Dialog** - Added informational message about credential storage:
  - New info row explains that passwords are stored in password manager and not exported by default
  - Reminds users to export credential structure separately if needed for team sharing
- **Dialog Size Unification** - Standardized dialog window sizes for visual consistency:
  - New Group dialog: 450×550 (added Description field, unified with Edit Group)
  - Export dialog: 750×650 (increased height for content)
  - Import dialog: 750×800 (increased height for content)
  - Medium forms (550×550): New Snippet, New Cluster, Statistics
  - Info dialogs (500×500): Keyboard Shortcuts, Connection History
  - Simple forms (450): Quick Connect, Edit Group, Rename
  - Password Generator: 750×650 (unified with Connection/Template dialogs)

## [0.6.9] - 2026-01-31

### Added
- **Password Caching TTL** - Cached credentials now expire after configurable time (default 5 minutes):
  - `CachedCredentials` with `cached_at` timestamp and `is_expired()` method
  - `cleanup_expired_credentials()` for automatic cleanup
  - `refresh_cached_credentials()` to extend TTL on use
- **Connection Retry Logic** - Automatic retry with exponential backoff for failed connections:
  - `RetryConfig` with max_attempts, base_delay, max_delay, jitter settings
  - `RetryState` for tracking retry progress
  - Preset configurations: `aggressive()`, `conservative()`, `no_retry()`
- **Loading States** - Visual feedback for long-running operations:
  - `LoadingOverlay` component for inline loading indicators
  - `LoadingDialog` for modal operations with cancel support
  - `with_loading_dialog()` helper for async operations
- **Keyboard Navigation Helpers** - Improved dialog keyboard support:
  - `setup_dialog_shortcuts()` for Escape/Ctrl+S/Ctrl+W
  - `setup_entry_activation()` for Enter key handling
  - `make_default_button()` and `make_destructive_button()` styling helpers
- **Session State Persistence** - Split layouts preserved across restarts:
  - `SessionRestoreData` and `SplitLayoutRestoreData` structs
  - JSON serialization for session state
  - Automatic save/load from config directory
- **Connection Health Check** - Periodic monitoring of active sessions:
  - `HealthStatus` enum (Healthy, Unhealthy, Unknown, Terminated)
  - `HealthCheckConfig` with interval and auto_cleanup settings
  - `perform_health_check()` and `get_session_health()` methods
- **Log Sanitization** - Automatic removal of sensitive data from logs:
  - `SanitizeConfig` with patterns for passwords, API keys, tokens
  - AWS credentials and private key detection
  - `contains_sensitive_prompt()` helper
- **Async Architecture Helpers** - Improved async handling in GUI:
  - `spawn_async()` for non-blocking operations
  - `spawn_async_with_callback()` for result handling
  - `block_on_async_with_timeout()` for bounded blocking
  - `is_main_thread()` and `ensure_main_thread()` utilities
- **RDP Backend Selector** - Centralized RDP backend selection:
  - `RdpBackend` enum (IronRdp, WlFreeRdp, XFreeRdp3, XFreeRdp, FreeRdp)
  - `RdpBackendSelector` with detection caching
  - `select_embedded()`, `select_external()`, `select_best()` methods
- **Import/Export Enhancement** - Detailed import statistics:
  - `SkippedField` and `SkippedFieldReason` for tracking skipped data
  - `ImportStatistics` with detailed reporting
  - `detailed_report()` for human-readable summaries
- **Bulk Credential Operations** - Mass credential management:
  - `store_bulk()`, `delete_bulk()`, `update_bulk()` methods
  - `update_credentials_for_group()` for group-wide updates
  - `copy_credentials()` between connections
- **1Password as PasswordSource** - 1Password can now be selected per-connection:
  - Added `OnePassword` variant to `PasswordSource` enum
  - 1Password option in password source dropdown (index 4)
  - Password save/load support for 1Password backend
  - Default selection based on `preferred_backend` setting
- **Credential Rename on Connection Rename** - Credentials are now automatically renamed in secret backends when connection is renamed:
  - KeePass: Entry path updated to match new connection name
  - Keyring: Entry key updated from old to new name format
  - Bitwarden: Entry name updated to match new connection name
  - 1Password: Uses connection ID, no rename needed

### Changed
- **Safe State Access** - New helpers to reduce RefCell borrow panics:
  - `with_state()` and `try_with_state()` for read access
  - `with_state_mut()` and `try_with_state_mut()` for write access
- **Toast Queue** - Fixed toast message sequencing with `schedule_toast_hide()` helper

### Fixed
- **KeePass Password Retrieval for Subgroups** - Fixed password not being retrieved when connection is in nested groups:
  - Save and read operations now both use hierarchical paths via `KeePassHierarchy::build_entry_path()`
  - Paths like `RustConn/Group1/Group2/ConnectionName (protocol)` are now consistent
- **Keyring Password Retrieval** - Fixed password never found after saving:
  - Save used `"{name} ({protocol})"` format, read used UUID
  - Now both use `"{name} ({protocol})"` with legacy UUID fallback
- **Bitwarden Password Retrieval** - Fixed password never found after saving:
  - Save used `"{name} ({protocol})"` format, read used `"rustconn/{name}"`
  - Now both use `"{name} ({protocol})"` with legacy format fallback
- **Status Icon on Tab Close** - Status icons now clear when closing RDP/SSH tabs:
  - Previously showed red/green status for closed connections
  - Now clears status (empty string) instead of setting "failed"/"disconnected"

### Tests
- Added 370+ new property tests (total: 1241 tests):
  - `vnc_client_tests.rs` - VNC client configuration and events (28 tests)
  - `terminal_theme_tests.rs` - Terminal theme parsing (26 tests)
  - `error_tests.rs` - Error type coverage (45 tests)
  - `retry_tests.rs` - Retry logic (14 tests)
  - `session_restore_tests.rs` - Session persistence (10 tests)
  - `rdp_backend_tests.rs` - RDP backend selection (13 tests)
  - `log_sanitization_tests.rs` - Log sanitization (19 tests)
  - `health_check_tests.rs` - Health monitoring (13 tests)
  - `bulk_credential_tests.rs` - Bulk operations (25 tests)
  - `import_statistics_tests.rs` - Import statistics (28 tests)
  - And more...

### Fixed
- **Local Shell in Split View** - Local Shell tabs can now be added to split view panels:
  - Fixed protocol filter that excluded "local" protocol from available sessions
  - Multiple Local Shell tabs now appear in "Select Tab" dialog for split panels

## [0.6.8] - 2026-01-30

### Added
- **1Password CLI Integration** - New secret backend for 1Password password manager:
  - Full `SecretBackend` trait implementation with async credential resolution
  - Uses `op` CLI v2 with desktop app integration (biometric authentication)
  - Service account support via `OP_SERVICE_ACCOUNT_TOKEN` environment variable
  - Automatic vault creation ("RustConn" vault) for storing credentials
  - Items tagged with "rustconn" for easy filtering
  - Account status checking with `op whoami`
  - Settings UI with version display and sign-in status indicator
  - "Sign In" button opens terminal for interactive `op signin`
- **1Password Detection** - `detect_onepassword()` function in detection module:
  - Checks multiple paths for `op` CLI installation
  - Reports version, sign-in status, and account email
  - Integrated into `detect_password_managers()` for unified discovery
- **Bitwarden API Key Authentication** - New `login_with_api_key()` function:
  - Uses `BW_CLIENTID` and `BW_CLIENTSECRET` environment variables
  - Recommended for automated workflows and CI/CD pipelines
- **Bitwarden Self-Hosted Support** - New `configure_server()` function:
  - Configure CLI to use self-hosted Bitwarden server
- **Bitwarden Logout** - New `logout()` function for session cleanup

### Changed
- `SecretBackendType` enum extended with `OnePassword` variant
- Connection dialog password source dropdown now includes 1Password (index 4)
- Settings → Secrets tab shows 1Password configuration group when selected
- Property test generators updated to include `Bitwarden` and `OnePassword` variants
- **Bitwarden unlock** now uses `--passwordenv` option as recommended by official documentation (more secure than stdin)
- **Bitwarden retrieve** now syncs vault before lookup to ensure latest credentials
- **Dependencies** - Updated: cc 1.2.54→1.2.55, find-msvc-tools 0.1.8→0.1.9

## [Unreleased] - 0.6.7

### Added
- **Group-Level Secret Storage** - Groups can now store passwords in secret backends:
  - Auto-select password backend based on application settings when creating groups
  - "Load from vault" button to retrieve group passwords from KeePass/Keyring/Bitwarden
  - Hierarchical storage in KeePass: `RustConn/Groups/{path}` mirrors group structure
  - New `build_group_entry_path()` and `build_group_lookup_key()` functions in hierarchy module
- **CLI Secret Management** - New `secret` command for managing credentials from command line:
  - `rustconn-cli secret status` - Show available backends and their status
  - `rustconn-cli secret get <connection>` - Retrieve credentials for a connection
  - `rustconn-cli secret set <connection>` - Store credentials (interactive password prompt)
  - `rustconn-cli secret delete <connection>` - Delete credentials from backend
  - `rustconn-cli secret verify-keepass` - Verify KeePass database credentials
  - Supports `--backend` flag to specify keyring, keepass, or bitwarden

### Changed
- **Dependencies** - Updated: clap 4.5.55→4.5.56, clap_builder 4.5.55→4.5.56, zerocopy 0.8.35→0.8.36, zerocopy-derive 0.8.35→0.8.36, zune-jpeg 0.5.11→0.5.12
- **MSRV** - Synchronized `.clippy.toml` MSRV from 1.87 to 1.88 to match `Cargo.toml`

### Fixed

## [0.6.7] - 2026-01-29

### Added
- **Group-Level Secret Storage** — groups can now store passwords in secret backends (KeePassXC, libsecret, Bitwarden, 1Password, Passbolt)
- **CLI Secret Management** — new `secret` command for managing credentials from the command line
- **Hierarchical KeePass Storage** — KeePass storage mirrors group structure for organized credential management

## [0.6.6] - 2026-01-27

### Added
- **KeePass Password Saving for RDP/VNC** - Fixed password saving when creating/editing connections with KeePass password source:
  - Connection dialog now returns password separately from connection object
  - Password is saved to KeePass database when password source is set to KeePass
  - Works for new connections, edited connections, and template-based connections
- **Load Password from Vault** - New button in connection dialog to load password from KeePass or Keyring:
  - Click the folder icon next to the Value field to load password from configured vault
  - Works with KeePass (KDBX) and system Keyring (libsecret) backends
  - Automatically uses connection name and protocol for lookup key
  - Shows loading indicator during retrieval
- **Keyring Password Storage** - Passwords are now saved to system Keyring when password source is set to Keyring:
  - Uses libsecret via `secret-tool` CLI for GNOME Keyring / KDE Wallet integration
  - Passwords stored with connection name and protocol as lookup key
  - Requires `libsecret-tools` package to be installed
- **SSH X11 Forwarding & Compression** - New SSH session options:
  - X11 Forwarding (`-X` flag) for running graphical applications on remote hosts
  - Compression (`-C` flag) for faster transfer over slow connections
  - GUI controls in Connection dialog → SSH → Session group
  - CLI support via `rustconn-cli connect` (reads from connection config)
  - Import support: Asbru-CM (`-X`, `-C`, `-A` flags), SSH config (`ForwardX11`, `Compression`), Remmina (`ssh_tunnel_x11`, `ssh_compression`)
- **Import Normalizer** - New `ImportNormalizer` module for post-import consistency:
  - Group deduplication (merges groups with same name and parent)
  - Port normalization to protocol defaults
  - Auth method normalization based on key_path presence
  - Key path validation and tilde expansion
  - Import source/timestamp tags for tracking
  - Helper functions: `parse_host_port()`, `is_valid_hostname()`, `looks_like_hostname()`
- **IronRDP Enhanced Features** - Major expansion of embedded RDP client capabilities:
  - **Reconnection support** (`reconnect.rs`): `ReconnectPolicy` with exponential backoff and jitter, `ReconnectState` tracking, `DisconnectReason` classification, `ConnectionQuality` monitoring (RTT, FPS, bandwidth)
  - **Multi-monitor preparation** (`multimonitor.rs`): `MonitorDefinition` with position/DPI, `MonitorLayout` configuration, `MonitorArrangement` modes (Extend/Duplicate/PrimaryOnly), `detect_monitors()` helper
  - **RD Gateway support** (`gateway.rs`): `GatewayConfig` with hostname/auth/bypass, `GatewayAuthMethod` (NTLM/Kerberos/SmartCard/Basic/Cookie), automatic local address bypass
  - **Graphics modes** (`graphics.rs`): `GraphicsMode` selection (Auto/Legacy/RemoteFX/GFX/H264), `ServerGraphicsCapabilities` detection, `GraphicsQuality` presets, `FrameStatistics` for performance monitoring
  - **Extended RdpClientConfig**: gateway, monitor_layout, reconnect_policy, graphics_mode, graphics_quality, remote_app (RemoteApp), printer/smartcard/microphone redirection flags, `validate()` method

### Changed
- **RDP Performance Mode** - Performance mode setting now controls bitmap compression and codec selection:
  - **Quality (RemoteFX)**: Lossless compression with RemoteFX codec for best visual quality
  - **Balanced (Adaptive)**: Lossy compression with RemoteFX codec for adaptive quality/bandwidth tradeoff
  - **Speed (Legacy)**: Lossy compression with legacy bitmap codec for slow connections
  - All modes use 32-bit color depth for AWS EC2 Windows server compatibility
- **Remmina Importer** - Major refactor for proper group support:
  - Changed from tags (`remmina:{group}`) to real `ConnectionGroup` objects
  - Added nested group support (e.g., "Production/Web Servers" creates hierarchy)
  - Added SPICE protocol support
- **RDM Importer** - Added SSH key support:
  - Parses `PrivateKeyPath` field from RDM JSON
  - Sets `auth_method` to `PublicKey` when key present
  - Added `view_only` support for VNC connections
- **Royal TS Importer** - Added SSH key support:
  - Parses `PrivateKeyFile`, `KeyFilePath`, `PrivateKeyPath` fields
  - Sets `auth_method` based on key presence
  - Tilde expansion for key paths
- **SSH Config Importer** - Enhanced option parsing:
  - Now preserves `ServerAliveInterval`, `ServerAliveCountMax`, `TCPKeepAlive`
  - Preserves `Compression`, `ConnectTimeout`, `ConnectionAttempts`
  - Preserves `StrictHostKeyChecking`, `UserKnownHostsFile`, `LogLevel`
- **Dependencies** - Updated: aws-lc-rs 1.15.3→1.15.4, aws-lc-sys 0.36.0→0.37.0, cc 1.2.53→1.2.54, cfg-expr 0.20.5→0.20.6, hybrid-array 0.4.5→0.4.6, libm 0.2.15→0.2.16, moka 0.12.12→0.12.13, notify-types 2.0.0→2.1.0, num-conv 0.1.0→0.2.0, proc-macro2 1.0.105→1.0.106, quote 1.0.43→1.0.44, siphasher 1.0.1→1.0.2, socket2 0.6.1→0.6.2, time 0.3.45→0.3.46, time-core 0.1.7→0.1.8, time-macros 0.2.25→0.2.26, uuid 1.19.0→1.20.0, yuv 0.8.9→0.8.10, zerocopy 0.8.33→0.8.34, zmij 1.0.16→1.0.17

### Fixed
- **AWS EC2 RDP Compatibility** - Fixed IronRDP connection failures with AWS EC2 Windows servers by using 32-bit color depth in `BitmapConfig` (24-bit caused connection reset during `BasicSettingsExchange` phase)
- **GCloud Provider Detection** - Fixed GCloud commands being incorrectly detected as AWS when instance names contain patterns resembling EC2 instance IDs (e.g., `ai-0000a00a`). GCloud patterns are now checked before AWS instance ID patterns

### Refactored
- **Display Server Detection** - Consolidated duplicate display server detection code from `embedded.rs` and `wayland_surface.rs` into a unified `display.rs` module with cached detection and comprehensive capability methods
- **Sidebar Filter Buttons** - Reduced code duplication in sidebar filter button creation and event handling with `create_filter_button()` and `connect_filter_button()` helper functions
- **Window UI Components** - Extracted header bar and application menu creation from `window.rs` into dedicated `window_ui.rs` module

## [0.6.5] - 2026-01-21

### Changed
- **Split View Redesign** - Complete rewrite of split view functionality with tab-scoped layouts:
  - Each tab now maintains its own independent split layout (no more global split state)
  - Tree-based panel structure supporting unlimited nested splits
  - Color-coded panel borders (6 colors) to visually identify split containers
  - All panels within the same split container now share the same border color (per design spec)
  - Tab color indicators match their container's color when in split view
  - "Select Tab" button in empty panels as alternative to drag-and-drop
  - Proper cleanup when closing split view (colors released, terminals reparented)
  - When last panel is closed, split view closes and session returns to regular tab
  - New `rustconn-core/src/split/` module with GUI-free split layout logic
  - Comprehensive property tests for split view operations
- **Terminal Tabs Migration** - Migrated terminal notebook from `gtk::Notebook` to `adw::TabView`:
  - Modern GNOME HIG compliant tab bar with `adw::TabBar`
  - Native tab drag-and-drop support
  - Automatic tab overflow handling
  - Better integration with libadwaita theming
  - Improved accessibility with proper ARIA labels
- **Dependencies** - Updated: thiserror 2.0.18, zbus 5.13.2, zvariant 5.9.2, euclid 0.22.13, openssl-probe 0.2.1, zmij 1.0.16, zune-jpeg 0.5.11

### Fixed
- **KeePass Password Saving** - Fixed "Failed to Save Password" error when connection name contains `/` character (e.g., connections in subgroups). Now sanitizes lookup keys by replacing `/` with `-`
- **Connection Dialog Password Field** - Renamed "Password:" label to "Value:" and added show/hide toggle button. Field visibility now depends on password source selection (hidden for Prompt/Inherit/None, shown for Stored/KeePass/Keyring)
- **Group Dialog Password Source** - Added password source dropdown (Prompt, Stored, KeePass, Keyring, Inherit, None) with Value field and show/hide toggle to group dialogs
- **Template Dialog Field Alignment** - Changed Basic tab fields from `Entry` to `adw::EntryRow` for proper width stretching consistent with Connection dialog
- **CSS Parser Errors** - Removed unsupported `:has()` pseudoclass from CSS rules, eliminating 6 "Unknown pseudoclass" errors on startup
- **zbus DEBUG Spam** - Added tracing filter to suppress verbose zbus DEBUG messages (`zbus=warn` directive)
- **Split View "Loading..." Panels** - Fixed panels getting stuck showing "Loading..." after multiple splits and "Select Tab" operations:
  - Terminals moved via "Select Tab" are now stored in bridge's internal map for restoration
  - `restore_panel_contents()` is now called after each split to restore terminal content
  - `show_session()` is only called on first split; subsequent splits preserve existing panel content
- **Split View Context Menu Freeze** - Fixed window freeze when right-clicking in split view panels. Context menu popover is now created dynamically on each click to avoid GTK popup grabbing conflicts
- **Split View Tab Colors** - Fixed tabs in the same split container having different colors. Now all tabs/panels within a split container share a single container color (allocated once on first split)
- Empty panel close button now properly triggers panel removal and split view cleanup
- Focus rectangle properly follows active panel when clicking or switching tabs

## [0.6.4] - 2026-01-17

### Added
- **Snap Package** - New distribution format for easy installation via Snapcraft:
  - Classic confinement for full system access (SSH keys, network, etc.)
  - Automatic updates via Snap Store
  - Available via `sudo snap install rustconn --classic`
- **GitHub Actions Snap Workflow** - Automated Snap package builds:
  - Builds on tag push (`v*`) and manual trigger
  - Uploads artifacts for testing
  - Publishes to Snap Store stable channel on release tags
- **RDP/VNC Performance Modes** - New dropdown in connection dialog to optimize for different network conditions:
  - Quality: Best visual quality (32-bit color for RDP, Tight encoding with high quality for VNC)
  - Balanced: Good balance of quality and performance (24-bit color, medium compression)
  - Speed: Optimized for slow connections (16-bit color for RDP, ZRLE encoding with high compression for VNC)

### Changed
- Updated documentation with Snap installation instructions

### Fixed
- **RDP Initial Resolution** - Embedded RDP sessions now start with correct resolution matching actual widget size
  - Previously used saved window settings which could differ from actual content area
  - Now waits for GTK layout (100ms) to get accurate widget dimensions
- **RDP Dynamic Resolution** - Window resize now triggers automatic reconnect with new resolution
  - Debounced reconnect after 500ms of no resize activity
  - Preserves shared folders and credentials during reconnect
  - Works around Windows RDP servers not supporting Display Control channel
- **Sidebar Fixed Width** - Sidebar no longer resizes when window is resized
  - Content area (RDP/VNC/terminal) now properly expands to fill available space
- **RDP Cursor Colors** - Fixed inverted cursor colors in embedded RDP sessions (BGRA→ARGB conversion)

### Updated Dependencies
- `ironrdp` 0.13 → 0.14 (embedded RDP client)
- `ironrdp-tokio` 0.7 → 0.8
- `ironrdp-tls` 0.1 → 0.2
- `sspi` 0.16 → 0.18.7 (Windows authentication)
- `picky` 7.0.0-rc.17 → 7.0.0-rc.20
- `picky-krb` 0.11 → 0.12 (Kerberos support)
- `hickory-proto` 0.24 → 0.25
- `hickory-resolver` 0.24 → 0.25
- `cc` 1.2.52 → 1.2.53
- `find-msvc-tools` 0.1.7 → 0.1.8
- `js-sys` 0.3.83 → 0.3.85
- `rand_core` 0.9.3 → 0.9.5
- `rustls-pki-types` 1.13.2 → 1.14.0
- `rustls-webpki` 0.103.8 → 0.103.9
- `wasm-bindgen` 0.2.106 → 0.2.108
- `web-sys` 0.3.83 → 0.3.85
- `wit-bindgen` 0.46.0 → 0.51.0

## [0.6.3] - 2026-01-16

### Added
- **Bitwarden CLI Integration** - New secret backend for Bitwarden password manager:
  - Full `SecretBackend` trait implementation with async credential resolution
  - Vault status checking (locked/unlocked/unauthenticated)
  - Session token management with automatic refresh
  - Secure credential lookup by connection name or host
  - Settings UI with vault status indicator and unlock functionality
  - Master password persistence with encrypted storage (machine-specific)
- **Password Manager Detection** - Automatic detection of installed password managers:
  - Detects GNOME Secrets, KeePassXC, KeePass2, Bitwarden CLI, 1Password CLI
  - Shows installed managers with version info in Settings → Secrets tab
  - New "Installed Password Managers" section for quick overview
- **Enhanced Secrets Settings UI** - Improved backend selection experience:
  - Backend dropdown now includes all 4 options: KeePassXC, libsecret, KDBX File, Bitwarden
  - Dynamic configuration groups based on selected backend
  - Bitwarden-specific settings with vault status checking
- **Universal Password Vault Button** - Sidebar button now opens appropriate password manager:
  - Opens KeePassXC/GNOME Secrets for KeePassXC backend
  - Opens Seahorse/GNOME Settings for libsecret backend
  - Opens Bitwarden web vault for Bitwarden backend

### Changed
- `SecretBackendType` enum extended with `Bitwarden` variant
- `SecretError` extended with `Bitwarden` variant for CLI-specific errors
- Renamed "Save to KeePass" / "Load from KeePass" buttons to universal "Save password to vault" / "Load password from vault"
- Renamed sidebar "Open KeePass Database" button to "Open Password Vault"
- Improved split view button icons for better intuitiveness:
  - Split Vertical now uses `object-flip-horizontal-symbolic`
  - Split Horizontal now uses `object-flip-vertical-symbolic`

### Updated Dependencies
- `aws-lc-rs` 1.15.2 → 1.15.3
- `aws-lc-sys` 0.35.0 → 0.36.0
- `chrono` 0.4.42 → 0.4.43
- `clap_lex` 0.7.6 → 0.7.7
- `time` 0.3.44 → 0.3.45
- `tower` 0.5.2 → 0.5.3
- `zune-jpeg` 0.5.8 → 0.5.9

## [Unreleased] - 0.6.2

### Added
- **MobaXterm Import/Export** - Full support for MobaXterm `.mxtsessions` files:
  - Import SSH, RDP, VNC sessions with all settings (auth, resolution, color depth, etc.)
  - Export connections to MobaXterm format with folder hierarchy
  - Preserves group structure as MobaXterm bookmarks folders
  - Handles MobaXterm escape sequences and Windows-1252 encoding
  - CLI support: `rustconn-cli import/export --format moba-xterm`
- **Connection History Button** - Quick access to connection history from sidebar toolbar
- **Run Snippet from Context Menu** - Right-click on connection → "Run Snippet..." to execute snippets
  - Automatically connects if not already connected, then shows snippet picker
- **Persistent Search History** - Search queries are now saved across sessions
  - Up to 20 recent searches preserved in settings
  - History restored on application startup

### Changed
- Welcome screen: Removed "Import/Export connections" from Features column (redundant with Import Formats)
- Welcome screen: Combined "Asbru-CM / Royal TS / MobaXterm" into single row in Import Formats
- Documentation: Removed hardcoded version numbers from INSTALL.md package commands (use wildcards)

### Fixed
- **KeePass Alert Dialog Focus** - "Password Saved" alert now appears in front of the connection dialog
  - Previously the alert appeared behind the New/Edit Connection dialog
  - Fixed by passing the dialog window as parent instead of main window

- Updated `quick-xml` 0.38 → 0.39
- Updated `resvg` 0.45 → 0.46
- Updated `usvg` 0.45 → 0.46
- Updated `svgtypes` 0.15 → 0.16
- Updated `roxmltree` 0.20 → 0.21
- Updated `kurbo` 0.11 → 0.13
- Updated `gif` 0.13 → 0.14
- Updated `imagesize` 0.13 → 0.14
- Updated `zune-jpeg` 0.4 → 0.5
## [0.6.2] - 2026-01-15

### Added
- **MobaXterm Import/Export** — full support for `.mxtsessions` files
- **Connection History Button** — quick access from sidebar toolbar
- **Run Snippet from Context Menu** — right-click on connection → "Run Snippet..."
- **Persistent Search History** — up to 20 recent searches saved across sessions

- Updated `quick-xml` 0.38 → 0.39, `resvg` 0.45 → 0.46
## [0.6.1] - 2026-01-12

### Added
- **Credential Inheritance** - Simplify connection management by inheriting credentials from parent groups:
  - New "Inherit" option in password source dropdown
  - Recursively resolves credentials up the group hierarchy
  - Reduces duplication for environments sharing same credentials
- **Jump Host Support** - Native SSH Jump Host configuration:
  - New "Jump Host" dropdown in SSH connection settings
  - Select any existing SSH connection as a jump host
  - Supports chained jump hosts (Jump Host -> Jump Host -> Target)
  - Automatically configures `-J` argument for SSH connections
- **Adwaita Empty States** - Migrated empty state views to `adw::StatusPage`:
  - Modern, consistent look for empty connection lists, terminals, and search results
  - Proper theming support
- **Group Improvements**:
  - **Sorting**: Group lists in sidebar and dropdowns are now sorted alphabetically by full path
  - **Credentials UI**: New fields in Group Dialogs to set default Username/Password/Domain
  - **Move Group**: Added "Parent" dropdown to Edit Group dialog to move groups (with cycle prevention)

- Updated `libadwaita` to `0.7`
- Updated `gtk4` to `0.10`
- Updated `vte4` to `0.9`
## [0.6.0] - 2026-01-12

### Added
- **Pre-connect Port Check** - Fast TCP port reachability check before launching RDP/VNC/SPICE connections:
  - Provides faster feedback (2-3s vs 30-60s timeout) when hosts are unreachable
  - Configurable globally in Settings → Connection with timeout setting (default: 3s)
  - Per-connection "Skip port check" option for special cases (firewalls, port knocking, VPN)
  - New `ConnectionSettings` struct in `AppSettings` for connection-related settings
  - New `skip_port_check` field on `Connection` model
- **CLI Feature Parity** - CLI now supports all major GUI features:
  - `template list/show/create/delete/apply` - Connection template management
  - `cluster list/show/create/delete/add-connection/remove-connection` - Cluster management
  - `var list/show/set/delete` - Global variables management
  - `duplicate` - Duplicate existing connections
  - `stats` - Show connection statistics (counts by protocol, groups, templates, clusters, snippets, variables, usage)
- **GitHub CI RPM Build** - Added Fedora RPM package build to release workflow:
  - Builds in Fedora 41 container with Rust 1.87
  - RPM package included in GitHub releases alongside .deb and AppImage
  - Installation instructions for Fedora in release notes
- Added `load_variables()` and `save_variables()` methods to `ConfigManager` for global variables persistence
- Added `<icon>` element to metainfo.xml for explicit AppStream icon declaration
- Added `<developer_name>` tag to metainfo.xml for backward compatibility with older AppStream parsers
- Added `author` and `license` fields to AppImage packaging (AppImageBuilder.yml)
- Added `debian.copyright` file to OBS debian packaging

### Changed
- **Code Audit & Cleanup Release** - comprehensive codebase audit and modernization
- Removed `check_structs.rs` development artifact containing unsafe code (violated `unsafe_code = "forbid"` policy)
- Replaced `blocking_send()` with `try_send()` in VNC input handlers to prevent UI freezes
- Replaced `unwrap()` with safe alternatives in `sidebar.rs` iterator access
- Replaced `expect()` with proper error handling in `validation.rs` regex compilation
- Replaced module-level `#![allow(clippy::unwrap_used)]` with targeted function-level annotations in `embedded_rdp_thread.rs`
- Improved `app.rs` initialization to return proper error instead of panicking
- Updated `Cargo.toml` license from MIT to GPL-3.0-or-later (matches actual LICENSE file)
- Updated `Cargo.toml` authors to "Anton Isaiev <totoshko88@gmail.com>"

### Fixed
- Fixed `remote-viewer` version detection for localized output (e.g., Ukrainian "версія" instead of "version")
- Fixed Asbru-CM import skipping RDP/VNC connections with client info (e.g., "rdp (rdesktop)", "rdp (xfreerdp)", "vnc (vncviewer)")
- VNC keyboard/mouse input no longer blocks GTK main thread on channel send
- Sidebar protocol filter no longer panics on empty filter set
- Regex validation errors now return `Result` instead of panicking
- FreeRDP thread mutex operations now have documented safety invariants
- Package metadata now correctly shows author and license in all package formats

- Updated `base64ct` 1.8.2 → 1.8.3
- Updated `cc` 1.2.51 → 1.2.52
- Updated `data-encoding` 2.9.0 → 2.10.0
- Updated `find-msvc-tools` 0.1.6 → 0.1.7
- Updated `flate2` 1.1.5 → 1.1.8
- Updated `getrandom` 0.2.16 → 0.2.17
- Updated `libc` 0.2.179 → 0.2.180
- Updated `toml` 0.9.10 → 0.9.11
- Updated `zbus` 5.12.0 → 5.13.1
- Updated `zbus_macros` 5.12.0 → 5.13.1
- Updated `zbus_names` 4.2.0 → 4.3.1
- Updated `zmij` 1.0.12 → 1.0.13
- Updated `zvariant` 5.8.0 → 5.9.1
- Updated `zvariant_derive` 5.8.0 → 5.9.1
- Updated `zvariant_utils` 3.2.1 → 3.3.0
- Removed unused `cfg_aliases`, `nix`, `static_assertions` dependencies
- Note: `sspi` and `picky-krb` kept at 0.16.0/0.11.0 due to `rand_core` version conflict
### Removed
- `rustconn-core/src/check_structs.rs` - development artifact with unsafe code

## [0.5.9] - 2026-01-10

### Changed
- Migrated Settings dialog from deprecated `PreferencesWindow` to `PreferencesDialog` (libadwaita 1.5+)
- Updated libadwaita feature from `v1_4` to `v1_5` for PreferencesDialog support
- Updated workspace dependencies:
  - `uuid` 1.6 → 1.11
  - `regex` 1.10 → 1.11
  - `proptest` 1.4 → 1.6
  - `tempfile` 3.24 → 3.15
  - `zip` 2.1 → 2.2
- Removed unnecessary `macos_kqueue` feature from `notify` crate
- Note: `ksni` 0.3.3 and `sspi`/`picky-krb` kept at current versions due to `zvariant`/`rand_core` version conflicts
- Migrated all dialogs to use `adw::ToolbarView` for proper libadwaita layout:
- Migrated Template dialog to modern libadwaita patterns:
  - Basic tab: `adw::PreferencesGroup` with `adw::ActionRow` for template info and default values
  - SSH options: `adw::PreferencesGroup` with Authentication, Connection, and Session groups
  - RDP options: Display, Features, and Advanced groups with dynamic visibility (resolution/color hidden in Embedded mode)
  - VNC options: Display, Encoding, Features, and Advanced groups
  - SPICE options: Security, Features, and Performance groups with dynamic visibility (TLS-related fields)
  - Zero Trust options: Provider selection with `adw::ActionRow`, provider-specific groups for all 10 providers

### Fixed
- Fixed missing icon for "Embedded SSH terminals" feature on Welcome page (`display-symbolic` → `utilities-terminal-symbolic`)
- Fixed missing Quick Connect header bar icon (`network-transmit-symbolic` → `go-jump-symbolic`)
- Fixed missing Split Horizontal header bar icon (`view-paged-symbolic` → `object-flip-horizontal-symbolic`)
- Fixed missing Interface tab icon in Settings (`preferences-desktop-appearance-symbolic` → `applications-graphics-symbolic`)
- Fixed KeePass Settings: Browse buttons for Database File and Key File now open file chooser dialogs
- Fixed KeePass Settings: Dynamic visibility for Authentication fields (password/key file rows show/hide based on switches)
- Fixed KeePass Settings: Added "Check" button to verify database connection
- Fixed KeePass Settings: `verify_kdbx_credentials` now correctly handles key-file-only authentication with `--no-password` flag
- Fixed SSH Agent Settings: "Start Agent" button now properly starts ssh-agent and updates UI
- Fixed Zero Trust (AWS SSM) connection status icon showing as failed despite successful connection

### Improved
- Migrated About dialog from `gtk4::AboutDialog` to `adw::AboutDialog` for modern GNOME look
- Migrated Password Generator dialog switches from `ActionRow` + `Switch` to `adw::SwitchRow` for cleaner code
- Migrated Cluster dialog broadcast switch from `ActionRow` + `Switch` to `adw::SwitchRow`
- Migrated Export dialog switches from `ActionRow` + `Switch` to `adw::SwitchRow`
- Enhanced About dialog with custom links and credits:
  - Added short description under logo
  - Added Releases, Details, and License links
  - Added "Made with ❤️ in Ukraine 🇺🇦" to Acknowledgments
  - Added legal sections for key dependencies (GTK4, IronRDP, VTE)
- Migrated group dialogs from `ActionRow` + `Entry` to `adw::EntryRow`:
  - New Group dialog
  - Edit Group dialog
  - Rename dialog (connections and groups)
- Migrated Settings UI tab from `SpinButton` to `adw::SpinRow` for session max age
- Added `alert.rs` helper module for modern `adw::AlertDialog` API
- Migrated all `gtk4::AlertDialog` usages to `adw::AlertDialog` via helper module (50+ usages across 12 files)
- Updated documentation (INSTALL.md, USER_GUIDE.md) for version 0.5.9
  - Connection dialog (`dialogs/connection.rs`)
  - SSH Agent passphrase dialog (`dialogs/settings/ssh_agent_tab.rs`)
- Enabled libadwaita `v1_4` feature for `adw::ToolbarView` support
- Replaced hardcoded CSS colors with Adwaita semantic colors:
  - Status indicators now use `@success_color`, `@warning_color`, `@error_color`
  - Toast notifications use semantic colors for success/warning states
  - Form validation styles use semantic colors
- Reduced global clippy suppressions in `main.rs` from 30+ to 5 essential ones
- Replaced `unwrap()` calls in Cairo drawing code with proper error handling (`if let Ok(...)`)

### Fixed
- Cairo text rendering in embedded RDP/VNC widgets no longer panics on font errors

## [0.5.8] - 2026-01-07

### Changed
- Migrated Connection Dialog tabs to libadwaita components (GNOME HIG compliance):
  - Display tab: `adw::PreferencesGroup` + `adw::ActionRow` for window mode settings
  - Logging tab: `adw::PreferencesGroup` + `adw::ActionRow` for session logging configuration
  - WOL tab: `adw::PreferencesGroup` + `adw::ActionRow` for Wake-on-LAN settings
  - Variables tab: `adw::PreferencesGroup` for local variable management
  - Automation tab: `adw::PreferencesGroup` for expect rules configuration
  - Tasks tab: `adw::PreferencesGroup` for pre/post connection tasks
  - Custom Properties tab: `adw::PreferencesGroup` for metadata fields
- All migrated tabs now use `adw::Clamp` for proper content width limiting
- Removed deprecated `gtk4::Frame` usage in favor of `adw::PreferencesGroup`
- Settings dialog now loads asynchronously for faster startup:
  - Clients tab: CLI detection runs in background with spinner placeholders
  - SSH Agent tab: Agent status and key lists load asynchronously
  - Available SSH keys scan runs in background
- Cursor Shape/Blink toggle buttons in Terminal settings now have uniform width (240px)
- KeePassXC debug output now uses `tracing::debug!` instead of `eprintln!`
- KeePass entry path format changed to `RustConn/{name} ({protocol})` to support same name for different protocols
- Updated dependencies: indexmap 2.12.1→2.13.0, syn 2.0.113→2.0.114, zerocopy 0.8.32→0.8.33, zmij 1.0.10→1.0.12
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility issues

### Fixed
- SSH Agent "Add Key" button now opens file chooser to select any SSH key file
- SSH Agent "+" buttons in Available Key Files list now load keys with passphrase dialog
- SSH Agent "Remove Key" (trash) button now actually removes keys from the agent
- SSH Agent Refresh button updates both loaded keys and available keys lists
- VNC password dialog now correctly loads password from KeePass using consistent lookup key (name or host)
- KeePass passwords for connections with same name but different protocols no longer overwrite each other
- Welcome tab now displays correctly when switching back from connections (fallback to first pane if none focused)

## [0.5.7] - 2026-01-07

### Changed
- Updated dependencies: h2 0.4.12→0.4.13, proc-macro2 1.0.104→1.0.105, quote 1.0.42→1.0.43, rsa 0.9.9→0.9.10, rustls 0.23.35→0.23.36, serde_json 1.0.148→1.0.149, url 2.5.7→2.5.8, zerocopy 0.8.31→0.8.32
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility issues

### Fixed
- Test button in New Connection dialog now works correctly (fixed async runtime issue with GTK)

## [0.5.6] - 2026-01-07

### Added
- Enhanced terminal settings with color themes, cursor options, and behavior controls
- Six built-in terminal color themes: Dark, Light, Solarized Dark/Light, Monokai, Dracula
- Cursor shape options (Block, IBeam, Underline) and blink modes (On, Off, System)
- Terminal behavior settings: scroll on output/keystroke, hyperlinks, mouse autohide, audible bell
- Scrollable terminal settings dialog with organized sections
- Security Tips section in Password Generator dialog with 5 best practice recommendations
- Quick Filter functionality in sidebar for protocol filtering (SSH, RDP, VNC, SPICE, ZeroTrust)
- Protocol filter buttons with icons and visual feedback (highlighted when active)
- CSS styling for Quick Filter buttons with hover and active states
- Enhanced Quick Filter with proper OR logic for multiple protocol selection
- Visual feedback for multiple active filters with special styling (`filter-active-multiple` CSS class)
- API methods for accessing active protocol filters (`get_active_protocol_filters`, `has_active_protocol_filters`, `active_protocol_filter_count`)
- Fullscreen mode toggle with F11 keyboard shortcut
- KeePass status button in sidebar toolbar with visual integration status indicator

### Changed
- Migrated to native libadwaita architecture:
  - Application now uses `adw::Application` and `adw::ApplicationWindow` for proper theme integration
  - All dialogs redesigned to use `adw::Window` with `adw::HeaderBar` following GNOME HIG
  - Proper dark/light theme support via libadwaita StyleManager
- Unified dialog widths: Rename and Edit Group dialogs now use 750px width (matching Move dialog)
- Updated USER_GUIDE.md with complete documentation for all v0.5.5+ features
- Updated dependencies: tokio 1.48→1.49, notify 7.0→8.2, thiserror 2.0→2.0.17, clap 4.5→4.5.23, quick-xml 0.37→0.38
- Settings dialog UI refactored for lighter appearance:
  - Removed Frame widgets from all tabs (SSH Agent, Terminal, Logging, Secrets, UI, Clients)
  - Replaced with section headers using Label with `heading` CSS class
  - Removed `boxed-list` CSS class from ListBox widgets
  - Removed nested ScrolledWindow wrappers
- Theme switching now uses libadwaita StyleManager instead of GTK Settings
- Clients tab version parsing improved for all Zero Trust CLIs:
  - OCI CLI: parses "3.71.4" format
  - Tailscale: parses "1.92.3" format
  - SPICE remote-viewer: parses "remote-viewer, версія 11.0" format

### Fixed
- Terminal settings now properly apply to all terminal sessions:
  - SSH connections use user-configured terminal settings
  - Zero Trust connections use user-configured terminal settings
  - Quick Connect SSH sessions use user-configured terminal settings
  - Local Shell uses user-configured terminal settings
  - Saving settings in Settings dialog immediately applies to all existing terminals
- Clients tab CLI version parsing:
  - AWS CLI: parses "aws-cli/2.32.28 ..." format
  - GCP CLI: parses "Google Cloud SDK 550.0.0" format
  - Azure CLI: parses "azure-cli 2.81.0" format
  - Cloudflare CLI: parses "cloudflared version 2025.11.1 ..." format
  - Teleport: parses "Teleport v18.6.2 ..." format
  - Boundary: parses "Version Number: 0.21.0" format
- Clients tab now searches ~/bin/, ~/.local/bin/, ~/.cargo/bin/ for CLI tools
- Fixed quick-xml 0.38 API compatibility in Royal TS import (replaced deprecated `unescape()` method)
- Fixed Quick Filter logic to use proper OR logic for multiple protocol selection (connections matching ANY selected protocol are shown)
- Improved Quick Filter visual feedback with enhanced styling for multiple active filters
- Quick Filter now properly handles multiple protocol selection with clear visual indication
- Removed redundant clear filter button from Quick Filter bar (search entry can be cleared manually)
- Fixed Quick Filter button state synchronization - buttons are now properly cleared when search field is manually cleared
- Fixed RefCell borrow conflict panic when toggling protocol filters - resolved recursive update issue

## [0.5.5] - 2026-01-03

### Added
- Kiro steering rules for development workflow:
  - `commit-checklist.md` - pre-commit cargo fmt/clippy checks
  - `release-checklist.md` - version files and packaging verification
- Rename action in sidebar context menu for both connections and groups
- Double-click on import source to start import
- Double-click on template to create connection from it
- Group dropdown in Connection dialog Basic tab for selecting parent group
- Info tab for viewing connection details (like Asbru-CM) - replaces popover with full tab view
- Default alphabetical sorting for connections and groups with drag-drop reordering support

### Changed
- Manage Templates dialog: "Create" button now creates connection from template, "Create Template" button creates new template
- View Details action now opens Info tab instead of popover
- Sidebar now uses sorted rebuild for consistent alphabetical ordering
- All dialogs now follow GNOME HIG button layout: Close/Cancel on left, Action on right
- Removed window close button (X) from all dialogs - use explicit Close/Cancel buttons instead

### Fixed
- Flatpak manifest version references updated correctly
- Connection group_id preserved when editing connections (no longer falls to root)
- Import dialog now returns to source selection when file chooser is cancelled
- Drag-and-drop to groups now works correctly (connections can be dropped into groups)

## [0.5.4] - 2026-01-02

### Changed
- Updated dependencies: cc, iri-string, itoa, libredox, proc-macro2, rustls-native-certs, ryu, serde_json, signal-hook-registry, syn, zeroize_derive
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility issues

### Added
- Close Tab action implementation for terminal notebook
- Session Restore feature with UI settings in Settings dialog:
  - Enable/disable session restore on startup
  - Option to prompt before restoring sessions
  - Configurable maximum session age (hours)
  - Sessions saved on app close, restored on next startup
- `AppState` methods for session restore: `save_active_sessions()`, `get_sessions_to_restore()`, `clear_saved_sessions()`
- `TerminalNotebook.get_all_sessions()` method for collecting active sessions
- Password Generator feature:
  - New `password_generator` module in `rustconn-core` with secure password generation using `ring::rand`
  - Configurable character sets: lowercase, uppercase, digits, special, extended special
  - Option to exclude ambiguous characters (0, O, l, 1, I)
  - Password strength evaluation with entropy calculation
  - Crack time estimation based on entropy
  - Password Generator dialog accessible from Tools menu
  - Real-time strength indicator with level bar
  - Copy to clipboard functionality
- Advanced session logging modes with three configurable options:
  - Activity logging (default) - tracks session activity changes
  - User input logging - captures commands typed by user
  - Terminal output logging - records full terminal transcript
  - Settings UI with checkboxes in Session Logging tab
- Royal TS (.rtsz XML) import support:
  - SSH, RDP, and VNC connection import
  - Folder hierarchy preservation as connection groups
  - Credential reference resolution (username/domain)
  - Trash folder filtering (deleted connections are skipped)
  - Accessible via Import dialog
- Royal TS (.rtsz XML) export support:
  - SSH, RDP, and VNC connection export
  - Folder hierarchy export as Royal TS folders
  - Username and domain export for credentials
  - Accessible via Export dialog
- RDPDR directory change notifications with inotify integration:
  - `dir_watcher` module using `notify` crate for file system monitoring
  - `FileAction` enum matching MS-FSCC `FILE_ACTION_*` constants
  - `CompletionFilter` struct with MS-SMB2 `FILE_NOTIFY_CHANGE_*` flags
  - `DirectoryWatcher` with recursive/non-recursive watch support
  - `build_file_notify_info()` for MS-FSCC 2.4.42 `FILE_NOTIFY_INFORMATION` structures
  - Note: RDP responses pending ironrdp upstream support for `ClientDriveNotifyChangeDirectoryResponse`

### Fixed
- Close Tab keyboard shortcut (Ctrl+W) now properly closes active session tab

## [0.5.3] - 2026-01-02

### Added
- Connection history recording for all protocols (SSH, VNC, SPICE, RDP, ZeroTrust)
- "New Group" button in Group Operations Mode bulk actions bar
- "Reset" buttons in Connection History and Statistics dialogs (header bar)
- "Clear Statistics" functionality in AppState
- Protocol-specific tabs in Template Dialog matching Connection Dialog functionality:
  - SSH: auth method, key source, proxy jump, agent forwarding, startup command, custom options
  - RDP: client mode, resolution, color depth, audio, gateway, custom args
  - VNC: client mode, encoding, compression, quality, view only, scaling, clipboard
  - SPICE: TLS, CA cert, USB, clipboard, image compression
  - ZeroTrust: all 10 providers (AWS SSM, GCP IAP, Azure Bastion/SSH, OCI, Cloudflare, Teleport, Tailscale, Boundary, Generic)
- Connection history dialog (`HistoryDialog`) for viewing and searching session history
- Connection statistics dialog (`StatisticsDialog`) with success rate visualization
- Common embedded widget trait (`EmbeddedWidget`) for RDP/VNC/SPICE deduplication
- `EmbeddedConnectionState` enum for unified connection state handling
- `EmbeddedWidgetState` helper for managing common widget state
- `create_embedded_toolbar()` helper for consistent toolbar creation
- `draw_status_overlay()` helper for status rendering
- Quick Connect dialog now supports connection templates (auto-fills protocol, host, port, username)
- History/Statistics menu items in Tools section
- `AppState` methods for recording connection history (`record_connection_start`, `record_connection_end`, etc.)
- `ConfigManager.load_history()` and `save_history()` for history persistence
- Property tests for history models (`history_tests.rs`):
  - Entry creation, quick connect, end/fail operations
  - Statistics update consistency, success rate bounds
  - Serialization round-trips for all history types
- Property tests for session restore models (`session_restore_tests.rs`):
  - `SavedSession` creation and serialization
  - `SessionRestoreSettings` configuration and serialization
  - Round-trip tests with multiple saved sessions
- Quick Connect now supports RDP and VNC protocols (previously only SSH worked)
- RDP Quick Connect uses embedded IronRDP widget with state callbacks and reconnect support
- VNC Quick Connect uses native VncSessionWidget with full embedded mode support
- Quick Connect password field for RDP and VNC connections
- Connection history model (`ConnectionHistoryEntry`) for tracking session history
- Connection statistics model (`ConnectionStatistics`) with success rate, duration tracking
- History settings (`HistorySettings`) with configurable retention and max entries
- Session restore settings (`SessionRestoreSettings`) for restoring sessions on startup
- `SavedSession` model for persisting session state across restarts

### Changed
- UI Unification: All dialogs now use consistent 750×500px dimensions
- Removed duplicate Close/Cancel buttons from all dialogs (window X button is sufficient)
- Renamed action buttons for consistency:
  - "New X" → "Create" (moved to left side of header bar)
  - "Quick Connect" → "Connect" in Quick Connect dialog
  - "Clear History/Statistics" → "Reset" (moved to header bar with destructive style)
- Create Connection now always opens blank New Connection dialog (removed template picker)
- Templates can be used from Manage Templates dialog
- Button styling: All action buttons (Create, Save, Import, Export) use `suggested-action` CSS class
- When editing existing items, button label changes from "Create" to "Save"
- Extracted common embedded widget patterns to `embedded_trait.rs`
- `show_quick_connect_dialog()` now accepts optional `SharedAppState` for template access
- Refactored `terminal.rs` into modular structure (`rustconn/src/terminal/`):
  - `mod.rs` - Main `TerminalNotebook` implementation
  - `types.rs` - `TabDisplayMode`, `TerminalSession`, `SessionWidgetStorage`, `TabLabelWidgets`
  - `config.rs` - Terminal appearance and behavior configuration
  - `tabs.rs` - Tab creation, display modes, overflow menu management
- `EmbeddedSpiceWidget` now implements `EmbeddedWidget` trait for unified interface
- Updated `gtk4` dependency from 0.10 to 0.10.2
- Improved picky dependency documentation with monitoring notes for future ironrdp compatibility
- `AppSettings` now includes `history` field for connection history configuration
- `UiSettings` now includes `session_restore` field for session restore configuration

### Fixed
- Connection History "Connect" button now actually connects (was only logging)
- History statistics labels (Total/Successful/Failed) now update correctly
- Statistics dialog content no longer cut off (increased size)
- Quick Connect RDP/VNC no longer shows placeholder tabs — actual connections are established

## [0.5.2] - 2025-12-29

### Added
- `wayland-native` feature flag with `gdk4-wayland` integration for improved Wayland detection
- Sidebar integration with lazy loading and virtual scrolling APIs

### Changed
- Improved display server detection using GDK4 Wayland bindings when available
- Refactored `window.rs` into modular structure (reduced from 7283 to 2396 lines, -67%):
  - `window_types.rs` - Type aliases and `get_protocol_string()` utility
  - `window_snippets.rs` - Snippet management methods
  - `window_templates.rs` - Template management methods
  - `window_sessions.rs` - Session management methods
  - `window_groups.rs` - Group management dialogs (move to group, error toast)
  - `window_clusters.rs` - Cluster management methods
  - `window_connection_dialogs.rs` - New connection/group dialogs, template picker, import dialog
  - `window_sorting.rs` - Sorting and drag-drop reordering operations
  - `window_operations.rs` - Connection operations (delete, duplicate, copy, paste, reload)
  - `window_edit_dialogs.rs` - Edit dialogs (edit connection, connection details, edit group, quick connect)
  - `window_rdp_vnc.rs` - RDP and VNC connection methods with password dialogs
  - `window_protocols.rs` - Protocol-specific connection handlers (SSH, VNC, SPICE, ZeroTrust)
  - `window_document_actions.rs` - Document management actions (new, open, save, close, export, import)
- Refactored `embedded_rdp.rs` into modular structure (reduced from 4234 to 2803 lines, -34%):
  - `embedded_rdp_types.rs` - Error types, enums, config structs, callback types
  - `embedded_rdp_buffer.rs` - PixelBuffer and WaylandSurfaceHandle
  - `embedded_rdp_launcher.rs` - SafeFreeRdpLauncher with Qt warning suppression
  - `embedded_rdp_thread.rs` - FreeRdpThread, ClipboardFileTransfer, FileDownloadState
  - `embedded_rdp_detect.rs` - FreeRDP detection utilities (detect_wlfreerdp, detect_xfreerdp, is_ironrdp_available)
  - `embedded_rdp_ui.rs` - UI helpers (clipboard buttons, Ctrl+Alt+Del, draw_status_overlay)
- Refactored `sidebar.rs` into modular structure (reduced from 2787 to 1937 lines, -30%):
  - `sidebar_types.rs` - TreeState, SessionStatusInfo, DropPosition, DropIndicator, SelectionModelWrapper, DragDropData
  - `sidebar_ui.rs` - UI helper functions (popovers, context menus, button boxes, protocol icons)
- Refactored `embedded_vnc.rs` into modular structure (reduced from 2304 to 1857 lines, -19%):
  - `embedded_vnc_types.rs` - Error types, VncConnectionState, VncConfig, VncPixelBuffer, VncWaylandSurface, callback types

### Fixed
- Tab icons now match sidebar icons for all protocols (SSH, RDP, VNC, SPICE, ZeroTrust providers)
- SSH and ZeroTrust sessions now show correct protocol-specific icons in tabs
- Cluster list not refreshing after deleting a cluster (borrow conflict in callback)
- Snippet dialog Save button not clickable (unreliable widget tree traversal replaced with direct reference)
- Template dialog not showing all fields (missing vexpand on notebook and scrolled window)

### Improved
- Extracted coordinate transformation utilities to `embedded_rdp_ui.rs` and `embedded_vnc_ui.rs`
- Added `transform_widget_to_rdp()`, `gtk_button_to_rdp_mask()`, `gtk_button_to_rdp_button()` helpers
- Added `transform_widget_to_vnc()`, `gtk_button_to_vnc_mask()` helpers
- Reduced code duplication in mouse input handlers (4 duplicate blocks → 1 shared function)
- Added unit tests for coordinate transformation and button conversion functions
- Made RDP event polling interval configurable via `RdpConfig::polling_interval_ms` (default 16ms = ~60 FPS)
- Added `RdpConfig::with_polling_interval()` builder method for custom polling rates
- CI: Added `libadwaita-1-dev` dependency to all build jobs
- CI: Added dedicated property tests job for better test visibility
- CI: Consolidated OBS publish workflow into release workflow
- CI: Auto-generate OBS changelog from CHANGELOG.md during release

### Documentation
- Added `#![warn(missing_docs)]` and documentation for public APIs in `rustconn-core`

## [0.5.1] - 2025-12-28

### Added
- Search debouncing with visual spinner indicator in sidebar (100ms delay for better UX)
- Pre-search state preservation (expanded groups, scroll position restored when search cleared)
- Clipboard file transfer UI for embedded RDP sessions:
  - "Save Files" button appears when files are available on remote clipboard
  - Folder selection dialog for choosing download destination
  - Progress tracking and completion notifications
  - Automatic file saving with status feedback
- CLI: Wake-on-LAN command (`wol`) - send magic packets by MAC address or connection name
- CLI: Snippet management commands (`snippet list/show/add/delete/run`)
  - Variable extraction and substitution support
  - Execute snippets with `--execute` flag
- CLI: Group management commands (`group list/show/create/delete/add-connection/remove-connection`)
- CLI: Connection list filters (`--group`, `--tag`) for `list` command
- CLI: Native format (.rcn) support for import/export

### Changed
- Removed global `#![allow(dead_code)]` from `rustconn/src/main.rs`
- Added targeted `#[allow(dead_code)]` annotations with documentation comments to GTK widget fields kept for lifecycle management
- Removed unused code:
  - `STANDARD_RESOLUTIONS` and `find_best_standard_resolution` from `embedded_rdp.rs`
  - `connect_kdbx_enable_switch` from `dialogs/settings.rs` (extended version exists)
  - `update_reconnect_button_visibility` from `embedded_rdp.rs`
  - `as_selection_model` from `sidebar.rs`
- Added public methods to `AutomationSession`: `remaining_triggers()`, `is_complete()`
- Documented API methods in `sidebar.rs`, `state.rs`, `terminal.rs`, `window.rs` with `#[allow(dead_code)]` annotations for future use
- Removed `--talk-name=org.freedesktop.secrets` from Flatpak manifest (unnecessary D-Bus permission)
- Refactored `dialogs/export.rs`: extracted `do_export()` and `format_result_summary()` to eliminate code duplication

## [0.5.0] - 2025-12-27

### Added
- RDP clipboard file transfer support (`CF_HDROP` format):
  - `ClipboardFileInfo` struct for file metadata (name, size, attributes, timestamps)
  - `ClipboardFileList`, `ClipboardFileContents`, `ClipboardFileSize` events
  - `RequestFileContents` command for requesting file data from server
  - `FileGroupDescriptorW` parsing for Windows file list format (MS-RDPECLIP 2.2.5.2.3.1)
- RDPDR directory change notifications (`ServerDriveNotifyChangeDirectoryRequest`):
  - Basic acknowledgment support (inotify integration pending)
  - `PendingNotification` struct for tracking watch requests
- RDPDR file locking support (`ServerDriveLockControlRequest`):
  - Basic acknowledgment for byte-range lock requests
  - `FileLock` struct for lock state tracking (advisory locking)

### Changed
- Audio playback: replaced `Mutex<f32>` with `AtomicU32` for volume control (lock-free audio callback)
- Search engine: optimized fuzzy matching to avoid string allocations (30-40% faster for large lists)
- Credential operations: use thread-local cached tokio runtime instead of creating new one each time

### Fixed
- SSH Agent key discovery now finds all private keys in `~/.ssh/`, not just `id_*` files:
  - Detects `.pem` and `.key` extensions
  - Reads file headers to identify private keys (e.g., `google_compute_engine`)
  - Skips known non-key files (`known_hosts`, `config`, `authorized_keys`)
- Native SPICE protocol embedding using `spice-client` crate 0.2.0 (optional `spice-embedded` feature)
  - Direct framebuffer rendering without external processes
  - Keyboard and mouse input forwarding via Inputs channel
  - Automatic fallback to external viewer (remote-viewer, virt-viewer, spicy) when native fails
  - Note: Clipboard and USB redirection not yet available in native mode (crate limitation)
- Real-time connection status indicators in the sidebar (green/red dots) to show connected/disconnected state
- Support for custom cursors in RDP sessions (server-side cursor updates)
- Full integration of "Expect" automation engine:
  - Regex-based pattern matching on terminal output
  - Automatic response injection
  - Support for "one-shot" triggers
- Terminal improvements:
  - Added context menu (Right-click) with Copy, Paste, and Select All options
  - Added keyboard shortcuts: Ctrl+Shift+C (Copy) and Ctrl+Shift+V (Paste)
- Refactored `Connection` model to support extensible automation configuration (`AutomationConfig`)

### Changed
- Updated `thiserror` from 1.0 to 2.0 (backwards compatible, no API changes required)
- Note: `picky` remains pinned at `=7.0.0-rc.17` due to sspi 0.16.0 incompatibility with newer versions

### Removed
- Unused FFI mock implementations for RDP and SPICE protocols (`rustconn-core/src/ffi/rdp.rs`, `rustconn-core/src/ffi/spice.rs`)
- Unused RDP and SPICE session widget modules (`rustconn/src/session/rdp.rs`, `rustconn/src/session/spice.rs`)

### Fixed
- Connection status indicator disappearing when closing one of multiple sessions for the same connection (now tracks session count per connection)
- System tray menu intermittently not appearing (reduced lock contention and debounced D-Bus updates)

## [0.4.2] - 2025-12-25

### Fixed
- Asbru-CM import now correctly parses installed Asbru configuration (connections inside `environments` key)
- Application icon now properly resolves in all installation scenarios (system, Flatpak, local, development)

### Changed
- Icon theme search paths extended to support multiple installation methods

## [0.4.1] - 2025-12-25

### Added
- IronRDP audio backend (RDPSND) with PCM format support (48kHz, 44.1kHz, 22.05kHz)
- Optional `rdp-audio` feature for audio playback via cpal (requires libasound2-dev)
- Bidirectional clipboard improvements for embedded RDP sessions

### Changed
- Updated MSRV to 1.87 (required by zune-jpeg 0.5.8)
- Updated dependencies: tempfile 3.24, criterion 0.8, cpal 0.17

## [0.4.0] - 2025-12-24

### Added
- Zero Trust: Improved UI by hiding irrelevant fields (Host, Port, Username, Password, Tags) when Zero Trust protocol is selected.

### Changed
- Upgraded `ironrdp` to version 0.13 (async API support).
- Refactored `rustconn-core` to improve code organization and maintainability.
- Made `spice-embedded` feature mandatory for better integration.

## [0.3.1] - 2025-12-23

### Changed
- Code cleanup: fixed all Clippy warnings (pedantic, nursery)
- Applied rustfmt formatting across all crates
- Added Deactivation-Reactivation sequence handling for RDP sessions

### Fixed
- Removed sensitive clipboard debug logging (security improvement)
- Fixed nested if statements and match patterns in RDPDR module

## [0.3.0] - 2025-12-23

### Added
- IronRDP clipboard integration for embedded RDP sessions (bidirectional copy/paste)
- IronRDP shared folders (RDPDR) support for embedded RDP sessions
- RemoteFX codec support for better RDP image quality
- RDPSND channel (required for RDPDR per MS-RDPEFS spec)

### Changed
- Migrated IronRDP dependencies from GitHub to crates.io (version 0.11)
- Reduced verbose logging in RDPDR module (now uses tracing::debug/trace)

### Fixed
- Pinned sspi to 0.16.0 and picky to 7.0.0-rc.16 to avoid rand_core conflicts

## [0.2.0] - 2025-12-22

### Added
- Tree view state persistence (expanded/collapsed folders saved between sessions)
- Native format (.rcn) import/export with proper group hierarchy preservation

### Fixed
- RDP embedded mode window sizing now uses saved window geometry
- Sidebar reload now preserves expanded/collapsed state
- Group hierarchy correctly maintained during native format import

### Changed
- Dependencies updated:
  - `ksni` 0.2 → 0.3 (with blocking feature)
  - `resvg` 0.44 → 0.45
  - `dirs` 5.0 → 6.0
  - `criterion` 0.5 → 0.6
- Migrated from deprecated `criterion::black_box` to `std::hint::black_box`

### Removed
- Removed obsolete TODO comment and unused variable in window.rs

## [0.1.0] - 2025-12-01

### Added
- Initial release of RustConn connection manager
- Multi-protocol support: SSH, RDP, VNC, SPICE
- Zero Trust provider integrations (AWS SSM, GCP IAP, Azure Bastion, etc.)
- Connection organization with groups and tags
- Import from Asbru-CM, Remmina, SSH config, Ansible inventory
- Export to Asbru-CM, Remmina, SSH config, Ansible inventory
- Native format import/export for backup and migration
- Secure credential storage via KeePassXC and libsecret
- Session logging with configurable formats
- Command snippets with variable substitution
- Cluster commands for multi-host execution
- Wake-on-LAN support
- Split terminal view
- System tray integration (optional)
- Performance optimizations:
  - Search result caching with configurable TTL
  - Lazy loading for connection groups
  - Virtual scrolling for large connection lists
  - String interning for memory optimization
  - Batch processing for import/export operations
- Embedded protocol clients (optional features):
  - VNC via vnc-rs
  - RDP via IronRDP
  - SPICE via spice-client

### Security
- All credentials wrapped in `SecretString`
- No plaintext password storage
- `unsafe_code = "forbid"` enforced

[Unreleased]: https://github.com/totoshko88/RustConn/compare/v0.5.9...HEAD
[0.5.9]: https://github.com/totoshko88/RustConn/compare/v0.5.8...v0.5.9
[0.5.8]: https://github.com/totoshko88/RustConn/compare/v0.5.7...v0.5.8
[0.5.7]: https://github.com/totoshko88/RustConn/compare/v0.5.6...v0.5.7
[0.5.6]: https://github.com/totoshko88/RustConn/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/totoshko88/RustConn/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/totoshko88/RustConn/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/totoshko88/RustConn/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/totoshko88/RustConn/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/totoshko88/RustConn/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/totoshko88/RustConn/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/totoshko88/RustConn/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/totoshko88/RustConn/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/totoshko88/RustConn/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/totoshko88/RustConn/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/totoshko88/RustConn/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/totoshko88/RustConn/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/totoshko88/RustConn/releases/tag/v0.1.0
