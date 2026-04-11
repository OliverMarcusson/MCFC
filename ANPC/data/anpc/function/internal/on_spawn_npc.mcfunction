execute as @e[tag=anpc, tag=component, tag=!registered] at @s run function anpc:internal/npc/on_spawn

scoreboard players operation @e[tag=anpc, tag=!registered] anpc.internal.npc.id = #next anpc.internal.npc.id
scoreboard players add #next anpc.internal.npc.id 1

tag @e[tag=anpc, tag=!registered] add registered
