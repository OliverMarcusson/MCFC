function anpc:internal/on_player_join

execute unless score @s anpc.internal.player.id matches 0.. run function anpc:internal/player/register

scoreboard players set @s anpc.internal.player.leave_game 0
