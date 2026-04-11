scoreboard objectives add anpc.internal dummy
scoreboard objectives add anpc.internal.npc.id dummy
scoreboard objectives add anpc.internal.npc.action_duration dummy
scoreboard objectives add anpc.internal.npc.dialogue_player dummy
scoreboard objectives add anpc.internal.npc.dialogue_duration dummy
scoreboard objectives add anpc.internal.npc.interact_cooldown dummy
scoreboard objectives add anpc.internal.npc.found_player dummy
scoreboard objectives add anpc.internal.player.id dummy
scoreboard objectives add anpc.internal.player.leave_game minecraft.custom:minecraft.leave_game
scoreboard objectives add anpc.api.npc dummy
scoreboard objectives add anpc.api.player.found_by_guard dummy

scoreboard players set #interaction_cooldown anpc.api.npc 10

team add anpc
team modify anpc collisionRule never

execute as @a unless score @s anpc.internal.player.id matches 0.. run function anpc:internal/player/register

effect give @e[type=minecraft:wandering_trader, tag=anpc, tag=component, tag=ai] minecraft:invisibility infinite 0 true
