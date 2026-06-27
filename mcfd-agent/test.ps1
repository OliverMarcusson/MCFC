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
} finally {
    Remove-Item -LiteralPath $classes -Recurse -Force -ErrorAction SilentlyContinue
}
