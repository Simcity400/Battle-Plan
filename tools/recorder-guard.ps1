function Get-GameBlockingRecorder {
    Get-CimInstance Win32_Process -Filter "Name = 'recorder.exe' OR Name = 'battle-plan-recorder.exe'" | Where-Object {
        $_.Name -ine 'recorder.exe' -or $_.CommandLine -cnotmatch '^\s*(?:"[^"]+"|[^\s"]+)\s+(?:"(?:answers|inspect|clock)"|answers|inspect|clock)(?:\s|$)'
    }
}
