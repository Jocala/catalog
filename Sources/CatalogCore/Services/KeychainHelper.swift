import Foundation
import Security

public enum KeychainHelper {
    private static let service = "com.jocala.catalog.smb"
    // Shipping decision: SMB passwords live ONLY in the app's own settings
    // (UserDefaults mirror). The Keychain is never touched for passwords —
    // no SecItem calls here means no login-keychain prompt is possible.

    public static func save(password: String, for account: String) {
        UserDefaults.standard.set(password, forKey: "smb_pass_\(account)")
    }

    public static func read(account: String) -> String? {
        let ud = UserDefaults.standard.string(forKey: "smb_pass_\(account)")
        return (ud?.isEmpty == false) ? ud : nil
    }

    // MARK: - Generic Data Storage

    static func save(data: Data, for service: String, account: String = "main") {
        delete(service: service, account: account)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        ]
        SecItemAdd(query as CFDictionary, nil)
    }

    static func readData(service: String, account: String = "main") -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var result: AnyObject?
        SecItemCopyMatching(query as CFDictionary, &result)
        return result as? Data
    }

    static func delete(service: String, account: String = "main") {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        SecItemDelete(query as CFDictionary)
    }
}
