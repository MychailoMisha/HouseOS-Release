param(
    [string]$GrubMkrescuePath = "grub-mkrescue",
    [string]$XorrisoPath = "",
    [string]$TarPath = "tar",
    [string]$IsoPath = "",
    [string]$ImagePath = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$buildDir = Join-Path $root "build"
$isoRoot = Join-Path $buildDir "iso_root"
$grubDir = Join-Path $isoRoot "boot\grub"          # створюється лише для GRUB
$limineDir = Join-Path $root "boot\limine"
$kernelBuild = Join-Path $root "kernel\build.ps1"
$kernelElf = Join-Path $root "kernel\target\i686-houseos\release\houseos-kernel"
$kernelBin = Join-Path $root "kernel\target\i686-houseos\release\houseos-kernel.bin"
$fsDir = Join-Path $root "fs"
$assetBackgroundDir = Join-Path $root "assets\background"

# Initrd одразу в корінь ISO (без зайвих копій)
$initrd = Join-Path $isoRoot "INITRD.TAR"
$initrdStaging = Join-Path $buildDir "initrd_staging"
$fsImage = Join-Path $buildDir "fs.img"
$fsVolumeImage = Join-Path $buildDir "fs_volume.img"
$imageWidth = 1024
$imageHeight = 768
$cursorWidth = 32
$cursorHeight = 32

# -------- FAT32 для файлової системи (ваша реалізація) --------
function New-Fat32Image {
    param(
        [string]$SourceDir,
        [string]$OutPath,
        [string]$Label = "HOUSEOS"
    )

    if (-not (Test-Path $SourceDir)) {
        throw "FAT32 source folder not found at $SourceDir"
    }

    $bytesPerSector = 512
    $sectorsPerCluster = 1
    $clusterSize = $bytesPerSector * $sectorsPerCluster
    $reservedSectors = 32
    $numFats = 2

    function New-Node([string]$path, $parent) {
        $item = Get-Item -LiteralPath $path
        $node = [PSCustomObject]@{
            Name = $item.Name
            Path = $item.FullName
            IsDir = $item.PSIsContainer
            Size = if ($item.PSIsContainer) { 0 } else { [int]$item.Length }
            Children = @()
            Parent = $parent
            ClusterStart = 0
            ClusterCount = 0
        }
        if ($node.IsDir) {
            foreach ($child in (Get-ChildItem -LiteralPath $path -Force)) {
                $node.Children += New-Node $child.FullName $node
            }
        }
        return $node
    }

    function Get-AllDirs($node) {
        $list = @()
        if ($node.IsDir) {
            $list += $node
            foreach ($child in $node.Children) {
                if ($child.IsDir) {
                    $list += Get-AllDirs $child
                }
            }
        }
        return $list
    }

    function Get-AllFiles($node) {
        $list = @()
        if (-not $node.IsDir) {
            $list += $node
        } else {
            foreach ($child in $node.Children) {
                $list += Get-AllFiles $child
            }
        }
        return $list
    }

    function Convert-ToFatName([string]$name, [bool]$isDir) {
        $base = $name
        $ext = ""
        if (-not $isDir) {
            $dot = $name.LastIndexOf(".")
            if ($dot -gt 0 -and $dot -lt ($name.Length - 1)) {
                $base = $name.Substring(0, $dot)
                $ext = $name.Substring($dot + 1)
            }
        }
        $base = ($base.ToUpper() -replace '[^A-Z0-9]', '_')
        $ext = ($ext.ToUpper() -replace '[^A-Z0-9]', '_')
        if ($base.Length -gt 8) { $base = $base.Substring(0, 8) }
        if ($ext.Length -gt 3) { $ext = $ext.Substring(0, 3) }
        if ($base.Length -eq 0) { $base = "FILE" }

        $name11 = New-Object byte[] 11
        for ($i = 0; $i -lt 11; $i++) { $name11[$i] = 0x20 }
        $b = [System.Text.Encoding]::ASCII.GetBytes($base)
        [Array]::Copy($b, 0, $name11, 0, [Math]::Min(8, $b.Length))
        if ($ext.Length -gt 0) {
            $e = [System.Text.Encoding]::ASCII.GetBytes($ext)
            [Array]::Copy($e, 0, $name11, 8, [Math]::Min(3, $e.Length))
        }
        return $name11
    }

    function Make-DirEntry([byte[]]$name11, [byte]$attr, [int]$cluster, [uint32]$size) {
        $entry = New-Object byte[] 32
        [Array]::Copy($name11, 0, $entry, 0, 11)
        $entry[11] = $attr
        $hi = [UInt16](($cluster -shr 16) -band 0xFFFF)
        $lo = [UInt16]($cluster -band 0xFFFF)
        [Array]::Copy([BitConverter]::GetBytes($hi), 0, $entry, 20, 2)
        [Array]::Copy([BitConverter]::GetBytes($lo), 0, $entry, 26, 2)
        [Array]::Copy([BitConverter]::GetBytes([UInt32]$size), 0, $entry, 28, 4)
        return $entry
    }

    function Compute-Clusters($node) {
        if ($node.IsDir) {
            foreach ($child in $node.Children) {
                Compute-Clusters $child
            }
            $entryCount = $node.Children.Count + 2
            $bytes = $entryCount * 32
            $node.ClusterCount = [Math]::Ceiling($bytes / $clusterSize)
            if ($node.ClusterCount -lt 1) { $node.ClusterCount = 1 }
        } else {
            if ($node.Size -gt 0) {
                $node.ClusterCount = [Math]::Ceiling($node.Size / $clusterSize)
            } else {
                $node.ClusterCount = 0
            }
        }
    }

    $rootNode = New-Node $SourceDir $null
    $rootNode.Name = ""
    Compute-Clusters $rootNode

    $nextCluster = 2
    $rootNode.ClusterStart = $nextCluster
    $nextCluster += $rootNode.ClusterCount
    $dirList = Get-AllDirs $rootNode | Where-Object { $_ -ne $rootNode }
    foreach ($dir in $dirList) {
        $dir.ClusterStart = $nextCluster
        $nextCluster += $dir.ClusterCount
    }
    $fileList = Get-AllFiles $rootNode
    foreach ($file in $fileList) {
        if ($file.ClusterCount -gt 0) {
            $file.ClusterStart = $nextCluster
            $nextCluster += $file.ClusterCount
        } else {
            $file.ClusterStart = 0
        }
    }

    $usedClusters = $nextCluster - 2
    $totalClusters = [Math]::Max($usedClusters + 32, 128)
    $fatSizeSectors = [Math]::Ceiling((($totalClusters + 2) * 4) / $bytesPerSector)
    $totalSectors = $reservedSectors + ($numFats * $fatSizeSectors) + ($totalClusters * $sectorsPerCluster)
    $imageSize = $totalSectors * $bytesPerSector
    $img = New-Object byte[] $imageSize

    function Write-U16([byte[]]$buf, [int]$offset, [int]$val) {
        [Array]::Copy([BitConverter]::GetBytes([UInt16]$val), 0, $buf, $offset, 2)
    }
    function Write-U32([byte[]]$buf, [int]$offset, [uint32]$val) {
        [Array]::Copy([BitConverter]::GetBytes([UInt32]$val), 0, $buf, $offset, 4)
    }

    $img[0] = 0xEB
    $img[1] = 0x58
    $img[2] = 0x90
    [Array]::Copy([System.Text.Encoding]::ASCII.GetBytes("MSWIN4.1"), 0, $img, 3, 8)
    Write-U16 $img 11 $bytesPerSector
    $img[13] = [byte]$sectorsPerCluster
    Write-U16 $img 14 $reservedSectors
    $img[16] = [byte]$numFats
    Write-U16 $img 17 0
    Write-U16 $img 19 0
    $img[21] = 0xF8
    Write-U16 $img 22 0
    Write-U16 $img 24 63
    Write-U16 $img 26 255
    Write-U32 $img 28 0
    Write-U32 $img 32 $totalSectors
    Write-U32 $img 36 $fatSizeSectors
    Write-U16 $img 40 0
    Write-U16 $img 42 0
    Write-U32 $img 44 2
    Write-U16 $img 48 1
    Write-U16 $img 50 6
    $img[64] = 0x80
    $img[66] = 0x29
    Write-U32 $img 67 0x12345678
    $labelText = ($Label.ToUpper() + "           ").Substring(0, 11)
    [Array]::Copy([System.Text.Encoding]::ASCII.GetBytes($labelText), 0, $img, 71, 11)
    [Array]::Copy([System.Text.Encoding]::ASCII.GetBytes("FAT32   "), 0, $img, 82, 8)
    $img[510] = 0x55
    $img[511] = 0xAA

    $fsInfoOffset = $bytesPerSector
    Write-U32 $img ($fsInfoOffset + 0) 0x41615252
    Write-U32 $img ($fsInfoOffset + 484) 0x61417272
    Write-U32 $img ($fsInfoOffset + 488) ([uint32]::MaxValue)
    Write-U32 $img ($fsInfoOffset + 492) ([uint32]::MaxValue)
    $img[$fsInfoOffset + 510] = 0x55
    $img[$fsInfoOffset + 511] = 0xAA

    $bootCopyOffset = $bytesPerSector * 6
    [Array]::Copy($img, 0, $img, $bootCopyOffset, $bytesPerSector)
    $fsCopyOffset = $bytesPerSector * 7
    [Array]::Copy($img, $fsInfoOffset, $img, $fsCopyOffset, $bytesPerSector)

    $fatEntries = New-Object UInt32[] ($totalClusters + 2)
    $fatEntries[0] = 0x0FFFFFF8
    $fatEntries[1] = 0x0FFFFFFF

    function Set-Chain([int]$start, [int]$count) {
        if ($count -le 0) { return }
        for ($i = 0; $i -lt $count; $i++) {
            $cluster = $start + $i
            if ($i -lt ($count - 1)) {
                $fatEntries[$cluster] = [UInt32]($cluster + 1)
            } else {
                $fatEntries[$cluster] = 0x0FFFFFFF
            }
        }
    }

    Set-Chain $rootNode.ClusterStart $rootNode.ClusterCount
    foreach ($dir in $dirList) { Set-Chain $dir.ClusterStart $dir.ClusterCount }
    foreach ($file in $fileList) {
        if ($file.ClusterCount -gt 0) { Set-Chain $file.ClusterStart $file.ClusterCount }
    }

    $fatOffset = $reservedSectors * $bytesPerSector
    for ($i = 0; $i -lt $fatEntries.Length; $i++) {
        $entryBytes = [BitConverter]::GetBytes([UInt32]$fatEntries[$i])
        [Array]::Copy($entryBytes, 0, $img, $fatOffset + ($i * 4), 4)
    }
    $fatBytes = $fatSizeSectors * $bytesPerSector
    [Array]::Copy($img, $fatOffset, $img, $fatOffset + $fatBytes, $fatBytes)

    $dataOffset = $fatOffset + ($numFats * $fatBytes)

    function Write-DataToClusters([int]$startCluster, [int]$clusterCount, [byte[]]$data) {
        if ($clusterCount -le 0) { return }
        $remaining = $data.Length
        $srcOffset = 0
        for ($i = 0; $i -lt $clusterCount; $i++) {
            $cluster = $startCluster + $i
            $dstOffset = $dataOffset + ($cluster - 2) * $clusterSize
            $chunk = [Math]::Min($clusterSize, $remaining)
            if ($chunk -gt 0) {
                [Array]::Copy($data, $srcOffset, $img, $dstOffset, $chunk)
                $srcOffset += $chunk
                $remaining -= $chunk
            }
        }
    }

    function Write-Dir($node) {
        $entryCount = $node.Children.Count + 2
        $dirBytes = New-Object byte[] ($entryCount * 32)
        $offset = 0
        $dot = New-Object byte[] 11
        $dot[0] = 0x2E
        for ($i = 1; $i -lt 11; $i++) { $dot[$i] = 0x20 }
        $entry = Make-DirEntry $dot 0x10 $node.ClusterStart 0
        [Array]::Copy($entry, 0, $dirBytes, $offset, 32)
        $offset += 32

        $dotdot = New-Object byte[] 11
        $dotdot[0] = 0x2E
        $dotdot[1] = 0x2E
        for ($i = 2; $i -lt 11; $i++) { $dotdot[$i] = 0x20 }
        $parentCluster = if ($null -ne $node.Parent) { $node.Parent.ClusterStart } else { 0 }
        $entry = Make-DirEntry $dotdot 0x10 $parentCluster 0
        [Array]::Copy($entry, 0, $dirBytes, $offset, 32)
        $offset += 32

        foreach ($child in $node.Children) {
            $name11 = Convert-ToFatName $child.Name $child.IsDir
            $attr = if ($child.IsDir) { 0x10 } else { 0x20 }
            $cluster = if ($child.ClusterCount -gt 0) { $child.ClusterStart } else { 0 }
            $size = if ($child.IsDir) { 0 } else { [uint32]$child.Size }
            $entry = Make-DirEntry $name11 $attr $cluster $size
            [Array]::Copy($entry, 0, $dirBytes, $offset, 32)
            $offset += 32
        }

        Write-DataToClusters $node.ClusterStart $node.ClusterCount $dirBytes
        foreach ($child in $node.Children) {
            if ($child.IsDir) { Write-Dir $child }
        }
    }

    Write-Dir $rootNode

    foreach ($file in $fileList) {
        if ($file.ClusterCount -le 0) { continue }
        $data = [System.IO.File]::ReadAllBytes($file.Path)
        Write-DataToClusters $file.ClusterStart $file.ClusterCount $data
    }

    [System.IO.File]::WriteAllBytes($OutPath, $img)
}

# -------- Перевірка шляхів --------
function New-HouseOsDiskImage {
    param(
        [string]$VolumePath,
        [string]$OutPath
    )

    $bytesPerSector = 512
    $partitionStart = 64
    $volumeBytes = [System.IO.File]::ReadAllBytes($VolumePath)
    $partitionSectors = [int][Math]::Ceiling($volumeBytes.Length / $bytesPerSector)
    $diskSectors = [int]($partitionStart + $partitionSectors)
    $diskBytes = New-Object byte[] ($diskSectors * $bytesPerSector)

    $entry = 446
    $diskBytes[$entry + 0] = 0x80
    $diskBytes[$entry + 1] = 0x01
    $diskBytes[$entry + 2] = 0x01
    $diskBytes[$entry + 3] = 0x00
    $diskBytes[$entry + 4] = 0x0C
    $diskBytes[$entry + 5] = 0xFE
    $diskBytes[$entry + 6] = 0xFF
    $diskBytes[$entry + 7] = 0xFF
    [Array]::Copy([BitConverter]::GetBytes([UInt32]$partitionStart), 0, $diskBytes, $entry + 8, 4)
    [Array]::Copy([BitConverter]::GetBytes([UInt32]$partitionSectors), 0, $diskBytes, $entry + 12, 4)
    $diskBytes[510] = 0x55
    $diskBytes[511] = 0xAA

    [Array]::Copy($volumeBytes, 0, $diskBytes, $partitionStart * $bytesPerSector, $volumeBytes.Length)
    [System.IO.File]::WriteAllBytes($OutPath, $diskBytes)
}

if ([string]::IsNullOrWhiteSpace($IsoPath)) {
    $IsoPath = Join-Path $buildDir "houseos.iso"
}
if ([string]::IsNullOrWhiteSpace($ImagePath)) {
    $ImagePath = Join-Path $root "assets\background.png"
}
$cursorPath = Join-Path $root "assets\cursor.png"

if (-not (Test-Path $kernelBuild)) {
    throw "Kernel build script not found at $kernelBuild"
}
if (-not (Test-Path $fsDir)) {
    throw "Filesystem folder not found at $fsDir"
}
New-Item -ItemType Directory -Force -Path $assetBackgroundDir | Out-Null
if (-not (Test-Path $ImagePath)) {
    throw "Image not found at $ImagePath"
}

if (-not (Get-Command $TarPath -ErrorAction SilentlyContinue)) {
    throw "tar not found. Install bsdtar or ensure tar is on PATH."
}

# -------- Очищення --------
if (Test-Path $isoRoot) {
    Remove-Item -Recurse -Force $isoRoot
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

# Створюємо лише корінь ISO (grubDir створиться пізніше, якщо буде потрібен)
New-Item -ItemType Directory -Force -Path $isoRoot | Out-Null

# -------- Збірка ядра --------
if (Test-Path $kernelElf) {
    Remove-Item -Force $kernelElf
}
if (Test-Path $kernelBin) {
    Remove-Item -Force $kernelBin
}

& $kernelBuild

if (-not (Test-Path $kernelElf)) {
    throw "Kernel ELF not found at $kernelElf"
}

# Копіюємо ядро в корінь ISO (єдине місце)
Copy-Item -Force $kernelElf (Join-Path $isoRoot "HOUSEOS.KRN")

Add-Type -AssemblyName System.Drawing

function Save-JpegHighQuality {
    param(
        [System.Drawing.Bitmap]$Bitmap,
        [string]$Path,
        [long]$Quality = 100
    )
    $codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() |
        Where-Object { $_.MimeType -eq "image/jpeg" } |
        Select-Object -First 1
    if ($null -eq $codec) {
        $Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Jpeg)
        return
    }
    $params = [System.Drawing.Imaging.EncoderParameters]::new(1)
    $params.Param[0] = [System.Drawing.Imaging.EncoderParameter]::new(
        [System.Drawing.Imaging.Encoder]::Quality,
        $Quality
    )
    $Bitmap.Save($Path, $codec, $params)
    $params.Dispose()
}

# -------- Генерація курсору (RAW) --------
if (-not (Test-Path $cursorPath)) {
    New-Item -ItemType Directory -Force -Path (Split-Path $cursorPath) | Out-Null
    $cursorBmp = [System.Drawing.Bitmap]::new($cursorWidth, $cursorHeight, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $cursorGfx = [System.Drawing.Graphics]::FromImage($cursorBmp)
    $cursorGfx.Clear([System.Drawing.Color]::Transparent)
    $cursorPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::Black)
    $cursorBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::White)
    $points = @(
        New-Object System.Drawing.Point 0,0
        New-Object System.Drawing.Point 0,22
        New-Object System.Drawing.Point 6,16
        New-Object System.Drawing.Point 9,28
        New-Object System.Drawing.Point 12,27
        New-Object System.Drawing.Point 9,15
        New-Object System.Drawing.Point 20,15
    )
    $cursorGfx.FillPolygon($cursorBrush, $points)
    $cursorGfx.DrawPolygon($cursorPen, $points)
    $cursorGfx.Dispose()
    $cursorBmp.Save($cursorPath, [System.Drawing.Imaging.ImageFormat]::Png)
    $cursorBmp.Dispose()
}

