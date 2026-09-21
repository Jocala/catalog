import SwiftUI
import WebKit
import AppKit

// Internal help viewer: bundled help.html in a WKWebView. Same page ships
// in the Windows app. Web links (jocala.com) open in the default browser;
// file-local navigation (anchors) stays in the viewer.
extension Notification.Name {
    static let showHelp = Notification.Name("JocalaCatalogShowHelp")
    /// Posted by the help WKWebView when the page's Close link
    /// (`catalog:close`) is clicked — HelpView dismisses itself.
    /// Same scheme is intercepted by the Windows WebView2 viewer.
    static let closeHelp = Notification.Name("JocalaCatalogCloseHelp")
}

struct HelpView: View {
    @AppStorage("theme_preference") private var themePreference = 0
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(spacing: 0) {
            // WKWebView resolves prefers-color-scheme from its own appearance,
            // not the SwiftUI color scheme — drive it from the same setting.
            // 2 = dark, 1 = light, anything else follows the system (nil).
            HelpWebView(appearance: themePreference == 2
                ? NSAppearance(named: .darkAqua)
                : themePreference == 1
                ? NSAppearance(named: .aqua) : nil)
            Divider()
            HStack {
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .frame(width: 780, height: 700)
        .onReceive(NotificationCenter.default.publisher(for: .closeHelp)) { _ in
            dismiss()
        }
    }
}

struct HelpWebView: NSViewRepresentable {
    let appearance: NSAppearance?
    func makeNSView(context: Context) -> WKWebView {
        let wv = WKWebView()
        wv.navigationDelegate = context.coordinator
        wv.appearance = appearance
        if let url = CatalogHelp.resourceURL(name: "help", ext: "html") {
            wv.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        }
        return wv
    }

    func updateNSView(_ nsView: WKWebView, context: Context) {
        nsView.appearance = appearance
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    class Coordinator: NSObject, WKNavigationDelegate {
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction) async -> WKNavigationActionPolicy {
            if let url = navigationAction.request.url,
               let scheme = url.scheme?.lowercased() {
                // In-page Close link (help.html header/footer).
                if scheme == "catalog" {
                    NotificationCenter.default.post(name: .closeHelp, object: nil)
                    return .cancel
                }
                if scheme == "http" || scheme == "https" {
                    NSWorkspace.shared.open(url)
                    return .cancel
                }
            }
            return .allow
        }
    }
}
