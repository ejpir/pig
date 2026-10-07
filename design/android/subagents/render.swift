// Renders the subagent mocks to PNG with the system's WebKit, then composes
// overview.png. macOS only; no browser to install and no network calls.
//
//   swift design/android/subagents/render.swift [screen-name ...]
import AppKit
import WebKit

let here = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let screens = here.appendingPathComponent("screens")
let fonts = here.appendingPathComponent("../../../assets/fonts").standardized
let (width, height, scale) = (412.0, 915.0, 2.0)

/// Loads a page offscreen, waits for its fonts and scripts, and saves it at 2x.
final class Shot: NSObject, WKNavigationDelegate {
  let view = WKWebView(frame: NSRect(x: 0, y: 0, width: width, height: height))
  var done = false

  func render(_ page: URL) -> URL {
    done = false
    view.navigationDelegate = self
    view.loadFileURL(page, allowingReadAccessTo: here.appendingPathComponent("../../..").standardized)
    while !done { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))
    let out = page.deletingPathExtension().appendingPathExtension("png")
    let config = WKSnapshotConfiguration()
    config.snapshotWidth = NSNumber(value: width * scale / NSScreen.main!.backingScaleFactor)
    var finished = false
    view.takeSnapshot(with: config) { image, error in
      guard let image, let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
            let png = bitmap.representation(using: .png, properties: [:]) else {
        fatalError("Could not snapshot \(page.lastPathComponent): \(String(describing: error))")
      }
      try! png.write(to: out)
      finished = true
    }
    while !finished { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
    return out
  }

  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { done = true }
}

/// The screens side by side under their titles, as the other studies' overviews.
func overview(_ pngs: [URL]) -> URL {
  CTFontManagerRegisterFontsForURL(fonts.appendingPathComponent("IBMPlexSans-SemiBold.ttf") as CFURL, .process, nil)
  let (columns, gap, caption) = (4.0, 40.0, 44.0)
  let rows = ceil(Double(pngs.count) / columns)
  let size = NSSize(width: columns * width + (columns + 1) * gap, height: rows * (height + caption) + (rows + 1) * gap)
  // At 1x: the overview is for a glance, and the screens have the detail.
  let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width), pixelsHigh: Int(size.height),
                                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
  NSColor(red: 0xec / 255, green: 0xe7 / 255, blue: 0xe1 / 255, alpha: 1).setFill()
  NSRect(origin: .zero, size: size).fill()
  let font = NSFont(name: "IBMPlexSans-SemiBold", size: 20) ?? .boldSystemFont(ofSize: 20)
  for (index, png) in pngs.enumerated() {
    let x = gap + Double(index % Int(columns)) * (width + gap)
    // AppKit's origin is the bottom left: `top` is the caption's top edge.
    let top = size.height - gap - Double(index / Int(columns)) * (height + caption + gap)
    let html = try! String(contentsOf: png.deletingPathExtension().appendingPathExtension("html"), encoding: .utf8)
    let title = html.range(of: "<title>(.*?)</title>", options: .regularExpression)
      .map { String(html[$0]).replacingOccurrences(of: "<title>", with: "").replacingOccurrences(of: "</title>", with: "") } ?? ""
    NSAttributedString(string: title, attributes: [.font: font, .foregroundColor: NSColor(red: 0x25 / 255, green: 0x2f / 255, blue: 0x3d / 255, alpha: 1)]).draw(at: NSPoint(x: x + 4, y: top - 26))
    let frame = NSRect(x: x, y: top - caption - height, width: width, height: height)
    NSGraphicsContext.saveGraphicsState()
    NSBezierPath(roundedRect: frame, xRadius: 36, yRadius: 36).addClip()
    NSImage(contentsOf: png)!.draw(in: frame, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: false, hints: nil)
    NSGraphicsContext.restoreGraphicsState()
    NSColor(red: 0xcb / 255, green: 0xc3 / 255, blue: 0xbb / 255, alpha: 1).setStroke()
    NSBezierPath(roundedRect: frame.insetBy(dx: -0.5, dy: -0.5), xRadius: 36.5, yRadius: 36.5).stroke()
  }
  NSGraphicsContext.restoreGraphicsState()
  let out = here.appendingPathComponent("overview.png")
  try! bitmap.representation(using: .png, properties: [:])!.write(to: out)
  return out
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let wanted = Set(CommandLine.arguments.dropFirst())
let pages = try! FileManager.default.contentsOfDirectory(at: screens, includingPropertiesForKeys: nil)
  .filter { $0.pathExtension == "html" && $0.lastPathComponent.first!.isNumber }
  .sorted { $0.lastPathComponent < $1.lastPathComponent }
let shot = Shot()
for page in pages where wanted.isEmpty || wanted.contains(page.deletingPathExtension().lastPathComponent) {
  print("screens/" + shot.render(page).lastPathComponent)
}
if wanted.isEmpty {
  print(overview(pages.map { $0.deletingPathExtension().appendingPathExtension("png") }).lastPathComponent)
}
