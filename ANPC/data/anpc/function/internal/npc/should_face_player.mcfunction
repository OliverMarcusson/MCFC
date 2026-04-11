execute unless entity @a[tag=current] run return 0
execute on passengers if score @s[tag=dialogue_box] anpc.internal.npc.dialogue_duration matches -1.. run return 1
execute if entity @s[tag=guard, tag=alerted] run return 1

return fail
