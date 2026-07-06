import Foundation

/// Port of sentence_cut() from oracle.rs.
/// Finds the end of the last complete sentence in `text` after byte offset `from`.
/// Returns the offset just past the punctuation, or nil if no sentence has completed.
/// Chunks shorter than a few characters are not worth an early delivery.
func sentenceCut(_ text: String, from offset: Int) -> Int? {
    let chars = Array(text)
    guard offset < chars.count else { return nil }

    var cut: Int? = nil
    var i = offset
    while i < chars.count {
        let c = chars[i]
        if c == "." || c == "!" || c == "?" || c == "…" {
            let end = i + 1 // c.len_utf8() == 1 for these chars
            // Check if next char is whitespace or end-of-text
            let isEnd = end >= chars.count || chars[end].isWhitespace
            // Minimum chunk length check (end >= 4 in original, measured from offset)
            if isEnd && end - offset >= 4 {
                cut = end
            }
        }
        i += 1
    }
    return cut
}

/// Clean a reply fragment: trim whitespace and stray quotes.
func cleanFragment(_ s: String) -> String {
    var t = s.trimmingCharacters(in: .whitespacesAndNewlines)
    if t.hasPrefix("\"") { t.removeFirst() }
    if t.hasSuffix("\"") { t.removeLast() }
    return t.trimmingCharacters(in: .whitespacesAndNewlines)
}

/// Split streamed text into sentence-sized chunks for progressive delivery.
/// Ported from the SSE streaming logic in oracle.rs.
struct SentenceStream {
    private var accumulated = ""
    private var delivered = 0

    /// Feed a new text fragment; returns any newly completed sentences.
    mutating func feed(_ fragment: String) -> [String] {
        accumulated += fragment
        var chunks: [String] = []

        while let cut = sentenceCut(accumulated, from: delivered) {
            let chunk = String(accumulated[accumulated.index(accumulated.startIndex, offsetBy: delivered)..<accumulated.index(accumulated.startIndex, offsetBy: cut)])
            let cleaned = cleanFragment(chunk)
            if !cleaned.isEmpty {
                chunks.append(cleaned)
            }
            delivered = cut
        }
        return chunks
    }

    /// Flush any remaining text past the last sentence break.
    mutating func flush() -> String? {
        guard delivered < accumulated.count else { return nil }
        let rest = String(accumulated[accumulated.index(accumulated.startIndex, offsetBy: delivered)...])
        let cleaned = cleanFragment(rest)
        delivered = accumulated.count
        return cleaned.isEmpty ? nil : cleaned
    }
}
