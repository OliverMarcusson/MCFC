data modify entity @s Rotation[0] set from entity @e[type=minecraft:mannequin, tag=anpc, tag=npc, tag=current, limit=1] Rotation[0]
data modify entity @s Rotation[1] set value 0.0f

$execute if function anpc:internal/npc/component/guard_vision/should_show run return run function anpc:internal/npc/component/guard_vision/show with storage anpc:internal npc.$(id)
execute if function anpc:internal/npc/component/guard_vision/should_hide run return run function anpc:internal/npc/component/guard_vision/hide