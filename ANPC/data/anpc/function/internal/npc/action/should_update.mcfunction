execute if score @s anpc.internal.npc.action_duration matches -1 run return fail
execute if score @s anpc.internal.npc.action_duration matches 1.. run return fail
execute if data entity @e[tag=anpc, tag=component, tag=ai, tag=current, limit=1] wander_target run return fail
execute store result score #result anpc.internal run data get entity @e[type=minecraft:wandering_trader, tag=anpc, tag=component, tag=ai, tag=current, limit=1] OnGround
execute unless score #result anpc.internal matches 1 run return fail
return 1
