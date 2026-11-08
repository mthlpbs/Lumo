Microsoft WebView2Loader.dll 1.0.2592.51, Windows x64, unmodified.

Source: the `build/native/x64/WebView2Loader.dll` entry in the official
Microsoft.Web.WebView2 1.0.2592.51 NuGet package. The bytes also match the
loader in webview2-com-sys 0.33.0. Microsoft's accompanying license is in
LICENSE.txt and reproduced in THIRD-PARTY-NOTICES.txt.

This 165,336-byte library locates the separately installed Evergreen WebView2
Runtime. It is not a browser engine. Lumo loads it by absolute path only when
web search is explicitly opened. No WebView2 DLL is statically imported at
Lumo startup. Both installers and the portable ZIP include the loader.
