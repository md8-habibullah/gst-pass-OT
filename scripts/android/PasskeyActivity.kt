// Hexis: the sheet that answers a passkey request. Installed by scripts/android_patch_manifest.py.
//
// It opens when a person taps a Hexis entry in the system's passkey sheet. It is not exported: only the PendingIntents the
// provider service made can start it. What it does:
//
//   unlock       the vault is locked: ask for the master password (a fingerprint never opens a vault), then hand the system the
//                passkeys that fit. This is what the "Unlock" action in the system sheet leads to.
//   get          sign in: a fresh fingerprint or screen lock, then the core signs the website's request.
//   create       save a new passkey: pick the login it belongs on (the ones that fit are listed first) or make a new one, then
//                a fresh fingerprint or screen lock, then the core makes and stores the passkey BEFORE the answer is returned.
//   app locked   the app lock is closed: say so; nothing about the vault is shown.
//
// Rules kept here (the core enforces the rest): the screen cannot be captured (FLAG_SECURE), touches while another window
// covers it are ignored, the master password is wiped from its field as soon as it is sent, a locked vault is opened with
// the master password only, and every sign-in or new passkey on an open vault needs a fresh biometric or screen-lock check
// unless the master password was typed in this very sheet.

package __PROVIDER_PACKAGE__

import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.GetCredentialResponse
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialUnknownException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.provider.PendingIntentHandler
import com.google.android.material.button.MaterialButton
import com.google.android.material.color.MaterialColors
import __PACKAGE__.R
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors

