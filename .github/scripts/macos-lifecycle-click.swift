import CoreGraphics
import Darwin
import Foundation

guard CommandLine.arguments.count == 4,
      ["move", "click"].contains(CommandLine.arguments[1]),
      let x = Double(CommandLine.arguments[2]),
      let y = Double(CommandLine.arguments[3]) else {
    fputs("usage: macos-lifecycle-click <move|click> <x> <y>\n", stderr)
    exit(2)
}

let mode = CommandLine.arguments[1]
let point = CGPoint(x: x, y: y)
CGWarpMouseCursorPosition(point)
let source = CGEventSource(stateID: .hidSystemState)

if mode == "move" {
    guard let event = CGEvent(
        mouseEventSource: source,
        mouseType: .mouseMoved,
        mouseCursorPosition: point,
        mouseButton: .left
    ) else {
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
        exit(1)
    }
    event.post(tap: .cghidEventTap)
    usleep(100_000)
}
