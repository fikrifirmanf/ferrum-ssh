---
name: "Ferrum Forge"
description: "Native desktop SSH workspace in warm iron neutrals with one ember accent, a Prompt F mark, a shared form kit and consistent motion. Approved native implementation."
platforms: ["macOS", "Windows", "Linux"]
colors:
  background: "#141416"
  sidebar: "#101012"
  surface: "#1C1C1F"
  surface-hover: "#26262A"
  terminal: "#0E0E10"
  terminal-text: "#E8E6E1"
  text-primary: "#F2F0EB"
  text-secondary: "#ADABA5"
  text-faint: "#918F89"
  accent: "#FF8A4C"
  on-accent: "#1C0D05"
  selection: "#2E2019"
  border: "#2E2E33"
  control-border: "#6E6E76"
  focus: "#FF8A4C"
  success: "#6FD6A0"
  warning: "#EBC85A"
  danger: "#F7808C"
  data: "#8FB6DC"
typography:
  display:
    fontFamily: "Geist"
    fontSize: "28px"
    fontWeight: 600
    lineHeight: "1.2"
  title:
    fontFamily: "Geist"
    fontSize: "22px"
    fontWeight: 600
    lineHeight: "1.25"
  body:
    fontFamily: "Geist"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "1.45"
  control:
    fontFamily: "Geist"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: "1.4"
  caption:
    fontFamily: "Geist"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "1.5"
  terminal:
    fontFamily: "Geist Mono, Hack, ui-monospace, monospace"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "1.5"
spacing:
  base: "4px"
  xs: "4px"
  sm: "8px"
  inset: "12px"
  md: "16px"
  lg: "24px"
  xl: "32px"
  section: "48px"
rounded:
  small: "4px"
  control: "8px"
  window: "12px"
components:
  button-primary:
    background: "{colors.accent}"
    text-color: "{colors.on-accent}"
    radius: "{rounded.control}"
    height: "36px"
  input:
    height: "36px"
    radius: "{rounded.control}"
    border: "{colors.control-border}"
  button-secondary:
    background: "{colors.surface}"
    text-color: "{colors.text-primary}"
    border: "{colors.control-border}"
    height: "36px"
  button-ghost:
    text-color: "{colors.text-secondary}"
    height: "36px"
  host-row:
    height: "56px"
    compact-height: "42px"
  chrome:
    titlebar-height: "44px"
    statusbar-height: "28px"
    navigation-width: "216px"
    rail-width: "56px"
    inspector-width: "320px"
motion:
  fast: "120ms"
  control: "180ms"
  panel: "240ms"
  exit: "140ms"
contrast:
  primary-on-page:
    foreground: text-primary
    background: background
    usage: text
  primary-on-surface:
    foreground: text-primary
    background: surface
    usage: text
  secondary-on-page:
    foreground: text-secondary
    background: background
    usage: text
  secondary-on-surface:
    foreground: text-secondary
    background: surface
    usage: text
  secondary-on-selection:
    foreground: text-secondary
    background: selection
    usage: text
  secondary-on-hover:
    foreground: text-secondary
    background: surface-hover
    usage: text
  faint-on-page:
    foreground: text-faint
    background: background
    usage: text
  accent-on-page:
    foreground: accent
    background: background
    usage: text
  accent-on-surface:
    foreground: accent
    background: surface
    usage: text
  accent-on-selection:
    foreground: accent
    background: selection
    usage: text
  action-label:
    foreground: on-accent
    background: accent
    usage: text
  control-boundary:
    foreground: control-border
    background: surface
    usage: ui
  focus-on-page:
    foreground: focus
    background: background
    usage: ui
  success-on-page:
    foreground: success
    background: background
    usage: text
  warning-on-page:
    foreground: warning
    background: background
    usage: text
  danger-on-page:
    foreground: danger
    background: background
    usage: text
  data-on-surface:
    foreground: data
    background: surface
    usage: ui
  terminal-copy:
    foreground: terminal-text
    background: terminal
    usage: text
---

## Overview

Ferrum Forge is the approved redesign of the Rust/egui client for people managing several servers. Warm iron surfaces, a single ember accent, the Prompt F mark, Phosphor icons and customizable terminal fonts give it one recognizable look. Native tokens in src/themes.rs are the implementation source of truth; shared widgets live in src/ui/forms.rs and shared timing in src/ui/motion.rs.

