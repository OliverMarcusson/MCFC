param(
    [string]$JavaHome = $env:JAVA_HOME
)

$ErrorActionPreference = 'Stop'
if (-not $JavaHome) {
    $JavaHome = Split-Path -Parent (Split-Path -Parent (Get-Command javac -ErrorAction Stop).Source)
}

$javac = Join-Path $JavaHome 'bin\javac.exe'
$jar = Join-Path $JavaHome 'bin\jar.exe'
if (-not (Test-Path -LiteralPath $javac) -or -not (Test-Path -LiteralPath $jar)) {
    throw "A JDK with javac and jar is required; JAVA_HOME='$JavaHome'"
}

$root = Split-Path -Parent $PSCommandPath
$build = Join-Path $root 'build\classes'
$dist = Join-Path $root 'dist'
Remove-Item -LiteralPath (Join-Path $root 'build') -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $build, $dist | Out-Null

$asmJar = $env:MCFD_ASM_JAR
if (-not $asmJar -or -not (Test-Path -LiteralPath $asmJar)) {
    $prismAsm = Join-Path $env:APPDATA 'PrismLauncher\libraries\org\ow2\asm\asm\9.10.1\asm-9.10.1.jar'
    if (Test-Path -LiteralPath $prismAsm) {
        $asmJar = $prismAsm
    } else {
        $asmJar = Join-Path $root 'build\deps\asm-9.10.1.jar'
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $asmJar) | Out-Null
        Invoke-WebRequest -UseBasicParsing -Uri 'https://repo.maven.apache.org/maven2/org/ow2/asm/asm/9.10.1/asm-9.10.1.jar' -OutFile $asmJar
    }
}

$sources = Get-ChildItem -LiteralPath (Join-Path $root 'src\main\java') -Recurse -Filter '*.java' | Select-Object -ExpandProperty FullName
& $javac --add-modules jdk.attach -cp $asmJar -d $build @sources
if ($LASTEXITCODE -ne 0) { throw 'mcfd-agent Java compilation failed.' }

$agentManifest = Join-Path $root 'build\agent.mf'
@(
    'Manifest-Version: 1.0'
    'Premain-Class: dev.mcfc.agent.McfdAgent'
    'Agent-Class: dev.mcfc.agent.McfdAgent'
    'Can-Redefine-Classes: true'
    'Can-Retransform-Classes: true'
    ''
) | Set-Content -LiteralPath $agentManifest
& $jar cfm (Join-Path $dist 'mcfd-agent.jar') $agentManifest -C $build dev
if ($LASTEXITCODE -ne 0) { throw 'mcfd-agent JAR packaging failed.' }
$asmExtract = Join-Path $root 'build\asm'
New-Item -ItemType Directory -Force -Path $asmExtract | Out-Null
Push-Location $asmExtract
& $jar xf $asmJar org/objectweb/asm
Pop-Location
& $jar uf (Join-Path $dist 'mcfd-agent.jar') -C $asmExtract org/objectweb/asm
if ($LASTEXITCODE -ne 0) { throw 'mcfd-agent ASM shading failed.' }

$attachManifest = Join-Path $root 'build\attach.mf'
@('Manifest-Version: 1.0', 'Main-Class: dev.mcfc.agent.AttachMain', '') | Set-Content -LiteralPath $attachManifest
& $jar cfm (Join-Path $dist 'mcfd-agent-attach.jar') $attachManifest -C $build dev/mcfc/agent/AttachMain.class
if ($LASTEXITCODE -ne 0) { throw 'mcfd-agent attach launcher packaging failed.' }

Write-Host "Built $dist\mcfd-agent.jar and mcfd-agent-attach.jar"
