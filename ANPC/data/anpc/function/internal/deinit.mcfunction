scoreboard objectives remove anpc.internal
scoreboard objectives remove anpc.internal.npc.id
scoreboard objectives remove anpc.internal.npc.action_duration
scoreboard objectives remove anpc.internal.npc.dialogue_player
scoreboard objectives remove anpc.internal.npc.dialogue_duration
scoreboard objectives remove anpc.internal.npc.interact_cooldown
scoreboard objectives remove anpc.internal.npc.found_player
scoreboard objectives remove anpc.internal.player.id
scoreboard objectives remove anpc.internal.player.leave_game
scoreboard objectives remove anpc.api.npc
scoreboard objectives remove anpc.api.player.found_by_guard

team remove anpc

data remove storage anpc:internal npc
data remove storage anpc:api npc

kill @e[tag=anpc]
