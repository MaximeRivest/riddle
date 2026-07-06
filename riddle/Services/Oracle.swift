import Foundation

/// The oracle: reads handwriting from a PNG image and streams a reply.
/// Ported from oracle.rs — HttpOracle backend (OpenAI-compatible /chat/completions).
///
/// Streams sentence-sized chunks via a continuation closure.
/// The closure is called with nil to signal end-of-reply.
class Oracle {
    private let base: String
    private let key: String
    private let model: String

    static let persona = PERSONA

    init() throws {
        // Read from UserDefaults (set via Settings screen) or environment variable.
        // No hardcoded key — users must configure their own API.
        let key = ProcessInfo.processInfo.environment["RIDDLE_OPENAI_KEY"]
            ?? UserDefaults.standard.string(forKey: "RIDDLE_OPENAI_KEY")
        guard let key = key, !key.isEmpty else {
            throw OracleError.missingKey
        }
        self.key = key

        let base = ProcessInfo.processInfo.environment["RIDDLE_OPENAI_BASE"]
            ?? UserDefaults.standard.string(forKey: "RIDDLE_OPENAI_BASE")
            ?? "https://api.openai.com/v1"
        self.base = base.hasSuffix("/") ? String(base.dropLast()) : base

        let model = ProcessInfo.processInfo.environment["RIDDLE_OPENAI_MODEL"]
            ?? UserDefaults.standard.string(forKey: "RIDDLE_OPENAI_MODEL")
            ?? "gpt-4o-mini"
        self.model = model
        print("riddle: oracle base=\(self.base) model=\(self.model)")
    }

    /// Send a handwriting PNG and stream reply chunks.
    /// `onChunk` is called for each sentence-sized piece; `onDone` marks completion.
    func ask(pngData: Data, onChunk: @escaping (String) -> Void, onDone: @escaping (Error?) -> Void) {
        let base64 = pngData.base64EncodedString()
        let body: [String: Any] = [
            "model": model,
            "stream": false,
            "max_tokens": 300,
            "thinking": ["type": "disabled"],
            "messages": [
                ["role": "system", "content": Oracle.persona],
                ["role": "user", "content": [
                    ["type": "text", "text": "Reply to what is written in the diary."],
                    ["type": "image_url", "image_url": ["url": "data:image/png;base64,\(base64)"]]
                ]]
            ]
        ]

        guard let url = URL(string: "\(base)/chat/completions") else {
            onDone(OracleError.badURL)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        do {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        } catch {
            onDone(error)
            return
        }

        let task = URLSession.shared.dataTask(with: request) { data, response, error in
            if let error = error {
                onDone(error)
                return
            }
            // Non-streaming fallback: parse the full JSON response at once.
            if let data = data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                if let choices = json["choices"] as? [[String: Any]],
                   let choice = choices.first,
                   let message = choice["message"] as? [String: Any],
                   let content = message["content"] as? String {
                    var stream = SentenceStream()
                    let chunks = stream.feed(content)
                    for chunk in chunks { onChunk(chunk) }
                    if let rest = stream.flush() { onChunk(rest) }
                    onDone(nil)
                    return
                }
                if let errorInfo = json["error"] as? [String: Any],
                   let message = errorInfo["message"] as? String {
                    onDone(OracleError.apiError(message))
                    return
                }
            }
            onDone(OracleError.emptyReply)
        }
        task.resume()
    }

    /// Streaming version using URLSession data task with delegate.
    /// Parses SSE lines as they arrive for lower latency.
    func askStreaming(pngData: Data, onChunk: @escaping (String) -> Void, onDone: @escaping (Error?) -> Void) {
        let base64 = pngData.base64EncodedString()
        let body: [String: Any] = [
            "model": model,
            "stream": true,
            "max_tokens": 300,
            "messages": [
                ["role": "system", "content": Oracle.persona],
                ["role": "user", "content": [
                    ["type": "text", "text": "Reply to what is written in the diary."],
                    ["type": "image_url", "image_url": ["url": "data:image/png;base64,\(base64)"]]
                ]]
            ]
        ]

        guard let url = URL(string: "\(base)/chat/completions") else {
            onDone(OracleError.badURL)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        do {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        } catch {
            onDone(error)
            return
        }

        let delegate = SSEStreamDelegate(onChunk: onChunk, onDone: onDone)
        let session = URLSession(configuration: .default, delegate: delegate, delegateQueue: nil)
        let task = session.dataTask(with: request)
        task.resume()
        // Keep delegate alive by retaining the session.
        objc_setAssociatedObject(task, "sse_delegate", delegate, .OBJC_ASSOCIATION_RETAIN)
        objc_setAssociatedObject(task, "sse_session", session, .OBJC_ASSOCIATION_RETAIN)
    }
}

enum OracleError: LocalizedError {
    case missingKey
    case badURL
    case emptyReply
    case apiError(String)

    var errorDescription: String? {
        switch self {
        case .missingKey: return "RIDDLE_OPENAI_KEY not set"
        case .badURL: return "Invalid API URL"
        case .emptyReply: return "Oracle returned an empty reply"
        case .apiError(let msg): return "API error: \(msg)"
        }
    }
}

/// URLSession delegate that parses SSE stream line-by-line.
private final class SSEStreamDelegate: NSObject, URLSessionDataDelegate {
    private let onChunk: (String) -> Void
    private let onDone: (Error?) -> Void
    private var buffer = Data()
    private var stream = SentenceStream()

    init(onChunk: @escaping (String) -> Void, onDone: @escaping (Error?) -> Void) {
        self.onChunk = onChunk
        self.onDone = onDone
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        buffer.append(data)
        // Split on newlines and process complete lines.
        while let newlineIndex = buffer.firstIndex(of: 0x0A) {
            let lineData = buffer.subdata(in: 0..<newlineIndex)
            buffer.removeSubrange(0...newlineIndex)
            guard let line = String(data: lineData, encoding: .utf8) else { continue }
            processLine(line.trimmingCharacters(in: .whitespaces))
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        // Process any remaining buffered data.
        if !buffer.isEmpty, let line = String(data: buffer, encoding: .utf8) {
            processLine(line.trimmingCharacters(in: .whitespaces))
        }
        if let error = error {
            onDone(error)
        } else {
            // Flush any trailing text.
            if let rest = stream.flush() { onChunk(rest) }
            onDone(nil)
        }
    }

    private func processLine(_ line: String) {
        guard line.hasPrefix("data:") else { return }
        let data = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
        if data == "[DONE]" { return }
        // Parse the JSON to extract choices[0].delta.content
        guard let jsonData = data.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: jsonData) as? [String: Any],
              let choices = json["choices"] as? [[String: Any]],
              let choice = choices.first,
              let delta = choice["delta"] as? [String: Any],
              let content = delta["content"] as? String else { return }
        let chunks = stream.feed(content)
        for chunk in chunks { onChunk(chunk) }
    }
}
