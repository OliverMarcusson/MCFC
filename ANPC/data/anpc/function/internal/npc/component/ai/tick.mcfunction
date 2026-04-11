data modify entity @e[tag=anpc, tag=npc, tag=current, limit=1] Pos set from entity @s Pos

item replace entity @s weapon.mainhand with minecraft:air

execute if function anpc:internal/npc/component/ai/should_disable_movement run return run function anpc:internal/npc/component/ai/disable_movement
execute if function anpc:internal/npc/component/ai/should_enable_movement run return run function anpc:internal/npc/component/ai/enable_movement with storage anpc:internal npc
