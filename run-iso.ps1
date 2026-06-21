param(
    [string]$QemuBiosPath = "C:\Program Files\qemu\qemu-system-i386.exe",
    [string]$QemuUefiPath = "C:\Program Files\qemu\qemu-system-x86_64.exe",
    [ValidateSet("auto", "bios", "uefi")]
    [string]$Firmware = "auto",
    [string]$OvmfCodePath = "",
    [string]$OvmfVarsPath = "",
    [string]$GrubMkrescuePath = "grub-mkrescue",
    [string]$XorrisoPath = "",
    [string]$TarPath = "tar",
    [string]$IsoPath = "",
    [string]$ImagePath = "",
    [string]$DataDiskPath = "",
    [switch]$HostPhysicalDisks,
    [switch]$AllowHostDiskWrite,
    [string]$HostDiskIndexes = "",
    [switch]$NoHttpsProxy
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$makeIso = Join-Path $root "make-iso.ps1"
$limineDir = Join-Path $root "boot\limine"

if ([string]::IsNullOrWhiteSpace($IsoPath)) {
    $IsoPath = Join-Path $root "build\houseos.iso"
}

if (-not (Test-Path $makeIso)) {
    throw "make-iso.ps1 not found at $makeIso"
}

function Get-WritableIsoPath([string]$Path) {
    if (-not (Test-Path $Path)) {
        return $Path
    }

    try {
        Remove-Item -Force $Path
        return $Path
    }
    catch {
        $dir = Split-Path -Parent $Path
        $base = [System.IO.Path]::GetFileNameWithoutExtension($Path)
        for ($i = 1; $i -le 50; $i++) {
            $suffix = if ($i -eq 1) { "run" } else { "run$i" }
            $candidate = Join-Path $dir "$base-$suffix.iso"
            if (Test-Path $candidate) {
                try {
                    Remove-Item -Force $candidate
                }
                catch {
                    continue
                }
            }
            Write-Warning "ISO is locked: $Path. Building to $candidate instead."
            return $candidate
        }
        throw "Cannot find writable ISO path. Close the emulator or unlock $Path."
    }
}

$IsoPath = Get-WritableIsoPath $IsoPath
& $makeIso -GrubMkrescuePath $GrubMkrescuePath -XorrisoPath $XorrisoPath -TarPath $TarPath -IsoPath $IsoPath -ImagePath $ImagePath

if (-not (Test-Path $IsoPath)) {
    throw "ISO not found at $IsoPath"
}

if ([string]::IsNullOrWhiteSpace($DataDiskPath)) {
    $DataDiskPath = Join-Path $root "build\fs.img"
}

function Find-ExistingPath([string[]]$candidates) {
    foreach ($c in $candidates) {
        if (-not [string]::IsNullOrWhiteSpace($c) -and (Test-Path $c)) {
            return $c
        }
    }
    return $null
}

function Has-UefiPayload {
    param([string]$LimineDirPath)
    $uefiCd = Join-Path $LimineDirPath "limine-uefi-cd.bin"
    $bootX64 = Join-Path $LimineDirPath "BOOTX64.EFI"
    $bootIa32 = Join-Path $LimineDirPath "BOOTIA32.EFI"
    return (Test-Path $uefiCd) -and ((Test-Path $bootX64) -or (Test-Path $bootIa32))
}

$hasUefiPayload = Has-UefiPayload -LimineDirPath $limineDir

$ovmfCodeCandidate = Find-ExistingPath @(
    $OvmfCodePath,
    "C:\Program Files\qemu\share\edk2-x86_64-code.fd",
    "C:\Program Files\qemu\share\OVMF\OVMF_CODE.fd",
    "C:\Program Files\qemu\share\OVMF.fd",
    "C:\msys64\usr\share\edk2-ovmf\x64\OVMF_CODE.fd",
    "C:\msys64\usr\share\edk2-ovmf\OVMF_CODE.fd"
)
$ovmfVarsCandidate = Find-ExistingPath @(
    $OvmfVarsPath,
    "C:\Program Files\qemu\share\edk2-x86_64-vars.fd",
    "C:\Program Files\qemu\share\OVMF\OVMF_VARS.fd",
    "C:\msys64\usr\share\edk2-ovmf\x64\OVMF_VARS.fd",
    "C:\msys64\usr\share\edk2-ovmf\OVMF_VARS.fd"
)

$hasQemuUefi = Test-Path $QemuUefiPath
$canUefi = $hasUefiPayload -and ($null -ne $ovmfCodeCandidate) -and $hasQemuUefi

switch ($Firmware) {
    "uefi" {
        if (-not $canUefi) {
            throw "UEFI requested but missing files. Need Limine UEFI payload in boot\\limine and OVMF firmware (OVMF_CODE.fd)."
        }
    }
    "bios" { }
    "auto" {
        if ($canUefi) {
            $Firmware = "uefi"
        } else {
            $Firmware = "bios"
        }
    }
}

$netArgs = @("-netdev", "user,id=net0", "-device", "rtl8139,netdev=net0")
$bootArgs = @("-boot", "order=d,menu=off")
$diskArgs = @()
if (-not [string]::IsNullOrWhiteSpace($DataDiskPath) -and (Test-Path $DataDiskPath)) {
    $ext = [System.IO.Path]::GetExtension($DataDiskPath).ToLowerInvariant()
    $diskFormat = if ($ext -eq ".vdi") { "vdi" } else { "raw" }
    $diskArgs = @("-drive", "file=$DataDiskPath,format=$diskFormat,if=ide,index=1,media=disk")
    Write-Host "HouseOS data disk attached: $DataDiskPath"
} else {
    Write-Warning "Data disk not found. Full Disk will only see disks that QEMU exposes."
}

function Get-RequestedHostDiskIndexes {
    if (-not [string]::IsNullOrWhiteSpace($HostDiskIndexes)) {
        $items = @()
        foreach ($part in ($HostDiskIndexes -split ",")) {
            $trimmed = $part.Trim()
            if ($trimmed.Length -eq 0) {
                continue
            }
            $num = 0
            if ([int]::TryParse($trimmed, [ref]$num) -and $num -ge 0) {
                $items += $num
            }
        }
        return $items
    }

    try {
        return @(Get-CimInstance Win32_DiskDrive | Sort-Object Index | ForEach-Object { [int]$_.Index })
    }
    catch {
        Write-Warning "Cannot enumerate physical disks automatically. Pass -HostDiskIndexes 0,1 or run PowerShell as Administrator."
        return @()
    }
}

function Test-LocalTcpPort {
    param([int]$Port)
    $client = $null
    try {
        $client = [System.Net.Sockets.TcpClient]::new()
        $async = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
        if (-not $async.AsyncWaitHandle.WaitOne(800, $false)) {
            $client.Close()
            return $false
        }
        $client.EndConnect($async)
        $client.Close()
        return $true
    }
    catch {
        if ($null -ne $client) {
            $client.Close()
        }
        return $false
    }
}

$hostDiskArgs = @()
if ($HostPhysicalDisks) {
    $hostIdeSlots = @(0, 3)
    $hostSlotIndex = 0
    $requestedDisks = @(Get-RequestedHostDiskIndexes)
    if ($requestedDisks.Count -eq 0) {
        Write-Warning "No host physical disks selected. Full Disk will only show QEMU image disks."
    }
    foreach ($diskIndex in $requestedDisks) {
        if ($hostSlotIndex -ge $hostIdeSlots.Count) {
            Write-Warning "Only two host disks can be attached on the legacy IDE bus while the ISO CD-ROM and HouseOS data disk are present."
            break
        }
        $ideIndex = $hostIdeSlots[$hostSlotIndex]
        $physicalPath = "\\.\PhysicalDrive$diskIndex"
        if ($AllowHostDiskWrite) {
            $hostDiskArgs += @("-drive", "file=$physicalPath,format=raw,if=ide,index=$ideIndex,media=disk")
            Write-Warning "DANGEROUS: host physical disk attached writable: $physicalPath at IDE index $ideIndex"
        } else {
            $hostDiskArgs += @("-drive", "file=$physicalPath,format=raw,if=ide,index=$ideIndex,media=disk,snapshot=on")
            Write-Host "Host physical disk attached read/snapshot: $physicalPath at IDE index $ideIndex"
        }
        $hostSlotIndex += 1
    }
}

$proxyProcess = $null
if (-not $NoHttpsProxy) {
    $proxyScript = Join-Path $root "tools\https-proxy.ps1"
    if (Test-Path $proxyScript) {
        if (Test-LocalTcpPort -Port 18080) {
            Write-Host "HouseOS HTTPS proxy already running on host port 18080"
        } else {
            $proxyProcess = Start-Process -FilePath "powershell" -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $proxyScript, "-Port", "18080") -WindowStyle Hidden -PassThru
            Start-Sleep -Milliseconds 1200
            if ($proxyProcess.HasExited) {
                Write-Warning "HouseOS HTTPS proxy did not stay running. Check build\https-proxy.log or make sure port 18080 is free."
            } elseif (-not (Test-LocalTcpPort -Port 18080)) {
                Write-Warning "HouseOS HTTPS proxy started but port 18080 is not reachable yet. Check build\https-proxy.log if HTTPS fails."
            } else {
                Write-Host "HouseOS HTTPS proxy started on host port 18080"
            }
        }
    } else {
        Write-Warning "HTTPS proxy script not found at $proxyScript"
    }
}

