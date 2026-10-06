import Foundation
import CoreGraphics

// Synthetic input for record.sh, in screen points:
//   act click X Y | move X Y | type TEXT | key NAME | scroll X Y DY
// The pointer glides between points so the recording shows where it goes.
// Shortcuts with modifiers go through System Events instead (see record.sh).
let a = CommandLine.arguments
let src = CGEventSource(stateID: .hidSystemState)
func p(_ i: Int) -> CGPoint { CGPoint(x: Double(a[i])!, y: Double(a[i+1])!) }
func glide(to target: CGPoint) {
    let start = CGEvent(source: nil)!.location
    let steps = 14
    for s in 1...steps {
        let t = Double(s) / Double(steps)
        let e = t * t * (3 - 2 * t)
        let pt = CGPoint(x: start.x + (target.x - start.x) * e, y: start.y + (target.y - start.y) * e)
        CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: pt, mouseButton: .left)!.post(tap: .cghidEventTap)
        usleep(16000)
    }
}
let keys: [String: CGKeyCode] = ["return": 36, "tab": 48, "space": 49, "delete": 51, "escape": 53,
  "left": 123, "right": 124, "down": 125, "up": 126]
switch a[1] {
case "move": glide(to: p(2))
case "click":
    let pt = p(2); glide(to: pt); usleep(60000)
    for type in [CGEventType.leftMouseDown, .leftMouseUp] {
        CGEvent(mouseEventSource: src, mouseType: type, mouseCursorPosition: pt, mouseButton: .left)!.post(tap: .cghidEventTap)
        usleep(40000)
    }
case "type":
    for ch in a[2].utf16 {
        var c = ch
        for down in [true, false] {
            let e = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: down)!
            e.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c)
            e.post(tap: .cghidEventTap)
        }
        usleep(UInt32(Int.random(in: 35000...75000)))
    }
case "key":
    for down in [true, false] {
        CGEvent(keyboardEventSource: src, virtualKey: keys[a[2]]!, keyDown: down)!.post(tap: .cghidEventTap)
        usleep(30000)
    }
case "scroll":
    let pt = p(2); glide(to: pt)
    let total = Int(a[4])!; let n = 20
    for _ in 0..<n {
        CGEvent(scrollWheelEvent2Source: src, units: .pixel, wheelCount: 1, wheel1: Int32(total / n), wheel2: 0, wheel3: 0)!.post(tap: .cghidEventTap)
        usleep(16000)
    }
default: print("unknown")
}
