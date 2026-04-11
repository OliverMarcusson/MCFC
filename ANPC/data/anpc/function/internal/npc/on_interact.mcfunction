tag @s add current
execute as @e[tag=anpc, tag=component] if score @s anpc.internal.npc.id = @e[type=minecraft:mannequin, tag=anpc, tag=npc, tag=current, limit=1] anpc.internal.npc.id run tag @s add current
execute store result storage anpc:internal npc.id int 1 run scoreboard players get @s anpc.internal.npc.id

execute if entity @s[tag=dialogue] run function anpc:internal/npc/npcs/dialogue/on_interact

tag @e[tag=current] remove current
