// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "JocalaCatalogSwift",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "CatalogSwift", targets: ["CatalogSwiftApp"])
    ],
    targets: [
        .target(
            name: "CatalogCore",
            path: "Sources/CatalogCore",
            linkerSettings: [.linkedLibrary("sqlite3"), .linkedLibrary("z")]
        ),
        .executableTarget(
            name: "CatalogSwiftApp",
            dependencies: ["CatalogCore"],
            path: "Sources/CatalogSwiftApp",
            linkerSettings: [.linkedLibrary("sqlite3"), .linkedLibrary("z")]
        )
    ]
)
