# Ports

A desktop port killer: a main window plus a menu bar (tray) popover that list the servers listening on this machine, with their memory and CPU, and let you open or stop them.

## Stack

- Tauri v2 (Rust) in `src-tauri/`. The tray icon, the popover window and window behavior live in `src-tauri/src/lib.rs`.
- SvelteKit (SPA, Svelte 5 runes) in `src/`. `/` is the main window, `/tray` is the popover.
- shadcn-svelte primitives in `src/lib/components/ui/`, Tailwind CSS v4, Lucide icons.
- Package manager: `bun`. Run the app with `bun tauri dev`; check types with `bun run check`.
- Lint with `bun run lint` (Oxlint), format with `bun run format` (Oxfmt), test with `bun run test` (Vitest) and `cargo test` in `src-tauri/`. CI in `.github/workflows/ci.yml` runs all of them plus `cargo fmt --check` and `cargo clippy`.

## UI rules

- Use primitives. Build every screen from the shadcn-svelte components in `$lib/components/ui` before writing custom markup; add a missing one with `bunx --bun shadcn-svelte@latest add <name>`.
- Use CSS variables for every color. Define tokens in `src/routes/layout.css` (light and dark) and use them through Tailwind semantic classes (`bg-background`, `text-muted-foreground`, `text-warning`). Never hardcode color values in components.
- Follow Apple's design approach: system font, restraint, short animations on `transform` and `opacity`, and respect `prefers-reduced-motion` and `prefers-reduced-transparency`.
