# Embedded one-time installer. No user-supplied commands or task arguments.
$ErrorActionPreference = 'Stop'
$ownerSid = '%%SID%%'
if ($ownerSid -notmatch '^S-1-5-21-(\d+-){3}\d+$') { throw 'Invalid owner identity' }
$programFiles = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion').ProgramFilesDir
if ($programFiles -ne 'C:\Program Files') { throw 'This relay helper requires the standard C:\Program Files installation path' }
$root = Join-Path $programFiles 'GPO German Relay'
foreach ($path in @($programFiles, $root)) {
    if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Helper path must not be a link' }
}
if ((Get-FileHash -LiteralPath 'C:\VPN\sing-box.exe' -Algorithm SHA256).Hash -ne '7BBEF1DEA9189EE12799AE834EA4B4658355DA25C47A21AD8804904C0CCD9410') { throw 'Expected the verified sing-box 1.14.2 executable. Existing installation was left alone.' }
$source = Get-Content -LiteralPath 'C:\VPN\sing-box-client.json' -Raw | ConvertFrom-Json
$relay = @($source.outbounds | Where-Object { $_.type -eq 'vless' -and $_.server -eq '92.5.127.89' -and $_.server_port -eq 443 })[0]
if (-not $relay -or $relay.uuid -notmatch '^[0-9a-fA-F-]{36}$') { throw 'Missing Frankfurt relay credentials' }

