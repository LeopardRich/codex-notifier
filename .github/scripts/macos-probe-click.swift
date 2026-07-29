import CoreGraphics
import Darwin
import Foundation

guard CommandLine.arguments.count == 4,
      let mode = CommandLine.arguments.dropFirst().first,
      ["move", "click"].contains(mode),
      let x = Double(CommandLine.arguments[2]),
      let y = Double(CommandLine.arguments[3])
else {
    fputs("usage: macos-probe-click <move|click> <x> <y>\n", stderr)
    exit(2)
}

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
