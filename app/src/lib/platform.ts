/** Running on Windows (WebView2)? Elsewhere (Linux and WSL: WebKitGTK) the app names
 *  paths, the trash and the key store the way that system does. Node (the tests) is not. */
export const isWindows = typeof navigator !== "undefined" && navigator.userAgent.includes("Windows");

/** A photo folder as this system spells one, for examples. */
export const examplePhotos = isWindows ? "D:/Photos" : "/home/me/Photos";
