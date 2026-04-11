$execute unless data storage anpc:internal npc.$(id).actions_copy[0] run data modify storage anpc:internal npc.$(id).actions_copy set from storage anpc:internal npc.$(id).actions

# Move
data modify storage anpc:internal npc.action set value "pathfind"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/pathfind with storage anpc:internal npc

# Jump
data modify storage anpc:internal npc.action set value "jump"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/jump

# Profile
data modify storage anpc:internal npc.action set value "set_profile"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/set_profile with storage anpc:internal npc

# Movement speed
data modify storage anpc:internal npc.action set value "set_movement_speed"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/set_movement_speed with storage anpc:internal npc

# Movement speed
data modify storage anpc:internal npc.action set value "set_ai"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/set_ai with storage anpc:internal npc

# Pose
data modify storage anpc:internal npc.action set value "pose"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/pose with storage anpc:internal npc

# Rotate
data modify storage anpc:internal npc.action set value "rotate"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/rotate with storage anpc:internal npc

# Idle
data modify storage anpc:internal npc.action set value "idle"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/idle with storage anpc:internal npc

# Function
data modify storage anpc:internal npc.action set value "run_function"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
$execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/run_function with storage anpc:internal npc.$(id).actions_copy[0]

# Teleport
data modify storage anpc:internal npc.action set value "position"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/position with storage anpc:internal npc

# Swing
data modify storage anpc:internal npc.action set value "swing"
$execute store success score #success anpc.internal run data modify storage anpc:internal npc.action set from storage anpc:internal npc.$(id).actions_copy[0].action
$execute if score #success anpc.internal matches 0 run function anpc:internal/npc/action/actions/swing with storage anpc:internal npc.$(id).actions_copy[0]

$data remove storage anpc:internal npc.$(id).actions_copy[0]
