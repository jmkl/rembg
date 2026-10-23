$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$RembgRoot = Split-Path -Parent $Root
$HostName = 'hello.dcsms.gpaste'
$InstallDir = Join-Path $env:LOCALAPPDATA 'GPasTe'
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

Push-Location $Root
try {
    cargo build --release -p gpaste
    Copy-Item (Join-Path $RembgRoot 'target\release\gpaste_host.exe') (Join-Path $InstallDir 'gpaste_host.exe') -Force
} finally { Pop-Location }

$ExtensionId = Read-Host 'Extension Id'
if (-not $ExtensionId) { throw 'Extension ID is required.' }
$ManifestPath = Join-Path $InstallDir 'hello.dcsms.gpaste.json'
$Manifest = @{
    name = $HostName
    description = 'Rust GUI to ChatGPT bridge'
    path = (Join-Path $InstallDir 'gpaste_host.exe')
    type = 'stdio'
    allowed_origins = @("chrome-extension://$ExtensionId/")
} | ConvertTo-Json -Depth 4
[System.IO.File]::WriteAllText($ManifestPath, $Manifest, [System.Text.UTF8Encoding]::new($false))
$RegistryPath = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$HostName"
New-Item -Path $RegistryPath -Force | Out-Null
Set-ItemProperty -Path $RegistryPath -Name '(default)' -Value $ManifestPath
$RegistryPath = "HKCU:\Software\Google\Vivaldi\NativeMessagingHosts\$HostName"
New-Item -Path $RegistryPath -Force | Out-Null
Set-ItemProperty -Path $RegistryPath -Name '(default)' -Value $ManifestPath
Write-Host "Installed native host: $ManifestPath"
Write-Host 'Load the extension folder as Unpacked in chrome://extensions, then restart Chrome.'
