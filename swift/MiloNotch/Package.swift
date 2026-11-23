// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "MiloNotch",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "MiloNotch", type: .static, targets: ["MiloNotch"])
    ],
    targets: [
        .target(name: "MiloNotch")
    ]
)
