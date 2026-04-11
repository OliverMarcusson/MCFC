data modify entity @s start_interpolation set value 0
data modify entity @s interpolation_duration set value 2
$data modify entity @s transformation.scale set value [$(guard_vision), 0.02f, $(guard_vision)]

tag @s add shown
