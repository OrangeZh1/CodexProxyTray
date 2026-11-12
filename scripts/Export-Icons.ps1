$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$project = Split-Path -Parent $PSScriptRoot
$icons = Join-Path $project 'src-tauri\icons'
New-Item -ItemType Directory -Path $icons -Force | Out-Null

# Rasterize the same geometric paths as ui/icon.svg. No generated raster is edited.
function Draw-Logo([int]$Size) {
    $bitmap = [Drawing.Bitmap]::new($Size, $Size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
        if ($Size -le 24) { $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::Half }
        $graphics.ScaleTransform($Size / 124.0, $Size / 124.0)
        $graphics.TranslateTransform(-2, -2)
        $tile = [Drawing.Drawing2D.GraphicsPath]::new()
        $tile.AddArc(4, 4, 58, 58, 180, 90)
        $tile.AddArc(66, 4, 58, 58, 270, 90)
        $tile.AddArc(66, 66, 58, 58, 0, 90)
        $tile.AddArc(4, 66, 58, 58, 90, 90)
        $tile.CloseFigure()
        $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#167c67'))
        try { $graphics.FillPath($brush, $tile) } finally { $brush.Dispose(); $tile.Dispose() }
        $path = [Drawing.Drawing2D.GraphicsPath]::new()
        $path.AddLine(76, 36, 58, 36)
        $path.AddBezier(58, 36, 40, 36, 31, 49, 31, 66)
        $path.AddBezier(31, 66, 31, 83, 40, 95, 58, 95)
        $path.AddLine(58, 95, 68, 95)
        $path.AddBezier(68, 95, 76, 95, 80, 92, 86, 86)
        $path.AddLine(86, 86, 101, 68)
        $pen = [Drawing.Pen]::new([Drawing.Color]::White, $(if ($Size -le 24) { 16 } else { 14 }))
        $pen.StartCap = $pen.EndCap = [Drawing.Drawing2D.LineCap]::Round
        $pen.LineJoin = [Drawing.Drawing2D.LineJoin]::Round
        try { $graphics.DrawPath($pen, $path) } finally { $pen.Dispose(); $path.Dispose() }
        $arrow = [Drawing.Drawing2D.GraphicsPath]::new()
        $arrow.AddLine(85, 87, 105, 58)
        $arrow.StartFigure()
        $arrow.AddLine(86, 58, 105, 58)
        $arrow.AddLine(105, 58, 105, 77)
        $pen = [Drawing.Pen]::new([Drawing.ColorTranslator]::FromHtml('#d9ebb8'), $(if ($Size -le 24) { 14 } else { 12 }))
        $pen.StartCap = $pen.EndCap = [Drawing.Drawing2D.LineCap]::Round
        $pen.LineJoin = [Drawing.Drawing2D.LineJoin]::Round
        try { $graphics.DrawPath($pen, $arrow) } finally { $pen.Dispose(); $arrow.Dispose() }
    } finally { $graphics.Dispose() }
    return $bitmap
}

function ConvertTo-IconDib([Drawing.Bitmap]$Bitmap) {
    $size=$Bitmap.Width
    $stream=[IO.MemoryStream]::new();$writer=[IO.BinaryWriter]::new($stream)
    try {
        # Classic BGRA DIB + bottom-up AND mask is understood by process-icon readers.
        $writer.Write([uint32]40);$writer.Write([int32]$size);$writer.Write([int32]($size*2))
        $writer.Write([uint16]1);$writer.Write([uint16]32);$writer.Write([uint32]0)
        $writer.Write([uint32]($size*$size*4));$writer.Write([int32]0);$writer.Write([int32]0);$writer.Write([uint32]0);$writer.Write([uint32]0)
        for($y=$size-1;$y -ge 0;$y--){for($x=0;$x -lt $size;$x++){$pixel=$Bitmap.GetPixel($x,$y);$writer.Write([byte]$pixel.B);$writer.Write([byte]$pixel.G);$writer.Write([byte]$pixel.R);$writer.Write([byte]$pixel.A)}}
        $stride=[int]([Math]::Floor(($size+31)/32)*4)
        for($y=$size-1;$y -ge 0;$y--){$mask=[byte[]]::new($stride);for($x=0;$x -lt $size;$x++){if($Bitmap.GetPixel($x,$y).A -eq 0){$index=[int][Math]::Floor($x/8);$mask[$index]=$mask[$index] -bor (128 -shr ($x%8))}};$writer.Write($mask)}
        return ,$stream.ToArray()
    } finally {$writer.Dispose();$stream.Dispose()}
}

$logo = Draw-Logo 512
try {
    if ($logo.GetPixel(0, 0).A -ne 0 -or $logo.GetPixel(256, 256).A -ne 255) { throw 'Logo alpha verification failed' }
    for ($y = 128; $y -lt 384; $y += 8) { for ($x = 128; $x -lt 384; $x += 8) { if ($logo.GetPixel($x, $y).A -ne 255) { throw 'Unexpected transparent hole inside tile' } } }
    $logo.Save((Join-Path $icons 'icon.png'), [Drawing.Imaging.ImageFormat]::Png)
} finally { $logo.Dispose() }

$sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
$images = foreach ($size in $sizes) {
    $bitmap = Draw-Logo $size
    $stream = [IO.MemoryStream]::new()
    try {
        $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
        if ($size -le 64) { $bitmap.Save((Join-Path $icons "icon-$size.png"), [Drawing.Imaging.ImageFormat]::Png) }
        if($size -eq 256){,$stream.ToArray()}else{ConvertTo-IconDib $bitmap}
    } finally { $bitmap.Dispose(); $stream.Dispose() }
}
$file = [IO.File]::Create((Join-Path $icons 'icon.ico'))
$writer = [IO.BinaryWriter]::new($file)
try {
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Length)
    $offset = 6 + 16 * $sizes.Length
    for ($i = 0; $i -lt $sizes.Length; $i++) {
        $size = if ($sizes[$i] -eq 256) { 0 } else { $sizes[$i] }
        $writer.Write([byte]$size); $writer.Write([byte]$size); $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32); $writer.Write([uint32]$images[$i].Length); $writer.Write([uint32]$offset)
        $offset += $images[$i].Length
    }
    foreach ($bytes in $images) { $writer.Write([byte[]]$bytes) }
} finally { $writer.Dispose(); $file.Dispose() }
Write-Output 'PNG / ICO exported; outer transparency and opaque interior verified.'
