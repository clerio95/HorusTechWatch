' setup_task.vbs
' Registers horustechwatch.exe as a Windows Scheduled Task that fires every
' 20 minutes, runs one poll cycle (--once flag), writes state.json / health.json,
' then exits.
'
' HOW TO USE:
'   1. Copy horustechwatch.exe, config.toml, and run_once.bat to INSTALL_DIR.
'   2. Edit INSTALL_DIR below if needed.
'   3. From an elevated command prompt: cscript setup_task.vbs
'
' NOTE: The password is stored here in plain text only for registration. Windows
' Task Scheduler encrypts it in its own credential store — delete or restrict
' access to this script after running it if that is a concern.
'
' To start the task immediately after registering:
'   schtasks /Run /TN "HorusTechWatch"
'
' To remove the task:
'   schtasks /Delete /TN "HorusTechWatch" /F

Option Explicit

' ── EDIT THESE IF NEEDED ─────────────────────────────────────────────────────
Const INSTALL_DIR   = "C:\HorusTechWatch"
Const TASK_USER     = "pst-zam-04\usuario"
Const TASK_PASSWORD = "123"
Const REPEAT_EVERY  = "PT20M"   ' ISO 8601 duration — change to e.g. "PT15M" for 15 min
' ─────────────────────────────────────────────────────────────────────────────

Const TASK_NAME  = "HorusTechWatch"
Const EXE_NAME   = "horustechwatch.exe"

' Task Scheduler COM constants
Const TASK_TRIGGER_TIME        = 1
Const TASK_ACTION_EXEC         = 0
Const TASK_LOGON_PASSWORD      = 1
Const TASK_RUNLEVEL_LUA        = 0
Const TASK_CREATE_OR_UPDATE    = 6
Const TASK_INSTANCES_IGNORE_NEW = 2

Dim oFSO, oService, oFolder, oTask, oTrigger, oRepeat, oAction, oSettings, oPrincipal
Dim sExePath

sExePath = INSTALL_DIR & "\" & EXE_NAME

Set oFSO = CreateObject("Scripting.FileSystemObject")
If Not oFSO.FileExists(sExePath) Then
    WScript.Echo "ERROR: " & sExePath & " not found." & vbCrLf & _
                 "Copy horustechwatch.exe to " & INSTALL_DIR & " first."
    WScript.Quit 1
End If

On Error Resume Next

Set oService = CreateObject("Schedule.Service")
If Err.Number <> 0 Then
    WScript.Echo "ERROR: Could not create Schedule.Service. Run as Administrator."
    WScript.Quit 1
End If
oService.Connect
If Err.Number <> 0 Then
    WScript.Echo "ERROR: Task Scheduler connect failed: " & Err.Description
    WScript.Quit 1
End If

On Error GoTo 0

Set oFolder = oService.GetFolder("\")
Set oTask   = oService.NewTask(0)

' Registration info
oTask.RegistrationInfo.Description = _
    "Horustech concentrator health-check publisher. " & _
    "Polls device over TCP every 20 minutes, writes state.json / health.json to the network share, " & _
    "then exits. Read-only — sends no write or control commands to the device."

' Principal — run as stored user (works whether or not the user is logged in).
Set oPrincipal       = oTask.Principal
oPrincipal.UserId    = TASK_USER
oPrincipal.LogonType = TASK_LOGON_PASSWORD
oPrincipal.RunLevel  = TASK_RUNLEVEL_LUA

' Settings
Set oSettings = oTask.Settings
oSettings.Enabled                    = True
oSettings.Hidden                     = False   ' visible in Task Scheduler for monitoring
oSettings.StartWhenAvailable         = True    ' catch up if machine was off at trigger time
oSettings.RunOnlyIfIdle              = False
oSettings.DisallowStartIfOnBatteries = False
oSettings.StopIfGoingOnBatteries     = False
oSettings.ExecutionTimeLimit         = "PT5M"  ' 5-minute hard cap; a normal poll takes < 30s
oSettings.MultipleInstances          = TASK_INSTANCES_IGNORE_NEW  ' skip if previous run still going

' Trigger — fire every REPEAT_EVERY indefinitely, starting from a fixed past date
' so the schedule is stable across reboots.
Set oTrigger            = oTask.Triggers.Create(TASK_TRIGGER_TIME)
oTrigger.StartBoundary  = "2000-01-01T00:00:00"
oTrigger.Enabled        = True

Set oRepeat                    = oTrigger.Repetition
oRepeat.Interval               = REPEAT_EVERY
oRepeat.Duration               = ""     ' empty = repeat indefinitely
oRepeat.StopAtDurationEnd      = False

' Action — run the exe directly with --once; no console window in non-interactive sessions.
Set oAction              = oTask.Actions.Create(TASK_ACTION_EXEC)
oAction.Path             = sExePath
oAction.Arguments        = "--once"
oAction.WorkingDirectory = INSTALL_DIR

' Register the task with stored credentials.
On Error Resume Next
oFolder.RegisterTaskDefinition _
    TASK_NAME, oTask, TASK_CREATE_OR_UPDATE, _
    TASK_USER, TASK_PASSWORD, TASK_LOGON_PASSWORD

If Err.Number <> 0 Then
    WScript.Echo "ERROR: Could not register task: " & Err.Description & vbCrLf & _
                 "Make sure you are running as Administrator."
    WScript.Quit 1
End If
On Error GoTo 0

WScript.Echo "Task """ & TASK_NAME & """ registered successfully." & vbCrLf & _
             vbCrLf & _
             "  Install dir : " & INSTALL_DIR & vbCrLf & _
             "  Runs as     : " & TASK_USER & vbCrLf & _
             "  Trigger     : every " & REPEAT_EVERY & " (repeats indefinitely)" & vbCrLf & _
             "  Action      : " & sExePath & " --once" & vbCrLf & _
             "  Time limit  : 5 minutes per run" & vbCrLf & _
             "  On overlap  : new trigger skipped if previous run still active" & vbCrLf & _
             "  Visible in  : Task Scheduler (search for HorusTechWatch)" & vbCrLf & _
             vbCrLf & _
             "To run NOW without waiting for the next 20-minute mark:" & vbCrLf & _
             "  schtasks /Run /TN """ & TASK_NAME & """"