try {
if ($Firmware -eq "uefi") {
    if (-not (Test-Path $QemuUefiPath)) {
        throw "QEMU UEFI binary not found at $QemuUefiPath. Install QEMU x86_64 or pass -QemuUefiPath."
    }

    if ($ovmfVarsCandidate) {
        $varsRuntime = Join-Path $root "build\OVMF_VARS.fd"
        Copy-Item -Force $ovmfVarsCandidate $varsRuntime
        & $QemuUefiPath `
            "-m" "512M" `
            "-display" "gtk,zoom-to-fit=on,full-screen=on" `
            "-drive" "if=pflash,format=raw,readonly=on,file=$ovmfCodeCandidate" `
            "-drive" "if=pflash,format=raw,file=$varsRuntime" `
            "-cdrom" $IsoPath `
            @bootArgs `
            @diskArgs `
            @hostDiskArgs `
            @netArgs
    } else {
        & $QemuUefiPath `
            "-m" "512M" `
            "-display" "gtk,zoom-to-fit=on,full-screen=on" `
            "-bios" $ovmfCodeCandidate `
            "-cdrom" $IsoPath `
            @bootArgs `
            @diskArgs `
            @hostDiskArgs `
            @netArgs
    }
} else {
    if (-not (Test-Path $QemuBiosPath)) {
        throw "QEMU BIOS binary not found at $QemuBiosPath. Install QEMU i386 or pass -QemuBiosPath."
    }
    & $QemuBiosPath `
        "-cdrom" $IsoPath `
        "-m" "384M" `
        "-display" "gtk,zoom-to-fit=on,full-screen=on" `
        "-vga" "std" `
        @bootArgs `
        @diskArgs `
        @hostDiskArgs `
        @netArgs
}
}
finally {
    if ($null -ne $proxyProcess -and -not $proxyProcess.HasExited) {
        Stop-Process -Id $proxyProcess.Id -Force
    }
}
