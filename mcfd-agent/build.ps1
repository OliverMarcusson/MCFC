param(
    [string]$JavaHome = $env:JAVA_HOME
)

$ErrorActionPreference = 'Stop'

# Major version from the JDK's `release` file: "17.0.20" -> 17, "1.8.0_504" -> 8.
function Get-JdkMajor([string]$JdkHome) {
    $release = Join-Path $JdkHome 'release'
    if (-not (Test-Path -LiteralPath (Join-Path $JdkHome 'bin\javac.exe')) -or -not (Test-Path -LiteralPath $release)) { return 0 }
    $line = Select-String -LiteralPath $release -Pattern '^JAVA_VERSION="(\d+)(?:\.(\d+))?' | Select-Object -First 1
    if (-not $line) { return 0 }
    $parts = $line.Matches[0].Groups
    if ($parts[1].Value -eq '1') { return [int]$parts[2].Value }
    return [int]$parts[1].Value
}

# JAVA_HOME and the javac on PATH may be an older JDK, so fall back to the usual install folders.
$candidates = @($JavaHome)
$pathJavac = Get-Command javac -ErrorAction SilentlyContinue
if ($pathJavac) { $candidates += Split-Path -Parent (Split-Path -Parent $pathJavac.Source) }
$candidates += Get-ChildItem -Directory -ErrorAction SilentlyContinue -Path (
    'C:\Program Files\Java', 'C:\Program Files\Eclipse Adoptium', 'C:\Program Files\Microsoft',
    'C:\Program Files\Zulu', 'C:\Program Files\Amazon Corretto'
) | Where-Object Name -Match 'jdk' | Sort-Object Name -Descending | ForEach-Object FullName
$JavaHome = $candidates | Where-Object { $_ -and (Get-JdkMajor $_) -ge 17 } | Select-Object -First 1
if (-not $JavaHome) {
    throw 'mcfd-agent needs JDK 17 or newer. Install one or pass -JavaHome.'
}
# test.ps1 runs javac and java from PATH after calling this script.
$env:PATH = (Join-Path $JavaHome 'bin') + ';' + $env:PATH

$javac = Join-Path $JavaHome 'bin\javac.exe'
$jar = Join-Path $JavaHome 'bin\jar.exe'

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
