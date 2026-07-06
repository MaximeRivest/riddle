import Foundation
import CoreGraphics
import CoreText
import UIKit

/// Handwriting synthesis: rasterize reply text in Dancing Script, thin it to
/// single-pixel pen paths (Zhang-Suen), trace them into ordered strokes.
/// Ported from riddle/src/script.rs.

struct Line {
    var width: Int
    var height: Int
    /// Bit mask of inked pixels, row-major.
    var mask: [Bool]
}

struct HandwritingSynthesis {

    /// Rasterize one line of text at `px` height into a boolean mask.
    /// Ported from rasterize_line() in script.rs.
    static func rasterizeLine(text: String, font: CTFont, px: CGFloat) -> Line {
        let ascent = CTFontGetAscent(font)
        let descent = CTFontGetDescent(font)
        let totalHeight = ascent + descent

        // Measure advance width.
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        let attrString = CFAttributedStringCreate(nil, text as CFString, attrs as CFDictionary)!
        let line = CTLineCreateWithAttributedString(attrString)
        let lineWidth = CTLineGetTypographicBounds(line, nil, nil, nil)

        let width = max(Int(ceil(lineWidth)) + 4, 1)
        let height = max(Int(ceil(totalHeight)) + 4, 1)

        // Render to a grayscale bitmap.
        let bytesPerRow = width
        var pixels = [UInt8](repeating: 255, count: width * height)
        let colorSpace = CGColorSpaceCreateDeviceGray()
        guard let ctx = CGContext(data: &pixels,
                                  width: width,
                                  height: height,
                                  bitsPerComponent: 8,
                                  bytesPerRow: bytesPerRow,
                                  space: colorSpace,
                                  bitmapInfo: CGImageAlphaInfo.none.rawValue) else {
            return Line(width: 1, height: 1, mask: [false])
        }
        ctx.setFillColor(CGColor(gray: 1, alpha: 1))
        ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))

        ctx.textPosition = CGPoint(x: 0, y: descent)
        CTLineDraw(line, ctx)

        // Convert to boolean mask: pixel is "inked" if luminance < 128.
        var mask = [Bool](repeating: false, count: width * height)
        for y in 0..<height {
            for x in 0..<width {
                // Core Graphics uses bottom-left origin; flip to top-down.
                let srcY = height - 1 - y
                let luma = pixels[srcY * width + x]
                mask[y * width + x] = luma < 128
            }
        }
        return Line(width: width, height: height, mask: mask)
    }

    /// Measure the advance width of text without rasterizing.
    static func measure(text: String, font: CTFont) -> CGFloat {
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        let attrString = CFAttributedStringCreate(nil, text as CFString, attrs as CFDictionary)!
        let line = CTLineCreateWithAttributedString(attrString)
        return CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
    }

    /// Zhang-Suen thinning: reduce the mask to 1px-wide skeleton lines.
    /// Direct port of thin() from script.rs.
    static func thin(_ line: inout Line) {
        let w = line.width
        let h = line.height
        func idx(_ x: Int, _ y: Int) -> Int { y * w + x }

        while true {
            var changed = false
            for phase in 0..<2 {
                var toClear: [Int] = []
                for y in 1..<(h > 1 ? h - 1 : 1) {
                    for x in 1..<(w > 1 ? w - 1 : 1) {
                        if !line.mask[idx(x, y)] { continue }
                        let p = [
                            line.mask[idx(x, y - 1)],     // p2 N
                            line.mask[idx(x + 1, y - 1)], // p3 NE
                            line.mask[idx(x + 1, y)],     // p4 E
                            line.mask[idx(x + 1, y + 1)], // p5 SE
                            line.mask[idx(x, y + 1)],     // p6 S
                            line.mask[idx(x - 1, y + 1)], // p7 SW
                            line.mask[idx(x - 1, y)],     // p8 W
                            line.mask[idx(x - 1, y - 1)], // p9 NW
                        ]
                        let b = p.filter { $0 }.count
                        if !(2...6).contains(b) { continue }

                        var a = 0
                        for i in 0..<8 {
                            if !p[i] && p[(i + 1) % 8] { a += 1 }
                        }
                        if a != 1 { continue }

                        let c1: Bool, c2: Bool
                        if phase == 0 {
                            c1 = !(p[0] && p[2] && p[4])
                            c2 = !(p[2] && p[4] && p[6])
                        } else {
                            c1 = !(p[0] && p[2] && p[6])
                            c2 = !(p[0] && p[4] && p[6])
                        }
                        if c1 && c2 {
                            toClear.append(idx(x, y))
                        }
                    }
                }
                if !toClear.isEmpty {
                    changed = true
                    for i in toClear { line.mask[i] = false }
                }
            }
            if !changed { break }
        }
    }

    /// Trace the skeleton into polyline strokes, ordered left-to-right.
    /// Direct port of trace() from script.rs.
    static func trace(_ line: Line) -> [[(x: Int, y: Int)]] {
        let w = line.width
        let h = line.height
        func at(_ x: Int, _ y: Int) -> Bool {
            x >= 0 && y >= 0 && x < w && y < h && line.mask[y * w + x]
        }
        func neighbors(_ x: Int, _ y: Int) -> [(Int, Int)] {
            var out: [(Int, Int)] = []
            for dy in -1...1 {
                for dx in -1...1 {
                    if (dx != 0 || dy != 0) && at(x + dx, y + dy) {
                        out.append((x + dx, y + dy))
                    }
                }
            }
            return out
        }

        var visited = [Bool](repeating: false, count: w * h)
        func vis(_ x: Int, _ y: Int) { visited[y * w + x] = true }
        func seen(_ x: Int, _ y: Int) -> Bool { visited[y * w + x] }

        // Endpoints first (degree 1), then any remaining pixels (loops).
        var starts: [(Int, Int)] = []
        for y in 0..<h {
            for x in 0..<w {
                if at(x, y) && neighbors(x, y).count == 1 {
                    starts.append((x, y))
                }
            }
        }
        for y in 0..<h {
            for x in 0..<w {
                if at(x, y) { starts.append((x, y)) }
            }
        }

        var strokes: [[(Int, Int)]] = []
        for (sx, sy) in starts {
            if seen(sx, sy) { continue }
            var path: [(Int, Int)] = [(sx, sy)]
            vis(sx, sy)
            var cx = sx, cy = sy
            while true {
                let next = neighbors(cx, cy).first { !seen($0.0, $0.1) }
                if let (nx, ny) = next {
                    vis(nx, ny)
                    path.append((nx, ny))
                    cx = nx; cy = ny
                } else {
                    break
                }
            }
            if path.count >= 3 {
                strokes.append(path)
            }
        }
        // Sort left-to-right by min x.
        strokes.sort { $0.map { $0.0 }.min() ?? 0 < $1.map { $0.0 }.min() ?? 0 }
        return strokes
    }

    /// Word-wrap text to lines that fit maxPx at scale px.
    static func wrap(text: String, font: CTFont, maxPx: CGFloat) -> [String] {
        var lines: [String] = []
        for para in text.components(separatedBy: "\n") {
            var cur = ""
            for word in para.split(separator: " ", omittingEmptySubsequences: false) {
                let cand = cur.isEmpty ? String(word) : "\(cur) \(word)"
                if measure(text: cand, font: font) <= maxPx || cur.isEmpty {
                    cur = cand
                } else {
                    lines.append(cur)
                    cur = String(word)
                }
            }
            if !cur.isEmpty { lines.append(cur) }
        }
        return lines
    }

    /// Full pipeline: text → rasterize → thin → trace → screen-space strokes.
    /// Returns a WritePlan ready for animation.
    static func planReply(text: String,
                          font: CTFont,
                          screenWidth: CGFloat,
                          screenHeight: CGFloat,
                          yStart: CGFloat? = nil) -> WritePlan {
        let maxW = screenWidth - 2 * DiaryConfig.marginX
        let lines = wrap(text: text, font: font, maxPx: maxW)
        let lineH = DiaryConfig.replyPx * 1.25
        let totalH = lineH * CGFloat(lines.count)
        var y = yStart ?? max((screenHeight - totalH) / 3, 60)

        var strokes: [PlannedStroke] = []
        var region = CGRect.zero
        var seed: UInt32 = 0x1234

        for lineText in lines {
            var raster = rasterizeLine(text: lineText, font: font, px: DiaryConfig.replyPx)
            thin(&raster)
            let lineStrokes = trace(raster)
            let x0 = (screenWidth - CGFloat(raster.width)) / 2
            // Pseudo-random wobble for organic feel.
            seed = seed &* 1664525 &+ 1013904223
            let wobble = CGFloat((seed >> 16) % 7) - 3

            for s in lineStrokes {
                // Flip y-axis: rasterizeLine's mask is bottom-up (CGContext),
                // but surface uses top-down (UIKit). Flip each stroke's y.
                let mapped = s.map { (x: x0 + CGFloat($0.x), y: y + CGFloat(raster.height - 1 - $0.y) + wobble) }
                for p in mapped {
                    if region == .zero {
                        region = CGRect(x: p.x - 5, y: p.y - 5, width: 10, height: 10)
                    } else {
                        region = region.union(CGRect(x: p.x - 5, y: p.y - 5, width: 10, height: 10))
                    }
                }
                strokes.append(PlannedStroke(points: mapped))
            }
            y += lineH
        }
        return WritePlan(strokes: strokes, region: region, nextY: y)
    }

    /// Load the Dancing Script font from the app bundle.
    static func loadDancingScriptFont(size: CGFloat) -> CTFont? {
        // Try to find and register the font from the app bundle.
        if let fontURL = Bundle.main.url(forResource: "DancingScript", withExtension: "ttf") {
            // Use CGFont + CTFontCreateWithGraphicsFont for reliable loading.
            if let dataProvider = CGDataProvider(url: fontURL as CFURL),
               let cgFont = CGFont(dataProvider) {
                return CTFontCreateWithGraphicsFont(cgFont, size, nil, nil)
            }
            // Fallback: register then create by name.
            var error: Unmanaged<CFError>?
            CTFontManagerRegisterFontsForURL(fontURL as CFURL, .process, &error)
            return CTFontCreateWithName("DancingScript" as CFString, size, nil)
        }
        // Fallback: use system handwriting font.
        return CTFontCreateWithName("SnellRoundhand" as CFString, size, nil)
    }
}
