$execute store result score #length anpc.internal run data get storage anpc:internal npc.$(id).dialogue_lines
$execute store result score #length_copy anpc.internal run data get storage anpc:internal npc.$(id).dialogues_copy[0]
scoreboard players operation #page anpc.internal = #length anpc.internal
scoreboard players operation #page anpc.internal -= #length_copy anpc.internal
scoreboard players add #page anpc.internal 1

$execute store result score @s anpc.internal.npc.dialogue_duration run data get storage anpc:internal npc.$(id).dialogues_copy[0][0].duration

$execute if score @s anpc.internal.npc.dialogue_duration matches 0.. run data modify entity @s text set value ["", {storage: "anpc:internal", nbt: "npc.$(id).dialogues_copy[0][0].text", interpret: true}, {text: "\n(", color: "gray"}, {score: {name: "#page", objective: "anpc.internal"}, color: "gray"}, {text: "/", color: "gray"}, {score: {name: "#length", objective: "anpc.internal"}, color: "gray"}, {text: ")", color: "gray"}]
$execute if score @s anpc.internal.npc.dialogue_duration matches -1 run data modify entity @s text set value ["", {storage: "anpc:internal", nbt: "npc.$(id).dialogues_copy[0][0].text", interpret: true}, {text: "\nClick to continue (", color: "gray"}, {score: {name: "#page", objective: "anpc.internal"}, color: "gray"}, {text: "/", color: "gray"}, {score: {name: "#length", objective: "anpc.internal"}, color: "gray"}, {text: ")", color: "gray"}]

# Stop previous sound
$execute if data storage anpc:internal npc.$(id).previous_dialogue_sound run function anpc:internal/npc/component/dialogue_box/stop_sound with storage anpc:internal npc.$(id)

# Store previous sound
$data modify storage anpc:internal npc.$(id).previous_dialogue_sound set from storage anpc:internal npc.$(id).dialogues_copy[0][0].sound.id

# Play sound
$function anpc:internal/npc/component/dialogue_box/play_sound with storage anpc:internal npc.$(id).dialogues_copy[0][0].sound

$data remove storage anpc:internal npc.$(id).dialogues_copy[0][0]
