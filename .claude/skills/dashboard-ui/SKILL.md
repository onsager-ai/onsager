---
name: dashboard-ui
description: Enforce shadcn/ui component usage in apps/dashboard. Native HTML form and interactive elements (input, select, button, textarea, checkbox, radio, dialog, etc.) are forbidden — use the shadcn/ui primitives under @/components/ui instead. Trigger when editing or creating .tsx files under apps/dashboard/, when adding forms/buttons/inputs/selects/modals to the web app, or when the user mentions "shadcn", "UI component", "form control", or "dashboard component".
---

# dashboard-ui

The Onsager dashboard (`apps/dashboard`) standardises on **shadcn/ui** for all
interactive and form UI. Native HTML elements for these controls are not
allowed — they bypass the design system, theming (next-themes + CSS variables),
and accessibility behavior baked into the shadcn primitives.

## The rule

In any `.tsx` under `apps/dashboard/src/` — **except** for the shadcn
primitives themselves in `apps/dashboard/src/components/ui/**`, which
legitimately wrap native elements:

- **Do not** write `<input>`, `<select>`, `<textarea>`, `<button>`,
  `<option>`, native `<dialog>`, native checkbox/radio `<input type="...">`,
  or an unstyled `<a>` that behaves like a button.
- **Do** import the shadcn equivalent from `@/components/ui/<name>`.
- Plain structural/text tags (`<div>`, `<span>`, `<p>`, `<h1>`–`<h6>`,
  `<ul>`, `<li>`, `<form>`, `<label>`, `<a>` for real navigation, etc.) are
  fine — the rule is specifically about interactive / form controls.

## Mapping

| Native                                  | Use instead                                     |
| --------------------------------------- | ----------------------------------------------- |
| `<button>`                              | `Button` from `@/components/ui/button`          |
| `<input type="text\|email\|...">`       | `Input` from `@/components/ui/input`            |
| `<input type="checkbox">`               | `Checkbox` from `@/components/ui/checkbox`      |
| `<input type="radio">`                  | `RadioGroup` from `@/components/ui/radio-group` |
| `<textarea>`                            | `Textarea` from `@/components/ui/textarea`      |
| `<select>` / `<option>`                 | `Select` from `@/components/ui/select`          |
| `<dialog>` / custom modal               | `Dialog` from `@/components/ui/dialog`          |
| Drawer / off-canvas                     | `Sheet` from `@/components/ui/sheet`            |
| Tooltip                                 | `Tooltip` from `@/components/ui/tooltip`        |
| Dropdown menu                           | `DropdownMenu` from `@/components/ui/dropdown-menu` |
| Tabs                                    | `Tabs` from `@/components/ui/tabs`              |
| Table                                   | `Table` from `@/components/ui/table`            |

## Installed components

These are already present under `apps/dashboard/src/components/ui/`:

`badge`, `button`, `card`, `dropdown-menu`, `input`, `scroll-area`,
`select`, `separator`, `sheet`, `sidebar`, `skeleton`, `table`, `tabs`,
`textarea`, `tooltip`.

If you need a component that isn't in the list (e.g. `dialog`, `checkbox`,
`radio-group`, `form`, `switch`), add it with:

```bash
cd apps/dashboard
pnpm dlx shadcn@latest add <name>
```

The CLI writes the file to `src/components/ui/<name>.tsx` using
`components.json` (style `base-nova`, neutral base color, `@/components/ui`
alias). Commit the generated file alongside the change that uses it.

## Checking existing code

Before claiming a dashboard change is done, grep for violations in files you
touched:

```bash
rg -n '<(button|input|select|textarea|option|dialog)[ />]' apps/dashboard/src \
  --glob '!apps/dashboard/src/components/ui/**'
```

Any hit in your diff should be replaced with the shadcn primitive.

## Why

- Consistent theming via CSS variables + `next-themes` dark mode.
- Keyboard / focus / ARIA handling lives in the primitive, not in each call site.
- Styling lives in one place (the `ui/` component) — ad-hoc native elements
  diverge quickly and are expensive to re-skin later.
