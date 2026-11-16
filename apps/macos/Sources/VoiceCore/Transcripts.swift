// Transcript normalization and word error rate, for the recognition evaluation.
// Spoken and written forms meet in one token form: lowercase, punctuation dropped,
// "$42" → "42 dollars", "9:30" → "9 30", English number words → digits.

import Foundation

public enum Transcripts {
    private static let units: [String: Int] = [
        "zero": 0, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8, "nine": 9,
        "ten": 10, "eleven": 11, "twelve": 12, "thirteen": 13, "fourteen": 14, "fifteen": 15, "sixteen": 16,
        "seventeen": 17, "eighteen": 18, "nineteen": 19, "twenty": 20, "thirty": 30, "forty": 40, "fifty": 50,
        "sixty": 60, "seventy": 70, "eighty": 80, "ninety": 90,
    ]
    private static let scales: [String: Int] = ["hundred": 100, "thousand": 1_000, "million": 1_000_000]

    public static func tokens(_ text: String) -> [String] {
        var spaced = ""
        let chars = Array(text.lowercased())
        for (i, c) in chars.enumerated() {
            let prevDigit = i > 0 && chars[i - 1].isNumber
            let nextDigit = i + 1 < chars.count && chars[i + 1].isNumber
            if c == "$" && nextDigit {
                spaced += " $"
            } else if c.isLetter || c.isNumber {
                spaced.append(c)
            } else if c == "," && prevDigit && nextDigit {
                continue
            } else if c == "'" && i > 0 && chars[i - 1].isLetter {
                continue
            } else if c == "." && prevDigit && nextDigit {
                spaced.append(c)
            } else {
                spaced += " "
            }
        }
        var raw: [String] = []
        for word in spaced.split(separator: " ").map(String.init) {
            if word.hasPrefix("$") {
                raw.append(String(word.dropFirst()))
                raw.append("dollars")
            } else {
                raw.append(word)
            }
        }
        return mergeNumbers(raw)
    }

    /// Same rules as voice-domain's normalizer: a tens word may take one unit, scales
    /// multiply, "and" joins only after a scale, anything else starts a new number.
    private static func mergeNumbers(_ raw: [String]) -> [String] {
        enum Last { case tens, small, scale }
        var out: [String] = []
        var i = 0
        while i < raw.count {
            guard let first = units[raw[i]] else {
                out.append(raw[i])
                i += 1
                continue
            }
            var total = 0
            var current = first
            var last: Last = first >= 20 ? .tens : .small
            var j = i + 1
            while j < raw.count {
                let w = raw[j]
                if let v = units[w] {
                    let fits = last == .scale || (last == .tens && (1...9).contains(v))
                    if !fits { break }
                    current += v
                    last = v >= 20 ? .tens : .small
                    j += 1
                } else if let scale = scales[w] {
                    if scale == 100 {
                        if last == .scale { break }
                        current *= 100
                    } else {
                        total += max(current, 1) * scale
                        current = 0
                    }
                    last = .scale
                    j += 1
                } else if w == "and", last == .scale, j + 1 < raw.count, units[raw[j + 1]] != nil {
                    j += 1
                } else {
                    break
                }
            }
            out.append(String(total + current))
            i = j
        }
        return out
    }

    /// Word error rate: (substitutions + deletions + insertions) / reference words.
    public static func wordErrorRate(reference: String, hypothesis: String) -> Double {
        let r = tokens(reference)
        let h = tokens(hypothesis)
        guard !r.isEmpty else { return h.isEmpty ? 0 : 1 }
        var prev = Array(0...h.count)
        for i in 1...r.count {
            var row = [i] + Array(repeating: 0, count: h.count)
            for j in 1...max(h.count, 1) where j <= h.count {
                row[j] = min(prev[j] + 1, row[j - 1] + 1, prev[j - 1] + (r[i - 1] == h[j - 1] ? 0 : 1))
            }
            prev = row
        }
        return Double(prev[h.count]) / Double(r.count)
    }
}
