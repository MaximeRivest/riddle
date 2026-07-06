import UIKit

/// Canvas view using a direct pixel Surface — mirrors the original riddle's approach.
/// Pen events draw directly to the Surface bitmap; draw() just blits it to screen.
class DiaryCanvasView: UIView {

    var onStrokeEnded: (() -> Void)?
    var onFiveFingerTap: (() -> Void)?

    /// The pixel surface (like the reMarkable framebuffer).
    private(set) var surface: Surface!

    /// The ink capture (like ink.rs Ink).
    private(set) var ink = Ink()

    private var penDown = false
    private var isEraser = false

    // MARK: - Init

    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        backgroundColor = UIColor.white
        isOpaque = true
        isMultipleTouchEnabled = true
        contentMode = .redraw

        // Create the surface matching the view size (in points = pixels for our scale).
        let w = max(Int(bounds.width), 1)
        let h = max(Int(bounds.height), 1)
        surface = Surface(width: w, height: h)

        let interaction = UIPencilInteraction()
        interaction.delegate = self
        addInteraction(interaction)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        // Recreate surface if size changed significantly.
        let w = max(Int(bounds.width), 1)
        let h = max(Int(bounds.height), 1)
        if surface == nil || surface.width != w || surface.height != h {
            surface = Surface(width: w, height: h)
            setNeedsDisplay()
        }
    }

    // MARK: - Touch handling (mirrors main.rs pen event loop)

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        if event?.allTouches?.count ?? 0 >= 5 {
            onFiveFingerTap?()
            return
        }
        guard let touch = touches.first else { return }
        // Only Apple Pencil can draw. Finger taps are ignored for drawing.
        guard touch.type == .pencil else { return }

        let point = touch.location(in: self)
        let pressure = touch.force > 0 ? touch.force / max(touch.maximumPossibleForce, 1) : 0.5

        penDown = true
        let r = isEraser ? 22 : 2 + Int(pressure * 3)
        if isEraser {
            ink.erasePoint(surface, Int(point.x), Int(point.y), r)
        } else {
            ink.penPoint(surface, Int(point.x), Int(point.y), r)
        }
        setNeedsDisplay()
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard penDown, let touch = touches.first else { return }

        // Use coalesced touches for smooth lines.
        let coalesced = event?.coalescedTouches(for: touch) ?? [touch]
        for t in coalesced {
            let point = t.location(in: self)
            let pressure = t.force > 0 ? t.force / max(t.maximumPossibleForce, 1) : 0.5
            let r = isEraser ? 22 : 2 + Int(pressure * 3)

            if isEraser {
                ink.erasePoint(surface, Int(point.x), Int(point.y), r)
            } else {
                ink.penPoint(surface, Int(point.x), Int(point.y), r)
            }
        }
        setNeedsDisplay()
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        penUp()
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        penUp()
    }

    private func penUp() {
        guard penDown else { return }
        penDown = false
        ink.penUp()
        onStrokeEnded?()
        setNeedsDisplay()
    }

    // MARK: - Render (just blit the surface to screen, like the original)

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext() else { return }
        ctx.setFillColor(UIColor.white.cgColor)
        ctx.fill(rect)

        // toImage() already handles the y-axis flip, so just draw directly.
        guard let surface = surface, let img = surface.toImage() else { return }
        img.draw(in: rect)
    }

    // MARK: - Public API

    /// Clear ink data only (stroke list + bbox), keep surface pixels intact.
    /// Used before dissolve so the dissolve can work on the actual ink pixels.
    func clearInkDataOnly() {
        ink.clear()
        setNeedsDisplay()
    }

    func clearInk() {
        ink.clear()
        // Clear the surface to white.
        let w = surface.width
        let h = surface.height
        surface.fillRect(0, 0, w, h, Surface.white)
        setNeedsDisplay()
    }

    /// Get pen strokes for question mark detection.
    /// Convert ink strokes to the format QuestionMarkDetector expects.
    var penStrokes: [Stroke] {
        ink.strokeList.filter { !$0.isEmpty }.map { strokePoints in
            Stroke(points: strokePoints.map { (x: CGFloat($0.x), y: CGFloat($0.y), pressure: CGFloat($0.r) / 5) }, isEraser: false)
        }
    }

    /// Export ink as PNG for the oracle.
    func exportToPNG() -> Data? {
        return ink.toPNG(surface)
    }

    func setEraser(_ on: Bool) {
        isEraser = on
    }

    var isEraserMode: Bool { isEraser }
}

// MARK: - Apple Pencil double-tap

extension DiaryCanvasView: UIPencilInteractionDelegate {
    func pencilInteractionDidTap(_ interaction: UIPencilInteraction) {
        isEraser.toggle()
    }
}

/// Stroke type for question mark detection (kept compatible).
struct Stroke {
    var points: [(x: CGFloat, y: CGFloat, pressure: CGFloat)]
    var isEraser: Bool

    var boundingBox: CGRect {
        guard !points.isEmpty else { return .zero }
        var minX = CGFloat.greatestFiniteMagnitude
        var minY = CGFloat.greatestFiniteMagnitude
        var maxX = -CGFloat.greatestFiniteMagnitude
        var maxY = -CGFloat.greatestFiniteMagnitude
        for p in points {
            minX = min(minX, p.x); minY = min(minY, p.y)
            maxX = max(maxX, p.x); maxY = max(maxY, p.y)
        }
        return CGRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY)
    }
}
