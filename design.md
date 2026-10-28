---
name: "Ferrum Workbench"
description: "Native desktop SSH workspace with compact host rows, a persistent inspector, and familiar blue actions. Approved native implementation."
platforms: ["macOS", "Windows", "Linux"]
colors:
  background: "#171B22"
  sidebar: "#12161C"
  surface: "#202630"
  surface-hover: "#293240"
  terminal: "#10141A"
  terminal-text: "#EEF1F6"
  text-primary: "#EEF1F6"
  text-secondary: "#A9B3C2"
  accent: "#789AFF"
  on-accent: "#0E1320"
  selection: "#243450"
  border: "#343E4D"
  control-border: "#65758B"
  focus: "#789AFF"
  success: "#6FCF9E"
  warning: "#E5BC75"
  danger: "#F18C96"
typography:
  display:
    fontFamily: "system-ui"
    fontSize: "32px"
    fontWeight: 600
    lineHeight: "1.2"
  title:
    fontFamily: "system-ui"
    fontSize: "24px"
    fontWeight: 600
    lineHeight: "1.25"
  body:
    fontFamily: "system-ui"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: "1.5"
  control:
    fontFamily: "system-ui"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: "1.4"
  caption:
    fontFamily: "system-ui"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "1.5"
  terminal:
    fontFamily: "Hack, ui-monospace, monospace"
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
    height: "64px"
    compact-height: "48px"
  chrome:
    titlebar-height: "48px"
    tabbar-height: "40px"
    statusbar-height: "32px"
    navigation-width: "208px"
    inspector-width: "288px"
motion:
  feedback: "160ms"
  panel: "200ms"
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
  terminal-copy:
    foreground: terminal-text
    background: terminal
    usage: text
---

## Overview

Ferrum Workbench is the approved refinement of the existing Rust/egui client for people managing several servers. Keep the hexagon identity, Phosphor icons, customizable terminal fonts, and blue action color. Use cool graphite surfaces, compact rows, and a right inspector to make host choice and connection details easier to scan. Native tokens in src/themes.rs are the implementation source of truth.

## Composition

The host library opens with a title and New host action, then search, group filters, a host table, and an inspector for the selected host. Connection addresses use monospace. Single-click selects; Connect or Enter opens a session. The terminal becomes the largest region during a session. Health and containers open in one optional utility panel. The session tab strip remains available when returning to the library.

## Colors

Dark is the primary interface theme. Light and Midnight retain the same semantic roles and geometry. All theme values are defined in src/themes.rs. Reserve accent for actions, keyboard focus, and selection; never use it for ordinary file rows or decorative fills. Green means a connected session or a reachable TCP port; use different labels for those states. Amber describes a warning or a production environment label. Red indicates failure or a destructive action. Structural dividers are subtle; controls use the stronger control-border token. The terminal theme stays independent of the interface theme.

The native interface uses actual connection state and marks stale health samples. Disposable visual QA uses explicit fixtures and does not probe demo addresses or save the vault. Arbitrary user accent colors still need the existing readable_on logic and contrast validation.

## Typography

Use the platform UI font: SF on macOS, Segoe UI on Windows, and the desktop sans fallback on Linux. Register a suitable bundled proportional fallback in egui rather than assuming CSS fonts exist there. Keep Hack as the terminal fallback and honor user fonts. Interface copy uses 14 px, explanations 16 px, titles 24 px, and metadata 12 px. Keep terminal output at 14 px by default. At the native layer, map these sizes to egui points and respect OS scale and Ferrum zoom. Use tabular figures for latency and file sizes.

## Layout

Use a 4 px spacing grid. Navigation is 208 px; its icon rail is 64 px. The host inspector starts at 288 px and can resize to 360 px. Standard host rows are 64 px; compact mode is 48 px. Use 24 px page insets and 16 px internal gaps. Controls have 8 px corners. Rows remain square and separated by rules. Desktop targets are 1440 × 900 and 1024 × 768. Below 1200 px, make the inspector an explicit overlay. Below 1080 px, collapse navigation. Preserve a full-width editor at narrow widths. Below 900 px, SFTP shows one pane at a time with a local/remote switch. The minimum supported native window is 760 × 480; shorter windows prioritize scrolling content and persistent actions.

## Components

