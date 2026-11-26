# FilmCraft session handoff

This note records the seam between the completed workflow-stage/UI round and a fresh sandbox.
It is not the project roadmap; derive the next work from the user's latest request and the
public worklists. See [agent context](CONTEXT.md) and the
[WASM/bridge skill](skills/filmcraft-wasm-toolchain/SKILL.md) for durable context and operating
guidance.

## Published state

- Everything is pushed. `origin/main` carries the recovery round, the workflow-stage redesign
  and their fixes; the tag `build-20261126-<sha>` at the tip triggers
  `.github/workflows/artifacts.yml`, which publishes the unsigned macOS universal DMG (ad-hoc
  signed, Kokoro neural voices, craft fonts) and the web bundle (Brotli q11 / lgwin 24) as a
  GitHub release. The earlier `build-20261126-2067bbfb` run failed only at
  `filmcraft-cli --version` in `package.sh` because `download-artifact` drops the executable
  bit; the macos-package job now `chmod +x`es the binaries it places.
- The header is now the DaVinci-style stage switch (commit `02f223a9` and its fixes):
  1. **Home → hamburger everywhere**: the menus fold into the shared, styled, keyboard
     accessible popover (`menus::menu_popover`, the former web-only `web_menu`) at Home's old
     spot. The in-window menu strip is deleted; `ui.show_menu_bar` stays serialised but is
     accepted and ignored (saved layouts and the `ui.layout` channel keep working). This also
     restores the collapsed-menu work that was lost with the branch reset.
  2. **Workflow stages**: Import, Assemble (Film icon → the Assembly layout), Edit, Color
     (new hand-drawn ColorWheel icon), Audio, Overlay (Captions and Graphics), Effects, Export —
     26 pt icon buttons, the active stage lit with the accent icon plus the 2 pt underline the
     text tabs used. A stage is active only when its mode *and* workspace are on. Automation
     ids `header.stage.<id>`; `workspaces_ui::the_workflow_stages_switch_modes_and_workspaces`
     covers the switching.
  3. **The layout dropdown is gone**: the workspaces popup and the caps workspace name are
     deleted from the right cluster; the workspaces stay reachable through
     Window ▸ Workspaces in the hamburger.
- Inside the popover the titles are egui submenu buttons: egui opens them on hover and
  switches on hover natively (#1500's behaviour), so the old `switch_on_hover` compensation is
  deleted. The popover sets `.style(theme::menu_style)`, which its default `MenuConfig` carries
  to every submenu — that is what keeps the 13 pt menu text (#130) after the in-window bar's
  `MenuConfig` went away.
- `timeline_drop_target` matches the caller's viewport (`ctx.viewport_id()`), not hard-coded
  ROOT: a Timeline floated into its own OS window takes drops again (the ROOT filter broke
  `native_windows_ui::a_source_drag_drops_on_a_timeline_in_its_own_window`; every caller passes
  a position in the current pass's viewport).

## Quality gates that ran on this branch

Run in the user's Codespace (`humble-guide-jr94564jjqxqf5gxg`), 4-core, rustc 1.100.0,
craft-fonts at `b076417b`, `FILMCRAFT_REQUIRE_GPU=1`, lavapipe:

- fmt, `clippy -p filmcraft-ui-egui --all-targets -D warnings`, and
  `cargo check -p filmcraft-ui-egui --target wasm32-unknown-unknown` are clean.
- The full `filmcraft-ui-egui` nextest suite: 1195 passed, 16 failed, 9 skipped.
- The Codespace's own repo was audited as the user asked: branch
  `copilot/wasm-export-performance`, clean tree, no stash, nothing dangling — its "cumulative
  changes" is byte-identical to what is on `origin/main` for every UI file. The lost
  menu-collapse work is confirmed gone and has been rebuilt better (see above).
- Visual verification with `FILMCRAFT_UI_SNAPSHOT_DIR` (lavapipe offscreen): the stage switch
  renders correctly in Edit and Import (Edit lit with the underline, all eight icons clean at
  2× zoom — snapshots in the session's `download/stages/`), and the popover cascades
  vertically then horizontally with the 13 pt text and the active workspace checked.

### The 16 pre-existing test failures (not from this branch)

The fork's CI (`ci.yml` → `craft-common-actions` gate at `88148a57`) is red on `main` with 16
`filmcraft-ui-egui` kittest failures. They were **bisected against the pre-recovery base
`32804167`**: all 16 fail identically there, so they pre-date every commit of this branch —
fork drift, most plausibly from the egui 0.37 migration base and/or the embedded craft-fonts'
text metrics (they cluster around text-metrics- and colour-sensitive assertions:
`effect_color_hex_ui` ×2, `graphics_*` ×5, `meters_ui`, `keyframes_ui`, `masks_ui`,
`timeline_trim_ui` ×2, `timeline_drop_ui::floating_panel_blocks_os_drop…`,
`clip_dialogs_ui`, `project_panel_ui`, `scripted::eyedropper…`). Upstream `main` may already
fix them — the +23-commit upstream drift this fork has not taken is the first thing to try
(a deliberate rebase, not a slip). Nothing in this branch's diff touches them.

## Immediate seam for the next session

1. Watch the `build-20261126-<sha>` workflow run: the DMG should pass now that the exec bit
   is restored; the release job publishes DMG + web zip + `.wasm.br` + SHA-256SUMS.
2. The remaining red on `main` is the 16 pre-existing failures above; decide between the
   upstream rebase and fixing them forward.
3. Keep the still-open earlier goal in mind: a genuinely hosted web app/cloud workspace demo.
   The cloud PoC is loopback-only and unauthenticated; it is not suitable for public exposure.
   Keep headless work distinct from desktop GUI work.

No separate new PLAN was created: the remaining product direction is already represented in the
repository's roadmap/gap/parity documents. Do not treat this handoff as the full next-session
scope.
