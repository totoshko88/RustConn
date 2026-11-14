//! The `.rsc` script language reference, shown from the Scripts manager.
//!
//! English by design: it is the language's own specification (instruction
//! names, argument syntax, the host API), not a piece of application UI, and
//! the same text is shipped in the user guide so the two cannot drift.

/// The full English reference for the `.rsc` scripting language.
pub const SCRIPT_LANGUAGE_REFERENCE: &str = r#"RUSTCONN SCRIPT LANGUAGE (.rsc) — REFERENCE

A script is a plain-text instruction file. One instruction per line. Blank
lines are ignored, and `#` or `;` starts a comment that runs to the end of
the line. Arguments are separated by spaces; an argument that contains spaces
must be quoted with " or '.

Scripts live in the script library folder (one file per script, named by its
script id) and are run against a live terminal session.


1. TERMINAL PRIMITIVES
----------------------

send <sequence>
    Send text and special keys to the session. The sequence syntax is the same
    as a connection's key sequence:
        send ls -la{ENTER}
        send {CTRL}C
        send {WAIT:500}text{ENTER}

    Inside `send`, a variable must be written ${name}. A bare $name is literal
    text there, because the sequence parser reserves $ for its own ${...}
    syntax; everywhere else ($name in print, ask, msg, run) both forms work.
    Special keys: {ENTER} {TAB} {ESC} {BACKSPACE} {DELETE} {UP} {DOWN} {LEFT}
    {RIGHT} {HOME} {END} {PGUP} {PGDN} {INSERT} {SPACE} {F1}..{F12}
    {CTRL}C {CTRL}D {CTRL}Z {CTRL}A {CTRL}E {CTRL}L

wait <milliseconds>
    Pause the script. The script resumes on a timer; nothing else happens
    while it waits.

expect <regex> [<timeout-seconds>]
    Wait until the session output matches the regular expression. The
    expression may be written bare (password:) or delimited with slashes
    (/Password: *$/). If the timeout elapses first, the script stops with a
    timeout error. The default timeout is 10 seconds.

capture <regex> <variable> [<timeout-seconds>]
    Like expect, but the first capture group of the match is stored in a
    variable. The matched text is consumed, so the next instruction sees only
    output that arrived afterwards.

readline <variable> [<timeout-seconds>]
    Wait for a complete line of output and store it (without the line break)
    in a variable.

getprompt <variable>
    Guess the current prompt of the session and store it. The guess is the
    text on the cursor's line with escape sequences removed. It is a
    best-effort read of what is on screen, so it fails on dynamic or
    menu-driven prompts.

log [<file-path>]
    Start appending the session output to a file. With no argument, logging
    stops. The file is written locally.

print <text>
    Print a message on the terminal in the session, with ANSI colouring.


2. USER INTERACTION
-------------------

ask "<prompt>" [hide] <variable>
    Ask the user for input and store the answer in a variable. With the "hide"
    keyword the input is masked (use it for passwords). Cancelling the dialog
    stops the script with an error.

msg <text> [ok]
    Show a message window. With the "ok" keyword the window has an OK button.
    An empty text hides the message window.


3. SESSION MANAGEMENT
---------------------

start <name-or-uuid> [manual] [-> uuid,tmp]
    Start a session. The selector is a connection name or a connection UUID.
    "manual" requests the manual-login flag. "-> uuid,tmp" stores the started
    session's UUID and temporary UUID in the named variables. The "cluster="
    option is not supported and stops the script with an error.

select <name-or-uuid> [-> uuid,tmp]
    Select an open session by connection name or by session UUID, and store
    its ids in the named variables. The selected session becomes the target
    of the following instructions.

stop <session-variable>
    Close the session whose UUID is stored in the variable.

split <first> <second> [vertical]
    Show two sessions side by side in a split layout. Both arguments name
    sessions (a variable holding a session UUID, or a connection name).
    Without "vertical" the panes are stacked horizontally.