function Save-CoverJpeg {
    param(
        [string]$SourcePath,
        [string]$OutPath,
        [int]$Width,
        [int]$Height,
        [long]$Quality = 86
    )
    if (-not (Test-Path $SourcePath)) {
        return $false
    }
    $src = [System.Drawing.Bitmap]::new($SourcePath)
    try {
        $bmp = [System.Drawing.Bitmap]::new($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $gfx = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $gfx.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $srcRatio = $src.Width / $src.Height
            $dstRatio = $Width / $Height
            if ($srcRatio -gt $dstRatio) {
                $cropH = $src.Height
                $cropW = [int][Math]::Ceiling($src.Height * $dstRatio)
                $cropX = [int](($src.Width - $cropW) / 2)
                $cropY = 0
            } else {
                $cropW = $src.Width
                $cropH = [int][Math]::Ceiling($src.Width / $dstRatio)
                $cropX = 0
                $cropY = [int](($src.Height - $cropH) / 2)
            }
            $srcRect = New-Object System.Drawing.Rectangle($cropX, $cropY, $cropW, $cropH)
            $dstRect = New-Object System.Drawing.Rectangle(0, 0, $Width, $Height)
            $gfx.DrawImage($src, $dstRect, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
        }
        finally {
            $gfx.Dispose()
        }
        Save-JpegHighQuality -Bitmap $bmp -Path $OutPath -Quality $Quality
        $bmp.Dispose()
    }
    finally {
        $src.Dispose()
    }
    return $true
}

$fsStaging = Join-Path $buildDir "fs_staging"
if (Test-Path $fsStaging) {
    Remove-Item -Recurse -Force $fsStaging
}
New-Item -ItemType Directory -Force -Path $fsStaging | Out-Null
$houseRoot = Join-Path $fsStaging "HOUSE_OS"
$houseFiles = Join-Path $houseRoot "FILES"
$houseSystem = Join-Path $houseRoot "SYSTEM"
$houseUsers = Join-Path $houseRoot "USERS"
$housePrograms = Join-Path $houseRoot "PROGRAMS"
$houseTemp = Join-Path $houseRoot "TEMP"
$houseFonts = Join-Path $houseRoot "FONTS"
$houseWallpapers = Join-Path $houseRoot "WALLPAPE"
New-Item -ItemType Directory -Force -Path $houseFiles, $houseSystem, $houseUsers, $housePrograms, $houseTemp, $houseFonts, $houseWallpapers | Out-Null
foreach ($item in (Get-ChildItem -LiteralPath $fsDir -Force)) {
    Copy-Item -LiteralPath $item.FullName -Destination $houseFiles -Recurse -Force
}
Set-Content -Path (Join-Path $houseRoot "README.TXT") -Encoding Ascii -Value @(
    "HouseOS main disk partition.",
    "System files live in SYSTEM, user files in USERS, and imported files in FILES."
)
Set-Content -Path (Join-Path $houseSystem "BOOT.TXT") -Encoding Ascii -Value "HouseOS boot/system folder."
Set-Content -Path (Join-Path $houseUsers "USER.TXT") -Encoding Ascii -Value "Default HouseOS user folder."
Set-Content -Path (Join-Path $housePrograms "APPS.TXT") -Encoding Ascii -Value "HouseOS applications folder."
Set-Content -Path (Join-Path $houseTemp "README.TXT") -Encoding Ascii -Value "Temporary files folder."
Set-Content -Path (Join-Path $houseFonts "README.TXT") -Encoding Ascii -Value "Optional fonts folder. Default UI uses built-in bitmap text."
Set-Content -Path (Join-Path $houseWallpapers "README.TXT") -Encoding Ascii -Value "Wallpaper JPG files used by the Wallpaper window."
$officeSeed = "HouseOffice document. Edit this file in Office and press Save."
Set-Content -Path (Join-Path $houseFiles "OFFICE.TXT") -Encoding Ascii -Value ($officeSeed.PadRight(8192))


# Створюємо FAT32 образ і копіюємо в корінь ISO (єдине місце)
$allWallpaperFiles = @(Get-ChildItem -LiteralPath $assetBackgroundDir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Extension -match '^\.(jpg|jpeg|png)$' } |
    Sort-Object Name)
