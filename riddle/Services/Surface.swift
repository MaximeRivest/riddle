import UIKit

/// Pixel surface — direct port of surface.rs from the original riddle.
/// Wraps a bitmap context that we draw to directly, like the reMarkable framebuffer.
final class Surface {
    let width: Int
    let height: Int
    private let bitmap: CGContext
    private var pixels: [UInt8]

    static let white: UInt8 = 255
    static let black: UInt8 = 0

    init(width: Int, height: Int) {
        self.width = width
        self.height = height
        self.pixels = [UInt8](repeating: 255, count: width * height)

        let colorSpace = CGColorSpaceCreateDeviceGray()
        guard let ctx = CGContext(data: &pixels,
                                  width: width, height: height,
                                  bitsPerComponent: 8,
                                  bytesPerRow: width,
                                  space: colorSpace,
                                  bitmapInfo: CGImageAlphaInfo.none.rawValue) else {
            fatalError("Cannot create surface context")
        }
        self.bitmap = ctx
        // Fill white.
        ctx.setFillColor(CGColor(gray: 1, alpha: 1))
        ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
    }

    // MARK: - Pixel operations (like surface.rs)

    /// Put a single pixel. Direct port of put_px.
    func putPx(_ x: Int, _ y: Int, _ gray: UInt8) {
        guard x >= 0, y >= 0, x < width, y < height else { return }
        pixels[y * width + x] = gray
    }

    /// Read pixel luminance. Direct port of luma().
    func luma(_ x: Int, _ y: Int) -> UInt8 {
        guard x >= 0, y >= 0, x < width, y < height else { return 255 }
        return pixels[y * width + x]
    }

    /// Stamp a filled circle. Direct port of stamp().
    func stamp(_ cx: Int, _ cy: Int, _ r: Int, _ gray: UInt8) {
        for dy in -r...r {
            for dx in -r...r {
                if dx * dx + dy * dy <= r * r {
                    putPx(cx + dx, cy + dy, gray)
                }
            }
        }
    }

    /// Draw a thick line by stamping along it. Direct port of brush_line().
    func brushLine(_ x0: Int, _ y0: Int, _ x1: Int, _ y1: Int, _ r: Int, _ gray: UInt8) {
        let dx = abs(x1 - x0)
        let dy = abs(y1 - y0)
        let steps = max(dx, dy, 1)
        for i in 0...steps {
            let x = x0 + (x1 - x0) * i / steps
            let y = y0 + (y1 - y0) * i / steps
            stamp(x, y, r, gray)
        }
    }

    /// Fill a rectangle. Direct port of fill_rect().
    func fillRect(_ x: Int, _ y: Int, _ w: Int, _ h: Int, _ gray: UInt8) {
        for row in y..<(y + h) {
            for col in x..<(x + w) {
                putPx(col, row, gray)
            }
        }
    }

    // MARK: - Snapshot

    /// Convert the surface to a UIImage for display.
    /// No flip needed — pixels are stored in top-down (UIKit) coordinates,
    /// and CGImage created from this data displays correctly in UIKit.
    func toImage() -> UIImage? {
        let colorSpace = CGColorSpaceCreateDeviceGray()
        let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue)
        guard let provider = CGDataProvider(data: Data(pixels) as CFData),
              let cgImage = CGImage(width: width, height: height,
                                    bitsPerComponent: 8, bitsPerPixel: 8,
                                    bytesPerRow: width, space: colorSpace,
                                    bitmapInfo: bitmapInfo,
                                    provider: provider, decode: nil,
                                    shouldInterpolate: false, intent: .defaultIntent) else { return nil }
        return UIImage(cgImage: cgImage)
    }

    /// Get the raw pixel array.
    func rawPixels() -> [UInt8] {
        return pixels
    }

    /// Get a mutable pointer to pixels (for dissolve).
    func mutablePixels() -> UnsafeMutableBufferPointer<UInt8> {
        return UnsafeMutableBufferPointer(start: &pixels, count: pixels.count)
    }

    /// Refresh the bitmap context after direct pixel manipulation.
    func flush() {
        // The bitmap context already shares the pixel buffer, so changes are immediate.
    }

    /// Export a region as grayscale PNG for the oracle.
    /// Direct port of ink.rs to_png(), with y-axis flip for correct orientation.
    func toPNG(x0: Int, y0: Int, x1: Int, y1: Int) -> Data? {
        let pad = 20
        let bx = max(x0 - pad, 0)
        let by = max(y0 - pad, 0)
        let bx1 = min(x1 + pad, width)
        let by1 = min(y1 + pad, height)
        let bw = bx1 - bx
        let bh = by1 - by
        guard bw > 0, bh > 0 else { return nil }

        // Downscale so long side ≤ 800px. Minimum factor 1 (no unnecessary downscale for small handwriting).
        let f = max((max(bw, bh) + 799) / 800, 1)
        let w = bw / f
        let h = bh / f

        var gray = [UInt8](repeating: 255, count: w * h)
        for oy in 0..<h {
            for ox in 0..<w {
                var acc: UInt32 = 0
                for sy in 0..<f {
                    for sx in 0..<f {
                        // Pixels are top-down (UIKit), PNG also expects top-down. No flip needed.
                        acc += UInt32(luma(bx + ox * f + sx, by + oy * f + sy))
                    }
                }
                gray[oy * w + ox] = UInt8(acc / UInt32(f * f))
            }
        }

        let colorSpace = CGColorSpaceCreateDeviceGray()
        let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue)
        guard let provider = CGDataProvider(data: Data(gray) as CFData),
              let cgImage = CGImage(width: w, height: h,
                                    bitsPerComponent: 8, bitsPerPixel: 8,
                                    bytesPerRow: w, space: colorSpace,
                                    bitmapInfo: bitmapInfo,
                                    provider: provider, decode: nil,
                                    shouldInterpolate: false, intent: .defaultIntent) else {
            return nil
        }
        let uiImage = UIImage(cgImage: cgImage)
        return uiImage.pngData()
    }
}

