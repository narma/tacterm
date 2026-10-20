param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch
)
$ErrorActionPreference = 'Stop'
$stem = "tacterm-$Version-windows-$Arch"
$installer = Join-Path $PSScriptRoot "../../target/release-assets/$stem-setup.exe"
$portable = Join-Path $PSScriptRoot "../../target/release-staging/$stem-portable/tacterm.exe"
$destination = Join-Path $env:RUNNER_TEMP ("tacterm-setup-test-" + [guid]::NewGuid())
$log = Join-Path $env:RUNNER_TEMP "$stem-setup.log"
try {
    $process = Start-Process -FilePath $installer -ArgumentList "/CURRENTUSER /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR=`"$destination`" /LOG=`"$log`"" -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode). See $log" }
    $installed = Join-Path $destination 'tacterm.exe'
    if ((Get-FileHash $installed).Hash -ne (Get-FileHash $portable).Hash) {
        throw 'Installed executable differs from portable executable'
    }
    foreach ($relative in @('LICENSE', 'licenses/corcovado/ORIGINAL_LICENSE', 'licenses/sugarloaf/src/font/resources/CascadiaCode/LICENSE')) {
        $sourceLicense = Join-Path (Split-Path $portable) $relative
        $installedLicense = Join-Path $destination $relative
        if ((Get-FileHash $sourceLicense).Hash -ne (Get-FileHash $installedLicense).Hash) {
            throw "Installed license differs from portable: $relative"
        }
    }
    $reported = & $installed --version
    if ($LASTEXITCODE -ne 0 -or "$reported" -notlike "*$Version*") { throw "Unexpected version: $reported" }
    $helpOutput = & $installed --help
    if ($LASTEXITCODE -ne 0 -or "$helpOutput" -notlike '*--working-dir*') { throw 'Installed CLI help is incomplete' }
} finally {
    $uninstaller = Join-Path $destination 'unins000.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Uninstaller failed: $($process.ExitCode)" }
        if (Test-Path -LiteralPath (Join-Path $destination 'tacterm.exe')) { throw 'Uninstaller left tacterm.exe behind' }
    }
}
