import Foundation

enum BookFormat: String, CaseIterable {
    case pdf, epub

    static let ebookFormats: Set<BookFormat> = [.epub]

    init?(ext: String) {
        self.init(rawValue: ext.lowercased())
    }
}

let supportedBookExts: Set<String> = Set(BookFormat.allCases.map { $0.rawValue })
