import SwiftUI
import AppKit
import CatalogCore

struct SearchResult: Identifiable {
    let filePath: String
    let fileName: String
    let bookId: Int64?
    let title: String?
    let author: String?
    let series: String?
    let tags: [String]
    let authorSort: String
    var id: String { filePath }
    init(filePath: String, fileName: String, bookId: Int64?, title: String?, author: String?, series: String?, tags: [String], authorSort: String = "") {
        self.filePath = filePath; self.fileName = fileName; self.bookId = bookId
        self.title = title; self.author = author; self.series = series
        self.tags = tags; self.authorSort = authorSort
    }
}

struct SearchFormSheet: View {
    @Environment(\.dismiss) private var dismiss

    @State private var searchQuery = ""
    @State private var searchTitle = ""
    @State private var searchAuthor = ""
    @State private var searchSeries = ""
    @State private var searchTag = ""
    @State private var availableTags: [TagSummary] = []
    @State private var availableSeries: [SeriesSummary] = []
    @State private var expandSeriesOnTag = true
    @State private var isSearching = false
    @State private var dbError: String? = nil

    let onSearch: ([SearchResult]) -> Void
    var onSearchSeries: (([SeriesSummary], String) -> Void)? = nil

    private var hasAnyField: Bool {
        !searchQuery.isEmpty || !searchTitle.isEmpty || !searchAuthor.isEmpty || !searchSeries.isEmpty || !searchTag.isEmpty
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Group {
                    if isSearching {
                        VStack {
                            Spacer()
                            ProgressView("Searching...")
                            Spacer()
                        }
                    } else {
                        searchForm
                    }
                }
                if !isSearching {
                    Divider()
                    HStack(spacing: 12) {
                        Button("Cancel") { dismiss() }
                            .keyboardShortcut(.cancelAction)
                        Spacer()
                        Button("Search") { performSearch() }
                            .buttonStyle(.borderedProminent)
                            .disabled(!hasAnyField)
                            .keyboardShortcut(.defaultAction)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .background(Color(nsColor: .windowBackgroundColor))
                }
            }
            .navigationTitle("Search Books")
        }
        .frame(width: 640)
        .fixedSize(horizontal: false, vertical: true)
        .alert("Calibre Database", isPresented: Binding(get: { dbError != nil }, set: { if !$0 { dbError = nil } })) {
            Button("OK", role: .cancel) { dbError = nil }
        } message: { Text(dbError ?? "") }
    }

    private var searchForm: some View {
        VStack(spacing: 12) {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .foregroundColor(.gray)
                    TextField("Search all fields (e.g. james bond, author:asimov, tag:espionage)", text: $searchQuery)
                        .textFieldStyle(.plain)
                        .autocorrectionDisabled()
                        .onSubmit { performSearch() }
                    if !searchQuery.isEmpty {
                        Button { searchQuery = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundColor(.gray)
                        }.buttonStyle(.plain)
                    }
                }
                .padding(10)
                .background(Color(nsColor: NSColor.controlBackgroundColor))
                .cornerRadius(10)
                .padding(.horizontal)

                Divider()

                formField("Title", text: $searchTitle)
                formField("Author", text: $searchAuthor)
                HStack(spacing: 8) {
                    Text("Series").foregroundColor(.primary).fontWeight(.bold).frame(width: 70, alignment: .leading)
                    ScrollableDropdown(
                        selection: $searchSeries,
                        options: [DropdownItem(value: "", label: "Any")] + availableSeries.map { DropdownItem(value: $0.name, label: "\($0.name) (\($0.bookCount))") }
                    )
                    .disabled(availableSeries.isEmpty)
                    Spacer()
                }.padding(.horizontal)
                HStack(spacing: 8) {
                    Text("Tag").foregroundColor(.primary).fontWeight(.bold).frame(width: 70, alignment: .leading)
                    ScrollableDropdown(
                        selection: $searchTag,
                        options: [DropdownItem(value: "", label: "Any")] + availableTags.map { DropdownItem(value: $0.name, label: "\($0.name) (\($0.bookCount))") }
                    )
                    .disabled(availableTags.isEmpty)
                    Spacer()
                }.padding(.horizontal)
                HStack(spacing: 8) {
                    Toggle("", isOn: $expandSeriesOnTag)
                        .toggleStyle(.switch)
                        .labelsHidden()
                    Text("Select series if any book matches tag")
                        .font(.caption)
                    Spacer()
                }.padding(.horizontal)

            }
            .padding(.top, 12)
            .padding(.bottom, 12)
        .task {
            // Single-database rule: tags/series come from the live SMB Calibre DB only.
            ReaderLog.shared.i("SearchForm", "task start loading tags/series")
            do {
                availableTags = try await SmbCatalogDB.shared.allTags()
                availableSeries = try await SmbCatalogDB.shared.allSeries()
                ReaderLog.shared.i("SearchForm", "task loaded tags=\(availableTags.count) series=\(availableSeries.count)")
            } catch {
                ReaderLog.shared.e("SearchForm", "tags/series failed \(error.localizedDescription)")
                dbError = error.localizedDescription
            }
        }
        .onChange(of: searchTag) { _, new in
            Task {
                do {
                    if let tag = availableTags.first(where: { $0.name == new }) {
                        availableSeries = try await SmbCatalogDB.shared.seriesByTag(tagId: tag.id)
                    } else {
                        availableSeries = try await SmbCatalogDB.shared.allSeries()
                    }
                } catch {
                    dbError = error.localizedDescription
                }
            }
        }
    }

    private func formField(_ label: String, text: Binding<String>) -> some View {
        HStack(spacing: 8) {
            Text(label)
                .foregroundColor(.primary)
                .fontWeight(.bold)
                .frame(width: 70, alignment: .leading)
            TextField("", text: text)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
        }
        .padding(.horizontal)
    }

    private func performSearch() {
        guard hasAnyField else { return }
        isSearching = true
        let tagExpand = expandSeriesOnTag && !searchTag.isEmpty && searchSeries.isEmpty && searchTitle.isEmpty && searchAuthor.isEmpty && searchQuery.isEmpty
        Task {
            do {
                let q = searchQuery.trimmingCharacters(in: .whitespaces)
                if tagExpand {
                    let tags = try await SmbCatalogDB.shared.allTags()
                    if let tag = tags.first(where: { $0.name.lowercased() == searchTag.lowercased() }) {
                        let seriesList = try await SmbCatalogDB.shared.seriesByTag(tagId: tag.id)
                        if let handler = onSearchSeries {
                            await MainActor.run { handler(seriesList, tag.name); dismiss() }
                        } else {
                            var expanded: [SearchedBook] = []
                            for s in seriesList {
                                let books = try await SmbCatalogDB.shared.booksBySeries(id: s.id)
                                for b in books { expanded.append(SearchedBook(id: b.id, title: b.title, author: b.author, path: b.path, series: s.name, tags: [], coverHash: b.coverHash, authorSort: b.authorSort)) }
                            }
                            var combined: [SearchResult] = []
                            for sb in expanded { combined.append(SearchResult(filePath: sb.path, fileName: URL(fileURLWithPath: sb.path).lastPathComponent, bookId: sb.id, title: sb.title, author: sb.author, series: sb.series, tags: sb.tags, authorSort: sb.authorSort)) }
                            await MainActor.run { onSearch(combined); dismiss() }
                        }
                        return
                    }
                }
                let searchedBooks = try await SmbCatalogDB.shared.searchBooks(
                    query: q,
                    title: searchTitle,
                    author: searchAuthor,
                    series: searchSeries,
                    tag: searchTag,
                    sortDescending: false
                )
                var combined: [SearchResult] = []
                for sb in searchedBooks {
                    let fileName = URL(fileURLWithPath: sb.path).lastPathComponent
                    combined.append(SearchResult(
                        filePath: sb.path,
                        fileName: fileName,
                        bookId: sb.id,
                        title: sb.title,
                        author: sb.author,
                        series: sb.series,
                        tags: sb.tags,
                        authorSort: sb.authorSort
                    ))
                }
                await MainActor.run {
                    onSearch(combined)
                    dismiss()
                }
            } catch {
                await MainActor.run {
                    isSearching = false
                    dbError = error.localizedDescription
                }
            }
        }
    }
}

