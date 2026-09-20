import Foundation

// Catalog row models (moved 2026-09-19 from LibraryManager.swift tail — the
// LibraryManager import actor is gone; macOS reads the live metadata.db
// read-only via SmbCatalogDB and these structs carry query results to SwiftUI).

public struct AuthorBook: Identifiable {
    public let id: Int64
    public let title: String
    public let author: String
    public let path: String
    public let coverHash: String
    public let timestamp: String
    public let root_folder: String
    public let authorSort: String
    public init(id: Int64, title: String, author: String, path: String, coverHash: String, timestamp: String, root_folder: String, authorSort: String = "") {
        self.id = id; self.title = title; self.author = author; self.path = path
        self.coverHash = coverHash; self.timestamp = timestamp; self.root_folder = root_folder
        self.authorSort = authorSort
    }
}

public struct AuthorSummary: Identifiable {
    public let id: Int64
    public let name: String
    public let sort: String
    public let bookCount: Int
    public let firstBookPath: String?

    public init(id: Int64, name: String, sort: String, bookCount: Int, firstBookPath: String?) {
        self.id = id; self.name = name; self.sort = sort
        self.bookCount = bookCount; self.firstBookPath = firstBookPath
    }
}

public struct SeriesSummary: Identifiable {
    public let id: Int64
    public let name: String
    public let bookCount: Int
    public let firstBookPath: String?
    public var author: String? = nil

    public init(id: Int64, name: String, bookCount: Int, firstBookPath: String?, author: String? = nil) {
        self.id = id; self.name = name; self.bookCount = bookCount
        self.firstBookPath = firstBookPath; self.author = author
    }
}

public struct TagSummary: Identifiable {
    public let id: Int64
    public let name: String
    public let bookCount: Int

    public init(id: Int64, name: String, bookCount: Int) {
        self.id = id; self.name = name; self.bookCount = bookCount
    }
}

public struct BookDetail {
    public let id: Int64
    public let title: String
    public let author: String
    public let series: String?
    public let seriesIndex: Float
    public let comments: String?
    public let tags: String?
    public let publisher: String?
    public let isbn: String?
    public let pubdate: String?
    public let timestamp: String
    public let authorSort: String
    public init(id: Int64, title: String, author: String, series: String?, seriesIndex: Float, comments: String?, tags: String?, publisher: String?, isbn: String?, pubdate: String?, timestamp: String, authorSort: String = "") {
        self.id = id; self.title = title; self.author = author; self.series = series
        self.seriesIndex = seriesIndex; self.comments = comments; self.tags = tags
        self.publisher = publisher; self.isbn = isbn; self.pubdate = pubdate
        self.timestamp = timestamp; self.authorSort = authorSort
    }
}

public struct SearchedBook: Identifiable {
    public let id: Int64
    public let title: String
    public let author: String
    public let path: String
    public let series: String?
    public let tags: [String]
    public let coverHash: String
    public let authorSort: String
    public init(id: Int64, title: String, author: String, path: String, series: String?, tags: [String], coverHash: String, authorSort: String = "") {
        self.id = id; self.title = title; self.author = author; self.path = path
        self.series = series; self.tags = tags; self.coverHash = coverHash
        self.authorSort = authorSort
    }
}