class PasskeyActivity : AppCompatActivity() {
    companion object {
        const val EXTRA_MODE = "hexis.mode"
        const val EXTRA_ITEM_ID = "hexis.itemId"
        const val EXTRA_KEY = "hexis.key"
        const val EXTRA_RP = "hexis.rp"
        const val EXTRA_USER = "hexis.user"
        const val MODE_GET = "get"
        const val MODE_CREATE = "create"
        const val MODE_UNLOCK = "unlock"
        const val MODE_APP_LOCKED = "appLocked"
    }

    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var content: LinearLayout
    private var mode = ""
    /** The master password was typed in this sheet: that is the user's verification for this request. */
    private var viaPassword = false
    private var finished = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // No screenshots, no screen recording, no preview in the recent apps; and a window drawn over this one swallows touches.
        window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        setFinishOnTouchOutside(false)
        mode = intent.getStringExtra(EXTRA_MODE) ?: ""
        val scroll = ScrollView(this)
        content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(22), dp(24), dp(20))
            filterTouchesWhenObscured = true
        }
        scroll.addView(content, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        scroll.filterTouchesWhenObscured = true
        setContentView(scroll)
        if (mode == MODE_APP_LOCKED) showAppLocked() else begin()
    }

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }

    @Deprecated("Back cancels the request")
    override fun onBackPressed() {
        cancel()
    }

    // ------------------------------------------------------------------ small view helpers

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    private fun title(text: String) {
        content.addView(TextView(this).apply {
            this.text = text
            setTextAppearance(com.google.android.material.R.style.TextAppearance_Material3_TitleLarge)
        })
    }

    private fun body(text: String, error: Boolean = false): TextView {
        val view = TextView(this).apply {
            this.text = text
            setTextAppearance(com.google.android.material.R.style.TextAppearance_Material3_BodyMedium)
            setPadding(0, dp(8), 0, 0)
            if (error) setTextColor(MaterialColors.getColor(this@PasskeyActivity, com.google.android.material.R.attr.colorError, 0xFFB3261E.toInt()))
        }
        content.addView(view)
        return view
    }

    private fun button(text: String, outlined: Boolean = false, onClick: () -> Unit): MaterialButton {
        val b = if (outlined) MaterialButton(this, null, com.google.android.material.R.attr.materialButtonOutlinedStyle) else MaterialButton(this)
        b.text = text
        b.isAllCaps = false
        b.setOnClickListener { onClick() }
        val lp = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        lp.topMargin = dp(10)
        content.addView(b, lp)
        return b
    }

    private fun reset() {
        content.removeAllViews()
    }

    private fun onMain(block: () -> Unit) {
        if (!isDestroyed) runOnUiThread(block)
    }

    // ------------------------------------------------------------------ results

    private fun cancel() {
        if (finished) return
        finished = true
        val result = Intent()
        when (mode) {
            MODE_CREATE -> PendingIntentHandler.setCreateCredentialException(result, CreateCredentialCancellationException())
            MODE_GET -> PendingIntentHandler.setGetCredentialException(result, GetCredentialCancellationException())
            else -> {}
        }
        setResult(RESULT_OK, result)
        finish()
    }

    private fun fail(message: String) {
        onMain {
            reset()
            title(getString(R.string.hexis_passkey_failed))
            body(message)
            button(getString(R.string.hexis_passkey_close)) { cancel() }
        }
    }

    // ------------------------------------------------------------------ flow

    private fun begin() {
        worker.execute {
            val state = Bridge.state(this)
            when {
                state.appLocked -> onMain { showAppLocked() }
                !state.hasAccounts -> fail(getString(R.string.hexis_passkey_no_vault))
                !state.unlocked -> onMain { showUnlock(state) }
                else -> proceed()
            }
        }
    }

    private fun showAppLocked() {
        reset()
        title(getString(R.string.hexis_passkey_app_locked_title))
        body(getString(R.string.hexis_passkey_app_locked_text))
        button(getString(R.string.hexis_passkey_open_app_button)) {
            val launch = packageManager.getLaunchIntentForPackage(packageName)
            if (launch != null) startActivity(launch)
            cancel()
        }
        button(getString(R.string.hexis_passkey_close), outlined = true) { cancel() }
    }

    /** Ask for the master password. `onDone` runs on a worker thread once the vault is open. */
    private fun showUnlock(state: VaultState, note: String? = null) {
        reset()
        title(getString(R.string.hexis_passkey_unlock_title))
        body(getString(R.string.hexis_passkey_unlock_text))
        val account = state.selected ?: state.accounts.firstOrNull()?.first
        if (state.accounts.size > 1) {
            // Which account's vault to open: the one chosen now is filled in, the others are outlined.
            state.accounts.forEach { (id, name) ->
                button(name, outlined = id != account) { showUnlock(VaultState(false, state.accounts, id, false)) }
            }
        }
        val field = EditText(this).apply {
            hint = getString(R.string.hexis_passkey_master_password)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            imeOptions = EditorInfo.IME_ACTION_DONE
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            setSingleLine()
        }
        content.addView(field, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(14) })
        val error = body(note ?: "", error = true)
        error.visibility = if (note == null) View.GONE else View.VISIBLE
        lateinit var unlockButton: MaterialButton
        val submit = {
            val password = field.text.toString()
            field.setText("") // gone from the field the moment it is sent
            unlockButton.isEnabled = false
            worker.execute {
                val reply = Bridge.call(this, "unlock", JSONObject().put("accountId", account).put("password", password))
                if (reply.ok) {
                    viaPassword = true
                    proceed()
                } else {
                    onMain { showUnlock(state, reply.message) }
                }
            }
        }
        field.setOnEditorActionListener { _, id, _ ->
            if (id == EditorInfo.IME_ACTION_DONE) { submit(); true } else false
        }
        unlockButton = button(getString(R.string.hexis_passkey_unlock_button)) { submit() }
        button(getString(R.string.hexis_passkey_close), outlined = true) { cancel() }
        field.requestFocus()
    }

    /** The vault is open: do what the mode asks. Runs on a worker thread. */
    private fun proceed() {
        when (mode) {
            MODE_UNLOCK -> answerUnlock()
            MODE_GET -> startGet()
            MODE_CREATE -> startCreate()
            else -> fail(getString(R.string.hexis_passkey_unknown_request))
        }
    }

    // ------------------------------------------------------------------ unlock action

    private fun answerUnlock() {
        try {
            val request = PendingIntentHandler.retrieveBeginGetCredentialRequest(intent)
            if (request == null) return fail(getString(R.string.hexis_passkey_unknown_request))
            val response = if (Build.VERSION.SDK_INT >= 34) ProviderEntries.getResponse(this, request) else null
            if (response == null) return fail(getString(R.string.hexis_passkey_unknown_request))
            val result = Intent()
            PendingIntentHandler.setBeginGetCredentialResponse(result, response)
            finished = true
            setResult(RESULT_OK, result)
            finish()
        } catch (e: Exception) {
            fail(getString(R.string.hexis_passkey_unknown_request))
        }
    }

    // ------------------------------------------------------------------ sign in

    private fun startGet() {
        val request = PendingIntentHandler.retrieveProviderGetCredentialRequest(intent)
        val option = request?.credentialOptions?.filterIsInstance<GetPublicKeyCredentialOption>()?.firstOrNull()
        val info = request?.callingAppInfo
        if (request == null || option == null || info == null || Build.VERSION.SDK_INT < 34) return fail(getString(R.string.hexis_passkey_unknown_request))
        val caller = ProviderEntries.caller(this, info, option.clientDataHash) ?: return fail(getString(R.string.hexis_passkey_unknown_caller))
        val itemId = intent.getStringExtra(EXTRA_ITEM_ID) ?: return fail(getString(R.string.hexis_passkey_unknown_request))
        val key = intent.getStringExtra(EXTRA_KEY) ?: return fail(getString(R.string.hexis_passkey_unknown_request))
        val site = intent.getStringExtra(EXTRA_RP) ?: ""
        val user = intent.getStringExtra(EXTRA_USER) ?: ""
        onMain {
            reset()
            title(getString(R.string.hexis_passkey_sign_in_title, site))
            if (user.isNotEmpty()) body(user)
            button(getString(R.string.hexis_passkey_close), outlined = true) { cancel() }
        }
        val answer: (JSONObject) -> Unit = { data ->
            val response = GetCredentialResponse(PublicKeyCredential(data.getString("responseJson")))
            val result = Intent()
            PendingIntentHandler.setGetCredentialResponse(result, response)
            finished = true
            setResult(RESULT_OK, result)
            finish()
        }
        verifyUser(site) { verified ->
            val args = JSONObject().put("request", option.requestJson).put("caller", caller).put("itemId", itemId).put("key", key).put("userVerified", verified)
            if (verified) callWithPassword("sign", args, site, user, answer) else askWithoutLock("sign", args, site, user, answer)
        }
    }

    // ------------------------------------------------------------------ new passkey

    private fun startCreate() {
        val request = PendingIntentHandler.retrieveProviderCreateCredentialRequest(intent)
        val calling = request?.callingRequest as? CreatePublicKeyCredentialRequest
        val info = request?.callingAppInfo
        if (request == null || calling == null || info == null || Build.VERSION.SDK_INT < 34) return fail(getString(R.string.hexis_passkey_unknown_request))
        val caller = ProviderEntries.caller(this, info, calling.clientDataHash) ?: return fail(getString(R.string.hexis_passkey_unknown_caller))
        val options = Bridge.call(this, "createOptions", JSONObject().put("request", calling.requestJson).put("caller", caller))
        if (!options.ok) return fail(options.message)
        onMain { showCreateChoice(calling.requestJson, caller, options.data) }
    }

    private fun showCreateChoice(requestJson: String, caller: JSONObject, o: JSONObject) {
        reset()
        val site = o.optString("rpName").ifEmpty { o.optString("rpId") }
        val user = o.optString("userName").ifEmpty { o.optString("displayName") }
        title(getString(R.string.hexis_passkey_save_title, site))
        if (user.isNotEmpty()) body(getString(R.string.hexis_passkey_save_for, user))
        val suggestions = o.optJSONArray("suggestions") ?: JSONArray()
        if (suggestions.length() > 0) {
            body(getString(R.string.hexis_passkey_save_where))
            for (i in 0 until suggestions.length()) {
                val s = suggestions.getJSONObject(i)
                val sub = s.optString("subtitle")
                val label = if (sub.isNotEmpty()) "${s.getString("title")}\n$sub" else s.getString("title")
                val full = s.optBoolean("full", false)
                button(label, outlined = i > 0) { choose(requestJson, caller, JSONObject().put("existing", s.getString("id")), site, user) }.isEnabled = !full
            }
        }
        button(getString(R.string.hexis_passkey_new_login), outlined = suggestions.length() > 0) { choose(requestJson, caller, JSONObject().put("new", JSONObject().put("title", JSONObject.NULL)), site, user) }
        val others = o.optJSONArray("others") ?: JSONArray()
        if (others.length() > 0) {
            var open = false
            button(getString(R.string.hexis_passkey_other_login), outlined = true) {
                if (open) return@button
                open = true
                for (i in 0 until others.length()) {
                    val l = others.getJSONObject(i)
                    val sub = l.optString("subtitle")
                    val label = if (sub.isNotEmpty()) "${l.getString("title")}\n$sub" else l.getString("title")
                    button(label, outlined = true) { choose(requestJson, caller, JSONObject().put("existing", l.getString("id")), site, user) }
                }
            }
        }
        button(getString(R.string.hexis_passkey_close), outlined = true) { cancel() }
    }

    private fun choose(requestJson: String, caller: JSONObject, target: JSONObject, site: String, user: String) {
        reset()
        title(getString(R.string.hexis_passkey_save_title, site))
        if (user.isNotEmpty()) body(user)
        worker.execute {
            val answer: (JSONObject) -> Unit = { data ->
                val response = CreatePublicKeyCredentialResponse(data.getString("responseJson"))
                val result = Intent()
                PendingIntentHandler.setCreateCredentialResponse(result, response)
                finished = true
                setResult(RESULT_OK, result)
                finish()
            }
            verifyUser(site) { verified ->
                val args = JSONObject().put("request", requestJson).put("caller", caller).put("target", target).put("userVerified", verified)
                if (verified) callWithPassword("create", args, site, user, answer) else askWithoutLock("create", args, site, user, answer)
            }
        }
    }

    // ------------------------------------------------------------------ shared steps

    /**
     * Call the core; when it answers `password_required` (a login that asks for the master password, or any login while the
     * vault was opened with the PIN) ask for the master password and call again with it.
     */
    private fun callWithPassword(op: String, args: JSONObject, site: String, user: String, done: (JSONObject) -> Unit) {
        worker.execute {
            val reply = Bridge.call(this, op, args)
            when {
                reply.ok -> onMain { done(reply.data) }
                reply.code == "password_required" || reply.code == "wrong_password" -> onMain { askPassword(op, args, site, user, if (reply.code == "wrong_password") reply.message else null, done) }
                else -> fail(reply.message)
            }
        }
    }

    /**
     * The phone has no fingerprint and no screen lock, so there is nothing to verify the person with. Fail closed: the master password
     * is asked for this request, exactly as for a login that asks for it, and counts as the verification.
     */
    private fun askWithoutLock(op: String, args: JSONObject, site: String, user: String, done: (JSONObject) -> Unit) {
        onMain { askPassword(op, args, site, user, getString(R.string.hexis_passkey_no_lock), done) }
    }

    private fun askPassword(op: String, args: JSONObject, site: String, user: String, note: String?, done: (JSONObject) -> Unit) {
        reset()
        title(getString(R.string.hexis_passkey_password_title))
        body(getString(R.string.hexis_passkey_password_text))
        val field = EditText(this).apply {
            hint = getString(R.string.hexis_passkey_master_password)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            setSingleLine()
        }
        content.addView(field, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(14) })
        if (note != null) body(note, error = true)
        button(getString(R.string.hexis_passkey_continue)) {
            val password = field.text.toString()
            field.setText("")
            // the password counts as the user's verification for this request, so a request that requires it is not refused for lack of a fingerprint
            args.put("password", password).put("userVerified", true)
            callWithPassword(op, args, site, user, done)
        }
        button(getString(R.string.hexis_passkey_close), outlined = true) { cancel() }
        field.requestFocus()
    }

    /**
     * A fresh fingerprint or screen-lock check for this one request. A master password typed in this sheet counts instead.
     * `then(false)` when the phone has neither; the callers then ask for the master password instead of going on unverified.
     */
    private fun verifyUser(site: String, then: (Boolean) -> Unit) {
        if (viaPassword) return then(true)
        val allowed = BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL
        if (BiometricManager.from(this).canAuthenticate(allowed) != BiometricManager.BIOMETRIC_SUCCESS) return then(false)
        onMain {
            val prompt = BiometricPrompt(this, ContextCompat.getMainExecutor(this), object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    worker.execute { then(true) }
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    cancel()
                }
            })
            prompt.authenticate(
                BiometricPrompt.PromptInfo.Builder()
                    .setTitle(getString(R.string.hexis_passkey_verify_title))
                    .setSubtitle(site)
                    .setAllowedAuthenticators(allowed)
                    .build()
            )
        }
    }
}