/// Bounding box — port of fb.rs BBox.
struct BBox {
    var x0: Int = Int.max
    var y0: Int = Int.max
    var x1: Int = Int.min
    var y1: Int = Int.min

    var isEmpty: Bool { x0 > x1 }

    static let empty = BBox()

    mutating func add(_ x: Int, _ y: Int, _ pad: Int = 0) {
        x0 = min(x0, x - pad)
        y0 = min(y0, y - pad)
        x1 = max(x1, x + pad)
        y1 = max(y1, y + pad)
    }

    func rect() -> (x: Int, y: Int, w: Int, h: Int) {
        if isEmpty { return (0, 0, 0, 0) }
        return (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
    }
}

/// Ink — direct port of ink.rs.
/// Captures pen strokes, renders them to the surface, exports PNG for oracle.
final class Ink {
    /// Finished strokes as point lists (x, y, radius).
    private(set) var strokes: [[(x: Int, y: Int, r: Int)]] = []
    private var current: [(x: Int, y: Int, r: Int)] = []
    private var lastErase: (x: Int, y: Int)?
    private(set) var bbox = BBox.empty

    var isEmpty: Bool { strokes.isEmpty && current.isEmpty }

    /// Finished stroke list (current in-flight not included).
    var strokeList: [[(x: Int, y: Int, r: Int)]] { strokes }

    func clear() {
        strokes.removeAll()
        current.removeAll()
        lastErase = nil
        bbox = BBox.empty
    }

    /// Pen touched down or moved. Draws directly to surface. Port of pen_point().
    @discardableResult
    func penPoint(_ surf: Surface, _ x: Int, _ y: Int, _ r: Int) -> BBox {
        var dirty = BBox.empty
        if let last = current.last {
            surf.brushLine(last.x, last.y, x, y, min(r, last.r + 1), Surface.black)
            dirty.add(last.x, last.y, last.r + 2)
        } else {
            surf.stamp(x, y, r, Surface.black)
        }
        dirty.add(x, y, r + 2)
        current.append((x, y, r))
        bbox.add(x, y, r + 2)
        return dirty
    }

    /// Eraser. Port of erase_point().
    @discardableResult
    func erasePoint(_ surf: Surface, _ x: Int, _ y: Int, _ r: Int) -> BBox {
        var dirty = BBox.empty
        if let (px, py) = lastErase {
            surf.brushLine(px, py, x, y, r, Surface.white)
            dirty.add(px, py, r + 2)
        } else {
            surf.stamp(x, y, r, Surface.white)
        }
        dirty.add(x, y, r + 2)
        lastErase = (x, y)
        return dirty
    }

    /// Pen lifted. Port of pen_up().
    func penUp() {
        if !current.isEmpty {
            strokes.append(current)
            current.removeAll()
        }
        lastErase = nil
    }

    /// Export ink region as PNG for the oracle. Port of to_png().
    func toPNG(_ surf: Surface) -> Data? {
        guard !bbox.isEmpty else { return nil }
        return surf.toPNG(x0: bbox.x0, y0: bbox.y0, x1: bbox.x1, y1: bbox.y1)
    }

    /// One pass of the "diary drinks the ink" dissolve. Port of dissolve_pass().
    static func dissolvePass(_ surf: Surface, _ region: BBox, _ stage: Int, _ stages: Int) {
        guard !region.isEmpty else { return }
        for y in region.y0...region.y1 {
            for x in region.x0...region.x1 {
                if surf.luma(x, y) < 250 {
                    let h = Self.pxHash(x, y)
                    if h % UInt32(stages) <= UInt32(stage) {
                        surf.putPx(x, y, Surface.white)
                    }
                }
            }
        }
    }

    /// Deterministic per-pixel hash. Port of px_hash().
    static func pxHash(_ x: Int, _ y: Int) -> UInt32 {
        var h = UInt32(truncatingIfNeeded: x) &* 0x9E3779B1 ^ (UInt32(truncatingIfNeeded: y) &* 0x85EBCA6B)
        h ^= h >> 13
        h = h &* 0xC2B2AE35
        return h ^ (h >> 16)
    }
}
