on elementText(candidateElement)
    set output to ""
    try
        set output to output & " " & (name of candidateElement as text)
    end try
    try
        set output to output & " " & (value of candidateElement as text)
    end try
    return output
end elementText

on clickCenter(candidateElement, clickHelper, mode)
    tell application "System Events"
        set elementPosition to position of candidateElement
        set elementSize to size of candidateElement
    end tell
    set clickX to (item 1 of elementPosition) + ((item 1 of elementSize) div 2)
    set clickY to (item 2 of elementPosition) + ((item 2 of elementSize) div 2)
    do shell script quoted form of clickHelper & " " & mode & " " & clickX & " " & clickY
    return "x=" & clickX & " y=" & clickY
end clickCenter

on run
    set clickHelper to system attribute "CODEX_LIFECYCLE_CLICK_HELPER"
    set bannerHovered to system attribute "CODEX_LIFECYCLE_BANNER_HOVERED"
    set bannerPressed to system attribute "CODEX_LIFECYCLE_BANNER_PRESSED"
    set settingsAppPressed to system attribute "CODEX_LIFECYCLE_SETTINGS_APP_PRESSED"
    set settingsAllowPressed to system attribute "CODEX_LIFECYCLE_SETTINGS_ALLOW_PRESSED"
    set screenPressed to system attribute "CODEX_LIFECYCLE_SCREEN_PRESSED"
    set observeText to system attribute "CODEX_LIFECYCLE_OBSERVE_TEXT"
    set observations to ""

    tell application "System Events"
        repeat with candidateProcess in application processes
            try
                set processName to name of candidateProcess as text
                if processName is "NotificationCenter" or processName is "UserNotificationCenter" or processName is "System Settings" then
                    repeat with candidateWindow in windows of candidateProcess
                        set candidateElements to entire contents of candidateWindow
                        set windowText to ""
                        repeat with candidateElement in candidateElements
                            set windowText to windowText & my elementText(candidateElement)
                        end repeat
                        set observations to observations & " process=" & processName & " text=" & windowText

                        if processName is "NotificationCenter" and observeText is not "" and windowText contains observeText then
                            return "observed=notification text=" & observeText
                        end if

                        if processName is "UserNotificationCenter" and screenPressed is not "1" and windowText contains "screen and audio" then
                            repeat with candidateElement in candidateElements
                                try
                                    if role of candidateElement is "AXButton" and name of candidateElement is "Allow" then
                                        set clicked to my clickCenter(candidateElement, clickHelper, "click")
                                        return "pressed=screen " & clicked
                                    end if
                                end try
                            end repeat
                        end if

                        if processName is "System Settings" and bannerPressed is "1" then
                            if settingsAllowPressed is not "1" then
                                repeat with candidateElement in candidateElements
                                    try
                                        set elementName to name of candidateElement as text
                                        set elementRole to role of candidateElement as text
                                        if elementName is "Allow notifications" and (elementRole is "AXCheckBox" or elementRole is "AXSwitch" or elementRole is "AXButton") then
                                            perform action "AXPress" of candidateElement
                                            return "pressed=settings-allow role=" & elementRole
                                        end if
                                    end try
                                end repeat
                            end if
                            if settingsAppPressed is not "1" then
                                repeat with candidateElement in candidateElements
                                    try
                                        set elementName to name of candidateElement as text
                                        set elementRole to role of candidateElement as text
                                        if elementName contains "Codex Notifier" and (elementRole is "AXStaticText" or elementRole is "AXButton" or elementRole is "AXGroup") then
                                            set clicked to my clickCenter(candidateElement, clickHelper, "click")
                                            return "pressed=settings-app role=" & elementRole & " " & clicked
                                        end if
                                    end try
                                end repeat
                            end if
                        end if

                        if processName is "NotificationCenter" and windowText contains "Codex Notifier" and windowText contains "Notifications" then
                            repeat with candidateElement in candidateElements
                                try
                                    if role of candidateElement is "AXButton" and name of candidateElement is "Allow" then
                                        perform action "AXPress" of candidateElement
                                        return "pressed=notification-allow"
                                    end if
                                end try
                            end repeat
                            repeat with candidateElement in candidateElements
                                set textValue to my elementText(candidateElement)
                                if textValue contains "Codex Notifier" or textValue contains "Notifications may include" then
                                    if bannerHovered is not "1" then
                                        set clicked to my clickCenter(candidateElement, clickHelper, "move")
                                        return "hovered=banner " & clicked
                                    end if
                                    if bannerPressed is not "1" then
                                        set clicked to my clickCenter(candidateElement, clickHelper, "click")
                                        return "pressed=banner " & clicked
                                    end if
                                end if
                            end repeat
                        end if
                    end repeat
                end if
            end try
        end repeat
    end tell
    return "no-action" & observations
end run
