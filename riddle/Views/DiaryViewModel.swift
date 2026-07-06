import Foundation
import UIKit
import SwiftUI

/// The diary's view model — direct port of main.rs run() state machine.
///
/// State flow:
///   Listening → (idle 2.8s) → Drinking → Thinking → Replying → Lingering → Fading → Listening
@MainActor
final class DiaryViewModel: ObservableObject {

    @Published var state: DiaryState = .listening
    @Published var isEraser = false
    @Published var shouldClearCanvas = false
    @Published var surfaceImage: UIImage?
    @Published var replyText: String = ""
    @Published var showThinking = false

    var canvasView: DiaryCanvasView?
    private var oracle: Oracle?
    private var font: CTFont?

    private var idleTimer: Timer?
    private var dissolveTimer: Timer?
    private var lingerTimer: Timer?

    private var replyChunks: [String] = []
    private var oracleDone = false

    // MARK: - Lifecycle

    func start() {
        font = HandwritingSynthesis.loadDancingScriptFont(size: DiaryConfig.replyPx)
        do {
            oracle = try Oracle()
            print("riddle: oracle ready")
        } catch {
            print("riddle: oracle failed: \(error)")
        }
        state = .listening
        print("riddle: the diary is open")
    }

    func stop() {
        idleTimer?.invalidate()
        dissolveTimer?.invalidate()
        lingerTimer?.invalidate()
        replyTimer?.invalidate()
    }

    // MARK: - Canvas bridge

    func attachCanvas(_ view: DiaryCanvasView) {
        canvasView = view
    }

    // MARK: - Events

