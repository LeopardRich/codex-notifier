on findControlCenterMenuItem(controlCenterProcess)
    tell application "System Events"
        repeat with candidateItem in menu bar items of menu bar 1 of controlCenterProcess
            try
                set itemName to name of candidateItem as text
            on error
                set itemName to ""
            end try
            try
                set itemDescription to description of candidateItem as text
            on error
                set itemDescription to ""
            end try
            if itemName contains "Control Center" or itemDescription contains "Control Center" then
                return candidateItem
            end if
        end repeat
    end tell
    error "Control Center menu item not found"
end findControlCenterMenuItem

on run arguments
    if (count arguments) is not 1 or item 1 of arguments is not in {"enable", "disable"} then
        error "usage: macos-probe-focus.applescript <enable|disable>"
    end if
    set requestedMode to item 1 of arguments
    set clickHelper to system attribute "CODEX_PROBE_CLICK_HELPER"
    if clickHelper is "" then
        error "CODEX_PROBE_CLICK_HELPER is not set"
    end if

    tell application "System Events"
        if not (exists application process "ControlCenter") then
            error "ControlCenter process not found"
        end if
        set controlCenterProcess to application process "ControlCenter"
        set controlCenterItem to my findControlCenterMenuItem(controlCenterProcess)
        key code 53
        delay 1
        perform action "AXPress" of controlCenterItem
        delay 2
        set clickResult to do shell script quoted form of clickHelper & " focus"
        delay 2
        key code 53
        return "focus mode=" & requestedMode & " " & clickResult
    end tell
end run
