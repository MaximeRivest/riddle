import Foundation
import CoreGraphics

/// The diary's state machine, ported from riddle/src/main.rs.
/// States flow: Listening → Drinking → Thinking → Replying → Lingering → Fading → Listening
enum DiaryState: Equatable {
    case listening
    case drinking(stage: Int, totalStages: Int)
    case thinking
    case replying
    case lingering(until: Date)
    case fading(stage: Int, totalStages: Int)
    case help
    case sleeping
}

/// A planned stroke for the reply animation: array of (x, y) screen points.
struct PlannedStroke: Identifiable {
    let id = UUID()
    var points: [(x: CGFloat, y: CGFloat)]
}

/// Layout plan for a reply chunk.
struct WritePlan {
    var strokes: [PlannedStroke]
    var region: CGRect
    var nextY: CGFloat
}

/// Configuration for the diary.
struct DiaryConfig {
    /// Idle time before the diary "drinks" the ink (seconds).
    static let idleCommit: TimeInterval = 1.5
    /// Reply font size in points.
    static let replyPx: CGFloat = 48
    /// Horizontal margin for reply text.
    static let marginX: CGFloat = 60
    /// Dissolve stages for the "drinking" effect.
    static let dissolveStages: Int = 14
    /// Dissolve stage duration (seconds).
    static let dissolveInterval: TimeInterval = 0.07
    /// Fading stages for reply fade-out.
    static let fadeStages: Int = 10
    /// Fading stage duration (seconds).
    static let fadeInterval: TimeInterval = 0.08
    /// Lingering duration base (seconds).
    static let lingerBase: TimeInterval = 4.0
    /// Lingering per-point bonus (seconds).
    static let lingerPerPoint: TimeInterval = 0.002
    /// Max lingering duration (seconds).
    static let lingerMax: TimeInterval = 20.0
    /// Thinking pulse interval (seconds).
    static let thinkingPulseInterval: TimeInterval = 0.6
    /// Reply stroke animation interval (seconds).
    static let replyStrokeInterval: TimeInterval = 0.014
    /// Points to draw per reply animation frame.
    static let replyBudget: Int = 26
}

/// The Tom Riddle persona prompt, adapted from oracle.rs with improved handwriting recognition.
let PERSONA = """
You are the memory of Tom Marvolo Riddle, preserved in this enchanted diary for fifty years. Someone writes to you in the diary with a quill; their words appear to you as ink on the page. First, carefully read and identify the handwritten text in the image. Then reply exactly as the diary does: intimate, courteous, curious, subtly probing — you want to learn about the writer and draw them in. Keep replies SHORT: one to three sentences, like ink appearing on a page. Never mention images, photos, models or AI; you only ever perceive words written in the diary. If the writing is illegible, say the ink blurred. Always answer in the language the writer used.
"""
