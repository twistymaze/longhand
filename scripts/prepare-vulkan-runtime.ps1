# Stage the pinned x64 loader from LunarG's runtime redistribution archive.
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
if ($env:VSCMD_ARG_TGT_ARCH -ne 'x64') {
    throw 'The pinned Vulkan runtime package supports only the Windows x64 build target'
}
$version = '1.4.309.0'
$archiveHash = '7d969f4d7b44e387667d3148f61559497c22d50cbe3d50adc9e5409afbce2df1'
$loaderHash = '6f6b53f86d2be29c77fdcd69e8aa2034bb90fd116b0b9dc403cf091835a88d70'
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$archive = Join-Path $OutputDirectory "VulkanRT-$version-Components.zip"
$url = "https://sdk.lunarg.com/sdk/download/$version/windows/VulkanRT-$version-Components.zip"
function Test-Archive {
    (Test-Path -LiteralPath $archive) -and
        ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -eq $archiveHash)
}
# A network fault must not fail the build. Use an archive from an earlier run
# if its checksum is correct. Otherwise, try the download again after a fault.
$attempts = 5
for ($attempt = 1; -not (Test-Archive); $attempt++) {
    try {
        Invoke-WebRequest -UseBasicParsing $url -OutFile $archive -TimeoutSec 300
    } catch {
        if ($attempt -ge $attempts) { throw }
        # Standard output gives only the loader directory to prepare-nemo.ts.
        [Console]::Error.WriteLine("Vulkan runtime download attempt $attempt of $attempts failed: $($_.Exception.Message)")
        Start-Sleep -Seconds (5 * $attempt)
        continue
    }
    if (-not (Test-Archive)) {
        throw 'Vulkan runtime archive checksum mismatch'
    }
}
Expand-Archive -LiteralPath $archive -DestinationPath $OutputDirectory -Force
$loaderDir = Join-Path $OutputDirectory "VulkanRT-$version-Components/x64"
$loader = Join-Path $loaderDir 'vulkan-1.dll'
if ((Get-FileHash $loader -Algorithm SHA256).Hash.ToLowerInvariant() -ne $loaderHash) {
    throw 'Vulkan x64 loader checksum mismatch'
}
if ($env:GITHUB_ENV) {
    Add-Content -Path $env:GITHUB_ENV -Value "LONGHAND_VULKAN_RUNTIME_DIR=$loaderDir"
}
Write-Output $loaderDir