# Create a fresh fixed profile, rather than executing arbitrary config copied
# from a user-writable directory. Logs, TLS host, route and DNS are fixed.
$profile = @{
    log = @{ level = 'info'; output = (Join-Path $root 'relay.log'); timestamp = $true }
    inbounds = @(@{ type = 'tun'; tag = 'tun-in'; interface_name = 'tun0'; address = @('172.19.0.1/30'); auto_route = $true; strict_route = $true; stack = 'mixed' })
    outbounds = @(
        @{ type = 'vless'; tag = 'vless-out'; server = '92.5.127.89'; server_port = 443; uuid = $relay.uuid;
           tls = @{ enabled = $true; server_name = 'panel.demos25.me' };
           transport = @{ type = 'ws'; path = '/rbx-tunnel-9f2a'; headers = @{ Host = 'panel.demos25.me' } } },
        @{ type = 'direct'; tag = 'direct' })
    dns = @{ servers = @(@{ tag = 'remote-dns'; type = 'https'; server = '1.1.1.1'; detour = 'vless-out' }, @{ tag = 'local-dns'; type = 'udp'; server = '1.1.1.1' }); strategy = 'ipv4_only'; final = 'remote-dns' }
    route = @{ auto_detect_interface = $true; default_domain_resolver = 'local-dns'; rules = @(@{ action = 'hijack-dns'; protocol = 'dns' }, @{ action = 'sniff' }, @{ ip_is_private = $true; outbound = 'direct' }, @{ ip_cidr = @('92.5.127.89/32'); outbound = 'direct' }); final = 'vless-out' }
}
New-Item -ItemType Directory -Path $root -Force | Out-Null
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true, $false)
$acl.SetOwner([Security.Principal.SecurityIdentifier]'S-1-5-32-544')
foreach ($sid in @('S-1-5-18', 'S-1-5-32-544')) {
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]$sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
}
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]$ownerSid, 'ReadAndExecute', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
Set-Acl -LiteralPath $root -AclObject $acl
function Write-ProtectedText([string]$leaf, [string]$text) {
    $temporary = Join-Path $root ([Guid]::NewGuid().ToString() + '.tmp')
    [IO.File]::WriteAllText($temporary, $text, [Text.UTF8Encoding]::new($false))
    # Replace the directory entry, never write through an old hard link.
    Move-Item -LiteralPath $temporary -Destination (Join-Path $root $leaf) -Force
}
foreach ($leaf in @('sing-box.exe', 'profile.json', 'stop.ps1', 'ready-v1', 'relay.log')) {
    $destination = Join-Path $root $leaf
    if ((Test-Path -LiteralPath $destination) -and ((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Helper files must not be links' }
}
$temporaryBinary = Join-Path $root ([Guid]::NewGuid().ToString() + '.tmp')
Copy-Item -LiteralPath 'C:\VPN\sing-box.exe' -Destination $temporaryBinary
if ((Get-FileHash -LiteralPath $temporaryBinary -Algorithm SHA256).Hash -ne '7BBEF1DEA9189EE12799AE834EA4B4658355DA25C47A21AD8804904C0CCD9410') { throw 'Relay executable changed during setup' }
Move-Item -LiteralPath $temporaryBinary -Destination (Join-Path $root 'sing-box.exe') -Force
Write-ProtectedText 'profile.json' ($profile | ConvertTo-Json -Depth 12)
& (Join-Path $root 'sing-box.exe') check -c (Join-Path $root 'profile.json')
if ($LASTEXITCODE -ne 0) { throw 'Protected relay profile failed validation' }
$stopScript = @'
$ErrorActionPreference = 'Stop'
$scheduler = New-Object -ComObject 'Schedule.Service'
$scheduler.Connect()
$scheduler.GetFolder('\').GetTask('GPO_GermanRelayStart_v1').Stop(0)
$binary = Join-Path $PSScriptRoot 'sing-box.exe'
Get-CimInstance Win32_Process -Filter "Name='sing-box.exe'" | Where-Object { $_.ExecutablePath -eq $binary } | ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction Stop }
'@
Write-ProtectedText 'stop.ps1' $stopScript
Write-ProtectedText 'relay.log' ''
$scheduler = New-Object -ComObject 'Schedule.Service'
$scheduler.Connect()
$folder = $scheduler.GetFolder('\')
$descriptor = "O:BAG:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGX;;;$ownerSid)"
foreach ($operation in @('Start', 'Stop')) {
    $task = $scheduler.NewTask(0)
    $task.RegistrationInfo.Description = 'GPO German relay: fixed privileged operation. Main app remains a standard user.'
    $task.Principal.UserId = 'SYSTEM'
    $task.Principal.LogonType = 5
    $task.Principal.RunLevel = 1
    $task.Settings.Enabled = $true
    $task.Settings.Hidden = $true
    $task.Settings.AllowDemandStart = $true
    $task.Settings.DisallowStartIfOnBatteries = $false
    $task.Settings.StopIfGoingOnBatteries = $false
    $task.Settings.ExecutionTimeLimit = 'PT0S'
    $task.Settings.MultipleInstances = 2
    $action = $task.Actions.Create(0)
    $action.WorkingDirectory = $root
    if ($operation -eq 'Start') {
        $action.Path = Join-Path $root 'sing-box.exe'
        $action.Arguments = 'run -c "' + (Join-Path $root 'profile.json') + '"'
    } else {
        $action.Path = 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe'
        $action.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + (Join-Path $root 'stop.ps1') + '"'
    }
    # The normal user may read/run, never edit the SYSTEM task.
    $folder.RegisterTaskDefinition("GPO_GermanRelay${operation}_v1", $task, 6, 'SYSTEM', $null, 5, $descriptor) | Out-Null
}
Write-ProtectedText 'ready-v1' '1'
# Remove any old explicit permissions/ownership from files when repairing.
foreach ($leaf in @('sing-box.exe', 'profile.json', 'stop.ps1', 'ready-v1', 'relay.log')) {
    $path = Join-Path $root $leaf
    if (Test-Path -LiteralPath $path) {
        $fileAcl = New-Object Security.AccessControl.FileSecurity
        $fileAcl.SetOwner([Security.Principal.SecurityIdentifier]'S-1-5-32-544')
        $fileAcl.SetAccessRuleProtection($false, $false)
        Set-Acl -LiteralPath $path -AclObject $fileAcl
    }
}
