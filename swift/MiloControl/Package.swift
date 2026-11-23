// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "MiloControl",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "MiloControl", type: .static, targets: ["MiloControl"])
    ],
    targets: [
        .target(name: "MiloControl", swiftSettings: [.swiftLanguageMode(.v5)])
    ]
)
