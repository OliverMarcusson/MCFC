# TODO: Fix bad performance?

scoreboard players reset @s anpc.internal.npc.found_player
$execute as @a at @s run function anpc:internal/npc/npcs/guard/tick_player with storage anpc:internal npc.$(id)

scoreboard players set #success anpc.internal 0
execute as @a if score @s anpc.internal.player.id = @e[type=minecraft:mannequin, tag=anpc, tag=npc, tag=current, limit=1] anpc.internal.npc.found_player run scoreboard players set #success anpc.internal 1

execute if entity @s[tag=!alerted] if score #success anpc.internal matches 1 run return run return run function anpc:internal/npc/npcs/guard/alert
execute if entity @s[tag=alerted] unless score #success anpc.internal matches 1 run return run function anpc:internal/npc/npcs/guard/idle
