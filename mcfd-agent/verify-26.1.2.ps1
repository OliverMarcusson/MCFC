param(
    [string]$MinecraftJar = "C:\Users\Oliver\AppData\Roaming\PrismLauncher\libraries\com\mojang\minecraft\26.1.2\minecraft-26.1.2-client.jar"
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $MinecraftJar)) {
    throw "Minecraft 26.1.2 JAR not found: $MinecraftJar. Pass -MinecraftJar <path>."
}

# Keep this list in step with McfdAgent.EventClassVisitor. This verifies named
# Mojang methods exist in the exact version the bytecode adapter targets.
$targets = @{
    'net.minecraft.server.network.ServerGamePacketListenerImpl' = @(
        'handleContainerClick', 'handleSetCreativeModeSlot', 'handleContainerButtonClick',
        'handlePlaceRecipe', 'handleContainerClose', 'handleChat', 'handleChatCommand',
        'handleSignedChatCommand', 'handlePlayerAction', 'handleUseItemOn', 'handleUseItem',
        'handleInteract', 'handleAttack', 'handleSetCarriedItem', 'handleAnimate',
        'handlePlayerCommand', 'handleClientCommand', 'handleRenameItem', 'handleSelectTrade',
        'handleSignUpdate', 'handleEditBook', 'handleSetBeaconPacket', 'handlePickItemFromBlock',
        'handlePickItemFromEntity', 'handleTeleportToEntityPacket', 'handleChangeGameMode',
        'handlePlayerAbilities'
    )
    'net.minecraft.server.level.ServerPlayerGameMode' = @(
        'destroyBlock', 'changeGameModeForPlayer'
    )
    'net.minecraft.server.level.ServerPlayer' = @(
        'die', 'hurtServer', 'teleport', 'drop', 'onItemPickup', 'openMenu'
    )
    'net.minecraft.server.players.PlayerList' = @(
        'placeNewPlayer', 'remove', 'respawn'
    )
}

foreach ($class in $targets.Keys) {
    $methods = & javap -classpath $MinecraftJar -p $class 2>$null | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Could not inspect $class from $MinecraftJar"
    }
    foreach ($method in $targets[$class]) {
        if ($methods -notmatch ("\b" + [regex]::Escape($method) + "\(")) {
            throw "Missing 26.1.2 adapter target: $class.$method"
        }
    }
    Write-Host "verified $class"
}

Write-Host 'mcfd-agent 26.1.2 mapping verification passed'
