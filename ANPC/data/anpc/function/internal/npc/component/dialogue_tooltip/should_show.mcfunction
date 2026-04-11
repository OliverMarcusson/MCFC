execute if entity @s[tag=shown] run return fail
execute on vehicle on passengers if entity @s[tag=dialogue_box] if score @s anpc.internal.npc.dialogue_duration matches -1.. run return fail
execute on vehicle at @s unless entity @a[distance=..15] run return fail
return 1
