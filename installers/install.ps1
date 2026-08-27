# chore installer for Windows.
#
#   irm https://getchore.github.io/chore/install.ps1 | iex
#
# CHORE_VERSION      version to install, e.g. 0.1.0  (default: latest)
# CHORE_INSTALL_DIR  where the binary lands          (default: ~\.local\bin)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # the progress bar makes downloads slower

$repo = 'getchore/chore'
$dir = if ($env:CHORE_INSTALL_DIR) { $env:CHORE_INSTALL_DIR } else { Join-Path $HOME '.local\bin' }

# PROCESSOR_ARCHITECTURE lies inside a 32-bit host process; this does not.
$target = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    'X64'   { 'x86_64-pc-windows-msvc' }
    'Arm64' { 'aarch64-pc-windows-msvc' }
    default { throw "unsupported architecture: $_" }
}

$base = if ($env:CHORE_VERSION) {
    "https://github.com/$repo/releases/download/v$($env:CHORE_VERSION -replace '^v','')"
} else {
    # Resolves server-side, so there is no API call and no JSON to parse.
    "https://github.com/$repo/releases/latest/download"
}

$archive = "chore-$target.zip"
$tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "chore-$(New-Guid)")

try {
    $zip = Join-Path $tmp $archive
    Write-Host "downloading $archive"
    try { Invoke-WebRequest "$base/$archive" -OutFile $zip -UseBasicParsing }
    catch { throw "no release asset for $target" }

    # Best effort: a release without sidecars still installs, a mismatch does not.
    $want = $null
    try {
        $want = (Invoke-WebRequest "$base/$archive.sha256" -UseBasicParsing).Content.Trim().Split()[0]
    } catch { }
    if ($want -and $want -ine (Get-FileHash $zip -Algorithm SHA256).Hash) {
        throw "checksum mismatch for $archive"
    }

    Expand-Archive $zip -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Move-Item (Join-Path $tmp 'chore.exe') (Join-Path $dir 'chore.exe') -Force
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "installed to $dir"

# Read the user PATH from the registry, not $env:Path: the process copy is the
# merged machine+user value, and writing it back duplicates every machine
# entry into the user scope.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir".Trim(';'), 'User')
    Write-Host "added $dir to your PATH; open a new terminal"
}
$env:Path = "$dir;$env:Path"
