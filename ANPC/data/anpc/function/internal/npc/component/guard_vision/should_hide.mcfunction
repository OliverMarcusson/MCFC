execute unless entity @s[tag=shown] run return fail
execute on vehicle at @s unless entity @a[distance=..20] run return 1
return fail
