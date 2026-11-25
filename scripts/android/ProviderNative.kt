// Hexis: the passkey provider's bridge to the Rust core. Installed by scripts/android_patch_manifest.py.
//
// Two JNI functions in libhexis_lib.so (src-tauri/src/provider/jni.rs) and nothing else: `call` takes an operation name and
// a JSON text and answers a JSON text. The private key of a passkey never crosses this line; what comes back is the signed
// answer for a website's request.

package __PROVIDER_PACKAGE__

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

object ProviderNative {
    init {
        System.loadLibrary("hexis_lib")
    }

    @JvmStatic
    external fun init(dataDir: String): String

    @JvmStatic
    external fun call(op: String, args: String): String
}

/** One answer of the core. */
class Reply(private val json: JSONObject) {
    val ok: Boolean get() = json.optBoolean("ok", false)
    val data: JSONObject get() = json.optJSONObject("data") ?: JSONObject()
    val code: String get() = json.optString("code", "internal")
    val message: String get() = json.optString("message", "That did not work. Try again.")
}

/** The state a sheet or the service needs before it can decide what to show. */
class VaultState(val appLocked: Boolean, val accounts: List<Pair<String, String>>, val selected: String?, val unlocked: Boolean) {
    val hasAccounts: Boolean get() = accounts.isNotEmpty()
}

/** Calls into the core. Every method blocks (Argon2id takes a second): call it from a background thread, never the UI thread. */
object Bridge {
    @Volatile private var started = false

    @Synchronized
    private fun start(context: Context) {
        if (started) return
        val answer = JSONObject(ProviderNative.init(context.applicationContext.dataDir.absolutePath))
        if (!answer.optBoolean("ok", false)) throw IllegalStateException(answer.optString("message", "The vault could not be opened."))
        started = true
    }

    fun call(context: Context, op: String, args: JSONObject = JSONObject()): Reply {
        return try {
            start(context)
            Reply(JSONObject(ProviderNative.call(op, args.toString())))
        } catch (e: Exception) {
            // A fixed sentence: nothing from the exception goes to the sheet.
            Reply(JSONObject().put("ok", false).put("code", "internal").put("message", "The passkey service could not start."))
        }
    }

    fun state(context: Context): VaultState {
        val r = call(context, "state")
        if (!r.ok) return VaultState(r.code == "app_locked", emptyList(), null, false)
        val list = r.data.optJSONArray("accounts") ?: JSONArray()
        val accounts = (0 until list.length()).map { i ->
            val a = list.getJSONObject(i)
            a.getString("id") to a.getString("name")
        }
        return VaultState(false, accounts, r.data.optString("selected").ifEmpty { null }, r.data.optBoolean("unlocked", false))
    }
}
