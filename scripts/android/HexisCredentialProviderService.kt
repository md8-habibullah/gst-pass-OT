// Hexis: the passkey provider the system asks (Android 14 and later). Installed by scripts/android_patch_manifest.py.
//
// The system asks two things: which passkeys exist for this website, and where a new passkey can be saved. This service
// answers with entries and a PendingIntent for each; it never holds a key and returns no secret. Choosing an entry opens
// PasskeyActivity, which unlocks, asks for a fingerprint or screen lock and lets the core sign or make the passkey.
//
// The service is switched off in the manifest. Settings > Passkeys in Hexis switches it on, and only then can the system
// list Hexis as a passkey provider.

package __PROVIDER_PACKAGE__

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.os.CancellationSignal
import android.os.OutcomeReceiver
import androidx.annotation.RequiresApi
import __PACKAGE__.R
import androidx.credentials.exceptions.ClearCredentialException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.provider.AuthenticationAction
import androidx.credentials.provider.BeginCreateCredentialRequest
import androidx.credentials.provider.BeginCreateCredentialResponse
import androidx.credentials.provider.BeginCreatePublicKeyCredentialRequest
import androidx.credentials.provider.BeginGetCredentialRequest
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.BeginGetPublicKeyCredentialOption
import androidx.credentials.provider.CallingAppInfo
import androidx.credentials.provider.CreateEntry
import androidx.credentials.provider.CredentialProviderService
import androidx.credentials.provider.ProviderClearCredentialStateRequest
import androidx.credentials.provider.PublicKeyCredentialEntry
import org.json.JSONObject
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/** What is shared by the service and by the activity that answers "unlock Hexis". */
internal object ProviderEntries {
    // Seeded from the clock: a restarted process must not reuse the request codes of PendingIntents the system still holds (FLAG_UPDATE_CURRENT would overwrite their extras).
    private val counter = AtomicInteger((System.currentTimeMillis() / 1000).toInt() and 0x3fffffff)
    private const val NO_HASH = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" // 32 zero bytes: only the final request carries the real hash

    fun nextCode(): Int = counter.getAndIncrement()

    fun activityIntent(context: Context, mode: String): Intent =
        Intent(context, PasskeyActivity::class.java).setPackage(context.packageName).putExtra(PasskeyActivity.EXTRA_MODE, mode)

