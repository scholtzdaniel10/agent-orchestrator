---
name: Legate Roman Command Workspace
description: A terminal-first Roman command station for coding agents.
colors:
  bg: "rgb(17, 18, 21)"
  brand: "rgb(214, 38, 46)"
  text: "rgb(242, 242, 242)"
  on-accent: "rgb(255, 255, 255)"
  signal: "rgb(255, 230, 0)"
  standard: "#c9b17f"
  standard-paper: "#806128"
  paper-bg: "rgb(248, 244, 238)"
  paper-text: "rgb(32, 22, 20)"
  tint: "color-mix(in srgb, var(--brand) 2%, var(--bg))"
  inset: "color-mix(in srgb, var(--text) 3%, var(--tint))"
  deep: "color-mix(in srgb, var(--text) 5%, var(--tint))"
  raised: "color-mix(in srgb, var(--text) 8%, var(--tint))"
  border: "color-mix(in srgb, var(--text) 11%, var(--tint))"
  muted: "color-mix(in srgb, var(--text) 64%, var(--bg))"
typography:
  display:
    fontFamily: "Georgia, 'Times New Roman', serif"
    fontSize: "26px"
    fontWeight: 400
  wordmark:
    fontFamily: "Georgia, 'Times New Roman', serif"
    fontSize: "20px"
    fontWeight: 400
    letterSpacing: ".13em"
  body:
    fontFamily: "'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  title:
    fontSize: "13px"
    fontWeight: 600
  label:
    fontSize: "10px"
    lineHeight: 1.2
  branch:
    fontFamily: "'JetBrainsMono Nerd Font', 'JetBrainsMono NF', 'JetBrainsMono Nerd Font Mono', 'JetBrains Mono', 'Cascadia Code', 'Cascadia Mono', Consolas, ui-monospace, monospace"
    fontSize: "11px"
rounded:
  sm: "4px"
  field: "6px"
  default: "7px"
  workspace: "10px"
  lg: "11px"
  pill: "999px"
spacing:
  one: "1px"
  two: "2px"
  four: "4px"
  six: "6px"
  eight: "8px"
  ten: "10px"
  twelve: "12px"
  sixteen: "16px"
  twenty-four: "24px"
components:
  command-destination-active:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.standard}"
    rounded: "{rounded.default}"
    padding: "7px 2px"
  terminal-stage:
    backgroundColor: "{colors.bg}"
    rounded: "{rounded.workspace}"
  inspector:
    backgroundColor: "{colors.inset}"
    rounded: "{rounded.workspace}"
  new-task:
    backgroundColor: "{colors.brand}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.field}"
    height: "32px"
    padding: "0 12px"
---

# Design System: Legate Roman Command Workspace

## Overview

**Creative North Star: "Roman command station"**

Terminals own the working area. The permanent tool rail, resizable project list, inset terminal stage and focused inspector establish a working hierarchy. Roman identity comes from the laurel standard, serif wordmark, shield-shaped empty-state seal and antique gold details. Dense operational text uses the system UI face.

This documentation is source-derived from `apps/ui/src/app.rs`, the cascade in `apps/ui/assets/main.css`, and `.impeccable/surfaces/workspace.md`. No screenshot exists for this revision; rendered visual verification is pending. The tokens describe the default dark theme and explicitly named paper overrides; other existing selectable themes remain supported by the CSS.

**Key Characteristics:**

- Permanent navigation with icon and text labels.
- Flat recessed panels and thin dividers.
- Antique gold identity and selection accents; red brand actions and count badges in the default theme.
- Restrained serif display typography beside practical UI and monospace text.

## Colors

The default palette combines charcoal stone, red brand color, pale text and antique gold standards. Preserve the CSS color-mix expressions as the source of truth rather than replacing them with estimated hex values.

Gold `standard` colors the laurel, active tool labels, selected project folder icons, inspector icons and campaign seal. Paper and automatic light mode use `standard-paper`. Brand red drives accent borders, brand controls and navigation count badges. The signal color marks keyboard focus and working status; it is a separate semantic role from gold. Existing provider, diff and error colors retain their operational meanings.

The paper theme changes base and mixed surfaces, text, borders, signal and provider colors. The frontmatter's dark mixtures are not a replacement for the theme-specific declarations. Keep new work bound to the existing CSS variables.

## Typography

Georgia with Times New Roman and serif fallbacks gives the wordmark and campaign heading their Roman character. The wordmark is 20px, weight 400, with .13em tracking; the campaign heading is 26px, weight 400. The decorative seal letter is 38px.

The UI uses Segoe UI Variable Text, Segoe UI and system fallbacks at 13px/1.5, weight 400. Inspector headings are 13px/600; project names use weight 600. Rail labels use 10px/1.2; command status uses 12px and the footer 11px. Branch chips use the existing monospace stack at 11px. Counts and status figures use tabular numerals. Empty-state copy is limited to 34ch with line-height 1.6.

## Layout