## Composition

The host library opens with one toolbar (title, count, search, density, Import, New host), then folding groups of hosts and an inspector for the selected host. Connection addresses use monospace. Single-click selects; Connect or Enter opens a session. The terminal becomes the largest region during a session. Health and containers open in one optional utility panel. The session tab strip remains available when returning to the library.

## Colors

Iron (the Dark setting) is the primary interface theme. Light and Midnight retain the same semantic roles and geometry. All theme values are defined in src/themes.rs. Reserve ember for actions, keyboard focus, and selection; never use it for ordinary file rows, measured data or decorative fills. Measured values use the steel data color. Vaults saved with the old blue default move to ember once (Settings::migrate_accent); Cobalt stays available as a choice. Green means a connected session or a reachable TCP port; use different labels for those states. Amber describes a warning or a production environment label. Red indicates failure or a destructive action. Structural dividers are subtle; controls use the stronger control-border token. The terminal theme stays independent of the interface theme.

The native interface uses actual connection state and marks stale health samples. Disposable visual QA uses explicit fixtures and does not probe demo addresses or save the vault. Arbitrary user accent colors still need the existing readable_on logic and contrast validation.

## Typography

Geist and Geist Mono are bundled under the SIL Open Font License; the platform UI font stays behind them for scripts Geist lacks. egui has no font weights, so Medium and SemiBold are separate families (fonts::medium, fonts::semibold). Hack remains the terminal fallback and user fonts win. Interface copy uses 14 px, titles 22 px, and metadata 12 px. Keep terminal output at 14 px by default. At the native layer, map these sizes to egui points and respect OS scale and Ferrum zoom. Use tabular figures for latency and file sizes.

## Layout

Use a 4 px spacing grid. Navigation is 216 px; its icon rail is 56 px, and the rail is also used while an editor is open. The host inspector starts at 320 px and resizes from 288 to 360 px. Standard host rows are 56 px; compact mode is 42 px. Use 24 px page insets and 12 to 16 px internal gaps. Controls have 8 px corners, cards 12 px. Desktop targets are 1440 × 900 and 1024 × 768. Below 1200 px, make the inspector an explicit overlay. Below 1080 px, collapse navigation. Preserve a full-width editor at narrow widths. Below 900 px, SFTP shows one pane at a time with a local/remote switch. The minimum supported native window is 760 × 480; shorter windows prioritize scrolling content and persistent actions.

## Components

Use the widgets in src/ui/forms.rs (Btn, Input, address_line, segmented, switch, select, section, reveal, tag_input, command_line) and Phosphor glyphs, with semantic tokens in src/themes.rs. Primary buttons have a solid ember fill; secondary buttons have a strong neutral border; utility actions use text and an icon. Each icon action has a tooltip and an accessible name. Hover changes a surface; selection uses a fill plus a leading marker. Buttons take a 2 px accent outline with a 2 px offset on focus. Inputs warm their border, gain a soft ring and grow a 2 px heat line from the center of the bottom edge. Do not remove keyboard focus states.

Host rows show label, address, protocol, group, and probe result. Serial has no TCP latency. Hosts behind a jump host or proxy show Probe skipped. Unknown and disabled probes show a dash with a reason, never a green dot. Row menus contain edit, duplicate, pin, move, and delete. Keep visible Connect and Edit actions in the inspector; a menu is supplementary.

The editor is one 760 px column: the live equivalent command, a card with protocol, the `user@host:port` address line, label, group and tags, then folding sections for Authentication, Route and proxy, Behavior, Terminal and Notes. A closed section summarizes itself in its header. Show inherited settings with a dashed edge and their source. The Authentication segments are views onto settings that can all be set at once, so switching never erases a saved key or password. The command never contains a password or vault key. Protocol changes reveal the matching fields; Serial uses device and baud, Telnet has no SSH key, and Mosh retains its SSH bootstrap settings. Save keeps the library open; Save and connect opens a session. Keep test feedback next to Test connection and keep the footer visible while fields scroll.

SFTP keeps local and remote labels, path breadcrumbs, file rows, and a directional transfer action. Selection precedes transfer. The queue shows bytes, percent, state, cancel, and retry. Terminal tools use one panel for health or containers. Broadcast input stays off by default; enabled mode names the target panes and remains visible above the terminal. AI stays an optional command tool and never executes proposed commands automatically.