    /** Each entry gets its own request code: the system keeps a PendingIntent per code, and a shared one would show the wrong passkey. */
    fun pending(context: Context, intent: Intent): PendingIntent =
        PendingIntent.getActivity(context, nextCode(), intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    fun icon(context: Context): Icon = Icon.createWithResource(context, context.applicationInfo.icon)

    private fun hexOf(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }

    /** The list of browsers the system may trust to name a page's origin: Google's published list, shipped as a snapshot. */
    private fun allowlist(context: Context): String {
        return context.resources.openRawResource(R.raw.hexis_privileged_apps).bufferedReader().use { it.readText() }
    }

    /**
     * Who is asking, in the form the core takes. A browser on Google's list names the page's origin and brings the hash of
     * the client data it built; any other app is known by its package and the SHA-256 of its signing certificate, and the
     * core asks the website whether that app may use its passkeys. `null` when the caller cannot be identified.
     */
    @RequiresApi(34)
    fun caller(context: Context, info: CallingAppInfo, clientDataHash: ByteArray?): JSONObject? {
        val origin = try {
            info.getOrigin(allowlist(context))
        } catch (e: Exception) {
            null
        }
        if (origin != null) {
            val hash = if (clientDataHash != null) android.util.Base64.encodeToString(clientDataHash, android.util.Base64.URL_SAFE or android.util.Base64.NO_PADDING or android.util.Base64.NO_WRAP) else NO_HASH
            return JSONObject().put("kind", "browser").put("origin", origin).put("clientDataHash", hash)
        }
        val signers = try {
            info.signingInfo.apkContentsSigners
        } catch (e: Exception) {
            null
        }
        // An app signed by several certificates cannot be matched against the website's list: refuse it.
        if (signers == null || signers.size != 1) return null
        val digest = MessageDigest.getInstance("SHA-256").digest(signers[0].toByteArray())
        val cert = android.util.Base64.encodeToString(digest, android.util.Base64.URL_SAFE or android.util.Base64.NO_PADDING or android.util.Base64.NO_WRAP)
        return JSONObject().put("kind", "app").put("package", info.packageName).put("certSha256", cert)
    }

    /** The answer to "which passkeys do you have": one entry per passkey, or the one action that unlocks first. */
    @RequiresApi(34)
    fun getResponse(context: Context, request: BeginGetCredentialRequest): BeginGetCredentialResponse {
        val options = request.beginGetCredentialOptions.filterIsInstance<BeginGetPublicKeyCredentialOption>()
        val builder = BeginGetCredentialResponse.Builder()
        if (options.isEmpty()) return builder.build()
        val state = Bridge.state(context)
        if (state.appLocked) {
            // The app lock is on and closed: say so, and reveal nothing about the vault.
            return builder.setAuthenticationActions(listOf(AuthenticationAction(context.getString(R.string.hexis_passkey_open_app), pending(context, activityIntent(context, PasskeyActivity.MODE_APP_LOCKED))))).build()
        }
        if (!state.hasAccounts) return builder.build()
        if (!state.unlocked) {
            // A locked vault must not reveal which websites have passkeys: one action, and the passkeys appear after the unlock.
            return builder.setAuthenticationActions(listOf(AuthenticationAction(context.getString(R.string.hexis_passkey_unlock), pending(context, activityIntent(context, PasskeyActivity.MODE_UNLOCK)))))
                .build()
        }
        val caller = caller(context, request.callingAppInfo ?: return builder.build(), null) ?: return builder.build()
        val entries = ArrayList<androidx.credentials.provider.CredentialEntry>()
        for (option in options) {
            val found = Bridge.call(context, "getOptions", JSONObject().put("request", option.requestJson).put("caller", caller))
            if (!found.ok) continue
            val matches = found.data.optJSONArray("matches") ?: continue
            for (i in 0 until matches.length()) {
                val m = matches.getJSONObject(i)
                val user = m.optString("userName").ifEmpty { m.optString("displayName") }.ifEmpty { m.optString("title") }
                val intent = activityIntent(context, PasskeyActivity.MODE_GET)
                    .putExtra(PasskeyActivity.EXTRA_ITEM_ID, m.getString("itemId"))
                    .putExtra(PasskeyActivity.EXTRA_KEY, m.getString("key"))
                    .putExtra(PasskeyActivity.EXTRA_RP, m.optString("rpName").ifEmpty { m.optString("rpId") })
                    .putExtra(PasskeyActivity.EXTRA_USER, user)
                entries.add(
                    PublicKeyCredentialEntry.Builder(context, user, pending(context, intent), option)
                        .setDisplayName(m.optString("title").ifEmpty { null })
                        .setIcon(icon(context))
                        .build()
                )
            }
        }
        return builder.setCredentialEntries(entries).build()
    }
}

@RequiresApi(34)
@SuppressLint("NewApi")
class HexisCredentialProviderService : CredentialProviderService() {
    private val worker = Executors.newSingleThreadExecutor()

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }

    override fun onBeginGetCredentialRequest(
        request: BeginGetCredentialRequest,
        cancellationSignal: CancellationSignal,
        callback: OutcomeReceiver<BeginGetCredentialResponse, GetCredentialException>,
    ) {
        // Argon2id and the website lookups can take a moment: never on the thread the system calls us on.
        worker.execute {
            if (cancellationSignal.isCanceled) return@execute
            try {
                callback.onResult(ProviderEntries.getResponse(this, request))
            } catch (e: Exception) {
                callback.onResult(BeginGetCredentialResponse.Builder().build())
            }
        }
    }

    override fun onBeginCreateCredentialRequest(
        request: BeginCreateCredentialRequest,
        cancellationSignal: CancellationSignal,
        callback: OutcomeReceiver<BeginCreateCredentialResponse, CreateCredentialException>,
    ) {
        worker.execute {
            if (cancellationSignal.isCanceled) return@execute
            val builder = BeginCreateCredentialResponse.Builder()
            try {
                if (request is BeginCreatePublicKeyCredentialRequest) {
                    val rp = try { JSONObject(request.requestJson).getJSONObject("rp").optString("name").ifEmpty { JSONObject(request.requestJson).getJSONObject("rp").optString("id") } } catch (e: Exception) { "" }
                    val label = getString(R.string.hexis_passkey_save)
                    val intent = ProviderEntries.activityIntent(this, PasskeyActivity.MODE_CREATE)
                    val entry = CreateEntry.Builder(label, ProviderEntries.pending(this, intent))
                        .setDescription(rp.ifEmpty { null })
                        .setIcon(ProviderEntries.icon(this))
                        .build()
                    builder.addCreateEntry(entry)
                }
            } catch (e: Exception) {
                // an empty answer: the system shows no Hexis entry
            }
            callback.onResult(builder.build())
        }
    }

    override fun onClearCredentialStateRequest(
        request: ProviderClearCredentialStateRequest,
        cancellationSignal: CancellationSignal,
        callback: OutcomeReceiver<Void?, ClearCredentialException>,
    ) {
        // Nothing is cached here: the vault's own lock decides.
        callback.onResult(null)
    }
}
