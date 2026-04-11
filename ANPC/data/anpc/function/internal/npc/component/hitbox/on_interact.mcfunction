execute if score @s anpc.internal.npc.interact_cooldown matches 1.. run return fail

execute on vehicle at @s run function anpc:internal/npc/on_interact

scoreboard players operation @s anpc.internal.npc.interact_cooldown = #interaction_cooldown anpc.api.npc
