# Keyboard shortcuts

The **Keyboard shortcuts** help button appears immediately before the status
controls on every panel, including when tray icons are disabled. It opens one
scrollable sheet, grouped by Way-Shell actions, windows and layout, workspaces
and outputs, session actions, and named Sway modes. The design follows
[nwg-shell-config’s help window](https://github.com/nwg-piotr/nwg-shell-config/blob/master/nwg_shell_config/help.py).

Clicking the button again on the same output closes the sheet. Clicking another
output’s button moves it there. Escape, the close button, and clicking outside
also dismiss it. Opening another shell popup closes the sheet; confirmation
dialogs take priority. The sheet supports normal focus navigation and scrolling.

```sh
way-sh shortcuts show
way-sh shortcuts hide
way-sh shortcuts toggle
```

Show is idempotent. CLI show targets the focused workspace’s output, with the
first valid monitor as a fallback. CLI toggle closes an already visible sheet.
Unavailable configuration still allows the sheet to open and show internal
navigation. These commands append IPC opcodes 34, 35, and 36; older commands and
the four-byte reply format remain unchanged.

No key binding is installed automatically. For example, add this to Sway:

```sway
bindsym $mod+F1 exec way-sh shortcuts toggle
```

Or add this entry inside Niri’s existing `binds` block:

```kdl
Mod+F1 { spawn "way-sh" "shortcuts" "toggle"; }
```

## Configuration sources

The sheet represents **saved configuration**, which can differ from active
compositor bindings. Reload Sway after saving binding changes. If Niri reports a
rejected reload, the sheet explains that saved shortcuts might not be active.

The selected Way-Shell compositor backend determines the parser. For Sway the
source is `loaded_config_file_name` from
[GET_VERSION](https://github.com/swaywm/sway/blob/master/sway/sway-ipc.7.scd).
The last reported path survives temporary disconnections.

For Niri, Way-Shell obtains the socket peer’s PID and checks its startup
`--config`/`-c` argument, then its initial `NIRI_CONFIG`. Relative startup paths
use that process’s working directory. Otherwise it follows
[Niri’s user/system discovery rules](https://github.com/niri-wm/niri/blob/main/docs/wiki/Configuration%3A-Introduction.md).
Only relevant process metadata is retained; process environments are never
logged. If metadata is inaccessible, available environment/default paths are
used, with a diagnostic and an explicit override as the recovery option.

Both optional GSettings overrides take effect immediately:

```sh
gsettings set org.ldelossa.way-shell.window-manager sway-config-path '~/dotfiles/sway/config'
gsettings set org.ldelossa.way-shell.window-manager niri-config-path '/home/me/dotfiles/niri.kdl'
```

An empty string restores automatic discovery. Overrides accept absolute paths
and `~/`. An invalid or missing explicit path is reported without silently
choosing another file. Niri’s `load-config-file --path` event does not report the
new path: update `niri-config-path` when switching sources this way.

## Parsing and scope

`way-shell-shortcuts` is an unpublished, GTK-independent workspace library. Its
text and file-loading entry points accept environment/session context explicitly
and return bindings, dependency paths, and structured diagnostics.

Sway support includes `bindsym`, `bindcode`, their unbind commands, binding
options, variables, quoted and escaped arguments, comments, continued lines,
binding blocks and named modes. Includes support relative, absolute, home,
environment and glob paths, and are followed in order. Repeated and cyclic Sway
includes are skipped. Overrides retain distinctions such as release and device
restrictions. Command criteria and sequences remain visible as conditions and
in source details. Physical bindings use explicit keycode labels.

Niri is parsed with Knuffel, including KDL comments, disabled nodes, strings,
properties, native actions, `spawn`, and `spawn-sh`. Includes follow
[Niri’s positional merging rules](https://github.com/niri-wm/niri/blob/main/docs/wiki/Configuration%3A-Include.md),
including optional missing files. Duplicate bindings within one block are
diagnosed. `mod-key` and `mod-key-nested` are applied when the startup context is
known; otherwise `Mod` remains visible with a legend. Custom
`hotkey-overlay-title` strings are used for eligible actions, and explicit null
titles hide entries.

Native compositor actions and validated `way-sh` commands are eligible. The
executable may be an absolute path or appear inside a simple `env` or shell
wrapper. Arbitrary scripts, application launchers, lock tools, media utilities,
and external screenshot tools are omitted, even with custom titles. Native
compositor screenshot actions are included. Shell expansion and configuration
commands are never executed. Unknown native actions receive a literal fallback
label and source details, rather than a guessed description. All configuration
text is rendered as plain text.

Alternative keys combine only when their actions, arguments, mode, conditions
and descriptions match. Workspace targets stay explicit. This is a bounded
binding reader, not a complete compositor configuration validator: unsupported
or malformed syntax produces diagnostics. Limits are 2 MiB per file, 8 MiB
accumulated input and variable expansion, 4096 binding definitions, 32 nested
blocks/includes, and bounded include matches and
watched directories. A broken KDL section is omitted; independently valid
top-level parts can still be shown.

## Refresh and internal controls

Loading begins on first use and runs off the GTK thread. Configuration files,
include directories and recovery parents are monitored so edits, atomic saves,
new glob matches, deletion and recreation are detected. File events are
debounced by 150 ms. Hidden sheets become dirty and refresh on reopening.
Compositor reload/reconnection and override changes also trigger refresh.
Outdated workers cannot overwrite newer results or revive stopped UI.

On a parse failure, the previous complete result for the same source is shown
as stale; without one, independently parsed entries are marked incomplete.
Changing sources clears the old rows. Internal navigation is always available.
Retry and expandable diagnostics are available when loading fails.

The **Inside Way-Shell** section documents controls beside their implementing
components. Activities Escape clears search. The application switcher’s internal
controls require holding **Super**, including Escape to cancel; releasing Super
activates the selected window. Changing its opening binding does not change
these internal controls.

Stable CSS classes include `panel-shortcuts`, `panel-popup`,
`panel-popup-content`, `shortcuts-sheet`, `shortcuts-sections`,
`shortcuts-heading`, `shortcut-key`, `shortcuts-status`, and `shortcuts-details`.
Both bundled themes style the sheet. The shared `PanelPopup` owns placement,
focus, click-away surfaces, visibility and shutdown; feature services remain
independent. Clipboard history is not part of this feature.

## Verification

`cargo test --workspace` covers parser fixtures, discovery, stale-source
handling, command scope, and CLI/IPC compatibility. `shortcuts-smoke` runs in
the existing isolated Sway and nested-Niri component matrix, including an
800×600 display, both themes, atomic edits, malformed saves, missing-source
recovery, rapid updates, dismissal and shutdown. The full application runtime
test covers popup exclusion and IPC dispatch. Test configurations are private;
the user’s live compositor bindings are never changed.
