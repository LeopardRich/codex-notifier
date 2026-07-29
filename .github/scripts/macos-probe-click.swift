import CoreGraphics
import Darwin
import Foundation

guard CommandLine.arguments.count == 3,
      let x = Double(CommandLine.arguments[1]),
      let y = Double(CommandLine.arguments[2])
else {
    fputs("usage: macos-probe-click <x> <y>\n", stderr)
    exit(2)
}

let point = CGPoint(x: x, y: y)
CGWarpMouseCursorPosition(point)
let source = CGEventSource(stateID: .hidSystemState)

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