The application grid has a 56px header, a flexible `minmax(0, 1fr)` working row and a 28px status bar. The working row is a clipped flex container. A permanent 76px rail precedes the existing resizable sidebar and terminal workspace; supporting tools occupy the focused inspector. Rail destinations scroll independently and Settings stays at the bottom.

The header has 22px left and 20px right padding, a 30px standard, project/branch context, active count and a 32px-high New task control. The inspector has a 44px heading with a named tool and close button; its content receives 16px padding. Terminal tabs have a minimum 44px height.

The terminal stage has 10px outer margins, a one-pixel border and 10px corners. The inspector has `10px 10px 10px 0` margins and the same border/corners. Sidebar search receives 12px horizontal/bottom padding; project rows use 3px/8px margins and 10px internal padding. The footer begins at 88px to align beyond the tool rail.

At 1000px and below, hide the branch chip and active count, reduce header padding to 12px, context padding to 12px, header tool gap to 6px and workspace margin to 6px. Existing layout-state logic still controls sidebar/drawer sizing and inspector overlay behavior. This breakpoint does not introduce a mobile navigation replacement. Viewport calculations reserve 106px horizontally (76px rail plus 30px stage/inspector margins) and 36px vertically (16px inherited allowance plus 20px workspace margins) for the working area; preserve these insets when changing geometry. Standalone tool windows retain their existing layouts.

## Elevation & Depth

The Roman workspace uses tonal surface separation and one-pixel borders rather than new workspace shadows. `bg`, `inset` and `raised` distinguish working surfaces and active states. Preserve existing overlay-specific treatments outside this workspace scope.

## Shapes

Rail destinations use 7px corners, search and buttons 6px, branch/count chips 4px, and the terminal/inspector 10px. The 64px by 76px campaign seal uses `4px 4px 24px 24px` corners. Avoid extending this decorative silhouette to ordinary controls.

## Components

**Tool navigation.** Each destination combines an 18px icon and visible label in a minimum 49px-high button. Resting text is muted; hover uses inset fill and normal text; active uses raised fill, border and gold text. State is also exposed with `aria-pressed`. Count badges use brand fill and on-accent text; Lead working status uses a 6px signal dot with an accessible label.

**Actions.** New task opens the task dialog; New window remains adjacent. Primary controls use brand-accent fill and border with on-accent text, enforced by the final scoped `.roman-workspace .btn-primary` rule. Hover opacity is .92 and disabled opacity .45. The global keyboard-focus outline is 2px signal with 2px offset; rail buttons use -2px offset.

**Project list.** Selection uses raised fill, a visible border, gold folder icon and semibold name. Search retains a bordered bg field at minimum 32px height. Preserve the existing project/worktree selection and resize behavior.

**Inspector.** Permanent rail navigation selects the tool; the right inspector presents the active tool heading and explicit Close inspector action. Existing docked tools remain available.

**Empty stage.** A gold campaign seal and serif campaign heading sit above readable, narrow supporting copy and the task action. Empty content receives 32px padding and 14px gaps.

**Motion.** The workspace adds no decorative animation. Global reduced-motion rules reduce animations/transitions to .01ms, run animations once and force automatic scrolling. The select chevron additionally disables its transition. Dragging disables transitions. Preserve these accommodations.

## Do's and Don'ts

- **Do** preserve the terminal-first hierarchy and permanent labeled tool rail.
- **Do** distinguish gold identity/selection from brand actions and signal feedback.
- **Do** use theme variables and preserve paper/light overrides.
- **Do** retain visible keyboard focus, accessible state and reduced-motion behavior.
- **Don't** replace operational UI copy with decorative Roman vocabulary that obscures its purpose.
- **Don't** claim rendered visual approval from this source-derived document.


## Usage inspector

Provider sections use 24px avatars, 14px names, 26px observed percentages and 12px supporting text. Keep a 12px internal rhythm and 18px separation between providers. Never show a subscription percentage for Local, or a percentage without an observed reading. Display unknown usage explicitly. Model/version controls belong in Agent settings disclosure; the optional Cursor dashboard form uses two labeled columns, 34px themed fields, validation, saving state and server error feedback. Keep the percentage as raw input text while typing.

This Usage layout is source-reviewed; the rejected screenshot is reference evidence only. A new rendered visual check remains pending.

## Project workspaces and workflow setup

Panel placement, split selection, worktree selection and terminal focus belong to the project, not to a fixed Roman layout. The header Layout disclosure moves any tool among right, left, top and bottom docks. Workspace snapshots use `legate.project.v1:workspace:<project>`; pane seats, pane focus and unsent drafts use separate keys under the same project prefix. Only the main workspace owns layout snapshots; terminal/panel pop-outs do not rewrite them.

Switching uses a short busy state and a request generation so older asynchronous responses cannot replace the selected project's content. Lead replies run in independent backend sessions with project-bound tools. Settings > Projects provides an editable workflow goal and a previewable, copyable agent setup prompt. Copying generates guidance; it does not execute setup automatically.

These changes have source review and automated checks; final rendered interaction review remains pending.
