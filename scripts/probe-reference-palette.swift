// Reads a reference screenshot and reports the colors actually in it, so tokens.css can cite a
// measurement instead of an impression. Run with `swift scripts/probe-reference-palette.swift`:
//   <png>                 top colours by area (surfaces)
//   <png> vivid           most chromatic colours (the accent, whatever the shot uses it for)
//   <png> grid X,Y ...    the colour at fractional coordinates, e.g. 0.13,0.42
//
// swift scripts/probe-reference-palette.swift docs/reference-ui/05-main-dark-three-column.png
import AppKit
import Foundation

func pixels(from path: String) -> (w: Int, h: Int, buf: [UInt8])? {
    guard let img = NSImage(contentsOfFile: path),
          let tiff = img.tiffRepresentation,
          let rep = NSBitmapImageRep(data: tiff)
    else { return nil }
    let w = rep.pixelsWide, h = rep.pixelsHigh
    guard let ctx = CGContext(
        data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return nil }
    guard let cg = rep.cgImage else { return nil }
    ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
    guard let data = ctx.data else { return nil }
    let ptr = data.bindMemory(to: UInt8.self, capacity: w * h * 4)
    let buf = [UInt8](UnsafeBufferPointer(start: ptr, count: w * h * 4))
    return (w, h, buf)
}

func hex(_ r: Int, _ g: Int, _ b: Int) -> String {
    String(format: "#%02x%02x%02x", r, g, b)
}

let args = CommandLine.arguments
guard args.count >= 2 else { print("usage: probe <png> [mode]"); exit(1) }
guard let (w, h, buf) = pixels(from: args[1]) else { print("unreadable"); exit(1) }

var counts: [String: Int] = [:]
var samples: [(Int, Int, String)] = []

if args.count >= 3 && args[2] == "grid" {
    // Print the colour at fractional coordinates so a specific control can be read.
    for i in 3..<args.count {
        let parts = args[i].split(separator: ",")
        guard parts.count == 2, let fx = Double(parts[0]), let fy = Double(parts[1]) else { continue }
        let x = Int(fx * Double(w)), y = Int(fy * Double(h))
        // CGContext draws bottom-up, so a top-down fraction has to be flipped.
        let row = min(h - 1, h - 1 - y)
        let o = (row * w + min(x, w - 1)) * 4
        samples.append((x, y, hex(Int(buf[o]), Int(buf[o + 1]), Int(buf[o + 2]))))
    }
} else if args.count >= 3 && args[2] == "vivid" {
    // The accent is the most chromatic thing on screen; find it instead of guessing a pixel.
    var best: [(Int, String)] = []
    for y in stride(from: 0, to: h, by: 2) {
        for x in stride(from: 0, to: w, by: 2) {
            let o = (y * w + x) * 4
            let r = Int(buf[o]), g = Int(buf[o + 1]), b = Int(buf[o + 2])
            let chroma = max(r, g, b) - min(r, g, b)
            if chroma > 40 { best.append((chroma, hex(r, g, b))) }
        }
    }
    var seen: [String: Int] = [:]
    for (_, c) in best { seen[c, default: 0] += 1 }
    for (c, n) in seen.sorted(by: { $0.value > $1.value }).prefix(12) { print("\(c)  \(n) px") }
} else {
    // Quantised histogram: the dominant surfaces of the shot.
    for y in stride(from: 0, to: h, by: 2) {
        for x in stride(from: 0, to: w, by: 2) {
            let o = (y * w + x) * 4
            let r = Int(buf[o]), g = Int(buf[o + 1]), b = Int(buf[o + 2])
            counts[hex(r & 0xf8, g & 0xf8, b & 0xf8), default: 0] += 1
        }
    }
}
for s in samples { print("(\(s.0),\(s.1))  \(s.2)") }
let total = max(1, counts.values.reduce(0, +))
for (c, n) in counts.sorted(by: { $0.value > $1.value }).prefix(24) {
    print("\(c)  \(String(format: "%.2f", Double(n) / Double(total) * 100))%")
}
