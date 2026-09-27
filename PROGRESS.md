# Handoff: Bankrupt! cursor spike and MCFC fixes

Updated 2026-09-27. Bankrupt! is a property-trading board game for Minecraft Realms, built with MCFC. It lives in `C:\Users\Oliver\Documents\Development\bankrupt`.

- Design doc: https://marcusson.dev/bankrupt-design. It's a Pagina dev page, so uploads publish immediately.
- Test world: Prism instance `FO 26.3`, save `Bankrupt! Dev` (void world; the board builds itself once at 0 64 0). `Bankrupt Testing (1)` is the old spike world.

## Now: M2 core loop (bankrupt `main`, not pushed: no remote)

- Modules: `main.mcf` (entry points), `board.mcf`, `rig.mcf` (camera/cursor), `tokens.mcf` (NoAI baby mobs at scale 1.5 that walk through tile middles to their seat spot), `game.mcf` (seats, turns, buy, rent, sidebar).
- Works in game: join, hot seat, roll, walk, land, sidebar with spawn-egg icons and right-aligned diamonds. The token faces the player for 15 ticks before the dialog opens.
- Left for M2: a two-player test, and the README.
- Lessons: a module's `tick()` merges into the pack tick, so never call it (now a compile error). A tick that hits the command limit used to leak recursion frames into storage (180 MB, then OOM); load now clears `runtime.stack`.
- The VS Code extension bundles its own `mcfc-lsp`. Repackage (`npm run package:win32-x64` in `editors/vscode-mcfc`) and reinstall after compiler changes, or the editor shows stale errors.

## Earlier

**M0 passed on a Realm. M1 committed** (bankrupt `9b7e05d`): full 40-space board from `src/board.mcf`, the highlight resizes per tile, the action bar shows name and price, the Space dialog has Roll dice, and the token hops 2d6 with the camera following. Tile labels were dropped (unreadable at camera distance); polish waits for a resource pack, so keep visuals simple until then. MCFC `82a8f08` fixed imported records in `@WorldState`/`@PlayerState`.

**Pack trimmed** (bankrupt `9e3e858`): only the board, the spectator camera, the keyboard cursor, the highlight and the dialog remain. The spike-1 plan below is history; `rigTick` is now the spectator path.

**Spike 1 passes locally** (2026-09-27, bankrupt `79e3b82`):
- The keyboard cursor works while spectating, with key repeat.
- The camera follows the hovered tile.
- The highlight is aligned.
- The dialog gives a mouse pointer, and Buy and Pass fire.

Open items:
- Dialog price spacing. `"for " + sprite + "3"` has a wide gap before the sprite; without the space it's cramped. The default font has no thin space, so a fix needs a custom space font.
- Realm test.

## Committed this session (MCFC `main`)

- `3a74f30`: `Entity.remove()`, `Player.spectate(cam)` / `stopSpectating()`, `Entity.teleport(Vec3, yaw, pitch)`, `world.forceload`, `world.setSpawn`, `Block.isLoaded()`. Bankrupt! now has no `mc`/`mcf` calls; keep it that way (raw commands are a last resort, add the API instead).
- `a085842`: a `@WorldState` int used as a list index (`spaces[tokenSpace]`) read element 0.
- `7843e43`: int, boolean and enum `p.state.x` reads now land in a score. Before, they were always stale.
- `74cbfb9`: `std.dialog` takes a `Component` body.

- `d0763c5`: trig in std.
  - `std.math` has `atan`, `atan2`, `asin` and `acos`.
  - The compiler inlines `sin`, `cos` and `tan` to the builtin `/compute` calls.
- `1f3babb`: fixed nested calls to the same function, e.g. `f(x, f(y))`. The inner call's return flag ended the outer call early.
- `521d2af`: player input predicates now use the 26.2 map form, `"type_specific/player":{"input":{...}}`.
  - The 26.3 loot condition rename from `condition` to `type` had already landed in `03c4fc9`.
