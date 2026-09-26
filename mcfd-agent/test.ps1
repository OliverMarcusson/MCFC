$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot 'build.ps1')

$classes = Join-Path $env:TEMP ('mcfd-agent-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $classes | Out-Null
try {
    $source = Join-Path $PSScriptRoot 'src\test\java\dev\mcfc\agent\McfdHooksSelfTest.java'
    $agent = Join-Path $PSScriptRoot 'dist\mcfd-agent.jar'
    & javac -cp $agent -d $classes $source
    if ($LASTEXITCODE -ne 0) { throw 'javac failed' }
    & java -cp "$agent;$classes" dev.mcfc.agent.McfdHooksSelfTest
    if ($LASTEXITCODE -ne 0) { throw 'agent self-test failed' }
    # With a local 26.3 install, also build the sidebar packets against it,
    # on the Java 25 runtime the game ships with.
    $prism = "$env:APPDATA\PrismLauncher"
    $game = "$prism\libraries\com\mojang\minecraft\26.3\minecraft-26.3-client.jar"
    $java25 = "$prism\java\java-runtime-epsilon\bin\java.exe"
    $meta = "$prism\meta\net.minecraft\26.3.json"
    if ((Test-Path -LiteralPath $game) -and (Test-Path -LiteralPath $java25) -and (Test-Path -LiteralPath $meta)) {
        # The game's own library versions, as maven coordinates.
        $classpath = Join-Path $classes 'game-classpath.txt'
        @($game) + ((Get-Content -Raw $meta | ConvertFrom-Json).libraries | ForEach-Object {
            $group, $artifact, $version, $classifier = $_.name -split ':'
            $file = if ($classifier) { "$artifact-$version-$classifier.jar" } else { "$artifact-$version.jar" }
            "$prism\libraries\$($group -replace '\.', '\')\$artifact\$version\$file"
        } | Where-Object { Test-Path -LiteralPath $_ }) | Set-Content $classpath
        & $java25 -cp "$agent;$classes" dev.mcfc.agent.McfdHooksSelfTest $classpath
        if ($LASTEXITCODE -ne 0) { throw 'sidebar packet check failed' }
    }
} finally {
    Remove-Item -LiteralPath $classes -Recurse -Force -ErrorAction SilentlyContinue
}
