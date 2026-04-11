# Current NPC and components
tag @s add current
execute as @e[tag=anpc, tag=component] if score @s anpc.internal.npc.id = @e[type=minecraft:mannequin, tag=anpc, tag=npc, tag=current, limit=1] anpc.internal.npc.id run tag @s add current
execute store result storage anpc:internal npc.id int 1 run scoreboard players get @s anpc.internal.npc.id

# Current dialogue player
execute if entity @s[tag=dialogue] store result score #result anpc.internal on passengers run scoreboard players get @s[tag=dialogue_box] anpc.internal.npc.dialogue_player
execute if entity @s[tag=dialogue] as @a if score @s anpc.internal.player.id = #result anpc.internal run tag @s add current

# Model and components
execute as @e[tag=anpc, tag=component, tag=current] at @s run function anpc:internal/npc/component/tick
execute store result score #result anpc.internal run function anpc:internal/npc/action/should_tick with storage anpc:internal npc
execute if score #result anpc.internal matches 1 run function anpc:internal/npc/action/tick

# Guard
execute if entity @s[tag=guard] run function anpc:internal/npc/npcs/guard/tick with storage anpc:internal npc
execute if entity @s[tag=guard] as @a if score @e[type=minecraft:mannequin, tag=anpc, tag=npc, tag=current, limit=1] anpc.internal.npc.found_player = @s anpc.internal.player.id run scoreboard players set @s anpc.api.player.found_by_guard 1

# Current guard player
execute if entity @s[tag=guard] store result score #result anpc.internal run scoreboard players get @s anpc.internal.npc.found_player
execute if entity @s[tag=guard] as @a if score @s anpc.internal.player.id = #result anpc.internal run tag @s add current

data modify entity @s Rotation[1] set value 0.0f
execute if function anpc:internal/npc/should_face_player run rotate @s facing entity @a[tag=current, limit=1]
execute unless function anpc:internal/npc/should_face_player run data modify entity @s Rotation[0] set from entity @e[type=minecraft:wandering_trader, tag=anpc, tag=component, tag=ai, tag=current, limit=1] Rotation[0]

tag @e[tag=current] remove current
