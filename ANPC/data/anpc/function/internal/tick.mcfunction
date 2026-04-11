scoreboard players set @a anpc.api.player.found_by_guard 0
execute as @e[type=minecraft:mannequin, tag=anpc, tag=npc] at @s run function anpc:internal/npc/tick

function anpc:internal/test/decoy/tick

execute as @a at @s run function anpc:internal/player/tick
