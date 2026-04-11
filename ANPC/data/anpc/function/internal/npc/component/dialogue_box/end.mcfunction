data modify entity @s start_interpolation set value 0
data modify entity @s interpolation_duration set value 4
data modify entity @s transformation.scale set value [0.0f, 0.0f, 0.0f]

scoreboard players reset @s anpc.internal.npc.dialogue_player

$function anpc:internal/npc/component/dialogue_box/stop_sound with storage anpc:internal npc.$(id)

$data remove storage anpc:internal npc.$(id).previous_dialogue_sound
$data remove storage anpc:internal npc.$(id).dialogues_copy[0]
$data remove storage anpc:internal npc.$(id).dialogue_lines

scoreboard players reset @s anpc.internal.npc.dialogue_duration
