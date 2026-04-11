execute as @e[type=minecraft:text_display, tag=anpc, tag=component, tag=dialogue_box, tag=!current] if score @s anpc.internal.npc.dialogue_player = @a[tag=current, limit=1] anpc.internal.player.id run return fail
execute as @a[tag=!current] if score @e[type=minecraft:text_display, tag=anpc, tag=component, tag=dialogue_box, tag=current, limit=1] anpc.internal.npc.dialogue_player = @s anpc.internal.player.id run return fail
return 1
