# Bukkit-style MCFC API and API-conformance datapack

## Goal

Provide a vanilla-first, Bukkit-inspired MCFC API. MCFC compiles gameplay and
datapack-native facilities to functions and resources; `mcfd` remains an
optional capability-gated bridge for host services. A future best-effort JVM
agent may add server hooks which cannot be implemented by a vanilla datapack.

## Public API

- Add top-level `event`, `command`, and `task` declarations.
- Keep MCFC's snake_case style and existing `fn`, `tick`, `async`, and raw
  command features compatible.
- Generate vanilla implementations for lifecycle work, synthetic player join
  handling, scoreboard-based death events, `/trigger` commands, and scheduled
  tasks.
- Add managed typed `data` declarations for player, entity, and world state.
- Grow typed wrappers for scoreboard, teams, player/world/UI operations, and
  datapack resources while preserving raw commands and `assets/` passthrough.
- Gate non-vanilla hooks behind an explicit future `[helper.agent]` manifest
  capability. The current `mcfd` daemon must never claim event interception or
  cancellation.

## Syntax direction

```mcfc
data player.coins: int = 0

event player_join:
    let player = single(selector("@s"))
    player.send_message("Welcome!")

command home:
    let player = single(selector("@s"))
    player.send_message("Home requested")

task cleanup every_ticks(1200):
    debug("cleanup")
```

The first vanilla backend executes event and command handlers as the affected
player; they use `single(selector("@s"))` to obtain a player reference. It has
no synthetic event objects and is not cancellable. Agent-only event data and
cancellation remain a later, compile-time-gated backend.

## Delivery order

1. Introduce declarations across parser, AST, type checker, IR, backend, and
   LSP; preserve existing source compatibility.
2. Generate the vanilla lifecycle/event/trigger/task runtime and typed managed
   state.
3. Expand `mcfd` host capability results and add the agent manifest plus its
   structured event transport.
4. Add a conformance datapack with automated compiler assertions and a manual
   in-game scoreboard report.
5. Add version-pinned JVM hooks and generated callback wrappers. Dynamic
   attachment remains best-effort and is never required for vanilla packs.

## Agent foundation status

The `mcfd-agent/` module builds an attachable Java instrumentation agent and
Attach-API launcher. `[helper.agent] enabled = true` is emitted into the
per-pack descriptor; mcfd auto-attaches to the matching Minecraft JVM and
passes generated event/command subscriptions. The 26.1.2 adapter emits
versioned log records, dispatches typed callbacks for chat, inventory click,
player action, and block break, supports a broad generic player-event catalog,
and reserves real no-argument command roots while retaining `/trigger`
fallbacks. Packet-entry cancellation remains a global manifest policy; it is
not a synchronous per-event MCFC cancellation API.

## Verification

- Parser, type-checker, and generated-artifact tests cover every new
  declaration and diagnostic.
- `examples/bukkit_api_conformance` exercises the vanilla API and optional
  mcfd services. Exported functions report pass/fail/skip values to a
  scoreboard and provide cleanup.
- Agent-only declarations require `[helper.agent] enabled = true`; their
  generated descriptors list every requested route for auditability.

## Constraints

- Target the current Minecraft `26.1.2` datapack baseline and Windows-first
  `mcfd` distribution.
- Do not provide Java Bukkit binary compatibility, plugin interoperability, or
  fake cancellable events.
- Keep all host capabilities opt-in through the project manifest.
