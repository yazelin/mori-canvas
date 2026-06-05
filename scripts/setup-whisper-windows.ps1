$ErrorActionPreference = "Stop"
# Windows local whisper-server for the whiteboard's "自訂 · 本機 whisper" STT.
# whisper.cpp v1.8.4+ ships PREBUILT release zips — incl. cuBLAS (GPU) ones that bundle
# the CUDA runtime, so GPU needs NO CUDA toolkit. We just download + drop in; no compiling.
# (Adapted from mori-meeting-recorder/scripts/install-whisper-windows.ps1; grabs the
# whisper-server, not the CLI. Self-contained: installs into .\whisper\ in this repo.)
#
# Env: WHISPER_VERSION (v1.8.4) | WHISPER_PORT (8089) | WHISPER_MODEL (small | large-v3-turbo)

$ver = if ($env:WHISPER_VERSION) { $env:WHISPER_VERSION } else { "v1.8.4" }
$port = if ($env:WHISPER_PORT) { $env:WHISPER_PORT } else { "8089" }
$model = if ($env:WHISPER_MODEL) { $env:WHISPER_MODEL } else { "small" }
$root = Split-Path -Parent $PSScriptRoot
$binDir = "$root\whisper\bin"
$modelDir = "$root\whisper\models"
New-Item -ItemType Directory -Force -Path $binDir, $modelDir | Out-Null

if (-not (Test-Path "$binDir\whisper-server.exe")) {
    # NVIDIA GPU → cuBLAS (GPU) zip (driver >=550 → CUDA 12.4, else 11.8 for older drivers); else BLAS (CPU)
    $gpu = [bool](Get-Command nvidia-smi -ErrorAction SilentlyContinue)
    if ($gpu) {
        $drv = (nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>$null | Select-Object -First 1)
        $major = 0; if ($drv -match '^\s*(\d+)') { $major = [int]$Matches[1] }
        if ($major -ge 550) { $zip = "whisper-cublas-12.4.0-bin-x64.zip" } else { $zip = "whisper-cublas-11.8.0-bin-x64.zip" }
        Write-Host "→ NVIDIA GPU(驅動 $drv)→ GPU(cuBLAS)版 $zip(自帶 CUDA runtime,免裝 toolkit)"
    } else {
        $zip = "whisper-blas-bin-x64.zip"
        Write-Host "→ 無 NVIDIA GPU → CPU(BLAS)版 whisper.cpp"
    }
    $url = "https://github.com/ggml-org/whisper.cpp/releases/download/$ver/$zip"
    Invoke-WebRequest -Uri $url -OutFile "$env:TEMP\wb-whisper.zip"
    $unzip = "$env:TEMP\wb-whisper-unzip"
    if (Test-Path $unzip) { Remove-Item -Recurse -Force $unzip }
    Expand-Archive -Force "$env:TEMP\wb-whisper.zip" -DestinationPath $unzip

    # grab whisper-server.exe (older zips: server.exe) + ALL dlls beside it (ggml-cuda.dll + cuda runtime)
    $srv = Get-ChildItem -Path $unzip -Recurse -Include "whisper-server.exe", "server.exe" | Select-Object -First 1
    if (-not $srv) {
        Write-Error "zip 裡找不到 whisper-server.exe / server.exe(此 release 可能沒附 server)。改用其它 whisper-server 發行或自編。"
        exit 1
    }
    Copy-Item $srv.FullName "$binDir\whisper-server.exe" -Force
    Copy-Item "$($srv.Directory.FullName)\*.dll" $binDir -Force
    Remove-Item -Recurse -Force $unzip, "$env:TEMP\wb-whisper.zip"
    Write-Host "✓ installed: $binDir\whisper-server.exe (+ dlls)"
} else {
    Write-Host "✓ already installed: $binDir\whisper-server.exe"
}

$mf = "$modelDir\ggml-$model.bin"
if (-not (Test-Path $mf)) {
    Write-Host "→ downloading ggml-$model model…"
    Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$model.bin" -OutFile $mf
}

# run helper
$run = "$root\whisper\run-whisper.ps1"
"& `"$binDir\whisper-server.exe`" -m `"$mf`" --host 127.0.0.1 --port $port --inference-path /inference" | Set-Content -Encoding UTF8 $run

Write-Host ""
Write-Host "✓ done."
Write-Host "→ 啟動本機 whisper-server:  powershell -ExecutionPolicy Bypass -File whisper\run-whisper.ps1   (127.0.0.1:$port)"
Write-Host "→ 白板 ⚙ 設定 → 自訂 → 本機 whisper,網址填:  http://127.0.0.1:$port/inference"