if ($allWallpaperFiles.Count -eq 0) {
    $defaultWallpaperSources = @(
        $ImagePath,
        (Join-Path $root "assets\background1.jpg"),
        (Join-Path $root "assets\WnAVs.png")
    )
    for ($i = 0; $i -lt $defaultWallpaperSources.Count; $i++) {
        $assetOut = Join-Path $assetBackgroundDir "BG$($i + 1).JPG"
        [void](Save-CoverJpeg -SourcePath $defaultWallpaperSources[$i] -OutPath $assetOut -Width $imageWidth -Height $imageHeight -Quality 82)
    }
    $allWallpaperFiles = @(Get-ChildItem -LiteralPath $assetBackgroundDir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -match '^\.(jpg|jpeg|png)$' } |
        Sort-Object Name)
}
$wallpaperSeedFiles = New-Object 'System.Collections.Generic.List[System.IO.FileInfo]'
$usedWallpaperPaths = @{}
foreach ($preferred in @("BG1.JPG", "BG2.JPG", "BG3.JPG")) {
    $match = $allWallpaperFiles | Where-Object { $_.Name -ieq $preferred } | Select-Object -First 1
    if ($null -ne $match) {
        [void]$wallpaperSeedFiles.Add($match)
        $usedWallpaperPaths[$match.FullName.ToLowerInvariant()] = $true
    }
}
foreach ($file in $allWallpaperFiles) {
    if ($wallpaperSeedFiles.Count -ge 16) {
        break
    }
    $key = $file.FullName.ToLowerInvariant()
    if ($usedWallpaperPaths.ContainsKey($key)) {
        continue
    }
    if ($file.Name -ieq "background.png" -or $file.Name -ieq "BG1.png") {
        continue
    }
    [void]$wallpaperSeedFiles.Add($file)
    $usedWallpaperPaths[$key] = $true
}
for ($i = 0; $i -lt $wallpaperSeedFiles.Count -and $i -lt 16; $i++) {
    $outName = if ($i -lt 3) { "BG$($i + 1).JPG" } else { "USER$($i + 1).JPG" }
    $outPath = Join-Path $houseWallpapers $outName
    [void](Save-CoverJpeg -SourcePath $wallpaperSeedFiles[$i].FullName -OutPath $outPath -Width $imageWidth -Height $imageHeight -Quality 82)
}

