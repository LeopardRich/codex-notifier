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

on findControlCenterWindow(controlCenterProcess)
    tell application "System Events"
        repeat with candidateWindow in windows of controlCenterProcess
            try
                set windowPosition to position of candidateWindow
                set windowSize to size of candidateWindow
                if (item 2 of windowPosition) < 100 and (item 1 of windowSize) > 250 and (item 2 of windowSize) > 300 then
                    return candidateWindow
                end if
            end try
        end repeat
        return missing value
    end tell
end findControlCenterWindow

on findFocusControl(controlCenterWindow)
    tell application "System Events"
        set windowPosition to position of controlCenterWindow
        set windowSize to size of controlCenterWindow
        set windowLeft to item 1 of windowPosition
        set windowTop to item 2 of windowPosition
        set windowWidth to item 1 of windowSize
        set windowHeight to item 2 of windowSize
        set expectedX to windowLeft + ((windowWidth * 3) div 4)
        if windowHeight > 450 then
            set expectedY to windowTop + ((windowHeight * 56) div 100)
        else
            set expectedY to windowTop + ((windowHeight * 11) div 100)
        end if

        set bestValue to missing value
        set bestPositionX to 0
        set bestPositionY to 0
        set bestWidth to 0
        set bestHeight to 0
        set bestDistance to 100000000
        set candidateElements to entire contents of controlCenterWindow
        repeat with candidateElement in candidateElements
            try
                if role of candidateElement is "AXCheckBox" then
                    set elementPosition to position of candidateElement
                    set elementSize to size of candidateElement
                    set centerX to (item 1 of elementPosition) + ((item 1 of elementSize) div 2)
                    set centerY to (item 2 of elementPosition) + ((item 2 of elementSize) div 2)
                    set deltaX to centerX - expectedX
                    set deltaY to centerY - expectedY
                    set candidateDistance to (deltaX * deltaX) + (deltaY * deltaY)
                    if candidateDistance < bestDistance then
                        set bestDistance to candidateDistance
                        set bestValue to value of candidateElement as text
                        set bestPositionX to item 1 of elementPosition
                        set bestPositionY to item 2 of elementPosition
                        set bestWidth to item 1 of elementSize
                        set bestHeight to item 2 of elementSize
                    end if
                end if
            end try
        end repeat
        if bestValue is missing value then
            error "Focus control not found near the expected Control Center position"
        end if
        return {bestValue, expectedX, expectedY, bestPositionX, bestPositionY, bestWidth, bestHeight, bestDistance}
    end tell
end findFocusControl

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
        set controlCenterWindow to my findControlCenterWindow(controlCenterProcess)
        if controlCenterWindow is not missing value then
            perform action "AXPress" of controlCenterItem
            delay 1
        end if
        perform action "AXPress" of controlCenterItem
        delay 2
        set controlCenterWindow to my findControlCenterWindow(controlCenterProcess)
        if controlCenterWindow is missing value then
            error "Control Center window did not open"
        end if

        set beforeDetails to my findFocusControl(controlCenterWindow)
        set valueBefore to item 1 of beforeDetails
        set clickX to item 2 of beforeDetails
        set clickY to item 3 of beforeDetails
        set focusPositionX to item 4 of beforeDetails
        set focusPositionY to item 5 of beforeDetails
        set focusWidth to item 6 of beforeDetails
        set focusHeight to item 7 of beforeDetails
        if valueBefore is not targetValue then
            do shell script quoted form of clickHelper & " click " & clickX & " " & clickY
            delay 2
        end if
        set controlCenterWindow to my findControlCenterWindow(controlCenterProcess)
        if controlCenterWindow is missing value then
            perform action "AXPress" of controlCenterItem
            delay 2
        end if
        set controlCenterWindow to my findControlCenterWindow(controlCenterProcess)
        if controlCenterWindow is missing value then
            error "Control Center window did not reopen after changing Focus"
        end if
        set afterDetails to my findFocusControl(controlCenterWindow)
        set valueAfter to item 1 of afterDetails
        if valueAfter is not targetValue then
            error "Focus control did not reach requested value " & targetValue & "; before=" & valueBefore & ", after=" & valueAfter & ", click=" & clickX & "," & clickY & ", nearest=" & focusPositionX & "," & focusPositionY & "," & focusWidth & "," & focusHeight
        end if

        set controlCenterWindow to my findControlCenterWindow(controlCenterProcess)
        if controlCenterWindow is not missing value then
            perform action "AXPress" of controlCenterItem
        end if
        return "focus target=" & targetValue & " before=" & valueBefore & " after=" & valueAfter & " click=" & clickX & "," & clickY & " nearest=" & focusPositionX & "," & focusPositionY & "," & focusWidth & "," & focusHeight
    end tell
end run