Use existing egui controls and Phosphor glyphs, with semantic tokens in src/themes.rs. Primary buttons have a solid blue fill; secondary buttons have a strong neutral border; utility actions use text and an icon. Each icon action has a tooltip and an accessible name. Hover changes a surface; selection uses a fill plus a leading marker. Focus uses a 2 px accent outline with a 2 px offset. Do not remove keyboard outlines.

Host rows show label, address, protocol, group, and probe result. Serial has no TCP latency. Hosts behind a jump host or proxy show Probe skipped. Unknown and disabled probes show a dash with a reason, never a green dot. Row menus contain edit, duplicate, pin, move, and delete. Keep visible Connect and Edit actions in the inspector; a menu is supplementary.

The editor groups Address, Authentication, and Advanced options. Show inherited settings with their source. Protocol changes reveal the matching fields; Serial uses device and baud, Telnet has no SSH key, and Mosh retains its SSH bootstrap settings. Save keeps the library open; Save and connect opens a session. Keep test feedback next to Test connection and keep the footer visible while fields scroll.

SFTP keeps local and remote labels, path breadcrumbs, file rows, and a directional transfer action. Selection precedes transfer. The queue shows bytes, percent, state, cancel, and retry. Terminal tools use one panel for health or containers. Broadcast input stays off by default; enabled mode names the target panes and remains visible above the terminal. AI stays an optional command tool and never executes proposed commands automatically.

State requirements: loading preserves layout and names the operation; empty libraries offer Add host and Import hosts; no matches offer Clear filters; connection errors retain input and offer Retry or Edit host. Authentication failures never report a password value. Host-key changes block connection with old/new fingerprints and an explicit trust decision. Transfer errors retain the file and destination with Retry. Disabled controls explain the unmet prerequisite. Long labels truncate only with an accessible full name; errors and commands wrap. Support keyboard selection and reveal focused rows in long lists.

## Graphics & Effects

Retain the existing painted hexagon and bundled Phosphor icons. Avoid stock imagery, ornamental charts, gradients, and translucent panels. Charts are allowed only for measured health samples, with a timestamp and units. No imagery or network fonts are required. A small shadow is reserved for overlays; normal panels use shared edges and borders.

## Motion

Use 160 ms color changes and a 200 ms panel entrance. Avoid animated terminal reflow. Progress comes from measured bytes. Reduced motion disables panel travel and nonessential transitions. Connection state changes appear immediately with a textual label.

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

The host library defaults to List while retaining saved Grid preferences. A group becomes a filter within the library so users keep context when browsing. Existing group inheritance remains unchanged. Search filters saved hosts; Quick connect is a distinct command in the command palette so Enter has one meaning. For an empty library, expose Quick connect beside Add host and Import hosts.

Keep the existing keyboard commands, including command palette with Cmd/Ctrl+K and local terminal with Cmd/Ctrl+T. Host-list keys are Up/Down for selection and Enter for Connect. In the terminal, application shortcuts must preserve terminal input and remapping behavior. Display platform-correct shortcut labels.

Navigation groups are Connect (Hosts, Workspaces), Tools (Keychain, Port forwarding, Snippets), and Records (Known hosts, History and logs). Settings is anchored at the bottom. All navigation destinations remain available in the native application.

Other screens: Keychain uses a searchable key/identity list with fingerprint details; Port forwarding uses rules with source, destination, and run state; Snippets uses packages with a command preview and a named run target; Workspaces uses saved layouts and a preview of included hosts; Known hosts uses hostname, algorithm, and fingerprint rows; History and logs separates commands from connection events. Vault unlock offers password and available biometric options, with import as a secondary action. Settings retains its nine existing sections and uses a stable section list rather than wrapping tabs.

## Native implementation

1. Map tokens to the existing Palette and egui styles; retain theme and accent preferences.
2. Refine chrome, navigation, host list, inspector, and host editor without changing connection storage.
3. Refine terminal tools, SFTP file rows, and transfer feedback using existing backend state.
4. Apply list/detail patterns to keys, tunnels, snippets, workspaces, records, settings, and vault states.
5. Check native keyboard focus, display scaling, narrow windows, all themes, host-key prompts, and terminal selection.

The native implementation enables AccessKit and has been checked with keyboard selection and local transfers. Windows/Linux runtime behavior, screen-reader interaction, input methods, and remote integration still need platform-specific validation.
