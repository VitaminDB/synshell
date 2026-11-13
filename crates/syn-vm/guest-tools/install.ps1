# Гостевые инструменты syn-vm: драйвер virtio-serial (syn-vm-serial), гостевой агент syn-vm (служба),
# при наличии видеокарты syn-gpu — её инструменты (каталог win10\syn-gpu\ на диске).
# Запуск: install.cmd [/quiet] (администратор или SYSTEM). Коды: 0 — готово, 3010 — нужна перезагрузка
# (после неё установка закончится сама), иное — ошибка. Журнал: C:\ProgramData\syn-vm\install.log.
# Повторный запуск = обновление.
param([switch]$Quiet, [switch]$Resume)

$ErrorActionPreference = 'Stop'
$Data  = Join-Path $env:ProgramData 'syn-vm'
$Stage = Join-Path $Data 'stage'
$Log   = Join-Path $Data 'install.log'
$StateFile = Join-Path $Data 'install-state.txt'
$TaskName = 'syn-vm-install-resume'
$LocalCertSubject = 'CN=syn-vm local driver signing'

New-Item -ItemType Directory -Force -Path $Data | Out-Null
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

function Log([string]$m) {
    Add-Content -Path $Log -Value ('[{0}] {1}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $m) -Encoding UTF8
    if (-not $Quiet) { Write-Host $m }
}
function SetState([string]$s) { Set-Content -Path $StateFile -Value $s -Encoding UTF8 }
function Fail([string]$m, [int]$code = 1) {
    Log "ОШИБКА: $m"
    SetState "error: $m"
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    exit $code
}

try {
    Log "=== гостевые инструменты syn-vm (Resume=$Resume) ==="
    SetState 'running'

    # 1. Копия диска: после перезагрузки его может не быть в приводе.
    if (-not $Resume) {
        $Src = $PSScriptRoot.TrimEnd('\')
        if ($Src -ne $Stage) {
            if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
            New-Item -ItemType Directory -Force -Path $Stage | Out-Null
            Copy-Item -Recurse -Force "$Src\*" $Stage
            Get-ChildItem -Recurse $Stage | ForEach-Object { if (-not $_.PSIsContainer) { $_.IsReadOnly = $false } }
        }
    }
    $S = $Stage
    $NeedReboot = $false
    $HaveSerial = Test-Path (Join-Path $S 'win10\syn-vm-serial\syn-vm-serial.inf')

    if ($HaveSerial) {
        # 2. Драйверы с тестовой подписью: Secure Boot должен быть выключен.
        $sb = $false
        try { $sb = [bool](Confirm-SecureBootUEFI -ErrorAction Stop) } catch { $sb = $false }
        if ($sb) { Fail 'включён Secure Boot: драйверы с тестовой подписью не загрузятся. В syn-vm выберите прошивку «UEFI» без Secure Boot.' 2 }

        # 3. Открытая часть сертификата тестовой подписи.
        $cerDev = Join-Path $S 'win10\syn-vm-serial\syn-virt-test.cer'
        Import-Certificate -FilePath $cerDev -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
        Import-Certificate -FilePath $cerDev -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null

        # 4. Тестовая подпись включается до загрузки ядра — перезагрузка, затем продолжение само.
        $bcd = (bcdedit /enum '{current}') -join "`n"
        if ($bcd -notmatch 'testsigning\s+(Yes|Да)') {
            bcdedit /set testsigning on | Out-Null
            if ($LASTEXITCODE -ne 0) { Fail "bcdedit /set testsigning on: код $LASTEXITCODE" }
            $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$S\install.ps1`" -Resume -Quiet"
            $trigger = New-ScheduledTaskTrigger -AtStartup
            $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -RunLevel Highest
            Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Force | Out-Null
            Log 'тестовая подпись включена — нужна перезагрузка, установка закончится сама'
            SetState 'reboot'
            exit 3010
        }

        # 5. Драйвер virtio-serial: каталог .cat подписывается сертификатом этой машины.
        $Drv = Join-Path $Data 'driver-serial'
        if (Test-Path $Drv) { Remove-Item -Recurse -Force $Drv }
        New-Item -ItemType Directory -Force -Path $Drv | Out-Null
        Copy-Item -Force (Join-Path $S 'win10\syn-vm-serial\syn-vm-serial.inf'), (Join-Path $S 'win10\syn-vm-serial\syn-vm-serial.sys') $Drv
        Get-ChildItem $Drv | ForEach-Object { $_.IsReadOnly = $false }
        $cert = Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.Subject -eq $LocalCertSubject -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date).AddDays(30) } | Select-Object -First 1
        if (-not $cert) {
            $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $LocalCertSubject -CertStoreLocation Cert:\LocalMachine\My `
                -KeyExportPolicy NonExportable -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -NotAfter (Get-Date).AddYears(20)
            Log "создан сертификат машины $($cert.Thumbprint)"
        }
        $cer = Join-Path $Data 'local-signing.cer'
        Export-Certificate -Cert $cert -FilePath $cer | Out-Null
        Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
        Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
        $cat = Join-Path $Drv 'syn-vm-serial.cat'
        New-FileCatalog -Path $Drv -CatalogFilePath $cat -CatalogVersion 2.0 | Out-Null
        $sig = Set-AuthenticodeSignature -FilePath $cat -Certificate $cert -HashAlgorithm SHA256
        if ($sig.Status -ne 'Valid') { Fail "подпись каталога: $($sig.Status) $($sig.StatusMessage)" }
        $out = cmd /c ('chcp 65001 >nul & pnputil /add-driver "' + (Join-Path $Drv 'syn-vm-serial.inf') + '" /install') 2>&1 | Out-String
        $pnp = $LASTEXITCODE
        Log ("драйвер syn-vm-serial: pnputil код $pnp`n" + $out.Trim())
        if ($pnp -ne 0 -and $pnp -ne 3010 -and $pnp -ne 259) { Fail "pnputil /add-driver: код $pnp" }
        # Драйвер устройства, которое уже работает, заменится только после перезагрузки.
        if ($pnp -eq 3010) { $NeedReboot = $true }
    }

    # 6. Гостевой агент syn-vm (служба; обновление — замена exe и перезапуск службы).
    $agent = Join-Path $S 'win10\agent\syn-vm-agent.exe'
    $out = & $agent install 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { Fail "агент: $($out.Trim())" }
    Log $out.Trim()

    # 7. Видеокарта syn-gpu в машине — её инструменты (свой установщик, может попросить перезагрузку).
    $code = 0
    $gpu = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'PCI\VEN_1B36&DEV_F00D*' } | Select-Object -First 1
    $gpuInstall = Join-Path $S 'win10\syn-gpu\install.ps1'
    if ($gpu -and (Test-Path $gpuInstall)) {
        Log 'видеокарта syn-gpu: ставлю её инструменты'
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $gpuInstall -Quiet
        $code = $LASTEXITCODE
        Log "syn-gpu: код $code (журнал C:\ProgramData\syn-gpu\install.log)"
        if ($code -ne 0 -and $code -ne 3010) { Fail "инструменты syn-gpu: код $code" $code }
    }

    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    if ($NeedReboot -and $code -eq 0) { $code = 3010; Log 'драйвер syn-vm-serial заменится после перезагрузки' }
    SetState $(if ($code -eq 3010) { 'reboot' } else { 'ok' })
    Log ('=== готово' + $(if ($code -eq 3010) { ' (нужна перезагрузка)' } else { '' }) + ' ===')
    exit $code
} catch {
    Fail $_.Exception.Message
}
