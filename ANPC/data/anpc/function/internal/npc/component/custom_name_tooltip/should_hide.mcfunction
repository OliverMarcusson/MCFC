execute unless entity @s[tag=shown] run return fail
execute on vehicle on passengers if entity @s[tag=dialogue_box] if score @s anpc.internal.npc.dialogue_duration matches -1.. run return 1
execute on vehicle at @s unless entity @a[distance=..30] run return 1
return fail
