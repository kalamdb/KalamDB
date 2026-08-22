import Flutter

public class KalamLinkPlugin: NSObject, FlutterPlugin {
    public static func register(with registrar: FlutterPluginRegistrar) {
        // No-op — the native code is loaded via FFI (ffiPlugin: true).
        // This class is only needed so CocoaPods/Xcode links the static library.
    }
}
