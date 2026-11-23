// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "MiloSpeech",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "MiloSpeech", type: .static, targets: ["MiloSpeech"])
    ],
    dependencies: [
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.18.4", traits: [])
    ],
    targets: [
        .target(
            name: "MiloSpeech",
            dependencies: [.product(name: "FluidAudio", package: "FluidAudio")]
        )
    ]
)
