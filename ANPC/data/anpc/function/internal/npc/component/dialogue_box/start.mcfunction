data modify entity @s start_interpolation set value 0
data modify entity @s interpolation_duration set value 2
data modify entity @s transformation.scale set value [0.8f, 0.8f, 0.8f]

scoreboard players operation @s anpc.internal.npc.dialogue_player = @a[tag=current, limit=1] anpc.internal.player.id

$execute unless data storage anpc:internal npc.$(id).dialogues_copy[0] run data modify storage anpc:internal npc.$(id).dialogues_copy set from storage anpc:internal npc.$(id).dialogues

$data modify storage anpc:internal npc.$(id).dialogue_lines set from storage anpc:internal npc.$(id).dialogues_copy[0]

function anpc:internal/npc/component/dialogue_box/update with storage anpc:internal npc
