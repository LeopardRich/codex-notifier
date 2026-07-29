import AppKit
import Foundation

guard CommandLine.arguments.count == 2 else {
    fputs("usage: generate-icon.swift ICONSET_DIRECTORY\n", stderr)
    exit(2)
}

let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)

let variants: [(Int, String)] = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512@2x.png"),
]

for (pixels, name) in variants {
    let size = CGFloat(pixels)
    let image = NSImage(size: NSSize(width: size, height: size))
    image.lockFocus()

    NSColor(calibratedRed: 0.10, green: 0.12, blue: 0.14, alpha: 1).setFill()
    NSBezierPath(
        roundedRect: NSRect(x: 0, y: 0, width: size, height: size),
        xRadius: size * 0.20,
        yRadius: size * 0.20
    ).fill()

    NSColor(calibratedRed: 0.20, green: 0.78, blue: 0.66, alpha: 1).setFill()
    let bell = NSBezierPath()
    bell.move(to: NSPoint(x: size * 0.28, y: size * 0.35))
    bell.curve(
        to: NSPoint(x: size * 0.40, y: size * 0.72),
        controlPoint1: NSPoint(x: size * 0.34, y: size * 0.47),
        controlPoint2: NSPoint(x: size * 0.29, y: size * 0.68)
    )
    bell.curve(
        to: NSPoint(x: size * 0.60, y: size * 0.72),
        controlPoint1: NSPoint(x: size * 0.45, y: size * 0.80),
        controlPoint2: NSPoint(x: size * 0.55, y: size * 0.80)
    )
    bell.curve(
        to: NSPoint(x: size * 0.72, y: size * 0.35),
        controlPoint1: NSPoint(x: size * 0.71, y: size * 0.68),
        controlPoint2: NSPoint(x: size * 0.66, y: size * 0.47)
    )
    bell.close()
    bell.fill()

    NSColor(calibratedWhite: 0.96, alpha: 1).setFill()
    NSBezierPath(
        ovalIn: NSRect(
            x: size * 0.43,
            y: size * 0.22,
            width: size * 0.14,
            height: size * 0.14
        )
    ).fill()
    image.unlockFocus()

    guard let tiff = image.tiffRepresentation,
          let bitmap = NSBitmapImageRep(data: tiff),
          let png = bitmap.representation(using: .png, properties: [:]) else {
        fputs("failed to render icon\n", stderr)
        exit(1)
    }
    try png.write(to: output.appendingPathComponent(name))
}
