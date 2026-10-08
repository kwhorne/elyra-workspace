// The main window of a process (the largest; its window number, for
// `screencapture -l`). Usage: swift winid.swift <pid>
import CoreGraphics
import Foundation

let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as! [[String: Any]]
var best: (area: Double, number: Any)? = nil
for window in windows where (window[kCGWindowOwnerPID as String] as? Int32) == pid
    && (window[kCGWindowLayer as String] as? Int) == 0
{
    let bounds = window[kCGWindowBounds as String] as! [String: Any]
    let area = (bounds["Width"] as? Double ?? 0) * (bounds["Height"] as? Double ?? 0)
    if area > 100 * 100 && area > (best?.area ?? 0) {
        best = (area, window[kCGWindowNumber as String]!)
    }
}
if let best { print(best.number) }
