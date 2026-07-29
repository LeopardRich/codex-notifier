import CoreGraphics
import Darwin
import Foundation

guard CommandLine.arguments.count >= 2 else {
    fputs("usage: macos-probe-click <move|click> <x> <y> | <focus|dnd>\n", stderr)
    exit(2)
}

let mode = CommandLine.arguments[1]
let point: CGPoint
if ["focus", "dnd"].contains(mode) && CommandLine.arguments.count == 2 {
    let bounds = CGDisplayBounds(CGMainDisplayID())
    if mode == "focus" {
        let focusY = ProcessInfo.processInfo.operatingSystemVersion.majorVersion <= 14 ? 71.0 : 330.0
        point = CGPoint(x: bounds.maxX - 84.0, y: bounds.minY + focusY)
    } else {
        point = CGPoint(x: bounds.maxX - 220.0, y: bounds.minY + 97.0)
    }
    print("\(mode)-click x=\(Int(point.x)) y=\(Int(point.y))")
} else if ["move", "click"].contains(mode),
          CommandLine.arguments.count == 4,
          let x = Double(CommandLine.arguments[2]),
          let y = Double(CommandLine.arguments[3]) {
    point = CGPoint(x: x, y: y)
} else {
    fputs("usage: macos-probe-click <move|click> <x> <y> | <focus|dnd>\n", stderr)
    exit(2)
}

CGWarpMouseCursorPosition(point)
let source = CGEventSource(stateID: .hidSystemState)

if mode == "move" {
    guard let event = CGEvent(
        mouseEventSource: source,
        mouseType: .mouseMoved,
        mouseCursorPosition: point,
        mouseButton: .left
    ) else {
        fputs("failed to create mouse move event\n", stderr)
        exit(1)
    }
    event.post(tap: .cghidEventTap)
    exit(0)
}

for eventType in [CGEventType.leftMouseDown, CGEventType.leftMouseUp] {
    guard let event = CGEvent(
        mouseEventSource: source,
        mouseType: eventType,
        mouseCursorPosition: point,
        mouseButton: .left
    ) else {
        fputs("failed to create mouse event\n", stderr)
        exit(1)
    }
    event.post(tap: .cghidEventTap)
    usleep(100_000)
}
