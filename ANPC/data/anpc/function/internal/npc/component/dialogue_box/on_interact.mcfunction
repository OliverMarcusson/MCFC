execute unless function anpc:internal/npc/component/dialogue_box/is_allowed_player run return fail

# No active dialogue
execute unless score @s anpc.internal.npc.dialogue_duration matches -1.. run return run function anpc:internal/npc/component/dialogue_box/start with storage anpc:internal npc

# Contine dialogue
execute if score @s anpc.internal.npc.dialogue_duration matches ..0 run function anpc:internal/npc/component/dialogue_box/continue with storage anpc:internal npc