struct DropdownItem: Hashable {
    let value: String
    let label: String
}

/// Button + popover list capped at 12 visible rows, scrolls beyond that.
/// Replaces Picker(.menu), whose NSMenu grows unbounded and runs off-screen
/// with hundreds of tags/series.
struct ScrollableDropdown: View {
    static let maxVisibleRows = 12
    static let rowHeight: CGFloat = 28

    @Binding var selection: String
    let options: [DropdownItem]
    @State private var isOpen = false

    private var currentLabel: String {
        options.first(where: { $0.value == selection })?.label ?? "Any"
    }

    var body: some View {
        Button {
            isOpen.toggle()
        } label: {
            HStack {
                Text(currentLabel)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .foregroundColor(.primary)
                Spacer(minLength: 4)
                Image(systemName: isOpen ? "chevron.up" : "chevron.down")
                    .foregroundColor(.secondary)
                    .font(.caption)
            }
            .padding(.horizontal, 8)
            .frame(width: 228, height: 26, alignment: .leading)
            .background(Color(nsColor: .controlBackgroundColor))
            .cornerRadius(6)
        }
        .buttonStyle(.plain)
        .popover(isPresented: $isOpen, arrowEdge: .bottom) {
            VStack(spacing: 0) {
                ScrollView(.vertical) {
                    LazyVStack(spacing: 0) {
                        ForEach(options, id: \.value) { opt in
                            Button {
                                selection = opt.value
                                isOpen = false
                            } label: {
                                HStack {
                                    Text(opt.label)
                                        .lineLimit(1)
                                        .truncationMode(.tail)
                                        .foregroundColor(.primary)
                                    Spacer()
                                    if opt.value == selection {
                                        Image(systemName: "checkmark")
                                            .foregroundColor(.accentColor)
                                    }
                                }
                                .padding(.horizontal, 10)
                                .frame(height: Self.rowHeight, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .background(opt.value == selection ? Color.accentColor.opacity(0.15) : Color.clear)
                        }
                    }
                }
                .frame(height: min(CGFloat(max(options.count, 1)), CGFloat(Self.maxVisibleRows)) * Self.rowHeight)
                .scrollIndicators(options.count > Self.maxVisibleRows ? .visible : .automatic, axes: .vertical)
            }
            .frame(width: 280)
            .padding(.vertical, 4)
        }
    }
}
