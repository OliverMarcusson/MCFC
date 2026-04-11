execute on passengers if score @s[tag=dialogue_box] anpc.internal.npc.dialogue_duration matches -1.. run return fail
execute if score @s anpc.internal.npc.found_player matches 0.. run return fail
$execute unless data storage anpc:internal npc.$(id).actions[0] run return fail

return 1