New-Fat32Image -SourceDir $fsStaging -OutPath $fsVolumeImage -Label "HOUSEOS"
New-HouseOsDiskImage -VolumePath $fsVolumeImage -OutPath $fsImage
Copy-Item -Force $fsImage (Join-Path $isoRoot "FS.IMG")
Copy-Item -Force -Recurse $houseWallpapers (Join-Path $isoRoot "WALLPAPE")

# ========== СТВОРЕННЯ ФОНУ У JPEG ==========
$imageJpg = Join-Path $buildDir "background.jpg"
$src = [System.Drawing.Bitmap]::new($ImagePath)
$bmp = [System.Drawing.Bitmap]::new($imageWidth, $imageHeight, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$gfx.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$srcRatio = $src.Width / $src.Height
$dstRatio = $imageWidth / $imageHeight
if ($srcRatio -gt $dstRatio) {
    $cropH = $src.Height
    $cropW = [int][Math]::Ceiling($src.Height * $dstRatio)
    $cropX = [int](($src.Width - $cropW) / 2)
    $cropY = 0
} else {
    $cropW = $src.Width
    $cropH = [int][Math]::Ceiling($src.Width / $dstRatio)
    $cropX = 0
    $cropY = [int](($src.Height - $cropH) / 2)
}
$srcRect = New-Object System.Drawing.Rectangle($cropX, $cropY, $cropW, $cropH)
$dstRect = New-Object System.Drawing.Rectangle(0, 0, $imageWidth, $imageHeight)
$gfx.DrawImage($src, $dstRect, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
$gfx.Dispose()
$src.Dispose()
$bmp.Save($imageJpg, [System.Drawing.Imaging.ImageFormat]::Jpeg)
$bmp.Dispose()

# Копіюємо JPEG як модуль фону в корінь ISO (єдине місце)
Copy-Item -Force $imageJpg (Join-Path $isoRoot "BACKGROUND.JPG")
# (Більше нікуди не копіюємо — жодних копій у initrd)
# ===========================================

# -------- Створення курсору RAW --------
$cursorRaw = Join-Path $buildDir "cursor.raw"
$csrc = [System.Drawing.Bitmap]::new($cursorPath)
$cbmp = [System.Drawing.Bitmap]::new($cursorWidth, $cursorHeight, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$cgfx = [System.Drawing.Graphics]::FromImage($cbmp)
$cgfx.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
$cgfx.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::None
$cgfx.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
$cgfx.Clear([System.Drawing.Color]::Transparent)
$cgfx.DrawImage($csrc, 0, 0, $cursorWidth, $cursorHeight)
$cgfx.Dispose()

$csrc.Dispose()

$crect = New-Object System.Drawing.Rectangle(0, 0, $cursorWidth, $cursorHeight)
$cbd = $cbmp.LockBits($crect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
try {
    $cstride = $cbd.Stride
    $cabsStride = [Math]::Abs($cstride)
    $crowBytes = $cursorWidth * 4
    $craw = New-Object byte[] ($crowBytes * $cursorHeight)
    for ($y = 0; $y -lt $cursorHeight; $y++) {
        if ($cstride -lt 0) {
            $csrcRow = $cursorHeight - 1 - $y
        } else {
            $csrcRow = $y
        }
        $coffset = $csrcRow * $cabsStride
        $cptr = [IntPtr]::Add($cbd.Scan0, $coffset)
        [System.Runtime.InteropServices.Marshal]::Copy($cptr, $craw, $y * $crowBytes, $crowBytes)
    }
}
finally {
    $cbmp.UnlockBits($cbd)
    $cbmp.Dispose()
}

for ($i = 0; $i -lt $craw.Length; $i += 4) {
    $a = $craw[$i + 3]
    if ($a -lt 128) {
        $craw[$i] = 0xFF
        $craw[$i + 1] = 0x00
        $craw[$i + 2] = 0xFF
        $craw[$i + 3] = 0xFF
    }
}

$cheader = New-Object byte[] 8
[System.Buffer]::BlockCopy([BitConverter]::GetBytes([UInt32]$cursorWidth), 0, $cheader, 0, 4)
[System.Buffer]::BlockCopy([BitConverter]::GetBytes([UInt32]$cursorHeight), 0, $cheader, 4, 4)
$crawWithHeader = New-Object byte[] ($cheader.Length + $craw.Length)
[System.Buffer]::BlockCopy($cheader, 0, $crawWithHeader, 0, $cheader.Length)
[System.Buffer]::BlockCopy($craw, 0, $crawWithHeader, $cheader.Length, $craw.Length)
[System.IO.File]::WriteAllBytes($cursorRaw, $crawWithHeader)

Copy-Item -Force $cursorRaw (Join-Path $fsStaging "cursor.raw")

# -------- Створення initrd (прямо в корінь ISO) --------
if (Test-Path $initrdStaging) {
    Remove-Item -Recurse -Force $initrdStaging
}
New-Item -ItemType Directory -Force -Path $initrdStaging | Out-Null
Copy-Item -Force $cursorRaw (Join-Path $initrdStaging "cursor.raw")
& $TarPath "-C" $initrdStaging "-cf" $initrd "."
if ($LASTEXITCODE -ne 0) {
    throw "tar failed while building initrd"
}

# -------- GRUB або Limine --------
$grubAvailable = $false
if (Get-Command $GrubMkrescuePath -ErrorAction SilentlyContinue) {
    $grubAvailable = $true
}

if ($grubAvailable) {
    # Створюємо grub-директорію лише зараз
    New-Item -ItemType Directory -Force -Path $grubDir | Out-Null

    # GRUB конфіг: все посилається на корінь ISO
    $grubCfg = @"
set timeout=0
set default=0
insmod all_video
set gfxmode=1024x768x32
set gfxpayload=keep
terminal_output gfxterm
menuentry "HouseOS" {
    multiboot2 /HOUSEOS.KRN
    module2 /INITRD.TAR
    module2 /FS.IMG
    module2 /BACKGROUND.JPG
    boot
}
"@

    Set-Content -Path (Join-Path $grubDir "grub.cfg") -Value $grubCfg -Encoding Ascii

    & $GrubMkrescuePath "-o" $IsoPath $isoRoot
    if ($LASTEXITCODE -ne 0) {
        throw "grub-mkrescue failed while building the ISO"
    }
    Write-Host "GRUB ISO created at $IsoPath"
    exit 0
}

# -------- Limine (основний варіант) --------
if ([string]::IsNullOrWhiteSpace($XorrisoPath)) {
    if (Get-Command "xorriso" -ErrorAction SilentlyContinue) {
        $XorrisoPath = "xorriso"
    } elseif (Test-Path "C:\msys64\usr\bin\xorriso.exe") {
        $XorrisoPath = "C:\msys64\usr\bin\xorriso.exe"
    } elseif (Test-Path "C:\msys64\mingw64\bin\xorriso.exe") {
        $XorrisoPath = "C:\msys64\mingw64\bin\xorriso.exe"
    }
}

if (-not (Get-Command $XorrisoPath -ErrorAction SilentlyContinue)) {
    throw "Neither grub-mkrescue nor xorriso are available. Install GRUB tools or provide -XorrisoPath."
}

function Convert-ToMsysPath([string]$p) {
    $full = [System.IO.Path]::GetFullPath($p)
    if ($full -match '^[A-Za-z]:\\') {
        $drive = $full.Substring(0, 1).ToLower()
        $rest = $full.Substring(2) -replace '\\', '/'
        return "/$drive/$rest"
    }
    return $p
}

$limineBiosCd = Join-Path $limineDir "limine-bios-cd.bin"
$limineBiosSys = Join-Path $limineDir "limine-bios.sys"
if (-not (Test-Path $limineBiosCd) -or -not (Test-Path $limineBiosSys)) {
    throw "Limine BIOS files not found in $limineDir. Place limine-bios-cd.bin and limine-bios.sys there."
}

$limineExe = Join-Path $limineDir "limine.exe"
if (-not (Test-Path $limineExe)) {
    if (Get-Command "limine" -ErrorAction SilentlyContinue) {
        $limineExe = "limine"
    } elseif (Test-Path "C:\msys64\usr\bin\limine.exe") {
        $limineExe = "C:\msys64\usr\bin\limine.exe"
    } elseif (Test-Path "C:\msys64\mingw64\bin\limine.exe") {
        $limineExe = "C:\msys64\mingw64\bin\limine.exe"
    } else {
        $limineExe = $null
    }
}

if (-not $limineExe -or -not (Get-Command $limineExe -ErrorAction SilentlyContinue)) {
    throw "Limine tool not found. Place limine.exe in $limineDir or add limine to PATH."
}

# Копіюємо файли Limine в корінь ISO (тільки один раз)
Copy-Item -Force $limineBiosCd (Join-Path $isoRoot "limine-bios-cd.bin")
Copy-Item -Force $limineBiosSys (Join-Path $isoRoot "limine-bios.sys")
Copy-Item -Force $limineBiosSys (Join-Path $isoRoot "limine.sys")

$limineUefiCd = Join-Path $limineDir "limine-uefi-cd.bin"
$limineBootX64 = Join-Path $limineDir "BOOTX64.EFI"
$limineBootIa32 = Join-Path $limineDir "BOOTIA32.EFI"
$hasUefi = (Test-Path $limineUefiCd) -and ((Test-Path $limineBootX64) -or (Test-Path $limineBootIa32))
if ($hasUefi) {
    Copy-Item -Force $limineUefiCd (Join-Path $isoRoot "limine-uefi-cd.bin")
    $efiBootDir = Join-Path $isoRoot "EFI\BOOT"
    New-Item -ItemType Directory -Force -Path $efiBootDir | Out-Null
    if (Test-Path $limineBootX64) {
        Copy-Item -Force $limineBootX64 (Join-Path $efiBootDir "BOOTX64.EFI")
    }
    if (Test-Path $limineBootIa32) {
        Copy-Item -Force $limineBootIa32 (Join-Path $efiBootDir "BOOTIA32.EFI")
    }
}

# -------- Limine конфіг --------
$limineCfg = @"
timeout: 0
/HouseOS
    protocol: multiboot2
    resolution: 1024x768x32
    textmode: no
    kernel_path: boot():/HOUSEOS.KRN
    module_path: boot():/INITRD.TAR
    module_string: initrd
    module_path: boot():/FS.IMG
    module_string: fs
    module_path: boot():/BACKGROUND.JPG
    module_string: background.jpg
"@

$limineCfgPath = Join-Path $isoRoot "limine.conf"
Set-Content -Path $limineCfgPath -Value $limineCfg -Encoding Ascii -NoNewline
$legacyCfgPath = Join-Path $isoRoot "limine.cfg"
Set-Content -Path $legacyCfgPath -Value $limineCfg -Encoding Ascii -NoNewline

# -------- Створення ISO --------
$isoRootForXorriso = $isoRoot
$isoPathForXorriso = $IsoPath
if ($XorrisoPath -like "*\msys64\*") {
    $isoRootForXorriso = Convert-ToMsysPath $isoRoot
    $isoPathForXorriso = Convert-ToMsysPath $IsoPath
}

if ($hasUefi) {
    & $XorrisoPath "-as" "mkisofs" "-R" "-J" "-joliet-long" "-iso-level" "3" "-b" "limine-bios-cd.bin" "-no-emul-boot" "-boot-load-size" "4" "-boot-info-table" "--efi-boot" "limine-uefi-cd.bin" "-efi-boot-part" "--efi-boot-image" "--protective-msdos-label" "-o" $isoPathForXorriso $isoRootForXorriso
} else {
    & $XorrisoPath "-as" "mkisofs" "-R" "-J" "-joliet-long" "-iso-level" "3" "-b" "limine-bios-cd.bin" "-no-emul-boot" "-boot-load-size" "4" "-boot-info-table" "-o" $isoPathForXorriso $isoRootForXorriso
}
if ($LASTEXITCODE -ne 0) {
    throw "xorriso failed while building the ISO"
}

& $limineExe "bios-install" $IsoPath
if ($LASTEXITCODE -ne 0) {
    throw "limine bios-install failed for $IsoPath"
}
Write-Host "Limine ISO created at $IsoPath"
