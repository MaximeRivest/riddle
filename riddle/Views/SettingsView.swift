import SwiftUI

/// Settings screen for configuring the Oracle API.
/// Users enter their own API key, base URL, and model.
/// Supports any OpenAI-compatible API (OpenAI, OpenRouter, Volcano Ark, Groq, local server, etc.)
struct SettingsView: View {
    @AppStorage("RIDDLE_OPENAI_KEY") private var apiKey: String = ""
    @AppStorage("RIDDLE_OPENAI_BASE") private var baseURL: String = "https://api.openai.com/v1"
    @AppStorage("RIDDLE_OPENAI_MODEL") private var model: String = "gpt-4o-mini"
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationView {
            Form {
                Section(header: Text("API Configuration")) {
                    SecureField("API Key", text: $apiKey)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)

                    TextField("Base URL", text: $baseURL)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)

                    TextField("Model", text: $model)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                }

                Section(header: Text("Presets")) {
                    Button("OpenAI (gpt-4o-mini)") {
                        baseURL = "https://api.openai.com/v1"
                        model = "gpt-4o-mini"
                    }
                    Button("OpenRouter") {
                        baseURL = "https://openrouter.ai/api/v1"
                        model = "openai/gpt-4o-mini"
                    }
                    Button("Volcano Ark (doubao-seed-2-0-pro)") {
                        baseURL = "https://ark.cn-beijing.volces.com/api/plan/v3"
                        model = "doubao-seed-2-0-pro"
                    }
                    Button("Groq") {
                        baseURL = "https://api.groq.com/openai/v1"
                        model = "llama-3.2-90b-vision-preview"
                    }
                }

                Section(header: Text("Note")) {
                    Text("Any OpenAI-compatible, vision-capable API works. The model must support image input to read your handwriting.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done") {
                        dismiss()
                    }
                }
            }
        }
    }
}
