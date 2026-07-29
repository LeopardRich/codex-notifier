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
    if requestedMode is "enable" then
        set targetValue to "1"
    else
        set targetValue to "0"
    end if

    tell application "System Events"
        if not (exists application process "ControlCenter") then
            error "ControlCenter process not found"
        end if
        set controlCenterProcess to application process "ControlCenter"
        set controlCenterItem to my findControlCenterMenuItem(controlCenterProcess)
        if (count windows of controlCenterProcess) is 0 then
            perform action "AXPress" of controlCenterItem
            delay 2
        end if
        if (count windows of controlCenterProcess) is 0 then
            error "Control Center window did not open"
        end if

        set controlCenterWindow to window 1 of controlCenterProcess
        set windowPosition to position of controlCenterWindow
        set windowSize to size of controlCenterWindow
        set midpointX to (item 1 of windowPosition) + ((item 1 of windowSize) div 2)
        set midpointY to (item 2 of windowPosition) + ((item 2 of windowSize) div 2)
        set focusControl to missing value
        set focusControlY to 100000
        set candidateElements to entire contents of controlCenterWindow
        repeat with candidateElement in candidateElements
            try
                if role of candidateElement is "AXCheckBox" then
                    set elementPosition to position of candidateElement
                    set elementX to item 1 of elementPosition
                    set elementY to item 2 of elementPosition
                    if elementX is greater than or equal to midpointX and elementY < midpointY and elementY < focusControlY then
                        set focusControl to candidateElement
                        set focusControlY to elementY
                    end if
                end if
            end try
        end repeat
        if focusControl is missing value then
            error "Focus control not found in the top-right Control Center region"
        end if

        set valueBefore to value of focusControl as text
        set focusPosition to position of focusControl
        set focusSize to size of focusControl
        if valueBefore is not targetValue then
            perform action "AXPress" of focusControl
            delay 2
        end if
        set valueAfter to value of focusControl as text
        if valueAfter is not targetValue then
            error "Focus control did not reach requested value " & targetValue & "; before=" & valueBefore & ", after=" & valueAfter
        end if

        if (count windows of controlCenterProcess) > 0 then
            perform action "AXPress" of controlCenterItem
        end if
        return "focus target=" & targetValue & " before=" & valueBefore & " after=" & valueAfter & " position=" & focusPosition & " size=" & focusSize
    end tell
end run
