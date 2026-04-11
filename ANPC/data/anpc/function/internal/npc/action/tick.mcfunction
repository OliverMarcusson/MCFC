execute if function anpc:internal/npc/action/should_update run function anpc:internal/npc/action/update with storage anpc:internal npc
execute if score @s anpc.internal.npc.action_duration matches 1.. run scoreboard players remove @s anpc.internal.npc.action_duration 1
