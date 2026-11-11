# Roadmap

Each milestone gets its own design document in `docs/designs/`.

## M0: RDP paste prototype (`rdppaste`)

- [x] P1: workspace skeleton, `files-core` rules, CI
- [x] P2: `files-win` (clipboard, unbuffered I/O, delete-on-close, test fake)
- [x] P3: `files-jobs` (job runner, paste job, end-to-end tests)
- [x] P4: `rdppaste` command line
- [x] P5: documentation (handoff, architecture, test procedure, ADRs)
- [x] Manual RDP run: CUJ 1, 2 and 3 on a real server (2026-11-11)
- [ ] CUJ 6 (session drop) and the Q6 cases
- [ ] G2 measured with `typeperf`; Q1 freeze baseline recorded

## M1: first window

- [x] GPUI spike, to confirm RAM, OLE drop and the context menu on Windows
- [x] `files-app` (view state) and `files-ui-gpui` (rendering)
- [x] single-pane list; Ctrl+V pastes through a child `rdppaste` process (Q8)
- [x] interactive conflict handling (Q5): replace, skip existing, keep both

## M2: local operations

- [x] job queue with concurrency limits: not needed yet, see ADR 0005
- [x] local copy, move and delete through Windows APIs (IFileOperation): Ctrl+C/X/V, Delete, drag and drop
- [x] background folder sizes

## M3: daily use

- [x] tabs (Ctrl+T/W, Ctrl+Tab), two panes (F6, Ctrl+F6), favourites sidebar (Ctrl+D) with drives
- [x] copy path without quotes (toolbar button, Ctrl+Alt+C; Ctrl+Shift+C is often taken globally, e.g. by ShareX)

## M4: appearance

- [x] themes: dark, light, wallpaper-based (the Windows accent colour), any accent colour
- [x] density: normal, compact, very compact
- [ ] later, in a settings page: user-adjustable density steps
- [x] Polish and English UI (rdppaste's own messages stay English)

## M5: Windows integration

- [ ] mapping network drives
- [ ] native context menu

## M6: optional extras (each can be turned off)

- [ ] CLI integration
- [ ] updates from GitHub releases
- [ ] replacing Windows Explorer, with `--restore-explorer`
