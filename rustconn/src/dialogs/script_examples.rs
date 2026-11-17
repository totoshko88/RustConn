//! Bundled `.rsc` example scripts shipped with RustConn.
//!
//! Kept apart from the Scripts dialog so the UI code stays reviewable. The
//! ids are managed content: edits under an `example-*` id are backed up to
//! `<id>.rsc.bak` when a newer bundled version replaces them.

pub(crate) const BUNDLED_EXAMPLES: &[(&str, &str)] = &[
    (
        "example-shell-uptime",
        // A shell prompt ending in `$` (local shell, most SSH hosts).
        // `capture` grabs the uptime text itself; `readline` would return
        // the echoed `uptime` command line.
        "expect \"\\$\" 10\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
    ),
    (
        "example-openwrt-uptime",
        // OpenWrt / BusyBox root prompt ends in `#` (root@OpenWrt:~#).
        "expect \"#\" 15\nsend \"uptime{ENTER}\"\ncapture /up\\s+([^,]+)/ up 10\nprint \"Uptime: $up\"\n",
    ),
    (
        "example-openwrt-save-uci",
        // OpenWrt's UCI is the whole configuration: `uci export` prints every
        // package as `config` blocks and writes nothing to the router. The
        // prompt is `root@OpenWrtMx4300:~#`, so the device name is the host
        // between `@` and `:`; a non-root shell ends in `$` and is refused.
        "getprompt prompt\nsend \"{ENTER}\"\ncapture /@([A-Za-z0-9][A-Za-z0-9._-]*):[^#$\\s]*[#$]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#$])\\s*$/ mode 10\nif_goto $mode /^#$/ root\nmsg \"This script needs a root shell (the prompt ends in the hash sign). Log in as root, then start it again.\" ok\ngoto done\n\n:root\nset file ${CWD}/openwrt-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the UCI configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nlog $file\nprint \"Capturing the UCI configuration...\"\nsend \"uci export{ENTER}\"\nexpect \"/@[A-Za-z0-9][A-Za-z0-9._-]*:[^#$\\s]*#\\s*$/\" 60\nlog\nprint \"UCI configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"OpenWrt UCI save-config script finished.\"\n",
    ),
    (
        "example-ping",
        // Inside `send`, a variable must be written `${name}`; `$name` is
        // literal text there (see the language reference).
        "ask \"Host to ping:\" host\nsend \"ping -c 3 ${host}{ENTER}\"\nwait 8000\n",
    ),
    (
        "example-router-first-run",
        // A first-run wizard: asks in clear text, validates the answer and
        // branches. No password is requested or stored.
        //
        // `uci show ...` is captured on the quoted value (`hostname='...'`),
        // not on `hostname=`, because the echoed command line contains the
        // latter and would satisfy the capture before the output arrives.
        "expect \"#\" 15\n\n:ask\nask \"Configure this router now? Type yes or no:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ configure\nif_goto $answer /(?i)^\\s*no\\s*$/ skip\nprint \"Please answer yes or no.\"\ngoto ask\n\n:skip\nmsg \"Leaving the router unchanged.\" ok\ngoto done\n\n:configure\nsend \"uci show system.@system[0].hostname{ENTER}\"\ncapture /hostname='([^']+)'/ old_name 10\nprint \"Current hostname: $old_name\"\nask \"New hostname (letters, digits and dashes only):\" new_name\nif_goto $new_name /^[A-Za-z0-9][A-Za-z0-9-]*$/ apply\nprint \"Invalid hostname. Nothing was changed.\"\ngoto done\n\n:apply\nsend \"uci set system.@system[0].hostname=${new_name} && uci commit system{ENTER}\"\nexpect \"#\" 15\nprint \"Hostname set to $new_name. It takes effect on the next boot.\"\n\n:done\nprint \"First-run helper finished.\"\n",
    ),
    (
        "example-run-command",
        "set cmd \"echo hello from script\"\nrun \"$cmd\" 5 out=o code=c\nprint \"Exit code: $c\"\nprint \"Output: $o\"\n",
    ),
    (
        "example-cisco-save-config",
        // The SecureCRT "Cisco Save Running/Startup Config to Local File"
        // shape, reduced to this DSL: read the running configuration and save
        // it to a timestamped local file, after an explicit yes. The device is
        // never written to. No local shell and no literal absolute path: the
        // default uses ${CWD}, so the same script runs on Linux, macOS and
        // Windows and the question shows the resolved path.
        //
        // The two ENTERs make a fresh prompt arrive even when the session
        // already shows the old one: the first captures the host name, the
        // second the mode character.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/cisco-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Cisco save-config script finished.\"\n",
    ),
    (
        "example-cisco-inventory",
        // The inventory shape: two read-only show commands, one timestamped
        // log per device, and the same explicit yes before a local file is
        // written. Prompt regex and paging command are the two Cisco-specific
        // assumptions; edit them for another vendor.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/cisco-inventory-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the inventory to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nsend \"show version{ENTER}\"\nexpect \"/[#>]\\s*$/\" 60\nsend \"show interfaces status{ENTER}\"\nexpect \"/[#>]\\s*$/\" 60\nlog\nprint \"Inventory saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Cisco inventory script finished.\"\n",
    ),
    (
        "example-huawei-save-config",
        // The Huawei VRP profile of the save example: user view `<host>`,
        // `display` instead of `show`, and `screen-length 0 temporary`
        // instead of `terminal length 0`. System view `[host]` is refused
        // because the paging command is a user-view command.
        "getprompt prompt\nif_goto $prompt /^\\s*\\[/ config_mode\nsend \"{ENTER}\"\ncapture /<([A-Za-z0-9][A-Za-z0-9._-]*)>\\s*$/ device 10\nset file ${CWD}/huawei-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the current configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"screen-length 0 temporary{ENTER}\"\nexpect \"/<[A-Za-z0-9][A-Za-z0-9._-]*>\\s*$/\" 15\nlog $file\nprint \"Capturing the current configuration...\"\nsend \"display current-configuration{ENTER}\"\nexpect \"/<[A-Za-z0-9][A-Za-z0-9._-]*>\\s*$/\" 120\nlog\nprint \"Current configuration saved to $file\"\ngoto done\n\n:config_mode\nmsg \"This script must run in user view. Leave system view with return, then start the script again.\" ok\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Huawei save-config script finished.\"\n",
    ),
    (
        "example-hsgq-save-config",
        // HSGQ EPON OLT profile: `OLT#` is enable mode, `OLT>` is user
        // mode and `OLT(config)#` is configuration mode. HSGQ documents
        // `show current-config` but no pager command, so the `pager`
        // directive answers any `--More--` prompt instead.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in enable mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/hsgq-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the current configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the current configuration...\"\nsend \"show current-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Current configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"HSGQ save-config script finished.\"\n",
    ),
    (
        "example-dlink-xstack-save-config",
        // D-Link xStack managed switches (DGS-3100/3200/3600) print
        // `DGS-3100-24#` and switch to `... (config)#` in configuration
        // mode. `disable clipaging` turns off the `--More--` pager and
        // `show configuration running` is the read-only command.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script needs the administrator prompt (ending in the hash sign). Log in with administrator rights, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run outside configuration mode. Leave it with exit, then start the script again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/dlink-xstack-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"disable clipaging{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show configuration running{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"D-Link xStack save-config script finished.\"\n",
    ),
    (
        "example-dlink-dgs1210-save-config",
        // The DGS-1210 web-smart family: the prompt carries the model and the
        // account (`DGS-1210-24:admin#`), the CLI has no configuration mode
        // and no paging command, and `show config` is the read-only command.
        // Any `--More--` prompt is answered by the `pager` directive.
        // The file is named after the model, not the logged-in account.
        "getprompt prompt\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)(:[^#>\\s]*)?[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script needs the administrator prompt (ending in the hash sign). Log in with administrator rights, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/dlink-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the configuration...\"\nsend \"show config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"D-Link DGS-1210 save-config script finished.\"\n",
    ),
    (
        "example-juniper-save-config",
        // Junos: operational mode is `user@host>` and configuration mode is
        // `user@host#`, so the greater-than sign is the mode this profile
        // needs. `| no-more` suppresses the pager on the read command itself,
        // which is why Junos needs no separate paging command.
        "getprompt prompt\nif_goto $prompt /#\\s*$/ config_mode\nsend \"{ENTER}\"\ncapture /@([A-Za-z0-9][A-Za-z0-9._-]*)[>#]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([>#])\\s*$/ mode 10\nif_goto $mode /^>$/ operational\nmsg \"This script needs operational mode (the prompt ends in the greater-than sign). Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run in operational mode. Commit or discard your changes with exit, then start the script again.\" ok\ngoto done\n\n:operational\nset file ${CWD}/juniper-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nlog $file\nprint \"Capturing the configuration...\"\nsend \"show configuration | no-more{ENTER}\"\nexpect \"/@[A-Za-z0-9][A-Za-z0-9._-]*[>#]\\s*$/\" 120\nlog\nprint \"Configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Juniper save-config script finished.\"\n",
    ),
    (
        "example-tplink-save-config",
        // TP-Link JetStream switches use Cisco-shaped prompts (`Switch>`,
        // `Switch#`, `Switch(config)#`) but their CLI reference documents no
        // paging command, so the `pager` directive answers `--More--`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/tplink-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"TP-Link save-config script finished.\"\n",
    ),
    (
        "example-h3c-save-config",
        // H3C Comware (S/CR/MSR): user view `<H3C>`, system view `[H3C]`.
        // `screen-length disable` turns paging off for this session and
        // `display current-configuration` is the read-only command.
        "getprompt prompt\nif_goto $prompt /^\\s*\\[/ config_mode\nsend \"{ENTER}\"\ncapture /<([A-Za-z0-9][A-Za-z0-9._-]*)>\\s*$/ device 10\nset file ${CWD}/h3c-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the current configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"screen-length disable{ENTER}\"\nexpect \"/<[A-Za-z0-9][A-Za-z0-9._-]*>\\s*$/\" 15\nlog $file\nprint \"Capturing the current configuration...\"\nsend \"display current-configuration{ENTER}\"\nexpect \"/<[A-Za-z0-9][A-Za-z0-9._-]*>\\s*$/\" 120\nlog\nprint \"Current configuration saved to $file\"\ngoto done\n\n:config_mode\nmsg \"This script must run in user view. Leave system view with return, then start the script again.\" ok\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"H3C save-config script finished.\"\n",
    ),
    (
        "example-ruijie-save-config",
        // Ruijie RGOS (RG-S switches, RG-EG gateways) mirrors Cisco:
        // `Ruijie>` / `Ruijie#` / `Ruijie(config)#`, `terminal length 0`
        // and `show running-config`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/ruijie-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Ruijie save-config script finished.\"\n",
    ),
    (
        "example-zte-save-config",
        // ZTE ZXR10 switches and ZXA10 OLTs: `ZXR10>` / `ZXR10#` /
        // `ZXR10(config)#` (`ZXAN#` on the OLT). `show running-config` is
        // read-only. No pager command is documented for this CLI, so the
        // `pager` directive answers any `--More--` prompt.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run outside configuration mode. Leave it with exit, then start the script again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/zte-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"ZTE save-config script finished.\"\n",
    ),
    (
        "example-mikrotik-save-config",
        // RouterOS has no enable mode and no pager: `export` prints the
        // configuration as commands. `/export` is absolute, so a session left
        // inside `/interface` still exports everything; `verbose` includes
        // default values. RouterOS 6 shows secrets by default while RouterOS 7
        // needs `show-sensitive`, so the script reads the major version and
        // picks the form; the resulting log must still be protected.
        "send \"{ENTER}\"\ncapture /@([A-Za-z0-9][A-Za-z0-9._-]*)\\][^>]*>\\s*$/ device 10\nset file ${CWD}/mikrotik-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the configuration to '${file}'? The log will contain secrets. Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"/system resource print{ENTER}\"\ncapture /version:\\s*([0-9]+)/ ros_major 10\nexpect \"/@[A-Za-z0-9][A-Za-z0-9._-]*\\][^>]*>\\s*$/\" 10\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the configuration...\"\nif_goto $ros_major /^7/ ros7\nsend \"/export verbose{ENTER}\"\ngoto exported\n\n:ros7\nsend \"/export verbose show-sensitive{ENTER}\"\n\n:exported\nexpect \"/@[A-Za-z0-9][A-Za-z0-9._-]*\\][^>]*>\\s*$/\" 120\nlog\nprint \"Configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"MikroTik save-config script finished.\"\n",
    ),
    (
        "example-eltex-save-config",
        // Eltex MES switches and ESR routers use a Cisco-like CLI:
        // `console>` / `console#` / `console(config)#`, with
        // `show running-config`. The MES CLI has no pager command: its own
        // binary contains `--More--` but no `terminal length`, so the
        // `pager` directive answers the prompt. `detailed` is the full form:
        // it includes binary data such as SSH and SSL keys, so the log holds
        // secrets.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/eltex-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? The log will contain secrets. Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config detailed{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Eltex save-config script finished.\"\n",
    ),
    (
        "example-bdcom-save-config",
        // BDCOM switches and EPON/GPON OLTs: `Switch>` / `Switch#`, and the
        // configuration prompt is `switch_config#` with no parentheses, so
        // this profile refuses any prompt containing `_config`.
        // `terminal length 0` and `show running-config` are documented for
        // both families.
        "getprompt prompt\nif_goto $prompt /_config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run outside configuration mode. Leave it with exit, then start the script again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/bdcom-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"BDCOM save-config script finished.\"\n",
    ),
    (
        "example-ubiquiti-save-config",
        // EdgeOS (EdgeRouter, EdgeSwitch): the operational prompt ends in
        // `$` and configuration mode in `#`. `show configuration commands`
        // prints the configuration in set form; `| no-more` disables the
        // pager, and the `pager` directive is kept as a fallback.
        "getprompt prompt\nif_goto $prompt /#\\s*$/ config_mode\nsend \"{ENTER}\"\ncapture /@([A-Za-z0-9][A-Za-z0-9._-]*):[^$#\\s]*[$#]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([$#])\\s*$/ mode 10\nif_goto $mode /^\\$/ operational\nmsg \"This script needs operational mode (the prompt ends in the dollar sign). Leave configuration mode with exit discard, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run in operational mode. Leave configuration mode with exit discard, then start it again.\" ok\ngoto done\n\n:operational\nset file ${CWD}/ubiquiti-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the configuration...\"\nsend \"show configuration commands | no-more{ENTER}\"\nexpect \"/@[A-Za-z0-9][A-Za-z0-9._-]*:[^$#\\s]*\\$\\s*$/\" 120\nlog\nprint \"Configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Ubiquiti save-config script finished.\"\n",
    ),
    (
        "example-huawei-olt-save-config",
        // Huawei SmartAX OLTs (MA5600T/MA5680T/MA5800) differ from VRP:
        // `MA5680T>` is user mode, `MA5680T#` is enable and
        // `MA5680T(config)#` is configuration mode. `display
        // current-configuration` then asks for an optional filter and needs
        // an extra ENTER. SmartAX cannot disable paging, so the `pager`
        // directive answers its `---- More ----` prompt.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([>#])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in enable mode. Leave configuration mode with quit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/huawei-olt-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the current configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"/---- More .*----/\"\nlog $file\nprint \"Capturing the current configuration...\"\nsend \"display current-configuration{ENTER}\"\nsend \"{ENTER}\"\nexpect \"/[>#]\\s*$/\" 180\nlog\nprint \"Current configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Huawei OLT save-config script finished.\"\n",
    ),
    (
        "example-huawei-olt-provision-ont",
        // Write script for Huawei SmartAX OLTs (MA5600T/MA5680T/MA5800):
        // creates the service VLAN and uplink port, a service profile, a line
        // profile, the ONT and its service port. Every prompt is pre-filled
        // with an example value; the serial number has no default because it
        // belongs to the physical ONT. The PON frame is fixed at 0, which is
        // the only frame on these models. Requires an existing DBA profile and
        // traffic table, and free profile IDs. The profile prompts are
        // `config-gpon-srvprofile-N` on R011 and `config-srvprofile-gpon-N` on
        // other releases, so both orders are accepted; commands otherwise
        // vary by software version, so check a rejected one with ? on the OLT.
        // Nothing is saved: the operator saves the configuration manually
        // after the ONT comes online.
        r#"pager "/---- More .*----/"
getprompt prompt
if_goto $prompt /\(config/ config_mode
send "{ENTER}"
capture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\s*$/ device 10
send "{ENTER}"
capture /([>#])\s*$/ mode 10
if_goto $mode /^#$/ config
goto user_mode

:config_mode
msg "This script must run in enable mode. Leave configuration mode with quit, then start it again." ok
goto done

:user_mode
send "enable{ENTER}"
capture /(Password:|[#>])\s*$/ enable_reply 15
if_goto $enable_reply /^Password:$/ enable_password
if_goto $enable_reply /^#$/ config
msg "This script could not enter privileged mode. Check the enable password, then start it again." ok
goto done

:enable_password
ask "Enable password:" hide enable_pw
send "${enable_pw}{ENTER}"
capture /([#>])\s*$/ mode 10
if_goto $mode /^#$/ config
msg "This script could not enter privileged mode. Check the enable password, then start it again." ok
goto done

:config
send "config{ENTER}"
expect "/\(config\)#\s*$/" 15

:ask_vlan
ask "Service VLAN (1-4094):" default "500" vlan
if_goto $vlan /^([1-9]|[1-9][0-9]|[1-9][0-9]{2}|[1-3][0-9]{3}|40[0-8][0-9]|409[0-4])$/ ask_uplink_fs
print "The VLAN must be a whole number from 1 to 4094."
goto ask_vlan

:ask_uplink_fs
ask "Uplink frame/slot (example 0/19):" default "0/19" uplink_fs
if_goto $uplink_fs /^[0-9]{1,2}\/[0-9]{1,2}$/ ask_uplink_port
print "Use frame/slot as two numbers separated by a slash, for example 0/19."
goto ask_uplink_fs

:ask_uplink_port
ask "Uplink port on frame 0/<slot>:" default "0" uplink_port
if_goto $uplink_port /^[0-9]{1,2}$/ ask_pon_slot
print "The uplink port must be a number, for example 0."
goto ask_uplink_port

:ask_pon_slot
ask "PON slot (interface gpon 0/<slot>):" default "1" pon_slot
if_goto $pon_slot /^[0-9]{1,2}$/ ask_pon_port
print "The PON slot must be a number, for example 1."
goto ask_pon_slot

:ask_pon_port
ask "PON port on that slot:" default "0" pon_port
if_goto $pon_port /^[0-9]{1,2}$/ ask_ont_id
print "The PON port must be a number, for example 0."
goto ask_pon_port

:ask_ont_id
ask "ONT ID on this PON port (usually 1-128):" default "1" ont_id
if_goto $ont_id /^[1-9][0-9]{0,2}$/ ask_sn
print "The ONT ID must be a whole number from 1 to 999."
goto ask_ont_id

:ask_sn
ask "ONT serial number (16 hex digits, from the ONT label):" sn
if_goto $sn /^[0-9A-Fa-f]{16}$/ ask_srv_profile
print "The serial number must be exactly 16 hexadecimal digits."
goto ask_sn

:ask_srv_profile
ask "Service profile ID:" default "10" srv_profile
if_goto $srv_profile /^[1-9][0-9]{0,3}$/ ask_line_profile
print "The profile ID must be a whole number from 1 to 9999."
goto ask_srv_profile

:ask_line_profile
ask "Line profile ID:" default "10" line_profile
if_goto $line_profile /^[1-9][0-9]{0,3}$/ ask_dba
print "The profile ID must be a whole number from 1 to 9999."
goto ask_line_profile

:ask_dba
ask "Existing DBA profile ID:" default "10" dba
if_goto $dba /^[1-9][0-9]{0,3}$/ ask_traffic
print "The DBA profile ID must be a whole number from 1 to 9999."
goto ask_dba

:ask_traffic
ask "Existing traffic table ID:" default "10" traffic
if_goto $traffic /^[1-9][0-9]{0,5}$/ ask_eth
print "The traffic table index must be a whole number."
goto ask_traffic

:ask_eth
ask "Ethernet ports to enable on the ONT:" default "4" eth
if_goto $eth /^[1-8]$/ ask_desc
print "The Ethernet port count must be a number from 1 to 8."
goto ask_eth

:ask_desc
ask "ONT description (letters, digits, dot, dash, underscore):" default "RUSTCONN_EXAMPLE" desc
if_goto $desc /^[A-Za-z0-9_.-]{1,32}$/ preview
print "The description may contain only letters, digits, dot, dash and underscore (at most 32 characters)."
goto ask_desc

:preview
print "The OLT will now be configured with:"
print "  VLAN: $vlan"
print "  Uplink: $uplink_fs port $uplink_port"
print "  PON: 0/$pon_slot/$pon_port, ONT ID $ont_id"
print "  ONT serial: $sn"
print "  Service profile $srv_profile, line profile $line_profile"
print "  DBA profile $dba, traffic table $traffic"
print "  Ethernet ports: $eth, description: $desc"
ask "Apply these changes? Type yes to continue, or nothing to cancel:" apply
if_goto $apply /(?i)^\s*yes\s*$/ write
print "Nothing was sent to the OLT."
goto done

:write
set file ${CWD}/huawei-olt-provision-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log
log $file
print "Creating the VLAN and uplink port..."
send "vlan ${vlan} smart{ENTER}"
expect "/\(config\)#\s*$/" 30
send "port vlan ${vlan} ${uplink_fs} ${uplink_port}{ENTER}"
expect "/\(config\)#\s*$/" 30

print "Creating the service profile..."
send "ont-srvprofile gpon profile-id ${srv_profile}{ENTER}"
expect "/\((config-srvprofile-gpon|config-gpon-srvprofile)-[0-9]+\)#\s*$/" 30
send "ont-port eth ${eth}{ENTER}"
expect "/\((config-srvprofile-gpon|config-gpon-srvprofile)-[0-9]+\)#\s*$/" 30
send "port vlan eth 1-${eth} ${vlan}{ENTER}"
expect "/\((config-srvprofile-gpon|config-gpon-srvprofile)-[0-9]+\)#\s*$/" 30
send "commit{ENTER}"
expect "/\((config-srvprofile-gpon|config-gpon-srvprofile)-[0-9]+\)#\s*$/" 60
send "quit{ENTER}"
expect "/\(config\)#\s*$/" 15

print "Creating the line profile..."
send "ont-lineprofile gpon profile-id ${line_profile}{ENTER}"
expect "/\((config-lineprofile-gpon|config-gpon-lineprofile)-[0-9]+\)#\s*$/" 30
send "tcont 1 dba-profile-id ${dba}{ENTER}"
expect "/\((config-lineprofile-gpon|config-gpon-lineprofile)-[0-9]+\)#\s*$/" 30
send "gem add 1 eth tcont 1{ENTER}"
expect "/\((config-lineprofile-gpon|config-gpon-lineprofile)-[0-9]+\)#\s*$/" 30
send "gem mapping 1 0 vlan ${vlan}{ENTER}"
expect "/\((config-lineprofile-gpon|config-gpon-lineprofile)-[0-9]+\)#\s*$/" 30
send "commit{ENTER}"
expect "/\((config-lineprofile-gpon|config-gpon-lineprofile)-[0-9]+\)#\s*$/" 60
send "quit{ENTER}"
expect "/\(config\)#\s*$/" 15

print "Adding the ONT..."
send "interface gpon 0/${pon_slot}{ENTER}"
expect "/(config-if-gpon-0\/[0-9]+)#\s*$/" 15
send "ont add ${pon_port} ${ont_id} sn-auth ${sn} omci ont-lineprofile-id ${line_profile} ont-srvprofile-id ${srv_profile} desc ${desc}{ENTER}"
capture "/((?i)(\(y\/n\)|\[y\/n\])|\(config-if-gpon-0\/[0-9]+\)#\s*$)/" add_reply 60
if_goto $add_reply "/(?i)(\(y\/n\)|\[y\/n\])/" confirm_ont
goto service_port

:confirm_ont
send "y{ENTER}"
expect "/\(config-if-gpon-0\/[0-9]+\)#\s*$/" 60

:service_port
send "quit{ENTER}"
expect "/\(config\)#\s*$/" 15
print "Adding the service port..."
send "service-port vlan ${vlan} gpon 0/${pon_slot}/${pon_port} ont ${ont_id} gemport 1 multi-service user-vlan ${vlan} inbound traffic-table index ${traffic} outbound traffic-table index ${traffic}{ENTER}"
expect "/\(config\)#\s*$/" 60

log
print "Provisioning commands finished. The ONT should come online shortly."
print "The running configuration has NOT been saved. Save it on the OLT once the ONT works."
goto done

:done
print "Huawei OLT provision script finished."
"#,
    ),
    (
        "example-raisecom-save-config",
        // Raisecom ISCOM switches: `Raisecom>` / `Raisecom#` /
        // `Raisecom(config)#`, with `show running-config` in privileged
        // EXEC. The pager is answered generically because Raisecom documents
        // no pager command for this CLI.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/raisecom-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Raisecom save-config script finished.\"\n",
    ),
    (
        "example-qtech-save-config",
        // QTECH switches (QSW series): `QTECH>` / `QTECH#` /
        // `QTECH(config)#`, with `show running-config` in privileged EXEC.
        // The pager is answered generically because QTECH documents no pager
        // command for this CLI.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/qtech-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"QTECH save-config script finished.\"\n",
    ),
    (
        "example-zyxel-save-config",
        // Zyxel switches (GS/XGS series) use a Cisco-like CLI: `GS2210-24>`,
        // `GS2210-24#`, `GS2210-24(config)#`, with `show running-config`.
        // Zyxel switches page by default and document no pager command, so
        // the generic `pager` directive answers `--More--`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/zyxel-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Zyxel save-config script finished.\"\n",
    ),
    (
        "example-cdata-save-config",
        // C-Data OLTs (FD series) use the enable-mode `OLT#` prompt and a
        // Cisco-like `show running-config`. The pager is answered generically
        // because C-Data documents no pager command for this CLI.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in enable mode. Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/cdata-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"C-Data save-config script finished.\"\n",
    ),
    (
        "example-vsol-save-config",
        // VSOL (V-SOL) OLTs, same enable-mode `OLT#` shape and
        // `show running-config`; the pager is answered generically.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in enable mode. Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/vsol-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"VSOL save-config script finished.\"\n",
    ),
    (
        "example-maipu-save-config",
        // Maipu routers and switches: Cisco-shaped `Maipu>` / `Maipu#` /
        // `Maipu(config)#`, with `show running-config`. The pager is answered
        // generically.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/maipu-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Maipu save-config script finished.\"\n",
    ),
    (
        "example-hikvision-save-config",
        // Hikvision managed access/CCTV switches that expose the CLI:
        // Cisco-shaped enable mode and `show running-config`; the pager is
        // answered generically. Unmanaged models have no such CLI.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/hikvision-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Hikvision save-config script finished.\"\n",
    ),
    (
        "example-dahua-save-config",
        // Dahua managed access/CCTV switches that expose the CLI: same
        // Cisco-shaped enable mode and `show running-config`; the pager is
        // answered generically. Unmanaged models have no such CLI.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/dahua-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Dahua save-config script finished.\"\n",
    ),
    (
        "example-netgear-save-config",
        // Netgear ProSAFE managed switches (M4100 and similar): the prompt
        // may carry the model in parentheses, so the device name is taken from
        // the last word before the mode character.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\)?\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with exit, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/netgear-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\npager \"(--More--|---- More .*----|Press any key)\"\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Netgear save-config script finished.\"\n",
    ),
    (
        "example-aruba-save-config",
        // Aruba / HP ProCurve AOS-S switches: `show running-config` in
        // privileged mode, paging turned off with `no page`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/aruba-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"no page{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Aruba save-config script finished.\"\n",
    ),
    (
        "example-arista-save-config",
        // Arista EOS: `show running-config` in privileged mode, paging turned
        // off with `terminal length 0`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/arista-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Arista save-config script finished.\"\n",
    ),
    (
        "example-extreme-save-config",
        // Extreme EXOS: the prompt puts a space before `#` (`X460-48t.1 #`)
        // and prefixes `*` while there are uncommitted changes, which the
        // capture strips. The read command is `show configuration`, and
        // paging is turned off with `disable clipaging`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /\\*?([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script needs the administrator prompt (ending in the hash sign). Log in with administrator rights, then start it again.\" ok\ngoto done\n\n:config_mode\nmsg \"This script must run outside configuration mode. Leave it, then start the script again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/extreme-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"disable clipaging{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the configuration...\"\nsend \"show configuration{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Extreme save-config script finished.\"\n",
    ),
    (
        "example-allied-save-config",
        // Allied Telesis AlliedWare Plus: `awplus#` prompt,
        // `show running-config`, paging turned off with `terminal length 0`.
        "getprompt prompt\nif_goto $prompt /\\(config/ config_mode\nsend \"{ENTER}\"\ncapture /([A-Za-z0-9][A-Za-z0-9._-]*)\\s*[#>]\\s*$/ device 10\nsend \"{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\ngoto user_mode\n\n:config_mode\nmsg \"This script must run in EXEC mode. Leave configuration mode with end, then start it again.\" ok\ngoto done\n\n:user_mode\nsend \"enable{ENTER}\"\ncapture /(Password:|[#>])\\s*$/ enable_reply 15\nif_goto $enable_reply /^Password:$/ enable_password\nif_goto $enable_reply /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:enable_password\nask \"Enable password:\" hide enable_pw\nsend \"${enable_pw}{ENTER}\"\ncapture /([#>])\\s*$/ mode 10\nif_goto $mode /^#$/ privileged\nmsg \"This script could not enter privileged mode. Check the enable password, then start it again.\" ok\ngoto done\n\n:privileged\nset file ${CWD}/allied-${device}-${DATE_Y}${DATE_M}${DATE_D}-${TIME_H}${TIME_M}${TIME_S}.log\nask \"Save the running configuration to '${file}'? Type yes, no, another path, or nothing to cancel:\" answer\nif_goto $answer /(?i)^\\s*yes\\s*$/ save\nif_goto $answer /^\\s*$/ cancel\nif_goto $answer /(?i)^\\s*(n|no)\\s*$/ cancel\nset file $answer\n\n:save\nsend \"terminal length 0{ENTER}\"\nexpect \"/[#>]\\s*$/\" 15\nlog $file\nprint \"Capturing the running configuration...\"\nsend \"show running-config{ENTER}\"\nexpect \"/[#>]\\s*$/\" 120\nlog\nprint \"Running configuration saved to $file\"\ngoto done\n\n:cancel\nprint \"Nothing was saved.\"\n\n:done\nprint \"Allied Telesis save-config script finished.\"\n",
    ),
];
