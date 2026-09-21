import SwiftUI
import AppKit

// About dialog: product identity + donation appeal + PayPal banner.
// PayPal hosted-button URL is shared with adblink (same account).
extension Notification.Name {
    static let showAbout = Notification.Name("JocalaCatalogShowAbout")
}

/// Locates bundled read-only resources (Sources/CatalogSwiftApp/Resources).
// Same Bundle.module → main fallback as the donate banner.
enum CatalogHelp {
    static func resourceURL(name: String, ext: String) -> URL? {
        if let url = Bundle.module.url(forResource: name, withExtension: ext) {
            return url
        }
        return Bundle.main.url(forResource: name, withExtension: ext)
    }
}

struct AboutView: View {
    @Environment(\.openURL) private var openURL
    @Environment(\.dismiss) private var dismiss
    static let payPalURL = URL(string: "https://www.paypal.com/cgi-bin/webscr?cmd=_s-xclick&hosted_button_id=GKZMW456H6E5W")!
    static let siteURL = URL(string: "https://www.jocala.com")!

    var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    /// donatel.png ships in the target's processed Resources (Bundle.module,
    /// falling back to main bundle when run outside SPM layout).
    static func donateImage() -> NSImage? {
        if let url = Bundle.module.url(forResource: "donatel", withExtension: "png"),
           let img = NSImage(contentsOf: url) {
            return img
        }
        if let url = Bundle.main.url(forResource: "donatel", withExtension: "png"),
           let img = NSImage(contentsOf: url) {
            return img
        }
        return nil
    }

    var body: some View {
        VStack(spacing: 12) {
            Text("Jocala Catalog").font(.title2).bold()
            Text(version.isEmpty ? "" : "Version \(version)")
                .font(.caption).foregroundStyle(.secondary)
            Link("jocala.com", destination: Self.siteURL)
                .font(.body)
            Text("Donations defray server costs and fund development.")
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            if let img = Self.donateImage() {
                Button {
                    openURL(Self.payPalURL)
                } label: {
                    Image(nsImage: img)
                }
                .buttonStyle(.plain)
                .help("Donate via PayPal")
                .onHover { inside in
                    if inside {
                        NSCursor.pointingHand.set()
                    } else {
                        NSCursor.arrow.set()
                    }
                }
            } else {
                Button("Donate") { openURL(Self.payPalURL) }
                    .buttonStyle(.link)
            }
            Button("Close") { dismiss() }
                .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 24).padding(.vertical, 16)
        .frame(width: 340)
    }
}
