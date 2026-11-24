// Hexis: screen protection for the encrypted drive on Android.
//
// UNVERIFIED: written without an Android SDK or device, modelled on the Kotlin sources of
// tauri 2.12.7 (mobile/android .../plugin/Plugin.kt) and tauri-plugin-barcode-scanner 2.5.3
// (BarcodeScannerPlugin.kt). It has never been compiled or run.
//
// What it does: the `screen_secure` command (src-tauri/src/commands/drive.rs) calls
// `setSecure({ on })`. With `on = true` the window gets WindowManager.LayoutParams.FLAG_SECURE, so
// the system blocks screenshots, screen recording and the preview in the recent-apps list while a
// decrypted file is open. With `on = false` the flag is cleared again. No Android permission is
// needed for it.
//
// How it is wired:
//   * Rust: `commands::drive::secure_screen::init()` registers a Tauri plugin named
//     "secure-screen" whose setup calls
//     `api.register_android_plugin("dev.habibullah.hexis.securescreen", "SecureScreenPlugin")`;
//     lib.rs adds it with `builder.plugin(...)` under `#[cfg(target_os = "android")]`.
//   * Kotlin: this file must be compiled into the Android app module under the package below,
//     for example by copying it to
//     `src-tauri/gen/android/app/src/main/java/dev/habibullah/hexis/securescreen/SecureScreenPlugin.kt`
//     after `tauri android init` (the `gen/android` folder is generated and not kept in the
//     repository, which is why the source lives here).
//   * Nothing else changes: no manifest entry, no permission, no Gradle dependency beyond the
//     Tauri Android library every Tauri app already has.

package dev.habibullah.hexis.securescreen

import android.app.Activity
import android.view.WindowManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class SecureArgs {
    var on: Boolean = false
}

@TauriPlugin
class SecureScreenPlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun setSecure(invoke: Invoke) {
        val args = invoke.parseArgs(SecureArgs::class.java)
        // Window flags must be changed on the UI thread.
        activity.runOnUiThread {
            try {
                if (args.on) {
                    activity.window.setFlags(
                        WindowManager.LayoutParams.FLAG_SECURE,
                        WindowManager.LayoutParams.FLAG_SECURE
                    )
                } else {
                    activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                }
                invoke.resolve()
            } catch (e: Exception) {
                // A fixed sentence: nothing from the exception goes back to the page.
                invoke.reject("The screen protection could not be changed.")
            }
        }
    }
}
