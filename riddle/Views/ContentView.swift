import SwiftUI
import UIKit

/// The main diary view — direct port of main.rs run() structure.
struct ContentView: View {

    @StateObject private var viewModel = DiaryViewModel()
    @State private var showSettings = false

    var body: some View {
        GeometryReader { geo in
            ZStack {
                Color.white.ignoresSafeArea()

                // Drawing canvas — direct pixel surface like the original framebuffer.
                DiaryCanvasRepresentable(viewModel: viewModel)
                    .ignoresSafeArea()

                // Dissolve overlay — shows the ink fading during Drinking state.
                // Use .resizable() without aspectRatio to exactly match canvas position.
                if let image = viewModel.surfaceImage {
                    Image(uiImage: image)
                        .resizable()
                        .frame(width: geo.size.width, height: geo.size.height)
                        .ignoresSafeArea()
                        .allowsHitTesting(false)
                }

                // Thinking indicator — removed per user request.
                // The original uses a pulsing ink blot, but user prefers no indicator.

                // Reply text — shown during Replying/Lingering states.
                if !viewModel.replyText.isEmpty {
                    Text(viewModel.replyText)
                        .font(.custom("DancingScript", size: 28))
                        .foregroundColor(.black)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 60)
                        .allowsHitTesting(false)
                        .transition(.opacity)
                }

                // Help panel.
                if viewModel.state == .help {
                    HelpPanelView(dismiss: { viewModel.dismissHelp() })
                        .transition(.opacity)
                }

                // Eraser indicator.
                if viewModel.isEraser && viewModel.state == .listening {
                    VStack {
                        HStack {
                            Spacer()
                            Text("Eraser")
                                .font(.caption)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 6)
                                .background(Color.black.opacity(0.7))
                                .foregroundColor(.white)
                                .cornerRadius(8)
                                .padding()
                        }
                        Spacer()
                    }
                }
            }
        }
        .preferredColorScheme(.light)
        .statusBarHidden()
        .onAppear {
            viewModel.start()
            // Show settings on first launch if no API key configured.
            if UserDefaults.standard.string(forKey: "RIDDLE_OPENAI_KEY") == nil {
                showSettings = true
            }
        }
        .onDisappear { viewModel.stop() }
        .sheet(isPresented: $showSettings) {
            SettingsView()
        }
    }
}

// MARK: - Canvas UIViewRepresentable

struct DiaryCanvasRepresentable: UIViewRepresentable {
    let viewModel: DiaryViewModel

    func makeUIView(context: Context) -> DiaryCanvasView {
        let view = DiaryCanvasView()
        viewModel.attachCanvas(view)
        view.onStrokeEnded = {
            viewModel.onStrokeEnded()
        }
        view.onFiveFingerTap = {
            viewModel.onFiveFingerTap()
        }
        return view
    }

    func updateUIView(_ uiView: DiaryCanvasView, context: Context) {
        uiView.setEraser(viewModel.isEraser)
        if viewModel.shouldClearCanvas {
            uiView.clearInk()
            viewModel.canvasCleared()
        }
    }
}

// MARK: - Thinking indicator

struct ThinkingIndicator: View {
    @State private var pulse = false

    var body: some View {
        Circle()
            .fill(Color.black)
            .frame(width: 10, height: 10)
            .opacity(pulse ? 0.7 : 0.15)
            .animation(.easeInOut(duration: 0.8).repeatForever(autoreverses: true), value: pulse)
            .onAppear { pulse = true }
            .position(x: UIScreen.main.bounds.width / 2, y: UIScreen.main.bounds.height / 2)
    }
}

// MARK: - Help panel

struct HelpPanelView: View {
    let dismiss: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            Text("The Diary")
                .font(.system(size: 44, weight: .medium, design: .serif).italic())
                .foregroundColor(.black)

            VStack(spacing: 12) {
                Text("Write, then rest your quill:")
                Text("the diary drinks your ink and Tom replies.")
                Spacer().frame(height: 16)
                Text("Pencil double-tap to erase.")
                Text("Draw a large ? for this guide.")
                Text("Pinch with five fingers to leave.")
            }
            .font(.system(size: 22, design: .serif).italic())
            .foregroundColor(.black)
            .multilineTextAlignment(.center)

            Text("Touch pencil to page to close.")
                .font(.system(size: 18, design: .serif).italic())
                .foregroundColor(.gray)
        }
        .padding(50)
        .background(Color.white)
        .overlay(RoundedRectangle(cornerRadius: 0).stroke(Color.black, lineWidth: 3).padding(10))
        .overlay(RoundedRectangle(cornerRadius: 0).stroke(Color.black, lineWidth: 1).padding(20))
        .onTapGesture { dismiss() }
    }
}
