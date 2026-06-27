# Agent Events

With `[helper.agent] enabled = true`, MCFC accepts typed event declarations for the version-pinned JVM adapter.

```mcfc
event chat(event: chat_event):
    event.player.send_message("You said: $(event.message)")

event inventory_click(event: inventory_click_event):
    event.player.send_message("slot=$(event.slot), button=$(event.button)")
```

The generated datapack remains valid without an attached agent. `mcfd` performs best-effort dynamic attachment and reports status separately.

## Payloads

Detailed payloads include:

- `chat_event { player, message, cancelled }`
- `inventory_click_event { player, container_id, state_id, slot, button, cancelled }`
- `player_action_event { player, action, face, x, y, z, cancelled }`
- `block_break_event { player, x, y, z, cancelled }`

The broader `agent_event` has `player`, `player_name`, `source`, `payload`, and `cancelled` fields.

## Event Catalog

The expanded catalog includes chat, inventory click, player actions, block break, interactions, entity attack/interact, held-item changes, inventory open/close, sign/book edits, recipe placement, item pickup/drop, teleport, damage, connect, quit, respawn, and game-mode events.

## Commands

`command name:` always keeps its vanilla `/trigger mcfcc_name` fallback. When the agent is attached, the same declaration also reserves a real `/name` root command.

::: warning Experimental and version-pinned
Agent callbacks are pinned to Minecraft `26.2`. Cancellable packet-entry events can call `event.cancel()`. Lifecycle callbacks such as damage, teleport, join, quit, and respawn are observation-only.
:::