unsplit <session-variable>
    Remove the split layout the session belongs to.

totab <session-variable>
    Return a session from a split pane to its own tab.

towindow <session-variable>
    Move a session out of its tab into its own window.


4. LOCAL COMMANDS
-----------------

run "<command>" [<timeout-seconds>] [out=<variable>] [code=<variable>]
    Run a local command and wait for it to finish. The command's standard
    output is stored in the "out" variable and its exit code in the "code"
    variable. Sensitive environment variables (cloud, vault and CI tokens) are
    removed from the command's environment. The command runs off the UI
    thread, so a slow command does not freeze the application. It is
    stopped when its timeout expires; without a timeout the limit is 60
    seconds. A variable substituted into the command must not contain shell
    metacharacters, and an "ask ... hide" answer is never substituted.


5. CONTROL FLOW AND VARIABLES
------------------------------

set <variable> <value>
    Set a variable. The value may contain ${other} references.

goto <label>
    Jump to a label. Labels are defined as ":name" on their own line.

if_goto <variable> <regex> <label> [else <label>]
    Jump to <label> when the variable's value matches the regular expression,
    otherwise to the else label, otherwise fall through.

${name} and $name
    Reference a variable. The value is substituted when the instruction runs.
    Inside `send`, use ${name} only (see above).

A script that jumps in a loop is stopped after one million instructions.


6. EXAMPLE
----------

# Wait for the prompt, run a command and report its output.
expect "\$" 10
send "uptime{ENTER}"
readline line 5
print "Uptime: $line"

# Ask for a host, connect to it, and look at the disk.
ask "Host to inspect:" host
start "ssh-$host" -> sid
select $sid
expect "\$" 30
send "df -h{ENTER}"


7. BRANCHING ON USER INPUT
---------------------------

`ask` gives the script a value, and `if_goto` turns it into a decision. This is
the shape of a first-run wizard: ask, validate, then jump.

    :ask
    ask "Configure this router now? Type yes or no:" answer
    if_goto $answer /(?i)^\s*yes\s*$/ configure
    if_goto $answer /(?i)^\s*no\s*$/ skip
    print "Please answer yes or no."
    goto ask

    :skip
    msg "Leaving the router unchanged." ok
    goto done

    :configure
    ...

    :done
    print "Finished."

Rules that matter in that example:

  * `ask` without `hide` echoes what is typed: the answer is not a secret.
  * `if_goto` matches the variable as a regular expression, so `(?i)` makes
    the comparison case-insensitive and `\s*` tolerates stray spaces.
  * A question that accepts only two answers needs a third jump back to the
    question, or an unexpected answer silently falls through to whatever the
    next instruction happens to be.
  * `msg ... ok` shows a message and continues; it does not wait for a click.

See the bundled `example-router-first-run` for the complete script, including
reading the current value before overwriting it.

8. ERRORS
---------

A script stops when an instruction fails: an invalid regular expression, a
timeout while waiting for output, a cancelled input dialog, an unknown
connection, or a command that could not be run. The reason is shown as a
notification. One script at a time runs per session; a second script on the
same session is refused.
"#;

/// Shows the script language reference in a window.
pub fn show_script_reference(parent: Option<&gtk4::Widget>) {
    use adw::prelude::*;
    use gtk4::prelude::*;
    use libadwaita as adw;

    let dialog = adw::Dialog::builder()
        .title("RustConn Script Language")
        .content_width(760)
        .content_height(640)
        .build();

    let view = gtk4::TextView::builder()
        .monospace(true)
        .editable(false)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();
    view.buffer().set_text(SCRIPT_LANGUAGE_REFERENCE);

    let scrolled = gtk4::ScrolledWindow::builder()
        .child(&view)
        .hexpand(true)
        .vexpand(true)
        .build();
    dialog.set_child(Some(&scrolled));

    if let Some(parent) = parent {
        dialog.present(Some(parent));
    } else {
        dialog.present(None::<&gtk4::Widget>);
    }
}