    func onStrokeEnded() {
        guard case .listening = state else { return }
        idleTimer?.invalidate()
        idleTimer = Timer.scheduledTimer(withTimeInterval: DiaryConfig.idleCommit, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.commitPage() }
        }
    }

    func onFiveFingerTap() {
        exit(0)
    }

    // MARK: - State transitions (mirrors main.rs state machine)

    /// Idle timeout → commit the page. Port of main.rs Listening→Drinking transition.
    private func commitPage() {
        guard case .listening = state else { return }
        guard let canvas = canvasView else { return }

        let penStrokes = canvas.penStrokes
        guard !penStrokes.isEmpty else { return }

        // Check if user drew a "?" → show help.
        if QuestionMarkDetector.looksLikeQuestionMark(strokes: penStrokes) {
            shouldClearCanvas = true
            state = .help
            return
        }

        // Export PNG for oracle.
        guard let pngData = canvas.exportToPNG() else { return }

        // Snapshot the current surface for dissolve display.
        surfaceImage = canvas.surface.toImage()

        // Ask oracle NOW — model streams while diary drinks the ink (hides latency).
        replyChunks = []
        oracleDone = false
        oracle?.ask(pngData: pngData) { [weak self] chunk in
            Task { @MainActor in
                self?.replyChunks.append(chunk)
                self?.tryStartReply()
            }
        } onDone: { [weak self] error in
            Task { @MainActor in
                self?.oracleDone = true
                if let error = error {
                    print("riddle: oracle error: \(error)")
                    if self?.replyChunks.isEmpty ?? true {
                        self?.replyChunks.append("The ink blurs…")
                    }
                }
                self?.tryStartReply()
            }
        }

        // Capture ink bbox BEFORE clearing (needed for dissolve region).
        let inkRegion = canvas.ink.bbox

        // Clear ink data only (keep surface pixels for dissolve, like main.rs).
        // The surface pixels will be cleared after dissolve completes.
        canvas.ink.clear()

        // Start dissolving — 14 stages, 70ms each (like main.rs).
        state = .drinking(stage: 0, totalStages: DiaryConfig.dissolveStages)
        runDissolve(region: inkRegion)
    }

    /// Dissolve animation — port of main.rs Drinking state.
    private func runDissolve(region: BBox) {
        guard let canvas = canvasView else {
            state = .thinking
            return
        }

        var stage = 0
        let stages = DiaryConfig.dissolveStages

        dissolveTimer?.invalidate()
        dissolveTimer = Timer.scheduledTimer(withTimeInterval: DiaryConfig.dissolveInterval, repeats: true) { [weak self] timer in
            Task { @MainActor in
                guard let self = self else { timer.invalidate(); return }

                stage += 1
                Ink.dissolvePass(canvas.surface, region, stage, stages)
                self.surfaceImage = canvas.surface.toImage()

                if stage >= stages {
                    timer.invalidate()
                    canvas.clearInk()
                    self.surfaceImage = nil
                    self.state = .thinking
                    self.showThinking = true
                    self.tryStartReply()
                }
            }
        }
    }

    /// Try to start reply if we have content and dissolve is done. Port of Thinking→Replying.
    private func tryStartReply() {
        if case .drinking = state { return }  // Still dissolving.

        if case .thinking = state {
            guard !replyChunks.isEmpty else { return }
            showThinking = false
            let firstChunk = replyChunks.removeFirst()
            startReply(text: firstChunk)
            return
        }

        // Already replying? Append new chunks.
        if case .replying = state, !replyChunks.isEmpty {
            let chunk = replyChunks.removeFirst()
            replyText += " " + chunk
        }
    }

    // Reply animation state (like main.rs WritePlan)
    private var replyPlan: WritePlan?
    private var replyStrokeI = 0
    private var replyPointI = 0
    private var replyTimer: Timer?

    /// Start reply — port of main.rs plan_reply + Replying state.
    /// Uses handwriting synthesis (rasterize → thin → trace) then animates stroke by stroke.
    private func startReply(text: String) {
        state = .replying

        var fullText = text
        if !replyChunks.isEmpty {
            fullText += " " + replyChunks.joined(separator: " ")
            replyChunks.removeAll()
        }

        guard let canvas = canvasView, let surf = canvas.surface else {
            replyText = fullText
            scheduleLinger()
            return
        }

        // Load font for handwriting synthesis.
        let fontSize: CGFloat = 48
        let font = HandwritingSynthesis.loadDancingScriptFont(size: fontSize)

        // Generate handwriting strokes (like main.rs plan_reply).
        let plan = HandwritingSynthesis.planReply(
            text: fullText,
            font: font ?? CTFontCreateWithName("SnellRoundhand" as CFString, fontSize, nil),
            screenWidth: CGFloat(surf.width),
            screenHeight: CGFloat(surf.height)
        )

        if plan.strokes.isEmpty {
            replyText = fullText
            scheduleLinger()
            return
        }

        // Clear the surface for the reply.
        surf.fillRect(0, 0, surf.width, surf.height, Surface.white)

        // Start the writing animation — draw points on the surface one by one.
        replyPlan = plan
        replyStrokeI = 0
        replyPointI = 0
        replyText = ""
        surfaceImage = surf.toImage()

        replyTimer?.invalidate()
        replyTimer = Timer.scheduledTimer(withTimeInterval: DiaryConfig.replyStrokeInterval, repeats: true) { [weak self] timer in
            Task { @MainActor in
                guard let self = self, let canvas = self.canvasView, let surf = canvas.surface else {
                    timer.invalidate()
                    return
                }
                self.tickReply(surf: surf, timer: timer)
            }
        }
    }

    /// One frame of the reply writing animation — port of main.rs Replying state.
    /// Draws a budget of points per frame using brushLine + stamp on the surface.
    private func tickReply(surf: Surface, timer: Timer) {
        guard let plan = replyPlan else { timer.invalidate(); return }

        var budget = DiaryConfig.replyBudget

        while budget > 0 && replyStrokeI < plan.strokes.count {
            let stroke = plan.strokes[replyStrokeI]

            if replyPointI >= stroke.points.count {
                replyStrokeI += 1
                replyPointI = 0
                continue
            }

            let x = Int(stroke.points[replyPointI].x)
            let y = Int(stroke.points[replyPointI].y)

            if replyPointI > 0 {
                let px = Int(stroke.points[replyPointI - 1].x)
                let py = Int(stroke.points[replyPointI - 1].y)
                surf.brushLine(px, py, x, y, 2, Surface.black)
            } else {
                surf.stamp(x, y, 2, Surface.black)
            }

            replyPointI += 1
            budget -= 1
        }

        surfaceImage = surf.toImage()

        if replyStrokeI >= plan.strokes.count {
            timer.invalidate()
            scheduleLinger()
        }
    }

    /// Schedule the linger phase after reply is written.
    private func scheduleLinger() {
        let textLen = replyText.isEmpty ? 50 : replyText.count
        let linger = min(DiaryConfig.lingerBase + Double(textLen) * DiaryConfig.lingerPerPoint,
                         DiaryConfig.lingerMax)
        lingerTimer?.invalidate()
        lingerTimer = Timer.scheduledTimer(withTimeInterval: linger, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.startFading() }
        }
    }

    /// Fade — port of main.rs FadingReply state. Dissolves the reply ink.
    private func startFading() {
        guard let canvas = canvasView else {
            replyText = ""
            state = .listening
            return
        }

        // Fade the reply using dissolve (like main.rs FadingReply).
        var bounds = BBox(x0: 0, y0: 0, x1: canvas.surface!.width, y1: canvas.surface!.height)
        if let region = replyPlan?.region, region != .zero {
            bounds = BBox(x0: max(Int(region.minX), 0),
                          y0: max(Int(region.minY), 0),
                          x1: min(Int(region.maxX), canvas.surface!.width),
                          y1: min(Int(region.maxY), canvas.surface!.height))
        }
        var stage = 0
        let stages = DiaryConfig.fadeStages

        replyTimer?.invalidate()
        replyTimer = Timer.scheduledTimer(withTimeInterval: DiaryConfig.fadeInterval, repeats: true) { [weak self] timer in
            Task { @MainActor in
                guard let self = self, let canvas = self.canvasView else { timer.invalidate(); return }

                stage += 1
                Ink.dissolvePass(canvas.surface, bounds, stage, stages)
                self.surfaceImage = canvas.surface.toImage()

                if stage >= stages {
                    timer.invalidate()
                    canvas.surface?.fillRect(0, 0, canvas.surface!.width, canvas.surface!.height, Surface.white)
                    self.surfaceImage = nil
                    self.replyText = ""
                    self.replyPlan = nil
                    self.state = .listening
                }
            }
        }
    }

    func dismissHelp() {
        state = .listening
    }

    func canvasCleared() {
        shouldClearCanvas = false
    }
}