- `a1c4046`: two mcfd-agent dispatch fixes:
  - It skips packs whose handler function isn't loaded on the running server.
  - It runs its commands with output suppressed.
  - Cause: `mcfd` matches packs to a JVM by instance, not by world. So `saves/New World/datapacks/bukkit_api_conformance` got every event in Bankrupt Testing, and `Modified storage ...` showed in chat on each click.
  - Agent self-test: `mcfd-agent/test.ps1` passes.

## Blocked on Oliver

The fixed agent jar is built but not installed. To install it:

1. Close Minecraft.
2. From an admin PowerShell, copy `mcfd-agent\dist\mcfd-agent.jar` over `C:\Program Files\MCFC\mcfd\mcfd-agent.jar`.

Until then the old agent keeps printing the storage messages.

## Why the mouse cursor is dead

| Approach | Result |
|---|---|
| `tp` correction | View holds, but rubberbands by about one ping |
| `/rotate` correction | Flickers. A riding client sends its rotation every tick and corrections stack. |
| Spectating the camera entity | View locked, but the server pins the player's rotation to the camera. The log showed `d=0,0` on 534 of 536 ticks. |
| Crosshair mode (`bk_cross`) | Oliver rejected it: the cursor just follows the real camera |

## Spike 1 plan: locked spectator camera, keyboard cursor, dialogs

Build it in `bankrupt/src/main.mcf`, in the `specMode` branch of `rigTick`.

1. **Keyboard cursor.** In spectator mode, read `p.getCurrentInput().isForward()`, `isBackward()`, `isLeft()`, `isRight()` and `isJump()`. `isJump` already drives "space as right click".
   - A/D step the hovered ring space by -1/+1.
   - W/S jump by 10 spaces, one side of the board.
   - Key repeat: step on press, then every 4 ticks after 8 ticks held.
   - Sneak ends spectating, so it's re-spectated every tick. Sneak could act as cancel.
2. **Camera follows the hovered tile.** `shotTarget` uses `tokenSpace`; give it a space parameter.
3. **Space opens a dialog** (done via `std.dialog.menu`; diamond icon skipped: `menu` takes a plain-text body) with Buy (`run_command` `trigger bk_buy`) and Pass. This checks that a dialog gives a real mouse pointer while spectating.
   - Diamond icon in text: `{"type":"object","object":"atlas","atlas":"minecraft:items","sprite":"item/diamond"}`.
   - The atlas `minecraft:items` exists in the 26.3 client jar (`assets/minecraft/atlases/items.json`), and the codec keys `object`, `atlas` and `sprite` were read from the class files. The format hasn't been tested in game yet.
4. **Main open question:** do the input predicates still report WASD while spectating an entity? If not, fall back to hotbar scroll (already read via `getSelectedSlot`) plus dialogs.
5. **Build and install:**
   ```
   MCFC/target/debug/mcfc build bankrupt --out "C:/Users/Oliver/AppData/Roaming/PrismLauncher/instances/FO 26.3/minecraft/saves/Bankrupt! Dev/datapacks/bankrupt" --clean
   ```
   Then `/reload`. Read `bkdbg` lines and reload errors from `.../FO 26.3/minecraft/logs/latest.log`.

## Loose ends

- The bankrupt repo has uncommitted spike changes (README and `src/main.mcf`). Nobody asked to commit them.
- The design doc has the spike findings (2026-09-27). Its HTML source is `bankrupt-design.html` in the Claude scratchpad (copies in sessions 6589fee7 and bb67147d).
- Spike 1 dropped the old spectator mouse-reading code and the spectator left-click catcher; the keyboard path replaces both. Sneak-as-cancel is not done.
- The other session's dirty files are not mine; leave them alone: `BUKKIT_API_PLAN.md` and `npc.mcf` (deleted) and `editors/vscode-mcfc/package-lock.json`.
- The previous PROGRESS.md (Java-likeness batch, finished) is in git history at `f1790b6`.
