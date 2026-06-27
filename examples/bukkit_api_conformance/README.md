# Bukkit-style API conformance pack

This is a safe, vanilla-first smoke test for the first MCFC Bukkit-inspired
declarations. It does not require a mod or plugin. `mcfd` is optional, but is
needed for the time, random, and KV checks in `run_all`.

This test pack also requests the optional JVM agent. The request is safe when
the agent is unavailable: it does not change the datapack's vanilla behavior.

## Build

```powershell
cargo run -- build examples/bukkit_api_conformance/mcfc.toml --clean
```

Install `dist/` as a normal datapack, reload, then run:

```text
/function bukkit_api_conformance:run_all
/function bukkit_api_conformance:report
/trigger mcfcc_status
/function bukkit_api_conformance:cleanup
```

The generated runtime verifies:

- `data player.*` aliases for scoreboard-backed state
- first-seen player join dispatch
- death-count dispatch
- a one-shot load task
- vanilla `/trigger` command dispatch and agent `/status` dispatch
- existing player, UI, particle, bossbar, and host-bridge APIs
- typed agent callbacks for chat, inventory clicks, player actions, and block
  breaks, plus the expanded generic event catalog

`run_all` records a per-player pass count; `report` displays it. The vanilla
checks account for three points, and the optional mcfd time/random and KV
checks add one point each.

To repeat the death test without suppressing the next event, first copy the
current counter into the generated seen counter, then die:

```text
/scoreboard players operation @s mcfc_deaths_seen = @s mcfc_deaths
/kill @s
```

After the agent attaches, start the quiet Event Arcade with `/arcade`, then test:

- Send `spark` in chat for a +3 score bonus.
- Break blocks: every five gives an emerald.
- Attack an entity: every three-hit combo gives a cookie.
- Sneak-interact with an entity for +1 score.
- Right-click a block: the action bar shows its face and coordinates.
- Switch hotbar slots or close an inventory, then run `/status` to inspect the
  tracked event state.
- Run `/status` (not `/trigger`) to verify the agent-backed root command; the
  trigger command remains available as a fallback.

The compiler and agent still support the wider event catalog; this pack only
subscribes to the gameplay-oriented events above so normal play is not flooded
with acknowledgement messages.

All callbacks are experimental and version-pinned to 26.2. Cancellable typed
events may call `event.cancel()` inside the handler; lifecycle events are
observation-only and cannot be cancelled.
