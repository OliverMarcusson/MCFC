$data modify storage anpc:internal npc.$(id).movement_speed set from storage anpc:internal npc.$(id).actions_copy[0].movement_speed
$execute as @e[type=minecraft:wandering_trader, tag=anpc, tag=component, tag=ai, tag=current, tag=!disabled_movement, limit=1] run function anpc:internal/npc/component/ai/set_movement_speed with storage anpc:internal npc.$(id)