The container sidebar offers Open manager for a full-width workspace in the current SSH tab. Use a searchable table grouped by Compose project or Kubernetes namespace, with Logs, Stats, Inspect and Terminal below the selected row. Measured resources use steel and include update age. At short window heights, show either the list or selected details with an All containers return action. Keep the host terminal available through Back to terminal; container shells open in separate terminal tabs.

State requirements: loading preserves layout and names the operation; empty libraries offer Add host and Import hosts; no matches offer Clear filters; connection errors retain input and offer Retry or Edit host. Authentication failures never report a password value. Host-key changes block connection with old/new fingerprints and an explicit trust decision. Transfer errors retain the file and destination with Retry. Disabled controls explain the unmet prerequisite. Long labels truncate only with an accessible full name; errors and commands wrap. Support keyboard selection and reveal focused rows in long lists.

## Graphics & Effects

The mark is Prompt F: the F is the name, the ember `>_` is the job. In the app it is painted from convex polygons (ui::logo_mark), with a simplified drawing below 32 pt. App icon masters are packaging/icons/ferrum-icon.svg and ferrum-icon-small.svg; generate_icons.py renders every PNG and icon.ico. Avoid stock imagery, ornamental charts and translucent panels; gradients belong only to the app icon. Charts are allowed only for measured health samples, with a timestamp and units. No imagery or network fonts are required. A small shadow is reserved for overlays; normal panels use shared edges and borders.

## Motion

One curve (cubic out) and three durations: 120 ms for hover and press, 180 ms for switches, segments and chevrons, 240 ms for sections, inspector, navigation and page entrance. Exits run at 140 ms. Terminal text never animates: a session tool panel reflows the terminal once and only the panel contents fade in. Progress comes from measured bytes. Connection state changes appear immediately with a word. Host-key and credential prompts block at once with no entrance. Reduce motion (a setting, plus the macOS and GNOME system preference) lands every animation on its final value.

## Guardrails

- Keep Hosts, Keychain, Port forwarding, Snippets, Workspaces, Known hosts, History and logs, and Settings reachable.
- Keep shell output and session actions visually distinct from application navigation.
- Never equate a successful TCP probe with an authenticated SSH session.
- Never show CPU, uptime, or health figures without an SSH snapshot and update age.
- Preserve custom terminal schemes and user font choices when changing interface themes.
- Never hide the only Connect action behind hover or a context menu.
- Keep host-key warnings blocking and destructive actions explicitly named.
- Do not replace the host library with a decorative metric dashboard or repeated large cards.

## UX changes and scope

The host library defaults to List while retaining saved Grid preferences. Groups fold in place, and Show one group narrows the list with a removable chip. Existing group inheritance remains unchanged. Search filters saved hosts; Quick connect is a distinct command in the command palette so Enter has one meaning. For an empty library, expose Quick connect beside Add host and Import hosts.

Keep the existing keyboard commands, including command palette with Cmd/Ctrl+K and local terminal with Cmd/Ctrl+T. Host-list keys are Up/Down for selection and Enter for Connect. In the terminal, application shortcuts must preserve terminal input and remapping behavior. Display platform-correct shortcut labels.

Navigation groups are Connect (Hosts, Workspaces), Tools (Keychain, Port forwarding, Snippets), and Records (Known hosts, History and logs). Settings is anchored at the bottom. All navigation destinations remain available in the native application.

Other screens: Keychain uses a searchable key/identity list with fingerprint details; Port forwarding uses rules with source, destination, and run state; Snippets uses packages with a command preview and a named run target; Workspaces uses saved layouts and a preview of included hosts; Known hosts uses hostname, algorithm, and fingerprint rows; History and logs separates commands from connection events. Vault unlock offers password and available biometric options, with import as a secondary action. Settings retains its nine existing sections and uses a stable section list rather than wrapping tabs.

## Native implementation

Connection storage and backend behavior are unchanged by this design. New Settings fields (`accent_migrated`, `reduce_motion`) default safely for older vaults. When changing screens, check native keyboard focus, display scaling, narrow windows, all themes, host-key prompts, and terminal selection.

The native implementation enables AccessKit and has been checked with keyboard selection and local transfers. Windows/Linux runtime behavior, screen-reader interaction, input methods, and remote integration still need platform-specific validation.
