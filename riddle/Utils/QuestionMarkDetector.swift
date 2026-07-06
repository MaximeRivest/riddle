import Foundation
import UIKit

/// Detects whether the user's ink looks like a large "?".
/// Ported from help.rs looks_like_question_mark().
/// Uses local geometry — no oracle — so the guide works even with no network.
struct QuestionMarkDetector {

    static func looksLikeQuestionMark(strokes: [Stroke]) -> Bool {
        // Filter to pen strokes only (not eraser).
        let penStrokes = strokes.filter { !$0.isEraser }
        if penStrokes.isEmpty || penStrokes.count > 3 { return false }

        // Find the longest stroke as the "main" one.
        let mainI = penStrokes.indices.max(by: { penStrokes[$0].points.count < penStrokes[$1].points.count })!
        let main = penStrokes[mainI]
        if main.points.count < 12 { return false }

        // Bounding box of the main stroke.
        let bbox = main.boundingBox
        let w = bbox.width
        let h = bbox.height
        // Big, and taller than wide: a lone glyph, not a line of writing.
        if h < 140 || w < 35 || h < w { return false }

        // Any other stroke must be the dot: small, low, roughly under the glyph.
        for (i, s) in penStrokes.enumerated() {
            if i == mainI { continue }
            let dbox = s.boundingBox
            if max(dbox.width, dbox.height) > 45 { return false }
            let dotCenterY = dbox.midY
            if dotCenterY < bbox.minY + h * 0.6 { return false }
            let dotCenterX = dbox.midX
            if dotCenterX < bbox.minX - 40 || dotCenterX > bbox.maxX + 40 { return false }
        }

        // Normalize to top-down drawing order.
        var pts = main.points.map { (x: $0.x, y: $0.y) }
        if pts.first!.y > pts.last!.y { pts.reverse() }
        let start = pts.first!
        let end = pts.last!

        if start.y > bbox.minY + h * 0.4 || end.y < bbox.minY + h * 0.55 {
            return false
        }

        // The top arcs across most of the width…
        var topMinX = CGFloat.greatestFiniteMagnitude
        var topMaxX = -CGFloat.greatestFiniteMagnitude
        var topMaxXY: CGFloat = 0
        for p in pts {
            if p.y <= bbox.minY + h * 0.45 {
                if p.x > topMaxX { topMaxX = p.x; topMaxXY = p.y }
                topMinX = min(topMinX, p.x)
            }
        }
        if topMaxX == -CGFloat.greatestFiniteMagnitude || topMaxX - topMinX < w * 0.55 {
            return false
        }
        // …and comes back DOWN on the right (rules out the flat bar of a "7").
        if topMaxXY < bbox.minY + h * 0.08 { return false }

        // The descender stays narrow.
        var botMinX = CGFloat.greatestFiniteMagnitude
        var botMaxX = -CGFloat.greatestFiniteMagnitude
        for p in pts {
            if p.y >= bbox.minY + h * 0.66 {
                botMinX = min(botMinX, p.x)
                botMaxX = max(botMaxX, p.x)
            }
        }
        if botMaxX != -CGFloat.greatestFiniteMagnitude && botMaxX - botMinX > w * 0.6 {
            return false
        }
        return true
    }
}

/// The diary's help panel content.
struct HelpPanel {
    let title = "The Diary"
    let body = [
        "Write, then rest your quill:",
        "the diary drinks your ink and Tom replies.",
        "",
        "Flip the pencil double-tap to erase.",
        "Draw a large ? for this guide.",
        "Pinch with five fingers to leave.",
    ]
    let footer = "Touch pencil to page to close."
}
