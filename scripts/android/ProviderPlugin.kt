// Hexis: the switch for the passkey provider. Installed by scripts/android_patch_manifest.py.
//
// Settings > Passkeys in Hexis calls three commands (src-tauri/src/commands/passkey_provider.rs):
//
//   status        { supported, enabled }: the phone has Android's Credential Manager (Android 14 or later), and the provider
//                 service is switched on
//   setEnabled    switch the provider service on or off (PackageManager.setComponentEnabledSetting; the manifest ships it
//                 switched off, so a phone where nobody asked for passkeys never lists Hexis as a provider)
//   openSettings  open the system's own passkey settings, where the person still has to choose Hexis themselves; an app
//                 cannot choose itself, and that is the point
//
// No permission is needed for any of it.

package __PROVIDER_PACKAGE__

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class ProviderSwitchArgs {
    var on: Boolean = false
}

@TauriPlugin
class ProviderPlugin(private val activity: Activity) : Plugin(activity) {
    // By name: naming the class would load it, and on a phone older than Android 14 its parent class does not exist.
    private fun service() = ComponentName(activity.packageName, SERVICE_CLASS)

    private fun supported(): Boolean =
        Build.VERSION.SDK_INT >= 34 && activity.packageManager.hasSystemFeature("android.software.credentials")

    private fun currentStatus(): JSObject {
        val ok = supported()
        val on = ok && activity.packageManager.getComponentEnabledSetting(service()) == PackageManager.COMPONENT_ENABLED_STATE_ENABLED
        return JSObject().put("supported", ok).put("enabled", on)
    }

    @Command
    fun status(invoke: Invoke) {
        invoke.resolve(currentStatus())
    }

    @Command
    fun setEnabled(invoke: Invoke) {
        val args = invoke.parseArgs(ProviderSwitchArgs::class.java)
        if (!supported()) {
            invoke.reject("This phone cannot use passkeys from an app. It needs Android 14 or later.")
            return
        }
        try {
            val state = if (args.on) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DISABLED
            activity.packageManager.setComponentEnabledSetting(service(), state, PackageManager.DONT_KILL_APP)
            invoke.resolve(currentStatus())
        } catch (e: Exception) {
            // A fixed sentence: nothing from the exception goes back to the page.
            invoke.reject("The passkey switch could not be changed.")
        }
    }

    @Command
    fun openSettings(invoke: Invoke) {
        if (!supported()) {
            invoke.reject("This phone cannot use passkeys from an app. It needs Android 14 or later.")
            return
        }
        // The system's own screen for choosing providers, then the settings entry for credential providers, then Settings.
        val opened = tryOpenFromCredentialManager() ||
            tryStart(Intent("android.settings.CREDENTIAL_PROVIDER").setData(Uri.parse("package:" + activity.packageName))) ||
            tryStart(Intent(android.provider.Settings.ACTION_SETTINGS))
        if (opened) invoke.resolve() else invoke.reject("The system settings could not be opened.")
    }

    private fun tryOpenFromCredentialManager(): Boolean {
        return try {
            androidx.credentials.CredentialManager.create(activity).createSettingsPendingIntent().send()
            true
        } catch (e: Exception) {
            false
        }
    }

    private fun tryStart(intent: Intent): Boolean {
        return try {
            activity.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        } catch (e: Exception) {
            false
        }
    }

    private companion object {
        const val SERVICE_CLASS = "__PROVIDER_PACKAGE__.HexisCredentialProviderService"
    }
}
